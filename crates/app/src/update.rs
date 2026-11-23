//! App updates: periodic checks against GitHub releases, and a one-click
//! install when the app runs from an unpacked release tarball or an `AppImage`
//! it can replace ([`InstallKind`]).
//!
//! [`UpdateModel`] lives for the whole run (an app-level global, like the cover
//! service), so the state survives signing out and back in. The sidebar shows
//! it and starts installs; the workspace turns its events into notifications.
//!
//! How the app was installed is worked out with the first check, on the
//! background executor: it reads the `AppImage`'s header and probes whether
//! `bin/` and `share/` are writable, which is file I/O the UI thread
//! shouldn't wait on. Nothing needs it sooner: only a release a check found
//! can be installed.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Subscription, Task,
};
use hardcover_core::update::{InstallKind, Release, Updater, Version};

use hardcover_core::Error;

use crate::format::error_message;
use crate::io::{self, Priority};
use crate::preferences::AppPreferences;

/// First automatic check, after startup work has settled.
const FIRST_CHECK: Duration = Duration::from_secs(15);
/// Later automatic checks while the app stays open.
const CHECK_INTERVAL: Duration = Duration::from_hours(12);
/// Set to any value to turn off automatic checks, whatever Settings says.
const NO_CHECK_ENV: &str = "HARDCOVER_NO_UPDATE_CHECK";

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    /// A newer release exists.
    Available(Release),
    Installing(Release),
    /// Installed; the new version runs after a restart.
    Installed(Version),
}

/// Outcomes worth a notification.
#[derive(Debug, Clone)]
pub enum UpdateEvent {
    /// A check the user asked for found nothing newer.
    UpToDate(Version),
    /// A check the user asked for found a release.
    Found,
    /// A check the user asked for failed.
    CheckFailed(SharedString),
    InstallFailed(SharedString),
}

pub struct UpdateModel {
    updater: Updater,
    current: Version,
    /// The running executable, resolved at startup.
    exe: Option<PathBuf>,
    /// How this copy was installed; `None` until the first check works it out.
    install: Option<InstallKind>,
    state: UpdateState,
    check_task: Option<Task<()>>,
    install_task: Option<Task<()>>,
    /// The automatic checks; dropping it stops them.
    schedule_task: Option<Task<()>>,
    _preferences: Subscription,
}

impl EventEmitter<UpdateEvent> for UpdateModel {}

/// The app-wide [`UpdateModel`].
pub struct Updates(pub Entity<UpdateModel>);

impl Global for Updates {}

impl Updates {
    pub fn global(cx: &App) -> &Entity<UpdateModel> {
        &cx.global::<Self>().0
    }
}

/// Creates the model and starts automatic checks if they are on. `exe` is the
/// running executable, resolved at startup before an update could replace it.
/// Needs the preferences global.
pub fn init(exe: Option<PathBuf>, cx: &mut App) {
    let preferences = AppPreferences::global(cx).clone();
    let model = cx.new(|cx| {
        let mut model = UpdateModel {
            updater: Updater::new(),
            current: Version::current(env!("CARGO_PKG_VERSION")),
            exe,
            install: None,
            state: UpdateState::Idle,
            check_task: None,
            install_task: None,
            schedule_task: None,
            _preferences: cx.observe(&preferences, |this: &mut UpdateModel, preferences, cx| {
                let wanted = preferences.read(cx).automatic_update_checks();
                this.set_automatic_checks(wanted, cx);
            }),
        };
        let wanted = preferences.read(cx).automatic_update_checks();
        model.set_automatic_checks(wanted, cx);
        model
    });
    cx.set_global(Updates(model));
}

/// Why automatic checks can't run whatever the preference says, if they can't.
/// "Check for updates" works regardless.
pub fn automatic_checks_unavailable() -> Option<&'static str> {
    unavailable_reason(
        cfg!(debug_assertions),
        std::env::var_os(NO_CHECK_ENV).is_some(),
    )
}

fn unavailable_reason(debug_build: bool, opted_out: bool) -> Option<&'static str> {
    if opted_out {
        Some("Turned off by HARDCOVER_NO_UPDATE_CHECK.")
    } else if debug_build {
        Some("Development builds don't check on their own.")
    } else {
        None
    }
}

impl UpdateModel {
    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn current(&self) -> &Version {
        &self.current
    }

    /// Whether this copy can install updates itself (an unpacked release
    /// tarball in a writable prefix, or an `AppImage` in a writable
    /// directory), rather than only announce them.
    pub fn can_install(&self) -> bool {
        self.install
            .as_ref()
            .is_some_and(InstallKind::can_self_update)
    }

    /// Whether `release` has the file this copy updates from.
    pub fn has_download(&self, release: &Release) -> bool {
        self.install
            .as_ref()
            .is_some_and(|install| install.asset(release).is_some())
    }

