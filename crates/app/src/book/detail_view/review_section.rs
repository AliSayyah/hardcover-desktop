//! The detail pane's "Your review" section: the entry's review, and an editor
//! to write, change or delete it.
//!
//! Reviews are not part of the library sync (review text can be long): the
//! [`LibraryModel`] fetches one when a book page shows its entry, and keeps it
//! for the run. Like a notes draft, an unsaved review is kept when the pane
//! moves to another book, and restored when it comes back.

use std::collections::HashMap;
use std::time::Duration;

use crate::ui::Skeleton;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::link::Link;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, IntoElement, ParentElement as _,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::WRITE_REVIEWS;
use hardcover_core::{Date, LibraryEntry, Review, UserBookId};

use super::{BookDetailView, Written, section};
use crate::app_state::Services;
use crate::format::{date_text, needs_permission};
use crate::library::model::WriteKind;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::open_alert_dialog;

/// Wait before fetching a review, so arrowing through a list does not spend
/// the rate limit on every book passed over.
const REVIEW_DELAY: Duration = Duration::from_millis(400);

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

/// The review editor's controls and drafts.
pub(super) struct ReviewEditor {
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
    /// The entry whose review fetch waits out [`REVIEW_DELAY`].
    scheduled: Option<UserBookId>,
    _fetch_task: Option<Task<()>>,
}

impl ReviewEditor {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<BookDetailView>,
    ) -> (Self, Vec<Subscription>) {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What did you think of it?")
                .auto_grow(5, 16)
        });
        let date = cx.new(|cx| DatePickerState::new(window, cx).date_format("%b %-d, %Y"));
        let subscriptions = vec![
            cx.subscribe_in(
                &text,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.save_review(window, cx),
                    InputEvent::Change => {
                        this.review.saved = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&date, window, |_, _, _: &DatePickerEvent, _, cx| {
                cx.notify()
            }),
        ];
        let editor = Self {
            text,
            date,
            spoilers: false,
            editing: None,
            drafts: HashMap::new(),
            saved: None,
            writing: None,
            scheduled: None,
            _fetch_task: None,
        };
        (editor, subscriptions)
    }
}

