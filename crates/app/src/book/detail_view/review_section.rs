//! The book's "Your review" block: the entry's review in the `reader` role,
//! and an editor in place to write, change or delete it.
//!
//! Reviews are not part of the library sync (review text can be long): the
//! [`LibraryModel`] fetches one when a book page shows its entry, and keeps it
//! for the run. Like a notes draft, an unsaved review is kept when the pane
//! moves to another book, and restored when it comes back.
//!
//! Its own view, [`YourReview`]. Closing the editor (Save, Ctrl+Enter,
//! Cancel, or a delete) gives focus back to the button that opens it again,
//! when focus was in the editor.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::WRITE_REVIEWS;
use hardcover_core::{BookId, Date, LibraryEntry, Review, ReviewedEntry, UserBookId};

use super::your_book::{paragraphs, saved_note};
use crate::bidi::WrappingText;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{date_text, needs_permission};
use crate::library::model::{LibraryModel, LibraryWrite, WriteKind, WriteOutcome, WriteTarget};
use crate::resource::Demand;
use crate::session;
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::open_alert_dialog;
use crate::ui::kit::{KeyboardRing as _, RaisedField as _};
use crate::ui::{Block, ExternalLink, LoadFailed, Skeleton};

/// What the editor holds, or held when it was opened.
#[derive(Debug, Clone, PartialEq)]
struct Draft {
    text: String,
    spoilers: bool,
    date: Option<Date>,
}

impl Draft {
    /// What the editor opens with: the saved review, or a new one dated
    /// `today`. An existing review keeps its date, even none.
    fn base(saved: Option<&Review>, today: Date) -> Self {
        match saved {
            Some(review) => Draft {
                text: review.markdown.clone().unwrap_or_default(),
                spoilers: review.has_spoilers,
                date: review.reviewed_at,
            },
            None => Draft {
                text: String::new(),
                spoilers: false,
                date: Some(today),
            },
        }
    }

    /// Differs from `other` in a way worth saving.
    fn differs(&self, other: &Draft) -> bool {
        self.text.trim() != other.text.trim()
            || self.spoilers != other.spoilers
            || self.date != other.date
    }
}

/// True if the app may write over `saved`: there is none, or it is Markdown.
/// A review written on the website (only its own editor's form, and the plain
/// text Hardcover makes from it) is read-only here, so a save can't flatten it.
fn editable_here(saved: Option<&Review>) -> bool {
    saved.is_none_or(|r| r.markdown.is_some())
}

/// Which review write is in flight.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ReviewWrite {
    Save,
    Delete,
}