    /// Starts or stops automatic checks. They also need
    /// [`automatic_checks_unavailable`] to be `None`.
    fn set_automatic_checks(&mut self, wanted: bool, cx: &mut Context<Self>) {
        let enabled = wanted && automatic_checks_unavailable().is_none();
        if !enabled {
            self.schedule_task = None;
        } else if self.schedule_task.is_none() {
            self.schedule(cx);
        }
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        self.schedule_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIRST_CHECK).await;
            loop {
                if this.update(cx, |this, cx| this.check(false, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(CHECK_INTERVAL).await;
            }
        }));
    }

    /// Looks for a newer release. `manual` checks report every outcome;
    /// automatic ones stay quiet unless they find something.
    pub fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        match self.state {
            // Nothing to gain while busy or after installing.
            UpdateState::Checking | UpdateState::Installing(_) | UpdateState::Installed(_) => {
                return;
            }
            UpdateState::Idle | UpdateState::Available(_) => {}
        }
        let previous = std::mem::replace(&mut self.state, UpdateState::Checking);
        cx.notify();
        let updater = self.updater.clone();
        let current = self.current.clone();
        // Worked out once, with the first check.
        let detect = self.install.is_none().then(|| self.exe.clone());
        let checked = io::run(Priority::Background, move || {
            let install = detect.map(|exe| {
                exe.as_deref()
                    .map_or(InstallKind::Other, InstallKind::detect)
            });
            (install, updater.check(&current))
        });
        self.check_task = Some(cx.spawn(async move |this, cx| {
            let Some((install, result)) = checked.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(install) = install {
                    // An AppImage restarts from its file: the executable
                    // inside it is gone once this process exits.
                    if let Some(path) = install.restart_path() {
                        cx.set_restart_path(path.to_owned());
                    }
                    this.install = Some(install);
                }
                match result {
                    Ok(Some(release)) => {
                        this.state = UpdateState::Available(release);
                        if manual {
                            cx.emit(UpdateEvent::Found);
                        }
                    }
                    Ok(None) => {
                        this.state = UpdateState::Idle;
                        if manual {
                            cx.emit(UpdateEvent::UpToDate(this.current.clone()));
                        }
                    }
                    Err(e) => {
                        // Keep a release found earlier; a failed recheck doesn't unpublish it.
                        this.state = match previous {
                            UpdateState::Available(r) => UpdateState::Available(r),
                            _ => UpdateState::Idle,
                        };
                        if manual {
                            cx.emit(UpdateEvent::CheckFailed(failure_message(&e).into()));
                        } else {
                            eprintln!("hardcover: update check failed: {e}");
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    /// Downloads, verifies and installs the available release.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = &self.state else {
            return;
        };
        let Some(install) = self.install.clone().filter(InstallKind::can_self_update) else {
            return;
        };
        let release = release.clone();
        self.state = UpdateState::Installing(release.clone());
        cx.notify();
        let updater = self.updater.clone();
        let current = self.current.clone();
        let target = release.clone();
        // A download of tens of megabytes: a thread of its own, not one of
        // the I/O threads.
        let installed = io::run_on_own_thread("update-install", move || {
            updater.install(&target, &current, &install)
        });
        self.install_task = Some(cx.spawn(async move |this, cx| {
            let result = installed
                .await
                .unwrap_or_else(|| Err(Error::Storage("the install stopped unexpectedly".into())));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.state = UpdateState::Installed(release.version.clone()),
                    Err(e) => {
                        this.state = UpdateState::Available(release);
                        cx.emit(UpdateEvent::InstallFailed(failure_message(&e).into()));
                    }
                }
                cx.notify();
            });
        }));
    }
}

/// Update failures talk to GitHub, not Hardcover, so they get their own copy.
fn failure_message(error: &Error) -> String {
    match error {
        Error::Network(_) | Error::Timeout => {
            "Couldn't reach GitHub to check for updates. Check your connection and try again."
                .to_owned()
        }
        Error::Update(msg) => format!(
            "Couldn't update: {}. Try again, or download the release from GitHub.",
            msg.trim_end_matches('.')
        ),
        other => error_message(other),
    }
}

/// Release notes for the update dialog: the part before the install
/// instructions (an `<!-- install -->` marker), with GitHub's generated
/// notes tidied for reading in the app.
/// - Each change keeps its title: "by @login in <the pull request's URL>"
///   goes, as do the release's own bump ("Release 0.4.1") and the "Full
///   Changelog" line (the dialog links the release page).
/// - "What's Changed" and the "New Contributors" section go; another
///   heading becomes a bold line, so it doesn't outsize the dialog's title.
///
/// Lines in any other shape are kept as written. Empty when nothing is left.
pub fn release_notes(body: &str) -> String {
    let notes = body.split("<!-- install -->").next().unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut in_contributors = false;
    for line in notes.lines() {
        let trimmed = line.trim();
        if let Some(heading) = heading(trimmed) {
            in_contributors = heading.eq_ignore_ascii_case("New Contributors");
            if !in_contributors && !heading.eq_ignore_ascii_case("What's Changed") {
                lines.push(format!("**{heading}**"));
            }
            continue;
        }
        if in_contributors && !trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("**Full Changelog**") {
            continue;
        }
        match change_title(trimmed) {
            Some(title) if is_release_bump(title) => {}
            Some(title) => lines.push(format!("* {title}")),
            None => lines.push(line.trim_end().to_owned()),
        }
    }
    // Blank lines only between paragraphs, one at a time.
    let mut tidy = String::new();
    let mut blank = false;
    for line in &lines {
        if line.is_empty() {
            blank = !tidy.is_empty();
            continue;
        }
        if blank {
            tidy.push('\n');
            blank = false;
        }
        tidy.push_str(line);
        tidy.push('\n');
    }
    tidy.trim_end().to_owned()
}

