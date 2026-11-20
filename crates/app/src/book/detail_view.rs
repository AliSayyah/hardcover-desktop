//! The book: the pane beside a shelf, a list or search results, and the full
//! page over a destination (`hardcover-design`, Screens → Shelf and Full book
//! page). One view does both; [`BookDetailView::set_tools_in_bar`] makes it
//! the page.
//!
//! - **Pane:** its own object bar (no Back; "Lists" and "more" trailing),
//!   then, inset 24: the header (xl cover, the title in `display-24`), the
//!   progress or Not started panel, and the blocks.
//! - **Page:** the shell draws the bar (Back and this view's
//!   [`ObjectTools`]). Inset 32: a main column (xxl cover, `display-30`,
//!   the panel, About this book, Your review, Reviews, Similar books) and the
//!   rail (On Hardcover, Details, Reads, Notes), which folds under the main
//!   column in a narrow window (`ui::Columns`).
//!
//! Shows a library entry (with your book's controls and its reads) or a book
//! that is not in the library (with "Add to library"). Controls reflect the
//! entry in the [`LibraryModel`]; a write disables them, and on success the
//! returned entry replaces the model's. On failure a notification explains it
//! and the controls are reset to the model's (unchanged) values. A notes or
//! review draft is the exception: it is kept until saved or discarded.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{IconName, WindowExt as _, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, RenderImage, SharedString, Styled as _, Subscription, Task, Window,
    div,
};
use hardcover_core::auth::scopes::{READ_CATALOG, WRITE_LIBRARY};
use hardcover_core::{
    BookDetails, BookId, BookSummary, Client, EditionChange, EditionSummary, Error, LibraryEntry,
    Privacy, Rating, ReadId, ReadSession, ReadingStatus, Review, ReviewedEntry, UserBookId,
};

use self::catalog_sections::Similar;
use self::community_reviews::CommunityReviews;
use self::review_section::ReviewEditor;
use super::edition_picker::{EditionChosen, EditionPicker};
use super::read_dialog::{ReadDatesChosen, ReadDialog, ReadDialogKind};
use crate::app_state::Services;
use crate::design::size;
use crate::format::{error_message, read_dates_text};
use crate::library::model::{LibraryModel, WriteKind};
use crate::lists::book_lists;
use crate::lists::model::ListsModel;
use crate::ui::kit::open_alert_dialog;
use crate::ui::{Columns, EmptyState, ObjectBar, ObjectTools, blocks, folds};

mod catalog_sections;
mod community_reviews;
mod header;
mod review_section;
mod your_book;

/// What the pane shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    /// An entry in the library, by id (it follows status changes).
    Entry(UserBookId),
    /// A book from search; shown as an entry if it is in the library.
    Book(BookSummary),
}

/// A choice in the privacy control.
#[derive(Clone)]
pub struct PrivacyOption {
    privacy: Privacy,
    label: SharedString,
}

impl SelectItem for PrivacyOption {
    type Value = Privacy;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Privacy {
        &self.privacy
    }
}

fn privacy_options() -> Vec<PrivacyOption> {
    Privacy::ALL
        .into_iter()
        .map(|privacy| PrivacyOption {
            privacy,
            label: privacy.label().into(),
        })
        .collect()
}

/// The entry values the stateful controls (the privacy select) show.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    id: UserBookId,
    privacy: Option<Privacy>,
}

impl Shown {
    fn of(entry: &LibraryEntry) -> Self {
        Self {
            id: entry.user_book_id,
            privacy: entry.privacy,
        }
    }
}

/// The notes field's text compared with the entry's saved notes.
fn notes_differ(text: &str, saved: Option<&str>) -> bool {
    text.trim() != saved.unwrap_or("").trim()
}

enum Details {
    Loaded(Arc<BookDetails>),
    Failed(SharedString),
}

/// Wait before fetching details so arrowing through a list does not spend the
/// rate limit on every book passed over.
const DETAILS_DELAY: Duration = Duration::from_millis(250);

