//! Notes (`hardcover-design`, Shelf → Book pane, and Full book page): the
//! entry's private notes with "Edit…", or "No notes" with "Add a note…"; the
//! editor in place once opened, or while it holds a draft. A block in the
//! pane, a rail section on the page.
//!
//! Its own view, [`Notes`]: it keeps the field, the drafts set aside for
//! other entries and whether the editor is open. Closing the editor (Save,
//! Ctrl+Enter, Discard or Cancel) gives focus back to the button that opens
//! it again, when focus was in the editor.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::UserBookId;
use hardcover_core::auth::scopes::WRITE_LIBRARY;

use super::BookDetailView;
use super::your_book::{paragraphs, saved_note};
use crate::bidi::WrappingText;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::needs_permission;
use crate::library::model::{LibraryModel, LibraryWrite, WriteKind, WriteOutcome};
use crate::session;
use crate::ui::kit::{KeyboardRing as _, RaisedField as _, SmallControl as _};
use crate::ui::{Block, RailSection};

/// The notes field's text compared with the entry's saved notes.
fn notes_differ(text: &str, saved: Option<&str>) -> bool {
    text.trim() != saved.unwrap_or("").trim()
}

/// The Notes block of the entry shown. See the module doc.
pub(super) struct Notes {
    model: Entity<LibraryModel>,
    /// The entry shown, if the book is in the library.
    entry: Option<UserBookId>,
    /// A rail section (the page, the wide pane) rather than a block: set by
    /// the book view as it lays its blocks out, just before this renders.
    in_rail: Rc<Cell<bool>>,
    field: Entity<TextareaState>,
    /// The entry and saved notes the field was last filled from.
    base: Option<(UserBookId, Option<String>)>,
    /// Unsaved notes of entries no longer shown, restored when they are.
    drafts: HashMap<UserBookId, String>,
    /// The entry whose notes were just saved, until the field changes again.
    saved: Option<UserBookId>,
    /// The entry whose editor was opened ("Add a note…", "Edit…"). An
    /// unsaved draft keeps it open too.
    editing: Option<UserBookId>,
    /// "Edit…" or "Add a note…", whichever shows: closing the editor gives
    /// focus back to it.
    opener: FocusHandle,
    /// The editor, to tell whether focus is in it when it closes.
    editor: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Notes {
    pub(super) fn new(
        model: Entity<LibraryModel>,
        in_rail: Rc<Cell<bool>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Only you can see these notes")
                .auto_grow(3, 10)
        });
        let subscriptions =
            vec![
                cx.observe_in(&model, window, |this, _, window, cx| {
                    this.sync(window, cx);
                    cx.notify();
                }),
                cx.subscribe_in(&field, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter {
                            secondary: true, ..
                        } => this.save(window, cx),
                        InputEvent::Change => {
                            this.saved = None;
                            cx.notify();
                        }
                        _ => {}
                    }
                }),
                // Writing needs `write:library`.
                session::observe_grant(cx, |_, cx| cx.notify()),
            ];
        Self {
            model,
            entry: None,
            in_rail,
            field,
            base: None,
            drafts: HashMap::new(),
            saved: None,
            editing: None,
            opener: cx.focus_handle(),
            editor: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `entry`'s notes, setting aside a draft for the entry shown.
    pub(super) fn set_entry(
        &mut self,
        entry: Option<UserBookId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.entry == entry {
            return;
        }
        self.entry = entry;
        self.sync(window, cx);
        cx.notify();
    }

    /// Fills the field from the entry's saved notes unless it holds an
    /// unsaved draft for that entry. A draft for another entry is set aside
    /// and restored when that entry is shown again.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.entry.and_then(|id| {
            let entry = self.model.read(cx).entry(id)?;
            Some((id, entry.private_notes.clone()))
        });
        if self.base == next {
            return;
        }
        let text = self.field.read(cx).value().to_string();
        let same_entry = self.base.as_ref().map(|(id, _)| *id) == next.as_ref().map(|(id, _)| *id);
        if let Some((id, saved)) = &self.base
            && notes_differ(&text, saved.as_deref())
        {
            if same_entry {
                // The saved notes changed under a draft (or a save made them
                // equal): keep the text, and compare it with the new value.
                self.base = next;
                return;
            }
            self.drafts.insert(*id, text);
        }
        let Some((id, saved)) = next.clone() else {
            self.base = None;
            return;
        };
        let value = self
            .drafts
            .remove(&id)
            .unwrap_or_else(|| saved.unwrap_or_default());
        if *self.field.read(cx).value() != *value {
            self.field
                .update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.base = next;
        self.saved = None;
    }

    /// Unsaved notes in the field for the entry shown.
    fn dirty(&self, cx: &App) -> bool {
        self.base
            .as_ref()
            .is_some_and(|(_, saved)| notes_differ(&self.field.read(cx).value(), saved.as_deref()))
    }

    /// Words not saved: in the field, or set aside for another entry.
    pub(super) fn has_unsaved(&self, cx: &App) -> bool {
        self.dirty(cx) || !self.drafts.is_empty()
    }

    /// Whether the editor shows for `id`: opened, or holding a draft.
    fn open(&self, id: UserBookId, cx: &App) -> bool {
        self.editing == Some(id) || self.dirty(cx)
    }

    /// Opens the editor with the cursor at the end of the notes, ready to
    /// add a line.
    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.entry else {
            return;
        };
        self.editing = Some(id);
        self.saved = None;
        self.field.update(cx, |s, cx| {
            let end = s.value().len();
            s.set_selected_range(end..end, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Closes the editor; focus goes back to the button that opens it, if
    /// it was in the editor (or nowhere, its button gone with it).
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        if window.focused(cx).is_none() || self.editor.contains_focused(window, cx) {
            window.focus(&self.opener, cx);
        }
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.entry else {
            return;
        };
        if !self.dirty(cx) {
            return;
        }
        let text = self.field.read(cx).value().to_string();
        let write = LibraryWrite::entry(
            id,
            WriteKind::Notes,
            "Couldn't save your notes",
            move |client| client.set_private_notes(id, &text),
        );
        let outcome = self.model.update(cx, |m, cx| m.write(write, window, cx));
        cx.spawn_in(window, async move |this, cx| {
            if let WriteOutcome::Saved(_) = outcome.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.saved = Some(id);
                    if this.editing == Some(id) {
                        this.close(window, cx);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Puts the saved notes back in the field and closes the editor.
    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = self
            .base
            .as_ref()
            .and_then(|(_, saved)| saved.clone())
            .unwrap_or_default();
        self.field
            .update(cx, |s, cx| s.set_value(saved, window, cx));
        self.close(window, cx);
    }

    fn render_editor(&self, id: UserBookId, cx: &mut Context<Self>) -> AnyElement {
        let pending = self.model.read(cx).pending_write(id);
        let saving = pending == Some(WriteKind::Notes);
        let dirty = self.dirty(cx);
        let can_write = BookDetailView::can_write(cx);
        v_flex()
            .track_focus(&self.editor)
            .w_full()
            .gap_2()
            .child(
                Textarea::new(&self.field)
                    .disabled(saving)
                    .raised_field(saving, cx),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("save-notes")
                            .primary()
                            .small_control()
                            .label("Save")
                            .tooltip(if can_write {
                                "Ctrl+Enter".to_owned()
                            } else {
                                needs_permission(WRITE_LIBRARY)
                            })
                            .loading(saving)
                            .disabled(!dirty || pending.is_some() || !can_write)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        Button::new("discard-notes")
                            .ghost()
                            .small_control()
                            .label(if dirty { "Discard" } else { "Cancel" })
                            .disabled(saving)
                            .on_click(cx.listener(|this, _, window, cx| this.discard(window, cx))),
                    )
                    .when(dirty && !saving, |this| {
                        this.child(div().caption(cx).child("Not saved"))
                    }),
            )
            .into_any_element()
    }
}

impl Render for Notes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(id) = self.entry else {
            return div().into_any_element();
        };
        let (body, action): (AnyElement, Option<AnyElement>) = if self.open(id, cx) {
            (self.render_editor(id, cx), None)
        } else {
            let pending = self.model.read(cx).pending_write(id).is_some();
            let can_write = BookDetailView::can_write(cx);
            let opener = self.opener.clone();
            let edit = |id: &'static str, label: &'static str, window: &Window| {
                Button::new(id)
                    .small_control()
                    .label(label)
                    .keyboard_ring(&opener, window)
                    .disabled(pending || !can_write)
                    .when(!can_write, |this| {
                        this.tooltip(needs_permission(WRITE_LIBRARY))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx)))
            };
            let saved_text = self
                .model
                .read(cx)
                .entry(id)
                .and_then(|e| e.private_notes.as_deref())
                .map(str::trim)
                .filter(|n| !n.is_empty());
            match saved_text {
                Some(text) => (
                    v_flex()
                        .gap_2()
                        .max_w(size::PROSE)
                        .reader()
                        .children(paragraphs(text).map(WrappingText::new))
                        .when(self.saved == Some(id), |this| this.child(saved_note(cx)))
                        .into_any_element(),
                    Some(
                        edit("edit-notes", "Edit…", window)
                            .ghost()
                            .into_any_element(),
                    ),
                ),
                None => (
                    v_flex()
                        .items_start()
                        .gap_3()
                        .child(
                            div()
                                .body()
                                .text_color(colors(cx).fg_muted)
                                .child("No notes"),
                        )
                        .child(edit("add-notes", "Add a note…", window))
                        .into_any_element(),
                    None,
                ),
            }
        };
        if self.in_rail.get() {
            RailSection::new("Notes")
                .when_some(action, RailSection::action)
                .child(body)
                .into_any_element()
        } else {
            Block::new("Notes")
                .when_some(action, Block::action)
                .child(body)
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Focusable as _, TestAppContext, VisualTestContext};

    use super::super::tests::{rooted_view, show_entry};
    use super::*;
    use crate::ui::test_support::{press, redraw};

    fn focused(handle: &FocusHandle, cx: &mut VisualTestContext) -> bool {
        cx.update(|window, _| handle.is_focused(window))
    }

    /// C3: closing the editor, by Ctrl+Enter or Discard, gives focus back to
    /// the button that opens it, not to nothing.
    #[gpui_kit::test]
    fn closing_the_editor_gives_focus_back_to_its_button(cx: &mut TestAppContext) {
        let (view, model, cx) = rooted_view(cx);
        let id = show_entry(&view, &model, cx, |e| e.private_notes.is_none());
        let notes = view.read_with(cx, |v, _| v.notes.clone());
        let (opener, field) = notes.read_with(cx, |n, _| (n.opener.clone(), n.field.clone()));

        // "Add a note…" from the keyboard: the field takes focus.
        cx.update(|window, cx| {
            window.focus(&opener, cx);
            notes.update(cx, |n, cx| n.edit(window, cx));
        });
        redraw(cx);
        assert!(cx.update(|window, cx| field.read(cx).focus_handle(cx).is_focused(window)));

        cx.simulate_input("A thought");
        press(cx, "ctrl-enter");
        cx.run_until_parked();
        redraw(cx);
        let saved = model.read_with(cx, |m, _| m.entry(id).and_then(|e| e.private_notes.clone()));
        assert_eq!(saved.as_deref(), Some("A thought"));
        assert!(focused(&opener, cx), "Ctrl+Enter: focus on Edit…");

        // Discard, with focus in the field (a click doesn't move it).
        cx.update(|window, cx| notes.update(cx, |n, cx| n.edit(window, cx)));
        redraw(cx);
        cx.simulate_input(" More.");
        redraw(cx);
        cx.update(|window, cx| notes.update(cx, |n, cx| n.discard(window, cx)));
        redraw(cx);
        assert!(focused(&opener, cx), "Discard: focus on Edit…");
        let text = field.read_with(cx, |f, _| f.value().to_string());
        assert_eq!(text, "A thought", "the saved notes are back");
    }
}