/// A Markdown heading's text.
fn heading(line: &str) -> Option<&str> {
    let text = line.trim_start_matches('#');
    let level = line.len() - text.len();
    ((1..=6).contains(&level) && text.starts_with(' ')).then(|| text.trim())
}

/// The title of a change in GitHub's generated notes:
/// `* <title> by @<login> in https://github.com/<owner>/<repo>/pull/<n>`.
fn change_title(line: &str) -> Option<&str> {
    let item = line
        .strip_prefix("* ")
        .or_else(|| line.strip_prefix("- "))?;
    let (title, tail) = item.rsplit_once(" by @")?;
    let (login, url) = tail.split_once(" in ")?;
    let number = url
        .strip_prefix("https://github.com/")?
        .rsplit_once("/pull/")?
        .1;
    let valid = !login.is_empty()
        && !login.contains(char::is_whitespace)
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit());
    (valid && !title.trim().is_empty()).then(|| title.trim())
}

/// The release's own version bump, "Release 0.4.1".
fn is_release_bump(title: &str) -> bool {
    title
        .strip_prefix("Release ")
        .and_then(Version::parse)
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_checks_need_a_release_build_without_the_opt_out() {
        assert_eq!(unavailable_reason(false, false), None);
        assert!(unavailable_reason(true, false).is_some());
        assert!(unavailable_reason(false, true).is_some());
        assert!(
            unavailable_reason(true, true)
                .unwrap()
                .contains(NO_CHECK_ENV)
        );
    }

    #[test]
    fn release_notes_stop_at_the_install_marker() {
        let body = "## What's Changed\n* Fix\n\n<!-- install -->\n## Install\ntar …";
        assert_eq!(release_notes(body), "* Fix");
        assert_eq!(release_notes("Just notes\n"), "Just notes");
        assert_eq!(release_notes(""), "");
    }

    /// GitHub's generated notes, shaped as 0.4.1's.
    #[test]
    fn generated_notes_keep_only_the_changes() {
        let body = "## What's Changed\n\
            * Upgrade the dependencies by @AliSayyah in https://github.com/AliSayyah/hardcover-desktop/pull/90\n\
            * Series: a readable shelf in each series by @AliSayyah in https://github.com/AliSayyah/hardcover-desktop/pull/91\n\
            * Bump ureq by @dependabot[bot] in https://github.com/AliSayyah/hardcover-desktop/pull/94\n\
            * Release 0.4.1 by @AliSayyah in https://github.com/AliSayyah/hardcover-desktop/pull/93\n\
            \n\n\
            ## New Contributors\n\
            * @someone made their first contribution in https://github.com/AliSayyah/hardcover-desktop/pull/95\n\
            \n\
            **Full Changelog**: https://github.com/AliSayyah/hardcover-desktop/compare/v0.4.0...v0.4.1\n\
            \n\n\
            <!-- install -->\n## Install\n";
        assert_eq!(
            release_notes(body),
            "* Upgrade the dependencies\n\
             * Series: a readable shelf in each series\n\
             * Bump ureq"
        );
    }

    #[test]
    fn a_release_with_only_its_bump_has_no_notes() {
        let body = "## What's Changed\n\
            * Release 0.4.2 by @AliSayyah in https://github.com/AliSayyah/hardcover-desktop/pull/96\n\n\
            **Full Changelog**: https://github.com/AliSayyah/hardcover-desktop/compare/v0.4.1...v0.4.2\n";
        assert_eq!(release_notes(body), "");
    }

    #[test]
    fn written_notes_are_kept_with_headings_as_bold_lines() {
        let body = "# Highlights\n\nSeries in by order.\n\n\
            ## Fixes\n- Books by year in the stats\n* Release notes, by hand\n\
            * A title by @someone in https://example.com/pull/3\n";
        assert_eq!(
            release_notes(body),
            "**Highlights**\n\n\
             Series in by order.\n\n\
             **Fixes**\n\
             - Books by year in the stats\n\
             * Release notes, by hand\n\
             * A title by @someone in https://example.com/pull/3"
        );
    }
}