pub struct BookDetailView {
    model: Entity<LibraryModel>,
    subject: Option<Subject>,
    details: HashMap<BookId, Details>,
    details_loading: Option<BookId>,
    /// The book whose details request is in flight, after [`DETAILS_DELAY`].
    /// Only then does the placeholder animate: a pulse while arrowing past
    /// books would redraw the window every frame for nothing.
    details_fetching: Option<BookId>,
    _details_task: Option<Task<()>>,
    privacy: Entity<SelectState<Vec<PrivacyOption>>>,
    notes: Entity<TextareaState>,
    /// The entry and saved notes the notes field was last filled from.
    notes_base: Option<(UserBookId, Option<String>)>,
    /// Unsaved notes of entries no longer shown, restored when they are.
    notes_drafts: HashMap<UserBookId, String>,
    /// The entry whose notes were just saved, until the field changes again.
    notes_saved: Option<UserBookId>,
    /// The entry whose notes editor was opened ("Add a note…", "Edit"). An
    /// unsaved draft keeps it open too.
    notes_editing: Option<UserBookId>,
    /// One picker per book, so editions are fetched once per run.
    edition_pickers: HashMap<BookId, (Entity<EditionPicker>, Subscription)>,
    read_dialog: Option<(Entity<ReadDialog>, Subscription)>,
    shown: Option<Shown>,
    /// The cover URL last requested and its image once loaded, released when
    /// the pane moves to another cover.
    cover: Option<(String, Option<Arc<RenderImage>>)>,
    similar: HashMap<BookId, Similar>,
    similar_loading: Option<BookId>,
    _similar_task: Option<Task<()>>,
    /// "Show all tags" was pressed for the book shown.
    tags_expanded: bool,
    /// The full book page, under the object bar the shell draws: the book's
    /// tools go to that bar, and the page has its own layout (xxl cover,
    /// `display-30`, a rail). Otherwise the pane, which draws its own bar.
    tools_in_bar: bool,
    review: ReviewEditor,
    community_reviews: CommunityReviews,
    _subscriptions: Vec<Subscription>,
}

