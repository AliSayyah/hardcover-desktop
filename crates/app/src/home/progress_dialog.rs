//! "Update progress…" on Home: the page (or, for an audiobook, the time) a
//! book being read has reached, in a small dialog.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{Length, LibraryEntry, UserBookId};

use crate::app_state::Services;
use crate::format::{duration_text, parse_progress, position_text, progress_summary};
use crate::library::model::{LibraryModel, WriteKind};
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::open_dialog;

/// Opens the dialog for `entry`, which must be in `model`.
pub fn open_progress(
    model: Entity<LibraryModel>,
    entry: &LibraryEntry,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| ProgressDialog::new(model, entry, window, cx));
    let input = view.read(cx).input.clone();
    open_dialog(window, cx, move |dialog, window, _| {
        let ok_view = view.clone();
        // Enter anywhere in the dialog confirms. The dialog stays open until
        // the write succeeds, and closes itself then.
        dialog
            .title("Update progress")
            .w(rems(24.).to_pixels(window.rem_size()))
            .on_ok(move |_, window, cx| {
                ok_view.update(cx, |this, cx| this.save(window, cx));
                false
            })
            .child(view.clone())
    });
    input.update(cx, |s, cx| s.focus(window, cx));
}

pub struct ProgressDialog {
    model: Entity<LibraryModel>,
    id: UserBookId,
    input: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl ProgressDialog {
    fn new(
        model: Entity<LibraryModel>,
        entry: &LibraryEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let progress = progress_summary(entry);
        let (value, placeholder) = match (progress.page, progress.seconds) {
            _ if progress.is_audio() => (progress.seconds.map(position_text), "h:mm"),
            (Some(page), _) => (Some(page.to_string()), "Page"),
            _ => (None, "Page"),
        };
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(placeholder)
                .validate(|text, _| {
                    text.chars().all(|c| c.is_ascii_digit() || c == ':') && text.len() <= 8
                });
            if let Some(value) = value {
                state.set_value(value, window, cx);
            }
            state
        });
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.error = None;
                    cx.notify();
                }
            }),
        ];
        Self {
            model,
            id: entry.user_book_id,
            input,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !Services::global(cx).has_scope(WRITE_LIBRARY) {
            return;
        }
        let Some(entry) = self.model.read(cx).entry(self.id).cloned() else {
            return;
        };
        let update = match parse_progress(entry.length(), &self.input.read(cx).value()) {
            Ok(update) => update,
            Err(error) => {
                self.error = Some(error.into());
                cx.notify();
                return;
            }
        };
        let id = self.id;
        let model = self.model.clone();
        if !model.update(cx, |m, cx| m.begin_write(id, WriteKind::Progress, cx)) {
            self.error = Some("Another change to this book is still being saved.".into());
            cx.notify();
            return;
        }
        let read = entry.open_read().map(|r| r.id);
        let client = Services::global(cx).client().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.update_progress(id, read, update) })
                .await;
            model.update(cx, |m, cx| {
                m.end_write(id, cx);
                if let Ok(entry) = &result {
                    m.apply_written(entry.clone(), cx);
                }
            });
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(_) => window.close_dialog(cx),
                // The dialog stays open with what was typed, and says why
                // next to it.
                Err(error) => {
                    let message = Services::error_message(&error, Some(WRITE_LIBRARY), cx);
                    this.error = Some(format!("Couldn't update progress. {message}").into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for ProgressDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let saving = model.pending_write(self.id) == Some(WriteKind::Progress);
        let busy = model.pending_write(self.id).is_some();
        let length = model.entry(self.id).and_then(LibraryEntry::length);
        let can_write = Services::global(cx).has_scope(WRITE_LIBRARY);
        let muted = cx.theme().muted_foreground;
        let total = match length {
            Some(Length::Pages(total)) => Some(format!("of {total}")),
            Some(Length::Seconds(total)) => Some(format!("of {}", duration_text(total))),
            None => None,
        };
        let title = model.entry(self.id).map(|e| e.book.title.clone());
        v_flex()
            .gap_3()
            .children(title.map(|t| div().text_sm().truncate().child(t)))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Input::new(&self.input)
                            .w_24()
                            .disabled(busy || !can_write)
                            .raised_field(busy || !can_write, cx),
                    )
                    .when_some(total, |this, total| {
                        this.child(div().text_sm().text_color(muted).child(total))
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("save-progress")
                            .primary()
                            .label("Update")
                            .loading(saving)
                            .disabled(busy || !can_write)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
    }
}
