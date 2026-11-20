//! The "Export library" dialog: pick a format, fetch what the sync leaves out
//! (reviews, the lists' books, goals), choose where to save, and write the
//! file. The formats themselves are `hardcover_core::export`.
//!
//! Closing the dialog cancels a running export: nothing is saved, and no save
//! dialog or notification appears afterwards.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::{READ_GOALS, READ_LIBRARY, READ_LISTS};
use hardcover_core::export::{self, Backup, Format, Parts, Progress};
use hardcover_core::{Error, LibraryEntry, User};

use crate::app_state::Services;
use crate::format::needs_permission;
use crate::library::model::{LibraryModel, SyncStatus};

/// Opens the dialog over the workspace.
pub fn open_export(library: Entity<LibraryModel>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| ExportDialog::new(library, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        let ok_view = view.clone();
        // Enter anywhere in the dialog exports (without this, the dialog's
        // own Enter binding would just close it). It closes itself once the
        // file is saved.
        dialog
            .title("Export library")
            .w(rems(30.).to_pixels(window.rem_size()))
            .on_ok(move |_, window, cx| {
                ok_view.update(cx, |this, cx| this.start(window, cx));
                false
            })
            .child(view.clone())
    });
}

/// Where a running export is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Fetching(Progress),
    /// The save dialog is open.
    Choosing,
    Saving,
}

struct ExportDialog {
    library: Entity<LibraryModel>,
    format: Format,
    /// `Some` while an export runs.
    stage: Option<Stage>,
    error: Option<SharedString>,
    /// Set to stop the running export's requests.
    cancel: Arc<AtomicBool>,
    _task: Option<Task<()>>,
    _subscription: Subscription,
}

impl Drop for ExportDialog {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl ExportDialog {
    fn new(library: Entity<LibraryModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&library, |_, _, cx| cx.notify());
        Self {
            library,
            format: Format::GoodreadsCsv,
            stage: None,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
            _task: None,
            _subscription: subscription,
        }
    }

    /// The library is whole: loaded, and not being replaced by a sync (a
    /// first sync shows its pages as they arrive).
    fn library_ready(&self, cx: &App) -> bool {
        let library = self.library.read(cx);
        library.is_loaded() && *library.sync_status() != SyncStatus::Syncing
    }

    fn set_stage(&mut self, stage: Option<Stage>, cx: &mut Context<Self>) {
        self.stage = stage;
        cx.notify();
    }

    fn fail(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        self.set_stage(None, cx);
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stage.is_some() || !self.library_ready(cx) {
            return;
        }
        let Some(services) = cx.try_global::<Services>() else {
            return;
        };
        let client = services.client().clone();
        let parts = Parts {
            lists: services.has_scope(READ_LISTS),
            goals: services.has_scope(READ_GOALS),
        };
        let library = self.library.read(cx);
        let user: Option<User> = library.user().cloned();
        let entries: Vec<LibraryEntry> = library.entries().to_vec();
        let format = self.format;
        let name = format.file_name(chrono::Local::now().date_naive());
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        self.error = None;
        self.set_stage(Some(Stage::Fetching(Progress::Reviews(0))), cx);

        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let (progress_tx, progress_rx) = async_channel::unbounded::<Progress>();
            let fetch = cx.background_spawn(async move {
                let backup = Backup::new(chrono::Utc::now(), user, entries);
                let backup = export::gather(&client, backup, parts, &cancel, |p| {
                    let _ = progress_tx.send_blocking(p);
                })?;
                Ok::<_, Error>((backup, export::default_directory()))
            });
            // The sender goes when the fetch ends, which ends this loop.
            while let Ok(progress) = progress_rx.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.set_stage(Some(Stage::Fetching(progress)), cx)
                    })
                    .is_err()
                {
                    return;
                }
            }
            let (backup, directory) = match fetch.await {
                Ok(fetched) => fetched,
                Err(Error::Cancelled) => return,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        let scope = this.stage.and_then(stage_scope);
                        let message = Services::error_message(&e, scope, cx);
                        this.fail(format!("Couldn't export the library: {message}"), cx);
                    });
                    return;
                }
            };

            // Signed out meanwhile: drop it.
            let Ok(Some(prompt)) = this.update(cx, |this, cx| {
                cx.try_global::<Services>()?;
                this.set_stage(Some(Stage::Choosing), cx);
                Some(cx.prompt_for_new_path(&directory, Some(&name)))
            }) else {
                return;
            };
            let path = match prompt.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) | Err(_) => {
                    let _ = this.update(cx, |this, cx| this.set_stage(None, cx));
                    return;
                }
                Ok(Err(e)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.fail(format!("Couldn't open the save dialog: {e}"), cx)
                    });
                    return;
                }
            };

            if this
                .update(cx, |this, cx| this.set_stage(Some(Stage::Saving), cx))
                .is_err()
            {
                return;
            }
            let saved = cx
                .background_spawn(
                    async move { export::save(format, &backup, &path).map(|()| path) },
                )
                .await;
            let _ = this.update_in(cx, |this, window, cx| match saved {
                Ok(path) => {
                    this.set_stage(None, cx);
                    window.close_dialog(cx);
                    window.push_notification(saved_notification(path), cx);
                }
                Err(e) => this.fail(format!("Couldn't save the file: {e}"), cx),
            });
        }));
    }
}