impl BookDetailView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let privacy = cx.new(|cx| SelectState::new(privacy_options(), None, window, cx));
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Only you can see these notes")
                .auto_grow(3, 10)
        });

        let (review, review_subscriptions) = ReviewEditor::new(window, cx);
        let mut subscriptions =
            vec![
                cx.observe_in(&model, window, |this, _, window, cx| {
                    this.sync_controls(window, cx);
                    cx.notify();
                }),
                cx.subscribe_in(
                    &privacy,
                    window,
                    |this, _, event: &SelectEvent<Vec<PrivacyOption>>, window, cx| {
                        if let SelectEvent::Confirm(Some(privacy)) = event {
                            this.change_privacy(*privacy, window, cx);
                        }
                    },
                ),
                cx.subscribe_in(&notes, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter {
                            secondary: true, ..
                        } => this.save_notes(window, cx),
                        InputEvent::Change => {
                            this.notes_saved = None;
                            cx.notify();
                        }
                        _ => {}
                    }
                }),
            ];
        subscriptions.extend(review_subscriptions);
        subscriptions.push(Self::observe_review_scope(cx));
        // The list menu and the Owned switch follow the user's lists.
        if let Some(lists) = ListsModel::global(cx) {
            subscriptions.push(cx.observe(&lists, |_, _, cx| cx.notify()));
        }

        Self {
            model,
            subject: None,
            details: HashMap::new(),
            details_loading: None,
            details_fetching: None,
            _details_task: None,
            privacy,
            notes,
            notes_base: None,
            notes_drafts: HashMap::new(),
            notes_saved: None,
            notes_editing: None,
            edition_pickers: HashMap::new(),
            read_dialog: None,
            shown: None,
            cover: None,
            similar: HashMap::new(),
            similar_loading: None,
            _similar_task: None,
            tags_expanded: false,
            tools_in_bar: false,
            review,
            community_reviews: CommunityReviews::default(),
            _subscriptions: subscriptions,
        }
    }

    pub fn set_subject(
        &mut self,
        subject: Option<Subject>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.subject == subject {
            return;
        }
        self.subject = subject;
        self.shown = None;
        self.sync_controls(window, cx);
        self.load_details(cx);
        if let Some(book) = self.book(cx) {
            book_lists::book_shown(book.id, cx);
        }
        self.tags_expanded = false;
        // Drops a pending similar-books fetch for the book left behind.
        self._similar_task = None;
        self.similar_loading = None;
        self.load_similar(cx);
        self.community_reviews_subject_changed(cx);
        cx.notify();
    }

    /// Makes this view the full book page: its tools go to the object bar the
    /// shell draws over it ([`ObjectTools`]), and it takes the page's layout.
    /// Otherwise it is the pane, which draws its own bar.
    pub fn set_tools_in_bar(&mut self, in_bar: bool) {
        self.tools_in_bar = in_bar;
    }

    /// The library entry for the subject, if the book is in the library.
    fn entry<'a>(&self, model: &'a LibraryModel) -> Option<&'a LibraryEntry> {
        match self.subject.as_ref()? {
            Subject::Entry(id) => model.entry(*id),
            Subject::Book(book) => model.entry_for_book(book.id),
        }
    }

    fn book(&self, cx: &App) -> Option<BookSummary> {
        let model = self.model.read(cx);
        match self.subject.as_ref()? {
            Subject::Entry(id) => model.entry(*id).map(|e| e.book.clone()),
            Subject::Book(book) => Some(book.clone()),
        }
    }

    /// Points the controls at the entry's current values when they differ from
    /// what is shown. Never emits control events, so it cannot loop.
    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self.entry(self.model.read(cx));
        let notes = entry.map(|e| (e.user_book_id, e.private_notes.clone()));
        let next = entry.map(Shown::of);
        self.sync_notes(notes, window, cx);
        self.sync_review(window, cx);
        let Some(next) = next else {
            self.shown = None;
            return;
        };
        if self.shown.as_ref() == Some(&next) {
            return;
        }
        self.shown = Some(next.clone());
        self.privacy.update(cx, |s, cx| match &next.privacy {
            Some(p) => s.set_selected_value(p, window, cx),
            None => s.set_selected_index(None, window, cx),
        });
    }

    /// Fills the notes field from `next` (entry id and saved notes) unless the
    /// field holds an unsaved draft for that entry. A draft for another entry is
    /// set aside and restored when that entry is shown again.
    fn sync_notes(
        &mut self,
        next: Option<(UserBookId, Option<String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.notes_base == next {
            return;
        }
        let text = self.notes.read(cx).value().to_string();
        let same_entry =
            self.notes_base.as_ref().map(|(id, _)| *id) == next.as_ref().map(|(id, _)| *id);
        if let Some((id, saved)) = &self.notes_base
            && notes_differ(&text, saved.as_deref())
        {
            if same_entry {
                // The saved notes changed under a draft (or a save made them
                // equal): keep the text, and compare it with the new value.
                self.notes_base = next;
                return;
            }
            self.notes_drafts.insert(*id, text);
        }
        let Some((id, saved)) = next.clone() else {
            self.notes_base = None;
            return;
        };
        let value = self
            .notes_drafts
            .remove(&id)
            .unwrap_or_else(|| saved.unwrap_or_default());
        if *self.notes.read(cx).value() != *value {
            self.notes
                .update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.notes_base = next;
        self.notes_saved = None;
    }

    /// Unsaved notes in the field for the entry shown.
    fn notes_dirty(&self, cx: &App) -> bool {
        self.notes_base
            .as_ref()
            .is_some_and(|(_, saved)| notes_differ(&self.notes.read(cx).value(), saved.as_deref()))
    }

    /// Resets the controls to the model after a rejected or failed change.
    fn revert_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shown = None;
        self.sync_controls(window, cx);
        cx.notify();
    }

    // ----- details -----

    fn load_details(&mut self, cx: &mut Context<Self>) {
        let Some(book) = self.book(cx) else {
            return;
        };
        if matches!(self.details.get(&book.id), Some(Details::Loaded(_))) {
            return;
        }
        if self.details_loading == Some(book.id) {
            return;
        }
        self.details.remove(&book.id);
        self.details_loading = Some(book.id);
        self.details_fetching = None;
        let client = Services::global(cx).client().clone();
        let id = book.id;
        // Replacing the task cancels a fetch for a book no longer shown.
        self._details_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DETAILS_DELAY).await;
            let fetching = this.update(cx, |this, cx| {
                this.details_fetching = Some(id);
                cx.notify();
            });
            if fetching.is_err() {
                return;
            }
            let result = cx
                .background_spawn(async move { client.book_details(id) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.details_loading == Some(id) {
                    this.details_loading = None;
                }
                if this.details_fetching == Some(id) {
                    this.details_fetching = None;
                }
                let details = match result {
                    Ok(d) => Details::Loaded(Arc::new(d)),
                    Err(e) => {
                        Details::Failed(Services::error_message(&e, Some(READ_CATALOG), cx).into())
                    }
                };
                this.details.insert(id, details);
                this.load_similar(cx);
                this.load_community_reviews(cx);
                cx.notify();
            });
        }));
    }

    fn retry_details(&mut self, cx: &mut Context<Self>) {
        if let Some(book) = self.book(cx) {
            self.details.remove(&book.id);
        }
        self.details_loading = None;
        self.load_details(cx);
        cx.notify();
    }

    // ----- writes -----

    /// Runs one write on `id` in the background. The model applies the result
    /// whether or not this view still shows the book.
    fn run_write<W: Into<Written> + Send + 'static>(
        &mut self,
        id: UserBookId,
        kind: WriteKind,
        failure_title: &'static str,
        op: impl FnOnce(&Client) -> Result<W, Error> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model = self.model.clone();
        if !model.update(cx, |m, cx| m.begin_write(id, kind, cx)) {
            let title = match kind {
                WriteKind::Notes => Some("Notes not saved yet"),
                WriteKind::Review => Some("Review not saved yet"),
                _ => None,
            };
            if let Some(title) = title {
                window.push_notification(
                    Notification::info(
                        "Another change to this book is still being saved. Try again in a moment.",
                    )
                    .title(title),
                    cx,
                );
            }
            self.revert_controls(window, cx);
            return;
        }
        let client = Services::global(cx).client().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { op(&client).map(Into::into) })
                .await;
            model.update(cx, |m, cx| {
                m.end_write(id, cx);
                match &result {
                    // A review write never removes the entry: without one,
                    // the entry shown stays.
                    Ok(Written {
                        entry,
                        review: Some(review),
                        ..
                    }) => {
                        if let Some(entry) = entry {
                            m.apply_written(entry.clone(), cx);
                        }
                        m.apply_review(id, review.clone(), cx);
                    }
                    Ok(Written {
                        entry: Some(entry),
                        review: None,
                        ..
                    }) => m.apply_written(entry.clone(), cx),
                    Ok(Written {
                        entry: None,
                        review: None,
                        ..
                    }) => m.apply_removed(id, cx),
                    Err(_) => {}
                }
            });
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(written) => {
                    if let Some((title, message)) = written.warning {
                        window.push_notification(Notification::warning(message).title(title), cx);
                    }
                    if kind == WriteKind::Notes {
                        this.notes_saved = Some(id);
                        if this.notes_editing == Some(id) {
                            this.notes_editing = None;
                        }
                        cx.notify();
                    }
                    if kind == WriteKind::Review {
                        this.review_written(id, cx);
                    }
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(Services::error_message(
                            &error,
                            // A review write needs write:reviews and, for its
                            // status, write:library: an unnamed refusal can't
                            // be pinned on either.
                            match kind {
                                WriteKind::Review => None,
                                _ => Some(WRITE_LIBRARY),
                            },
                            cx,
                        ))
                        .title(failure_title),
                        cx,
                    );
                    this.revert_controls(window, cx);
                }
            });
        })
        .detach();
    }

    fn current_entry(&self, cx: &App) -> Option<LibraryEntry> {
        self.entry(self.model.read(cx)).cloned()
    }

    fn change_status(
        &mut self,
        status: ReadingStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        if entry.status == status {
            return;
        }
        let id = entry.user_book_id;
        self.run_write(
            id,
            WriteKind::Status,
            "Couldn't change the status",
            move |client| client.set_status(id, status).map(Some),
            window,
            cx,
        );
    }

    fn change_rating(
        &mut self,
        rating: Option<Rating>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let id = entry.user_book_id;
        // Sending the shown status keeps Hardcover from marking the book Read.
        let keep = entry.status;
        self.run_write(
            id,
            WriteKind::Rating,
            "Couldn't save the rating",
            move |client| client.set_rating(id, rating, keep).map(Some),
            window,
            cx,
        );
    }

    fn change_privacy(&mut self, privacy: Privacy, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        if entry.privacy == Some(privacy) {
            return;
        }
        let id = entry.user_book_id;
        self.run_write(
            id,
            WriteKind::Privacy,
            "Couldn't change who can see this book",
            move |client| client.set_privacy(id, privacy).map(Some),
            window,
            cx,
        );
    }

    fn save_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        if !self.notes_dirty(cx) {
            return;
        }
        let id = entry.user_book_id;
        let text = self.notes.read(cx).value().to_string();
        self.run_write(
            id,
            WriteKind::Notes,
            "Couldn't save your notes",
            move |client| client.set_private_notes(id, &text).map(Some),
            window,
            cx,
        );
    }

    fn discard_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = self
            .notes_base
            .as_ref()
            .and_then(|(_, saved)| saved.clone())
            .unwrap_or_default();
        self.notes
            .update(cx, |s, cx| s.set_value(saved, window, cx));
        self.notes_editing = None;
        cx.notify();
    }

    /// Opens the notes editor on the entry shown, with the cursor in it.
    fn edit_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        self.notes_editing = Some(entry.user_book_id);
        self.notes_saved = None;
        self.notes.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Whether the notes editor shows for `id`: opened, or holding a draft.
    fn notes_open(&self, id: UserBookId, cx: &App) -> bool {
        self.notes_editing == Some(id) || self.notes_dirty(cx)
    }

    fn open_edition_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let book = entry.book.id;
        let picker = match self.edition_pickers.get(&book) {
            Some((picker, _)) => picker.clone(),
            None => {
                let title: SharedString = entry.book.title.clone().into();
                let picker = cx.new(|cx| EditionPicker::new(book, title, cx));
                let subscription = cx.subscribe_in(
                    &picker,
                    window,
                    |this, _, EditionChosen(edition), window, cx| {
                        this.change_edition(edition.clone(), window, cx)
                    },
                );
                self.edition_pickers
                    .insert(book, (picker.clone(), subscription));
                picker
            }
        };
        let current = entry.edition.as_ref().map(|e| e.id);
        picker.update(cx, |p, cx| p.set_current(current, cx));
        EditionPicker::open(picker, window, cx);
    }

    fn change_edition(
        &mut self,
        edition: EditionSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        if entry.edition.as_ref().map(|e| e.id) == Some(edition.id) {
            return;
        }
        let id = entry.user_book_id;
        self.run_write(
            id,
            WriteKind::Edition,
            "Couldn't change the edition",
            move |client| client.set_edition(id, edition.id),
            window,
            cx,
        );
    }

    /// Opens the read dialog to add a read (`None`) or change `read`'s dates.
    fn open_read_dialog(
        &mut self,
        read: Option<ReadSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = match &read {
            Some(r) => ReadDialogKind::Edit {
                read: r.id,
                finished: r.is_finished(),
            },
            None => ReadDialogKind::Add,
        };
        let dialog = cx.new(|cx| ReadDialog::new(kind, read.as_ref(), window, cx));
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            |this, _, chosen: &ReadDatesChosen, window, cx| this.save_read(chosen, window, cx),
        );
        self.read_dialog = Some((dialog.clone(), subscription));
        ReadDialog::open(dialog, window, cx);
    }

    fn save_read(&mut self, chosen: &ReadDatesChosen, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let id = entry.user_book_id;
        let (started, finished) = (chosen.started, chosen.finished);
        match chosen.kind {
            ReadDialogKind::Add => {
                let Some(finished) = finished else {
                    return;
                };
                let edition = entry.edition.as_ref().map(|e| e.id);
                self.run_write(
                    id,
                    WriteKind::Reads,
                    "Couldn't add the read",
                    move |client| client.add_read(id, started, finished, edition).map(Some),
                    window,
                    cx,
                );
            }
            ReadDialogKind::Edit { read, .. } => {
                let unchanged = entry
                    .read(read)
                    .is_some_and(|r| r.started_at == started && r.finished_at == finished);
                if unchanged {
                    return;
                }
                self.run_write(
                    id,
                    WriteKind::Dates,
                    "Couldn't change the dates",
                    move |client| client.set_read_dates(read, started, finished).map(Some),
                    window,
                    cx,
                );
            }
        }
    }

    fn confirm_delete_read(&mut self, read: ReadId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let Some(session) = entry.read(read) else {
            return;
        };
        let id = entry.user_book_id;
        let description: SharedString = format!(
            "{}. Its dates and progress are deleted from Hardcover.",
            read_dates_text(session.started_at, session.finished_at)
        )
        .into();
        let view = cx.entity().downgrade();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title("Delete this read?")
                .description(description.clone())
                .confirm()
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.run_write(
                            id,
                            WriteKind::Reads,
                            "Couldn't delete the read",
                            move |client| client.delete_read(id, read).map(Some),
                            window,
                            cx,
                        )
                    });
                    true
                })
        });
    }

    fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let id = entry.user_book_id;
        let title: SharedString = format!("Remove “{}”?", entry.book.title).into();
        let view = cx.entity().downgrade();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title(title.clone())
                .description(
                    "Its status, rating, progress and reading dates are deleted from Hardcover.",
                )
                .confirm()
                .ok_text("Remove")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.remove(id, window, cx));
                    true
                })
        });
    }

    fn remove(&mut self, id: UserBookId, window: &mut Window, cx: &mut Context<Self>) {
        self.run_write(
            id,
            WriteKind::Remove,
            "Couldn't remove the book",
            move |client| client.remove_from_library(id).map(|()| None),
            window,
            cx,
        );
    }

    fn add_to_library(
        &mut self,
        status: ReadingStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Subject::Book(book)) = self.subject.clone() else {
            return;
        };
        let model = self.model.clone();
        if !model.update(cx, |m, cx| m.begin_add(book.id, cx)) {
            return;
        }
        let client = Services::global(cx).client().clone();
        let book_id = book.id;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.add_to_library(book_id, status) })
                .await;
            model.update(cx, |m, cx| {
                m.end_add(book_id, cx);
                if let Ok(entry) = &result {
                    m.apply_written(entry.clone(), cx);
                }
            });
            if let Err(error) = result {
                let _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        Notification::error(Services::error_message(
                            &error,
                            Some(WRITE_LIBRARY),
                            cx,
                        ))
                        .title("Couldn't add the book"),
                        cx,
                    );
                });
            }
        })
        .detach();
    }
}

