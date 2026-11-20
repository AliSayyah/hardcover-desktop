//! App updates: periodic checks against GitHub releases, and a one-click
//! install when the app runs from an unpacked release tarball or an AppImage
//! it can replace ([`InstallKind`]).
//!
//! [`UpdateModel`] lives for the whole run (an app-level global, like the cover
//! service), so the state survives signing out and back in. The sidebar shows
//! it and starts installs; the workspace turns its events into notifications.

use std::time::Duration;

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Subscription, Task,
};
use hardcover_core::update::{InstallKind, Release, Updater, Version};

use hardcover_core::Error;

use crate::format::error_message;
use crate::preferences::AppPreferences;

/// First automatic check, after startup work has settled.
const FIRST_CHECK: Duration = Duration::from_secs(15);
/// Later automatic checks while the app stays open.
const CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
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
    install: InstallKind,
    state: UpdateState,
    _check: Option<Task<()>>,
    _install: Option<Task<()>>,
    /// The automatic checks; dropping it stops them.
    _schedule: Option<Task<()>>,
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

/// Creates the model and starts automatic checks if they are on. `install`
/// is detected once at startup, before an update could replace anything.
/// Needs the preferences global.
pub fn init(install: InstallKind, cx: &mut App) {
    let preferences = AppPreferences::global(cx).clone();
    let model = cx.new(|cx| {
        let mut model = UpdateModel {
            updater: Updater::new(),
            current: Version::current(env!("CARGO_PKG_VERSION")),
            install,
            state: UpdateState::Idle,
            _check: None,
            _install: None,
            _schedule: None,
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
    /// tarball in a writable prefix, or an AppImage in a writable
    /// directory), rather than only announce them.
    pub fn can_install(&self) -> bool {
        self.install.can_self_update()
    }

    /// Whether `release` has the file this copy updates from.
    pub fn has_download(&self, release: &Release) -> bool {
        self.install.asset(release).is_some()
    }

    /// Starts or stops automatic checks. They also need
    /// [`automatic_checks_unavailable`] to be `None`.
    fn set_automatic_checks(&mut self, wanted: bool, cx: &mut Context<Self>) {
        let enabled = wanted && automatic_checks_unavailable().is_none();
        if !enabled {
            self._schedule = None;
        } else if self._schedule.is_none() {
            self.schedule(cx);
        }
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        self._schedule = Some(cx.spawn(async move |this, cx| {
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
        self._check = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { updater.check(&current) })
                .await;
            let _ = this.update(cx, |this, cx| {
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
        if !self.install.can_self_update() {
            return;
        }
        let release = release.clone();
        let install = self.install.clone();
        self.state = UpdateState::Installing(release.clone());
        cx.notify();
        let updater = self.updater.clone();
        let current = self.current.clone();
        self._install = Some(cx.spawn(async move |this, cx| {
            let target = release.clone();
            let result = cx
                .background_spawn(async move { updater.install(&target, &current, &install) })
                .await;
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

/// Release notes without the install instructions meant for the release page,
/// which start at an `<!-- install -->` marker.
pub fn release_notes(body: &str) -> String {
    body.split("<!-- install -->")
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
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
        assert_eq!(release_notes(body), "## What's Changed\n* Fix");
        assert_eq!(release_notes("Just notes\n"), "Just notes");
        assert_eq!(release_notes(""), "");
    }
}