impl BookDetailView {
    /// Follows the entry shown: sets an unsaved review aside or restores it,
    /// and fetches the review if the model wants it.
    pub(super) fn sync_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.entry(self.model.read(cx)).map(|e| e.user_book_id);
        if let Some((open, base)) = self.review.editing.clone()
            && Some(open) != current
        {
            let draft = self.review_draft(cx);
            if draft.differs(&base) {
                self.review.drafts.insert(open, (draft, base));
            }
            self.review.editing = None;
        }
        if self.review.saved != current {
            self.review.saved = None;
        }
        // Drafts of entries no longer in the library can't be saved.
        let model = self.model.read(cx);
        if model.is_loaded() {
            self.review
                .drafts
                .retain(|id, _| model.entry(*id).is_some());
        }
        let Some(id) = current else {
            self.review.scheduled = None;
            self.review._fetch_task = None;
            return;
        };
        if self.review.editing.is_none()
            && let Some((draft, base)) = self.review.drafts.remove(&id)
        {
            self.fill_review_editor(id, draft, base, false, window, cx);
        }
        if !self.model.read(cx).wants_review(id) {
            // Also drops a wait for a book left behind.
            self.review.scheduled = None;
            self.review._fetch_task = None;
            return;
        }
        if self.review.scheduled == Some(id) {
            return;
        }
        self.review.scheduled = Some(id);
        let model = self.model.clone();
        // Replacing the task cancels the wait for a book no longer shown.
        self.review._fetch_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVIEW_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.review.scheduled == Some(id) {
                    this.review.scheduled = None;
                    model.update(cx, |m, cx| m.fetch_review(id, cx));
                }
            });
        }));
    }

    fn retry_review(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        self.model
            .update(cx, |m, cx| m.fetch_review(entry.user_book_id, cx));
    }

    /// The saved review of `id`, if it has one.
    fn saved_review(&self, id: UserBookId, cx: &Context<Self>) -> Option<Review> {
        self.model
            .read(cx)
            .review(id)
            .and_then(|s| s.review.clone())
            .filter(Review::exists)
    }

    /// The review of `id` is being fetched again, or will be: an editor
    /// opened now would start from a review that may be out of date.
    fn review_refreshing(&self, id: UserBookId, cx: &Context<Self>) -> bool {
        let model = self.model.read(cx);
        model.review(id).is_some_and(|s| s.fetching) || model.wants_review(id)
    }

    fn review_draft(&self, cx: &Context<Self>) -> Draft {
        Draft {
            text: self.review.text.read(cx).value().to_string(),
            spoilers: self.review.spoilers,
            date: self.review.date.read(cx).date().start(),
        }
    }

    /// Unsaved changes in the editor.
    fn review_dirty(&self, cx: &Context<Self>) -> bool {
        self.draft_dirty(&self.review_draft(cx))
    }

    fn draft_dirty(&self, draft: &Draft) -> bool {
        self.review
            .editing
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
        self.review.text.update(cx, |s, cx| {
            s.set_value(draft.text.clone(), window, cx);
            if focus {
                s.focus(window, cx);
            }
        });
        self.review.date.update(cx, |s, cx| {
            s.set_date(CalendarDate::Single(draft.date), window, cx)
        });
        self.review.spoilers = draft.spoilers;
        self.review.editing = Some((id, base));
        self.review.saved = None;
        cx.notify();
    }

    fn open_review_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let id = entry.user_book_id;
        let saved = self.saved_review(id, cx);
        if !editable_here(saved.as_ref()) || self.review_refreshing(id, cx) {
            return;
        }
        let base = Draft::base(saved.as_ref(), chrono::Local::now().date_naive());
        self.fill_review_editor(id, base.clone(), base, true, window, cx);
    }

    fn close_review_editor(&mut self, cx: &mut Context<Self>) {
        self.review.editing = None;
        cx.notify();
    }

    fn cancel_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.review_dirty(cx) {
            self.close_review_editor(cx);
            return;
        }
        let view = cx.entity().downgrade();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title("Discard your changes to this review?")
                .confirm()
                .ok_text("Discard changes")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| this.close_review_editor(cx));
                    true
                })
        });
    }

    fn save_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let id = entry.user_book_id;
        if self.review.editing.as_ref().map(|(e, _)| *e) != Some(id)
            || !self.review_dirty(cx)
            || !Services::global(cx).has_scope(WRITE_REVIEWS)
            || !editable_here(self.saved_review(id, cx).as_ref())
        {
            return;
        }
        let draft = self.review_draft(cx);
        if draft.text.trim().is_empty() {
            return;
        }
        // Sending the shown status keeps Hardcover from changing it.
        let keep = entry.status;
        self.review.writing = Some(ReviewWrite::Save);
        self.run_write(
            id,
            WriteKind::Review,
            "Couldn't save your review",
            move |client| {
                client
                    .set_review(id, &draft.text, draft.spoilers, draft.date, keep)
                    .map(|written| {
                        let kept = written.review.text().is_some();
                        let mut written = Written::from(written);
                        if !kept {
                            written.warning = Some((
                                "Review not saved",
                                "Hardcover didn't keep your review. It is still in the \
                                 editor; try again."
                                    .to_owned(),
                            ));
                        }
                        written
                    })
            },
            window,
            cx,
        );
    }

    fn confirm_delete_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let id = entry.user_book_id;
        if !editable_here(self.saved_review(id, cx).as_ref()) {
            return;
        }
        let keep = entry.status;
        let title: SharedString = format!("Delete your review of “{}”?", entry.book.title).into();
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
                        this.review.writing = Some(ReviewWrite::Delete);
                        this.run_write(
                            id,
                            WriteKind::Review,
                            "Couldn't delete your review",
                            move |client| {
                                client.delete_review(id, keep).map(|written| {
                                    // Plain text left behind is Hardcover's
                                    // own copy; only Markdown is the app's.
                                    let kept = written.review.markdown.is_some();
                                    let mut written = Written::from(written);
                                    if kept {
                                        written.warning = Some((
                                            "Review not deleted",
                                            "Hardcover still has your review. Try again, \
                                             or delete it on the Hardcover website."
                                                .to_owned(),
                                        ));
                                    }
                                    written
                                })
                            },
                            window,
                            cx,
                        )
                    });
                    true
                })
        });
    }

    /// After a review write on `id` succeeded; the model has the review
    /// Hardcover returned.
    pub(super) fn review_written(&mut self, id: UserBookId, cx: &mut Context<Self>) {
        let write = self.review.writing.take();
        let kept = self
            .saved_review(id, cx)
            .is_some_and(|r| r.text().is_some());
        // A save Hardcover didn't keep leaves the draft in the editor.
        if write == Some(ReviewWrite::Save) && !kept {
            cx.notify();
            return;
        }
        if self.review.editing.as_ref().is_some_and(|(e, _)| *e == id) {
            self.review.editing = None;
        }
        self.review.drafts.remove(&id);
        self.review.saved = (write == Some(ReviewWrite::Save)).then_some(id);
        cx.notify();
    }

    // ----- rendering -----

    pub(super) fn render_review(
        &self,
        entry: &LibraryEntry,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = entry.user_book_id;
        let can_write = Services::global(cx).has_scope(WRITE_REVIEWS);
        let model = self.model.read(cx);
        let slot = model.review(id);
        let fetched = slot.is_some_and(|s| s.review.is_some());
        let fetching = slot.is_some_and(|s| s.fetching);
        let error = slot.and_then(|s| s.error.clone());
        let review = slot
            .and_then(|s| s.review.as_ref())
            .filter(|r| r.exists())
            .cloned();
        let editing = self.review.editing.as_ref().is_some_and(|(e, _)| *e == id);
        let body: AnyElement = if editing {
            let editable = editable_here(review.as_ref());
            self.render_review_editor(entry, review.is_some(), can_write, editable, cx)
                .into_any_element()
        } else if let Some(review) = review {
            self.render_saved_review(entry, review, can_write, cx)
                .into_any_element()
        } else if let Some(error) = error.clone().filter(|_| !fetched) {
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().text_color(cx.theme().muted_foreground).child(error))
                .child(
                    Button::new("retry-review")
                        .small()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_review(cx))),
                )
                .into_any_element()
        } else if !fetched {
            // Not fetched yet: a pulse only while the request runs.
            match fetching {
                true => v_flex()
                    .gap_2()
                    .child(Skeleton::line().h_4().w_full())
                    .child(Skeleton::line().h_4().w_2_3())
                    .into_any_element(),
                false => {
                    let bar = || div().h_4().bg(cx.theme().skeleton);
                    v_flex()
                        .gap_2()
                        .child(bar().w_full())
                        .child(bar().w_2_3())
                        .into_any_element()
                }
            }
        } else {
            self.render_no_review(entry, can_write, cx)
                .into_any_element()
        };
        // A review shown from before a failed refetch may be out of date.
        let refresh_failed = error.filter(|_| fetched).map(|error| {
            h_flex()
                .gap_2()
                .items_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Couldn't refresh")
                .child("·")
                .child(
                    Button::new("refresh-review")
                        .ghost()
                        .xsmall()
                        .label("Try again")
                        .tooltip(error)
                        .on_click(cx.listener(|this, _, _, cx| this.retry_review(cx))),
                )
        });
        section("Your review").child(body).children(refresh_failed)
    }

    fn render_saved_review(
        &self,
        entry: &LibraryEntry,
        review: Review,
        can_write: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.user_book_id;
        let muted = cx.theme().muted_foreground;
        let busy = self.model.read(cx).pending_write(id).is_some();
        let refreshing = self.review_refreshing(id, cx);
        let editable = editable_here(Some(&review));
        let mut facts: Vec<String> = Vec::new();
        if let Some(date) = review.reviewed_at {
            facts.push(format!("Reviewed {}", date_text(date)));
        }
        if review.has_spoilers {
            facts.push("Contains spoilers".to_owned());
        }
        // The app writes only Markdown; any other review is changed on
        // Hardcover, so a save here can't flatten it.
        let url = entry.book.web_url();
        let on_hardcover = move |note: &'static str| {
            h_flex()
                .flex_wrap()
                .gap_x_2()
                .text_sm()
                .text_color(muted)
                .child(note)
                .when_some(url, |this, url| {
                    this.child(
                        Link::new("edit-review-on-hardcover")
                            .href(url)
                            .child("Edit on Hardcover"),
                    )
                })
        };
        let body: AnyElement = match (review.markdown, review.plain) {
            (Some(markdown), _) => {
                TextView::markdown(ElementId::Name(format!("review-{id}").into()), markdown)
                    .selectable(true)
                    .into_any_element()
            }
            // Plain text from Hardcover: paragraphs as written, not Markdown.
            (None, Some(plain)) => v_flex()
                .gap_2()
                .children(
                    plain
                        .split('\n')
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(|p| div().child(p.to_owned())),
                )
                .into_any_element(),
            // Nothing to edit from, so no editor: saving would replace it.
            (None, None) => {
                return on_hardcover(
                    "You have a review of this book that this app can't show. \
                     Open it on Hardcover to read or change it.",
                )
                .into_any_element();
            }
        };
        let edit: AnyElement = if editable {
            Button::new("edit-review")
                .outline()
                .small()
                .label("Edit…")
                .disabled(!can_write || busy || refreshing)
                .when(!can_write, |this| {
                    this.tooltip(needs_permission(WRITE_REVIEWS))
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_review_editor(window, cx)))
                .into_any_element()
        } else {
            on_hardcover("Written on the Hardcover website, and changed there.").into_any_element()
        };
        v_flex()
            .gap_3()
            .child(div().text_sm().child(body))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .when(!facts.is_empty(), |this| this.child(facts.join(" · ")))
                    .child(edit)
                    .when(self.review.saved == Some(id), |this| {
                        this.child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .child(Icon::new(IconName::Check).xsmall())
                                .child("Saved"),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_no_review(
        &self,
        entry: &LibraryEntry,
        can_write: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = entry.user_book_id;
        let busy = self.model.read(cx).pending_write(id).is_some();
        let refreshing = self.review_refreshing(id, cx);
        h_flex()
            .flex_wrap()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("You haven't reviewed this book."),
            )
            .child(
                Button::new("write-review")
                    .small()
                    .icon(IconName::Plus)
                    .label("Write a review…")
                    .disabled(!can_write || busy || refreshing)
                    .when(!can_write, |this| {
                        this.tooltip(needs_permission(WRITE_REVIEWS))
                    })
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_review_editor(window, cx)),
                    ),
            )
    }

    fn render_review_editor(
        &self,
        entry: &LibraryEntry,
        has_review: bool,
        can_write: bool,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let writing = (pending == Some(WriteKind::Review))
            .then_some(self.review.writing)
            .flatten();
        let saving = writing.is_some();
        let draft = self.review_draft(cx);
        let can_save =
            can_write && editable && !draft.text.trim().is_empty() && self.draft_dirty(&draft);
        v_flex()
            .w_full()
            .gap_3()
            .child(
                Textarea::new(&self.review.text)
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
                            .checked(self.review.spoilers)
                            .disabled(saving)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.review.spoilers = *checked;
                                this.review.saved = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().child("Reviewed on"))
                            .child(
                                div()
                                    .w_48()
                                    .child(DatePicker::new(&self.review.date).disabled(saving)),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("save-review")
                            .primary()
                            .small()
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
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_review(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("cancel-review")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .disabled(saving)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel_review(window, cx)),
                            ),
                    )
                    .when(has_review, |this| {
                        this.child(div().flex_1()).child(
                            Button::new("delete-review")
                                .ghost()
                                .danger()
                                .small()
                                .label("Delete review…")
                                .loading(writing == Some(ReviewWrite::Delete))
                                .disabled(!can_write || !editable || pending.is_some())
                                .when(!can_write, |this| {
                                    this.tooltip(needs_permission(WRITE_REVIEWS))
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_delete_review(window, cx)
                                })),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(new.text.is_empty());

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