/// The outcome of a write, for [`BookDetailView::run_write`]: the entry to
/// apply, its review if the write was a review write, and a warning to show
/// despite success. No entry means it was removed, except after a review
/// write, which leaves the entry shown as it is.
struct Written {
    entry: Option<LibraryEntry>,
    review: Option<Review>,
    warning: Option<(&'static str, String)>,
}

impl From<Option<LibraryEntry>> for Written {
    fn from(entry: Option<LibraryEntry>) -> Self {
        Self {
            entry,
            review: None,
            warning: None,
        }
    }
}

impl From<ReviewedEntry> for Written {
    fn from(written: ReviewedEntry) -> Self {
        Self {
            entry: written.entry,
            review: Some(written.review),
            warning: None,
        }
    }
}

impl From<EditionChange> for Written {
    fn from(change: EditionChange) -> Self {
        Self {
            entry: Some(change.entry),
            review: None,
            warning: change.read_left_behind.map(|error| {
                (
                    "Edition changed, progress not moved",
                    format!(
                        "Your current read still counts progress against the old edition. {}",
                        error_message(&error)
                    ),
                )
            }),
        }
    }
}

impl BookDetailView {
    /// The subject's entry and book, if there is a subject.
    fn shown_book(&self, cx: &App) -> Option<(BookSummary, Option<LibraryEntry>)> {
        let entry = self.entry(self.model.read(cx)).cloned();
        let book = match (&self.subject, &entry) {
            (_, Some(entry)) => entry.book.clone(),
            (Some(Subject::Book(book)), None) => book.clone(),
            _ => return None,
        };
        Some((book, entry))
    }

