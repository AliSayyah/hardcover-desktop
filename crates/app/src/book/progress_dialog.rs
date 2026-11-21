//! "Update progress…" on the book pane and page, Home and the Series lane:
//! where the reader is in a book being read, in a small dialog.
//!
//! A segmented control beside the field picks what it counts: the page (or,
//! for an audiobook edition, the time, as "2:05" or "2h 5m") or a
//! percentage. It starts on the book's own unit, or on % when that was the
//! last choice for the book this session. A percentage is converted with the
//! edition's length and written as a page or a time, as the muted text
//! beside the field shows (`progress_input`). Without a known length it
//! isn't offered: the control is left out (the kit paints a disabled
//! segment like the selected one) and the line under the field says why. Switching units
//! converts what the field shows, and saving a converted value unchanged
//! writes the exact place it came from.
//!
//! Progress in time belongs to an audiobook edition's read. A book whose
//! edition counts pages but that has an audiobook offers "Choose an
//! audiobook edition…", which opens the edition picker over the dialog; the
//! edition change moves the read, and the field switches to time when it
//! lands.
//!
//! The dialog writes through `LibraryModel::write`, reporting failures
//! itself: it stays open with what was typed and the reason next to it.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Global, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{Error, LibraryEntry, Measure, ProgressChange, ProgressUpdate, UserBookId};

use super::edition_picker::{EditionChosen, EditionPicker};
use super::progress_input::{Filled, InputUnit, percent_text, position_of, preview};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{needs_permission, progress_summary};
use crate::library::model::{LibraryModel, LibraryWrite, STILL_SAVING, WriteKind, WriteOutcome};
use crate::library::writes;
use crate::session;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::open_dialog;
use crate::ui::kit::small_tab;

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
        // Enter in the field saves; on a button it presses that button. The
        // dialog stays open until the write succeeds, and closes itself then.
        dialog
            .title("Update progress")
            // A unit choice, the field and what it means on one line: a
            // small form's width.
            .w(size::DIALOG_FORM.to_pixels(window.rem_size()))
            .on_ok(move |_, window, cx| {
                ok_view.update(cx, |this, cx| this.save(window, cx));
                false
            })
            .child(view.clone())
    });
    input.update(cx, |s, cx| s.focus(window, cx));
}

/// The books whose dialog was last left on %, for the session.
#[derive(Default)]
struct PercentChosen(HashSet<UserBookId>);

impl Global for PercentChosen {}