/// The "Your review" block of the entry shown. See the module doc.
pub(super) struct YourReview {
    model: Entity<LibraryModel>,
    text: Entity<TextareaState>,
    date: Entity<DatePickerState>,
    spoilers: bool,
    /// The entry being edited and what the editor was filled with.
    editing: Option<(UserBookId, Draft)>,
    /// Unsaved reviews of entries no longer shown, with what each started from.
    drafts: HashMap<UserBookId, (Draft, Draft)>,
    /// The entry whose review was just saved, until the pane moves on.
    saved: Option<UserBookId>,
    writing: Option<ReviewWrite>,
    /// The entry shown, and the book.
    entry: Option<UserBookId>,
    book: Option<BookId>,
    /// The book whose own review the view has asked for ([`Self::fetch`]):
    /// an entry that appears for it later (the book was added) is asked for
    /// at once.
    asked: Option<BookId>,
    /// "Edit…" or "Write a review…", whichever shows: closing the editor
    /// gives focus back to it.
    opener: FocusHandle,
    /// The editor, to tell whether focus is in it when it closes.
    editor: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl YourReview {
    pub(super) fn new(
        model: Entity<LibraryModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What did you think of it?")
                .auto_grow(5, 16)
        });
        let date = cx.new(|cx| DatePickerState::new(window, cx).date_format("%b %-d, %Y"));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(
                &text,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.save(window, cx),
                    InputEvent::Change => {
                        this.saved = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&date, window, |_, _, _: &DatePickerEvent, _, cx| {
                cx.notify();
            }),
            // Writing needs `write:reviews`.
            session::observe_grant(cx, |_, cx| cx.notify()),
        ];
        Self {
            model,
            text,
            date,
            spoilers: false,
            editing: None,
            drafts: HashMap::new(),
            saved: None,
            writing: None,
            entry: None,
            book: None,
            asked: None,
            opener: cx.focus_handle(),
            editor: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows the review of `entry`, `book`'s entry (if it is in the library).
    pub(super) fn set_subject(
        &mut self,
        entry: Option<UserBookId>,
        book: Option<BookId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.book != book {
            self.asked = None;
        }
        self.book = book;
        if self.entry != entry {
            self.sync_entry(entry, window, cx);
            cx.notify();
        }
    }

    /// Asks for the entry's own review, if `demand` calls for it; an entry
    /// that appears for the book later is asked for then.
    pub(super) fn fetch(&mut self, demand: Demand, cx: &mut Context<Self>) {
        self.asked = self.book;
        if let Some(entry) = self.entry {
            self.model
                .update(cx, |m, cx| m.request_review(entry, demand, cx));
        }
    }

    /// The entry shown, from the model.
    fn current_entry<'a>(&self, cx: &'a App) -> Option<&'a LibraryEntry> {
        self.model.read(cx).entry(self.entry?)
    }

    /// Follows the model: the entry may have left the library.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self
            .entry
            .filter(|id| self.model.read(cx).entry(*id).is_some());
        self.sync_entry(entry, window, cx);
    }

    /// Follows the entry shown: sets an unsaved review aside or restores it.
    /// The review itself is asked for with the rest of the book
    /// ([`Self::fetch`]), or here when an entry appears for a book already
    /// asked about.
    fn sync_entry(
        &mut self,
        current: Option<UserBookId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((open, base)) = self.editing.clone()
            && Some(open) != current
        {
            let draft = self.review_draft(cx);
            if draft.differs(&base) {
                self.drafts.insert(open, (draft, base));
            }
            self.editing = None;
        }
        if self.saved != current {
            self.saved = None;
        }
        // Drafts of entries no longer in the library can't be saved.
        let model = self.model.read(cx);
        if model.is_loaded() {
            self.drafts.retain(|id, _| model.entry(*id).is_some());
        }
        let appeared = current.is_some() && current != self.entry;
        self.entry = current;
        let Some(id) = current else {
            return;
        };
        if self.editing.is_none()
            && let Some((draft, base)) = self.drafts.remove(&id)
        {
            self.fill_review_editor(id, draft, base, false, window, cx);
        }
        if appeared && self.book.is_some() && self.asked == self.book {
            self.model
                .update(cx, |m, cx| m.request_review(id, Demand::Active, cx));
        }
    }

    fn retry_review(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.entry else {
            return;
        };
        self.model
            .update(cx, |m, cx| m.request_review(entry, Demand::Now, cx));
    }

    /// The saved review of `id`, if it has one.
    fn saved_review(&self, id: UserBookId, cx: &App) -> Option<Arc<Review>> {
        self.model
            .read(cx)
            .reviews()
            .value(&id)
            .filter(|r| r.exists())
            .cloned()
    }

    /// The review of `id` is being fetched again, or will be: an editor
    /// opened now would start from a review that may be out of date.
    fn review_refreshing(&self, id: UserBookId, cx: &App) -> bool {
        let model = self.model.read(cx);
        model.reviews().is_loading(&id) || model.reviews().is_stale(&id, cx)
    }

    fn review_draft(&self, cx: &App) -> Draft {
        Draft {
            text: self.text.read(cx).value().to_string(),
            spoilers: self.spoilers,
            date: self.date.read(cx).date().start(),
        }
    }

    /// Unsaved changes in the editor.
    fn review_dirty(&self, cx: &App) -> bool {
        self.draft_dirty(&self.review_draft(cx))
    }

    /// A review not saved: in the editor, or set aside for another book.
    pub(super) fn review_unsaved(&self, cx: &App) -> bool {
        self.review_dirty(cx) || !self.drafts.is_empty()
    }

    fn draft_dirty(&self, draft: &Draft) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|(_, base)| draft.differs(base))
    }

    fn fill_review_editor(
        &mut self,
        id: UserBookId,
        draft: Draft,
        base: Draft,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text.update(cx, |s, cx| {
            s.set_value(draft.text, window, cx);
            if focus {
                // The cursor at the end, to go on writing.
                let end = s.value().len();
                s.set_selected_range(end..end, cx);
                s.focus(window, cx);
            }
        });
        self.date.update(cx, |s, cx| {
            s.set_date(CalendarDate::Single(draft.date), window, cx);
        });
        self.spoilers = draft.spoilers;
        self.editing = Some((id, base));
        self.saved = None;
        cx.notify();
    }

    fn open_review_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.entry else {
            return;
        };
        let saved = self.saved_review(id, cx);
        if !editable_here(saved.as_deref()) || self.review_refreshing(id, cx) {
            return;
        }
        let base = Draft::base(saved.as_deref(), crate::dates::today());
        self.fill_review_editor(id, base.clone(), base, true, window, cx);
    }

    /// Closes the editor; focus goes back to the button that opens it, if
    /// it was in the editor (or nowhere, its button gone with it).
    fn close_review_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        if window.focused(cx).is_none() || self.editor.contains_focused(window, cx) {
            window.focus(&self.opener, cx);
        }
        cx.notify();
    }

    fn cancel_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.review_dirty(cx) {
            self.close_review_editor(window, cx);
            return;
        }
        let view = cx.entity().downgrade();
        let opener = self.opener.clone();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let (view, opener) = (view.clone(), opener.clone());
            alert
                .title("Discard your changes to this review?")
                .confirm()
                .ok_text("Discard changes")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.editing = None;
                        cx.notify();
                    });
                    // The confirmation gives focus back to Cancel as it
                    // closes, and Cancel goes with the editor: after it.
                    let opener = opener.clone();
                    window.defer(cx, move |window, cx| window.focus(&opener, cx));
                    true
                })
        });
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, keep)) = self.current_entry(cx).map(|e| (e.user_book_id, e.status)) else {
            return;
        };
        if self.editing.as_ref().map(|(e, _)| *e) != Some(id)
            || !self.review_dirty(cx)
            || !session::has_scope(cx, WRITE_REVIEWS)
            || !editable_here(self.saved_review(id, cx).as_deref())
        {
            return;
        }
        let draft = self.review_draft(cx);
        if draft.text.trim().is_empty() {
            return;
        }
        // Sending the shown status keeps Hardcover from changing it.
        self.writing = Some(ReviewWrite::Save);
        self.write(
            LibraryWrite::entry(
                id,
                WriteKind::Review,
                "Couldn't save your review",
                move |client| client.set_review(id, &draft.text, draft.spoilers, draft.date, keep),
            ),
            |written| {
                written.review.text().is_none().then_some((
                    "Review not saved",
                    "Hardcover didn't keep your review. It is still in the editor; try again.",
                ))
            },
            window,
            cx,
        );
    }

    fn confirm_delete_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let (id, keep) = (entry.user_book_id, entry.status);
        let title: SharedString = format!("Delete your review of “{}”?", entry.book.title).into();
        if !editable_here(self.saved_review(id, cx).as_deref()) {
            return;
        }
        let view = cx.entity().downgrade();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title(title.clone())
                .description(
                    "Its text, spoiler flag and date are deleted from Hardcover. \
                     The book stays in your library.",
                )
                .confirm()
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.writing = Some(ReviewWrite::Delete);
                        this.write(
                            LibraryWrite::entry(
                                id,
                                WriteKind::Review,
                                "Couldn't delete your review",
                                move |client| client.delete_review(id, keep),
                            ),
                            // Plain text left behind is Hardcover's own copy;
                            // only Markdown is the app's.
                            |written| {
                                written.review.markdown.is_some().then_some((
                                    "Review not deleted",
                                    "Hardcover still has your review. Try again, or delete \
                                     it on the Hardcover website.",
                                ))
                            },
                            window,
                            cx,
                        );
                    });
                    true
                })
        });
    }

    /// After a review write on `id` succeeded; the model has the review
    /// Hardcover returned.
    fn review_written(&mut self, id: UserBookId, window: &mut Window, cx: &mut Context<Self>) {
        let write = self.writing.take();
        let kept = self
            .saved_review(id, cx)
            .is_some_and(|r| r.text().is_some());
        // A save Hardcover didn't keep leaves the draft in the editor.
        if write == Some(ReviewWrite::Save) && !kept {
            cx.notify();
            return;
        }
        if self.editing.as_ref().is_some_and(|(e, _)| *e == id) {
            self.close_review_editor(window, cx);
        }
        self.drafts.remove(&id);
        self.saved = (write == Some(ReviewWrite::Save)).then_some(id);
        cx.notify();
    }

    /// Runs a review write through the library model, which applies the
    /// result whether or not this view still shows the book, and shows a
    /// failure. `warning` names what to say despite success. A failure
    /// leaves the draft in the editor.
    fn write(
        &mut self,
        write: LibraryWrite<ReviewedEntry>,
        warning: impl FnOnce(&ReviewedEntry) -> Option<(&'static str, &'static str)> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let WriteTarget::Entry(id) = write.target() else {
            return;
        };
        // A review write needs write:reviews and, for its status,
        // write:library: an unnamed refusal can't be pinned on either.
        let write = write.any_scope();
        let outcome = self.model.update(cx, |m, cx| m.write(write, window, cx));
        cx.spawn_in(window, async move |this, cx| {
            let outcome = outcome.await;
            let _ = this.update_in(cx, |this, window, cx| match outcome {
                WriteOutcome::Saved(written) => {
                    if let Some((title, message)) = warning(&written) {
                        window.push_notification(Notification::warning(message).title(title), cx);
                    }
                    this.review_written(id, window, cx);
                }
                _ => {
                    this.writing = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ----- rendering -----

    /// The "Your review" block: the review in `reader` with "Edit…" on the
    /// heading, "No review yet" with "Write a review…", or the editor.
    fn render_block(&self, id: UserBookId, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let can_write = session::has_scope(cx, WRITE_REVIEWS);
        let muted = colors(cx).fg_muted;
        let model = self.model.read(cx);
        let reviews = model.reviews();
        let fetched = reviews.value(&id).is_some();
        let fetching = reviews.is_loading(&id);
        let error = reviews.error(&id).cloned();
        let review = self.saved_review(id, cx);
        let editing = self.editing.as_ref().is_some_and(|(e, _)| *e == id);
        let busy = model.pending_write(id).is_some();
        let refreshing = self.review_refreshing(id, cx);
        let opener = self.opener.clone();
        let open_editor = |id: &'static str, label: &'static str, cx: &mut Context<Self>| {
            Button::new(id)
                .small_control()
                .label(label)
                .keyboard_ring(&opener, window)
                .disabled(!can_write || busy || refreshing)
                .when(!can_write, |this| {
                    this.tooltip(needs_permission(WRITE_REVIEWS))
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_review_editor(window, cx)))
        };
        let mut action: Option<AnyElement> = None;
        let body: AnyElement = if editing {
            let editable = editable_here(review.as_deref());
            self.render_review_editor(id, review.is_some(), can_write, editable, cx)
                .into_any_element()
        } else if let Some(review) = review {
            if editable_here(Some(&review)) {
                action = Some(
                    open_editor("edit-review", "Edit…", cx)
                        .ghost()
                        .into_any_element(),
                );
            }
            self.render_saved_review(id, &review, cx)
        } else if let Some(error) = error.clone().filter(|_| !fetched) {
            LoadFailed::new("retry-review", error)
                .on_retry(cx.listener(|this, _, _, cx| this.retry_review(cx)))
                .into_any_element()
        } else if !fetched {
            // Not fetched yet: a pulse only while the request runs.
            let line = |skeleton: Skeleton| if fetching { skeleton } else { skeleton.still() };
            v_flex()
                .gap_2()
                .child(line(Skeleton::line()))
                .child(line(Skeleton::line().w_2_3()))
                .into_any_element()
        } else {
            action = Some(open_editor("write-review", "Write a review…", cx).into_any_element());
            div()
                .body()
                .text_color(muted)
                .child("No review yet")
                .into_any_element()
        };
        // A review shown from before a failed refetch may be out of date.
        let refresh_failed = error.filter(|_| fetched).map(|error| {
            LoadFailed::new(
                "refresh-review",
                format!("Couldn't refresh your review. {error}"),
            )
            .on_retry(cx.listener(|this, _, _, cx| this.retry_review(cx)))
        });
        Block::new("Your review")
            .when_some(action, crate::ui::Block::action)
            .child(body)
            .children(refresh_failed)
            .into_any_element()
    }

    fn render_saved_review(
        &self,
        id: UserBookId,
        review: &Review,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = editable_here(Some(review));
        let mut facts: Vec<String> = Vec::new();
        if let Some(date) = review.reviewed_at {
            facts.push(format!("Reviewed {}", date_text(date)));
        }
        if review.has_spoilers {
            facts.push("Contains spoilers".to_owned());
        }
        // The app writes only Markdown; any other review is changed on
        // Hardcover, so a save here can't flatten it.
        let url = self.current_entry(cx).and_then(|e| e.book.web_url());
        let on_hardcover = move |note: &'static str, cx: &mut Context<Self>| {
            v_flex()
                .items_start()
                .gap_1()
                .child(div().body().text_color(colors(cx).fg_muted).child(note))
                .when_some(url, |this, url| {
                    this.child(ExternalLink::new(
                        "edit-review-on-hardcover",
                        "Edit on Hardcover",
                        url,
                    ))
                })
        };
        let body: AnyElement = match (&review.markdown, &review.plain) {
            (Some(markdown), _) => TextView::markdown(
                ElementId::Name(format!("review-{id}").into()),
                markdown.clone(),
            )
            .selectable(true)
            .into_any_element(),
            // Plain text from Hardcover: paragraphs as written, not Markdown.
            (None, Some(plain)) => v_flex()
                .gap_2()
                .children(paragraphs(plain).map(WrappingText::new))
                .into_any_element(),
            // Nothing to edit from, so no editor: saving would replace it.
            (None, None) => {
                return on_hardcover(
                    "You have a review of this book that this app can't show. \
                     Open it on Hardcover to read or change it.",
                    cx,
                )
                .into_any_element();
            }
        };
        v_flex()
            .gap_2()
            .child(div().w_full().max_w(size::PROSE).reader().child(body))
            .when(!facts.is_empty() || self.saved == Some(id), |this| {
                this.child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .when(!facts.is_empty(), |this| {
                            this.child(div().caption(cx).tabular().child(facts.join(" · ")))
                        })
                        .when(self.saved == Some(id), |this| this.child(saved_note(cx))),
                )
            })
            .when(!editable, |this| {
                this.child(on_hardcover(
                    "Written on the Hardcover website, and changed there.",
                    cx,
                ))
            })
            .into_any_element()
    }

    fn render_review_editor(
        &self,
        id: UserBookId,
        has_review: bool,
        can_write: bool,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pending = self.model.read(cx).pending_write(id);
        let writing = (pending == Some(WriteKind::Review))
            .then_some(self.writing)
            .flatten();
        let saving = writing.is_some();
        let draft = self.review_draft(cx);
        let can_save =
            can_write && editable && !draft.text.trim().is_empty() && self.draft_dirty(&draft);
        v_flex()
            .track_focus(&self.editor)
            .w_full()
            .gap_3()
            .child(
                Textarea::new(&self.text)
                    .disabled(saving)
                    .raised_field(saving, cx),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x_6()
                    .gap_y_2()
                    .child(
                        Checkbox::new("review-spoilers")
                            .label("Contains spoilers")
                            .checked(self.spoilers)
                            .disabled(saving)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.spoilers = *checked;
                                this.saved = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().body().child("Reviewed on"))
                            .child(
                                div()
                                    .w(size::SELECT)
                                    .child(DatePicker::new(&self.date).disabled(saving)),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("save-review")
                            .primary()
                            .small_control()
                            .label("Save")
                            .tooltip(if !can_write {
                                needs_permission(WRITE_REVIEWS)
                            } else if !editable {
                                "Your review was changed on the website. Edit it on Hardcover."
                                    .to_owned()
                            } else {
                                "Ctrl+Enter".to_owned()
                            })
                            .loading(writing == Some(ReviewWrite::Save))
                            .disabled(!can_save || pending.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        Button::new("cancel-review")
                            .ghost()
                            .small_control()
                            .label("Cancel")
                            .disabled(saving)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel_review(window, cx)),
                            ),
                    )
                    .when(has_review, |this| {
                        this.child(div().flex_1()).child(
                            Button::new("delete-review")
                                .danger()
                                .small_control()
                                .label("Delete review…")
                                .loading(writing == Some(ReviewWrite::Delete))
                                .disabled(!can_write || !editable || pending.is_some())
                                .when(!can_write, |this| {
                                    this.tooltip(needs_permission(WRITE_REVIEWS))
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_delete_review(window, cx);
                                })),
                        )
                    }),
            )
    }
}

impl Render for YourReview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.entry {
            Some(id) => self.render_block(id, window, cx),
            None => div().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::super::tests::{rooted_view, show_entry};
    use crate::ui::test_support::{press, redraw};

    fn focused(handle: &FocusHandle, cx: &mut VisualTestContext) -> bool {
        cx.update(|window, _| handle.is_focused(window))
    }

    /// C3: closing the editor, by Ctrl+Enter or by discarding a draft,
    /// gives focus back to the button that opens it, not to nothing.
    #[gpui_kit::test]
    fn closing_the_editor_gives_focus_back_to_its_button(cx: &mut TestAppContext) {
        let (view, model, cx) = rooted_view(cx);
        // A book with no review of the reader's: the demo has two reviews.
        let id = show_entry(&view, &model, cx, |e| {
            e.status == hardcover_core::ReadingStatus::Read && e.rating.is_none()
        });
        let review = view.read_with(cx, |v, _| v.review.clone());
        let opener = review.read_with(cx, |r, _| r.opener.clone());
        assert!(
            review.read_with(cx, |r, cx| r.saved_review(id, cx).is_none()),
            "no review yet"
        );

        // "Write a review…" from the keyboard: the field takes focus.
        cx.update(|window, cx| {
            window.focus(&opener, cx);
            review.update(cx, |r, cx| r.open_review_editor(window, cx));
        });
        redraw(cx);
        cx.simulate_input("A fine book.");
        press(cx, "ctrl-enter");
        cx.run_until_parked();
        redraw(cx);
        let saved = review.read_with(cx, |r, cx| r.saved_review(id, cx));
        assert_eq!(
            saved.and_then(|r| r.markdown.clone()).as_deref(),
            Some("A fine book.")
        );
        assert!(focused(&opener, cx), "Ctrl+Enter: focus on Edit…");

        // A draft discarded after the confirmation: focus on Edit… again,
        // not on the editor's Cancel the confirmation went back to.
        cx.update(|window, cx| review.update(cx, |r, cx| r.open_review_editor(window, cx)));
        redraw(cx);
        cx.simulate_input(" More.");
        redraw(cx);
        cx.update(|window, cx| review.update(cx, |r, cx| r.cancel_review(window, cx)));
        redraw(cx);
        redraw(cx);
        assert!(cx.update(gpui_kit::component::WindowExt::has_active_dialog));
        press(cx, "tab");
        press(cx, "enter");
        cx.run_until_parked();
        redraw(cx);
        assert!(!cx.update(gpui_kit::component::WindowExt::has_active_dialog));
        assert!(review.read_with(cx, |r, _| r.editing.is_none()), "closed");
        assert!(focused(&opener, cx), "Discard changes: focus on Edit…");
    }

    fn draft(text: &str, spoilers: bool) -> Draft {
        Draft {
            text: text.to_owned(),
            spoilers,
            date: Date::from_ymd_opt(2026, 1, 2),
        }
    }

    #[test]
    fn a_draft_differs_by_text_flag_or_date_but_not_by_outer_space() {
        let base = draft("Loved it.", false);
        assert!(!draft("  Loved it.\n", false).differs(&base));
        assert!(draft("Loved it!", false).differs(&base));
        assert!(draft("Loved it.", true).differs(&base));
        let mut later = base.clone();
        later.date = Date::from_ymd_opt(2026, 1, 3);
        assert!(later.differs(&base));
    }

    #[test]
    fn an_existing_review_keeps_its_date_and_a_new_one_is_dated_today() {
        let today = Date::from_ymd_opt(2026, 5, 6).unwrap();
        let new = Draft::base(None, today);
        assert_eq!(new.date, Some(today));
        assert_eq!(new.text, "");

        let mut undated = Review::new("Loved it.", true, None);
        assert_eq!(Draft::base(Some(&undated), today).date, None);
        undated.reviewed_at = Date::from_ymd_opt(2025, 1, 2);
        let base = Draft::base(Some(&undated), today);
        assert_eq!(base.date, Date::from_ymd_opt(2025, 1, 2));
        assert_eq!(base.text, "Loved it.");
        assert!(base.spoilers);
    }

    #[test]
    fn only_a_markdown_review_or_none_is_edited_here() {
        assert!(editable_here(None));
        assert!(editable_here(Some(&Review::new("Mine.", false, None))));
        // Written on the website: Hardcover's plain copy, or nothing readable.
        let mut website = Review::new("Mine.", false, None);
        website.markdown = None;
        website.plain = Some("Mine.".to_owned());
        assert!(!editable_here(Some(&website)));
        website.plain = None;
        assert!(!editable_here(Some(&website)));
    }
}