    /// The pane: its bar over one scrolling column of the header, the panel
    /// and every block.
    fn render_pane(
        &mut self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        details: Option<&BookDetails>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = self.render_header(book, entry, details, false, window, cx);
        let mut column: Vec<AnyElement> = Vec::new();
        if let Some(entry) = entry {
            column.extend(self.render_notes(entry, false, cx));
        }
        column.push(self.render_about(book, cx));
        column.extend(self.render_tags(details, cx));
        column.extend(self.render_other_titles(details, cx));
        if let Some(entry) = entry {
            column.push(self.render_details(entry, false, cx));
            column.push(self.render_review(entry, cx).into_any_element());
            column.push(self.render_reads(entry, false, cx));
        }
        column.extend(self.render_on_hardcover(book, details, false, cx));
        column.extend(self.render_community_reviews(book.id, window, cx));
        column.extend(self.render_similar(book, details, cx));

        let content = v_flex()
            .w_full()
            .max_w(PANE_MAX)
            .gap_6()
            .child(header)
            .children(entry.and_then(|entry| self.render_panel(entry, cx)))
            .child(blocks().children(column));
        v_flex()
            .size_full()
            .child(ObjectBar::new().tools(self.render_tools(book, entry, cx)))
            .child(
                div()
                    // Keyed by book, so another book (a similar one) opens at
                    // the top.
                    .id(("book-detail", book.id.get() as u64))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px_6()
                    .pb_6()
                    .child(content),
            )
            .into_any_element()
    }