pub struct ProgressDialog {
    model: Entity<LibraryModel>,
    id: UserBookId,
    input: Entity<InputState>,
    /// What the field counts; its typing filter reads it too.
    unit: Rc<Cell<InputUnit>>,
    /// What the book was counted in when the field was filled. When the
    /// entry's changes (a new edition), the field is filled again.
    measure: Option<Measure>,
    /// Text the dialog put in the field, with the exact place it stands for:
    /// saving it unchanged writes that place, not a rounded percentage.
    filled: Option<Filled>,
    error: Option<SharedString>,
    /// The edition picker "Choose an audiobook edition…" opens.
    picker: Option<(Entity<EditionPicker>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl ProgressDialog {
    fn new(
        model: Entity<LibraryModel>,
        entry: &LibraryEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let measure = entry.measure();
        let native = InputUnit::native(measure);
        let percent_chosen = cx
            .try_global::<PercentChosen>()
            .is_some_and(|p| p.0.contains(&entry.user_book_id));
        let unit = if percent_chosen && has_length(measure) {
            InputUnit::Percent
        } else {
            native
        };
        let unit = Rc::new(Cell::new(unit));
        let input = cx.new(|cx| {
            let typing = unit.clone();
            InputState::new(window, cx)
                .placeholder(unit.get().placeholder())
                .validate(move |text, _| typing.get().accepts_typing(text))
        });
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, model, window, cx| {
                // A new edition changes the unit or the length, and the
                // place: the edition change moves the read.
                if let Some(entry) = model.read(cx).entry(this.id).cloned() {
                    let measure = entry.measure();
                    if measure != this.measure {
                        this.measure = measure;
                        if this.unit.get() != InputUnit::Percent || !has_length(measure) {
                            this.unit.set(InputUnit::native(measure));
                        }
                        this.fill(&entry, window, cx);
                    }
                }
                cx.notify();
            }),
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.error = None;
                    cx.notify();
                }
            }),
        ];
        let mut this = Self {
            model,
            id: entry.user_book_id,
            input,
            unit,
            measure,
            filled: None,
            error: None,
            picker: None,
            _subscriptions: subscriptions,
        };
        this.fill(entry, window, cx);
        this
    }

    /// Puts the read's current place in the field, in the field's unit.
    fn fill(&mut self, entry: &LibraryEntry, window: &mut Window, cx: &mut Context<Self>) {
        let unit = self.unit.get();
        let progress = progress_summary(entry);
        let position = match (progress.page, progress.seconds) {
            (Some(page), _) => Some(ProgressUpdate::Pages(page)),
            (None, Some(seconds)) => Some(ProgressUpdate::Seconds(seconds)),
            (None, None) => None,
        };
        let filled = position.and_then(|p| Filled::new(p, unit, entry.length()));
        let text = match &filled {
            Some(filled) => Some(filled.text.clone()),
            // A read with only Hardcover's percentage has no exact place.
            None if unit == InputUnit::Percent && position.is_none() => {
                progress.percent.map(|p| percent_text(p.into()))
            }
            None => None,
        };
        self.set_field(unit, text, filled, window, cx);
    }

    fn set_field(
        &mut self,
        unit: InputUnit,
        text: Option<String>,
        filled: Option<Filled>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: SharedString = text.unwrap_or_default().into();
        self.filled = filled;
        self.error = None;
        self.input.update(cx, |s, cx| {
            s.set_placeholder(unit.placeholder(), window, cx);
            s.set_value(text, window, cx);
        });
        cx.notify();
    }

    /// The place the field stands for (`progress_input::position_of`).
    fn position(&self, measure: Option<Measure>, cx: &App) -> Result<ProgressUpdate, String> {
        let text = self.input.read(cx).value();
        position_of(self.unit.get(), measure, &text, self.filled.as_ref())
    }

    /// Switches the field to `unit`, showing the same place in it.
    fn set_unit(&mut self, unit: InputUnit, window: &mut Window, cx: &mut Context<Self>) {
        if unit != self.unit.get()
            && let Some(entry) = self.model.read(cx).entry(self.id).cloned()
        {
            let measure = entry.measure();
            let position = self.position(measure, cx).ok();
            self.unit.set(unit);
            let chosen = &mut cx.default_global::<PercentChosen>().0;
            if unit == InputUnit::Percent {
                chosen.insert(self.id);
            } else {
                chosen.remove(&self.id);
            }
            let filled = position.and_then(|p| Filled::new(p, unit, entry.length()));
            let text = filled.as_ref().map(|f| f.text.clone());
            self.set_field(unit, text, filled, window, cx);
        }
        // Back to the field, so Enter saves.
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }

    fn choose_audiobook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.model.read(cx).entry(self.id).cloned() else {
            return;
        };
        let picker = if let Some((picker, _)) = &self.picker {
            picker.clone()
        } else {
            let title: SharedString = entry.book.title.clone().into();
            let picker = cx.new(|cx| EditionPicker::new(entry.book.id, title, cx));
            let subscription = cx.subscribe_in(
                &picker,
                window,
                |this, _, EditionChosen(edition), window, cx| {
                    let current = this
                        .model
                        .read(cx)
                        .entry(this.id)
                        .and_then(|e| e.edition.as_ref().map(|e| e.id));
                    if current != Some(edition.id) {
                        writes::set_edition(&this.model, this.id, edition.id, window, cx);
                    }
                },
            );
            self.picker = Some((picker.clone(), subscription));
            picker
        };
        let current = entry.edition.as_ref().map(|e| e.id);
        picker.update(cx, |p, cx| p.set_current(current, cx));
        EditionPicker::open(picker, window, cx);
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !session::has_scope(cx, WRITE_LIBRARY) {
            return;
        }
        let Some(entry) = self.model.read(cx).entry(self.id).cloned() else {
            return;
        };
        let update = match self.position(entry.measure(), cx) {
            Ok(update) => update,
            Err(error) => {
                self.error = Some(error.into());
                cx.notify();
                return;
            }
        };
        let id = self.id;
        let read = entry.open_read().map(|r| r.id);
        let write = LibraryWrite::entry(
            id,
            WriteKind::Progress,
            "Couldn't update progress",
            move |client| client.update_progress(id, read, update),
        )
        .reported_by_caller();
        let outcome = self.model.update(cx, |m, cx| m.write(write, window, cx));
        cx.spawn_in(window, async move |this, cx| {
            let outcome = outcome.await;
            let _ = this.update_in(cx, |this, window, cx| {
                // The dialog stays open with what was typed, and says why
                // next to it. Saving again goes on the read the move started.
                this.error = match outcome {
                    WriteOutcome::Saved(ProgressChange {
                        progress_left_out: None,
                        ..
                    }) => {
                        window.close_dialog(cx);
                        return;
                    }
                    WriteOutcome::Saved(ProgressChange {
                        progress_left_out: Some(Error::OutcomeUnknown),
                        ..
                    }) => Some(
                        "Moved to Currently reading. Hardcover didn't confirm the progress, so it may have been saved. Refresh to check before trying again."
                            .into(),
                    ),
                    WriteOutcome::Saved(ProgressChange {
                        progress_left_out: Some(error),
                        ..
                    }) => Some(
                        format!(
                            "Moved to Currently reading, but the progress wasn't saved. {}",
                            session::error_message(&error, Some(WRITE_LIBRARY), cx)
                        )
                        .into(),
                    ),
                    // Not a failure: the message says it may have been saved.
                    WriteOutcome::Failed {
                        error: Error::OutcomeUnknown,
                        message,
                    } => Some(message),
                    WriteOutcome::Failed { message, .. } => {
                        Some(format!("Couldn't update progress. {message}").into())
                    }
                    WriteOutcome::Busy => Some(STILL_SAVING.into()),
                    WriteOutcome::InLibrary | WriteOutcome::Ended => return,
                };
                cx.notify();
            });
        })
        .detach();
    }
}