/// "Library exported", where it went, and a button that shows the file in
/// the file manager.
fn saved_notification(path: PathBuf) -> Notification {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let folder = path.parent().map(display_path).unwrap_or_default();
    Notification::success(format!("{name} in {folder}"))
        .title("Library exported")
        .action(move |_, _, _| {
            let path = path.clone();
            Button::new("show-export")
                .label("Show in folder")
                .on_click(move |_, _, cx| cx.reveal_path(&path))
        })
}

/// The path with the home folder written `~`.
fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The scope a request at `stage` needs: what an `insufficient_scope` that
/// names none was about.
fn stage_scope(stage: Stage) -> Option<&'static str> {
    match stage {
        Stage::Fetching(Progress::Reviews(_)) => Some(READ_LIBRARY),
        Stage::Fetching(Progress::Lists { .. }) => Some(READ_LISTS),
        Stage::Fetching(Progress::Goals) => Some(READ_GOALS),
        Stage::Choosing | Stage::Saving => None,
    }
}

fn progress_text(stage: Stage) -> String {
    match stage {
        Stage::Fetching(Progress::Reviews(0)) => "Fetching reviews…".into(),
        Stage::Fetching(Progress::Reviews(n)) => format!("Fetching reviews… {n}"),
        Stage::Fetching(Progress::Lists { done, total }) => {
            format!("Fetching lists… {done} of {total}")
        }
        Stage::Fetching(Progress::Goals) => "Fetching goals…".into(),
        Stage::Choosing => "Choose where to save the file.".into(),
        Stage::Saving => "Saving…".into(),
    }
}

/// Muted help text.
fn help(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// Help under a radio's label. The label column is as wide as the whole
/// radio, so it runs past the end by the indicator (1rem) and its gap
/// (0.5rem); the padding keeps the text inside.
fn radio_help(text: &'static str, cx: &App) -> gpui_kit::Div {
    help(text, cx).pr(rems(1.5))
}

impl Render for ExportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.stage.is_some();
        let ready = self.library_ready(cx);
        let services = Services::global(cx);
        let lists = services.has_scope(READ_LISTS);
        let goals = services.has_scope(READ_GOALS);
        let muted = cx.theme().muted_foreground;

        let view = cx.entity().downgrade();
        let selected = Format::ALL.iter().position(|f| *f == self.format);
        let formats = RadioGroup::vertical("export-format")
            .child(
                Radio::new("export-csv")
                    .label("Goodreads CSV")
                    .child(radio_help(
                        "For StoryGraph, BookWyrm and other services that import a Goodreads \
                         export. Book Id is the Hardcover book id. Ratings are rounded to whole \
                         stars, halves up. Ignored books are left out.",
                        cx,
                    )),
            )
            .child(
                Radio::new("export-json")
                    .label("Full backup (JSON)")
                    .child(radio_help(
                        "Everything the app knows: statuses, ratings, reads, notes, reviews, lists \
                     and goals.",
                        cx,
                    )),
            )
            .selected_index(selected)
            .disabled(running)
            .on_change(move |ix, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    if let Some(format) = Format::ALL.get(*ix) {
                        this.format = *format;
                        cx.notify();
                    }
                });
            });

        // What the session can't read, as it affects the chosen format.
        let mut notes: Vec<String> = Vec::new();
        match self.format {
            Format::GoodreadsCsv if !lists => notes.push(format!(
                "Lists are left out: Bookshelves holds only the status, and Owned Copies is 0. {}",
                needs_permission(READ_LISTS)
            )),
            Format::Json => {
                if !lists {
                    notes.push(format!(
                        "Lists are left out. {}",
                        needs_permission(READ_LISTS)
                    ));
                }
                if !goals {
                    notes.push(format!(
                        "Goals are left out. {}",
                        needs_permission(READ_GOALS)
                    ));
                }
            }
            Format::GoodreadsCsv => {}
        }

        v_flex()
            .gap_4()
            .child(formats)
            .children(notes.into_iter().map(|note| help(note, cx)))
            .when(!ready && !running, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("The library is still syncing. Export it once the sync is done."),
                )
            })
            .when_some(self.stage, |this, stage| {
                this.child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .text_color(muted)
                        .child(Spinner::new().small())
                        .child(progress_text(stage)),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-export")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("start-export")
                            .primary()
                            .label("Export…")
                            .loading(running)
                            .disabled(running || !ready)
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reads_as_it_goes() {
        let text = |p| progress_text(Stage::Fetching(p));
        assert_eq!(text(Progress::Reviews(0)), "Fetching reviews…");
        assert_eq!(text(Progress::Reviews(120)), "Fetching reviews… 120");
        assert_eq!(
            text(Progress::Lists { done: 2, total: 7 }),
            "Fetching lists… 2 of 7"
        );
        assert_eq!(text(Progress::Goals), "Fetching goals…");
        assert_eq!(
            stage_scope(Stage::Fetching(Progress::Lists { done: 0, total: 1 })),
            Some(READ_LISTS)
        );
        assert_eq!(stage_scope(Stage::Saving), None);
    }

    #[test]
    fn paths_under_home_start_with_a_tilde() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        assert_eq!(display_path(&home.join("Documents")), "~/Documents");
        assert_eq!(display_path(&home), "~");
        assert_eq!(display_path(Path::new("/srv/x.csv")), "/srv/x.csv");
    }
}