    /// The full page: a main column beside the rail, both from the top; the
    /// rail folds under the main column when the page is too narrow.
    fn render_page(
        &mut self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        details: Option<&BookDetails>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = self.render_header(book, entry, details, true, window, cx);
        let mut main: Vec<AnyElement> = Vec::new();
        main.push(self.render_about(book, cx));
        main.extend(self.render_tags(details, cx));
        main.extend(self.render_other_titles(details, cx));
        if let Some(entry) = entry {
            main.push(self.render_review(entry, cx).into_any_element());
        }
        main.extend(self.render_community_reviews(book.id, window, cx));
        main.extend(self.render_similar(book, details, cx));

        let mut rail: Vec<AnyElement> = Vec::new();
        rail.extend(self.render_on_hardcover(book, details, true, cx));
        if let Some(entry) = entry {
            rail.push(self.render_details(entry, true, cx));
            rail.push(self.render_reads(entry, true, cx));
            rail.extend(self.render_notes(entry, true, cx));
        }

        // The page is the window less the sidebar and its own insets: GPUI
        // has no container queries.
        let rem = window.rem_size();
        let width = window.viewport_size().width
            - size::SIDEBAR.to_pixels(rem)
            - PAGE_INSET.to_pixels(rem) * 2.;
        let main = v_flex()
            .w_full()
            .gap_6()
            .child(header)
            .children(entry.and_then(|entry| self.render_panel(entry, cx)))
            .child(blocks().children(main));
        div()
            .id(("book-detail", book.id.get() as u64))
            .size_full()
            .overflow_y_scrollbar()
            .child(
                div().w_full().p_8().child(
                    Columns::new(main)
                        .rail(rail)
                        .folded(folds(width, rem, false)),
                ),
            )
            .into_any_element()
    }
}

/// The pane's content keeps to a readable width; the panel and blocks don't
/// stretch across a wide window.
const PANE_MAX: gpui_kit::Rems = gpui_kit::rems(48.);

/// The page's inset (32; `p_8`).
const PAGE_INSET: gpui_kit::Rems = gpui_kit::rems(2.);

impl ObjectTools for BookDetailView {
    fn object_tools(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.tools_in_bar {
            return Vec::new();
        }
        match self.shown_book(cx) {
            Some((book, entry)) => self.render_tools(&book, entry.as_ref(), cx),
            None => Vec::new(),
        }
    }
}

impl Render for BookDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((book, entry)) = self.shown_book(cx) else {
            self.load_cover(None, window, cx);
            return EmptyState::new(IconName::BookOpen, "Select a book to see its details")
                .into_any_element();
        };
        let details = match self.details.get(&book.id) {
            Some(Details::Loaded(d)) => Some(d.clone()),
            _ => None,
        };
        if self.tools_in_bar {
            self.render_page(&book, entry.as_ref(), details.as_deref(), window, cx)
        } else {
            self.render_pane(&book, entry.as_ref(), details.as_deref(), window, cx)
        }
    }
}