/// Whether a percentage can be converted: the length is known.
fn has_length(measure: Option<Measure>) -> bool {
    measure.and_then(Measure::length).is_some()
}

/// Why % is off for a book counted in `native`.
fn percent_unavailable(native: InputUnit) -> &'static str {
    match native {
        InputUnit::Time => "Percent needs the audiobook's length, which this edition doesn't list.",
        InputUnit::Page | InputUnit::Percent => {
            "Percent needs the page count, which this edition doesn't list."
        }
    }
}

impl Render for ProgressDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let saving = model.pending_write(self.id) == Some(WriteKind::Progress);
        let busy = model.pending_write(self.id).is_some();
        let entry = model.entry(self.id);
        let measure = entry.and_then(LibraryEntry::measure);
        let title = entry.map(|e| e.book.title.clone());
        let offer_audiobook =
            entry.is_some_and(|e| !e.is_audio() && e.book_audio_seconds.is_some());
        let can_write = session::has_scope(cx, WRITE_LIBRARY);
        let percent = has_length(measure);
        let muted = colors(cx).fg_muted;
        let unit = self.unit.get();
        let native = InputUnit::native(self.measure);
        let units = [native, InputUnit::Percent];
        let text = self.input.read(cx).value();
        let exact = self.filled.as_ref().and_then(|f| f.exact(&text));
        let meaning = preview(unit, measure, &text, exact);
        let view = cx.entity().downgrade();
        v_flex()
            .gap_3()
            .children(title.map(|t| div().body_strong().truncate().child(t)))
            .child(
                h_flex()
                    .gap_2()
                    // Without a length there is nothing to choose: the
                    // kit paints a disabled segment like the selected one,
                    // so % is left out and the line below says why.
                    .when(percent, |this| {
                        this.child(
                            TabBar::new("progress-unit")
                                .segmented()
                                .small_control()
                                .selected_index(usize::from(unit == InputUnit::Percent))
                                .child(small_tab(native.label()))
                                .child(small_tab(InputUnit::Percent.label()).aria_label("Percent"))
                                .on_click(move |ix, window, cx| {
                                    let unit = units[*ix];
                                    let _ =
                                        view.update(cx, |this, cx| this.set_unit(unit, window, cx));
                                }),
                        )
                    })
                    .child(
                        Input::new(&self.input)
                            .w(size::FIELD_NUMBER)
                            .disabled(busy || !can_write)
                            .raised_field(busy || !can_write, cx),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .body()
                            .tabular()
                            .text_color(muted)
                            .children(meaning),
                    ),
            )
            .when(!percent, |this| {
                this.child(div().caption(cx).child(percent_unavailable(native)))
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(div().body().text_color(colors(cx).danger).child(error))
            })
            .when(offer_audiobook && can_write, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .child(div().body().text_color(muted).child("Listening instead?"))
                        .child(
                            Button::new("choose-audiobook")
                                .small_control()
                                .label("Choose an audiobook edition…")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_audiobook(window, cx);
                                })),
                        ),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-progress")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("save-progress")
                            .primary()
                            .label("Update")
                            .loading(saving)
                            .disabled(busy || !can_write)
                            .when(!can_write, |this| {
                                this.tooltip(needs_permission(WRITE_LIBRARY))
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_says_why_it_is_off() {
        assert!(percent_unavailable(InputUnit::Time).contains("audiobook"));
        assert!(percent_unavailable(InputUnit::Page).contains("page count"));
    }
}
