//! The book detail pane, shared by the library and search destinations.
//!
//! Shows a library entry (with the "your book" controls and its reads) or a
//! book that is not in the library (with `Add to library`). Controls reflect the
//! entry in the [`LibraryModel`]; a write disables them, and on success the
//! returned entry replaces the model's. On failure a notification explains it
//! and the controls are reset to the model's (unchanged) values. A notes draft
//! is the exception: it is kept until saved or discarded.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::form::{Field, Form};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::link::Link;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, StyledExt as _,
    WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Axis, Context, ElementId, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Pixels, Render,
    RenderImage, SharedString, Styled as _, StyledImage as _, Subscription, Task, Window, div, img,
    prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::{READ_CATALOG, WRITE_LIBRARY};
use hardcover_core::{
    BookDetails, BookId, BookSummary, Client, Date, EditionChange, EditionSummary, Error, Length,
    LibraryEntry, Privacy, ProgressUpdate, Rating, ReadId, ReadSession, ReadingStatus, UserBookId,
};

use self::catalog_sections::Similar;
use super::edition_picker::{EditionChosen, EditionPicker};
use super::read_dialog::{ReadDatesChosen, ReadDialog, ReadDialogKind};
use super::star_rating::StarRating;
use crate::app_state::{Services, cover_image, release_cover_image};
use crate::format::{
    average_rating_text, duration_text, edition_text, error_message, parse_position, position_text,
    progress_summary, read_dates_text, series_position_text, status_label,
};
use crate::library::model::{LibraryModel, WriteKind};

mod catalog_sections;

/// What the pane shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    /// An entry in the library, by id (it follows status changes).
    Entry(UserBookId),
    /// A book from search; shown as an entry if it is in the library.
    Book(BookSummary),
}

/// A status in the status controls.
#[derive(Clone)]
pub struct StatusOption {
    status: ReadingStatus,
    label: SharedString,
}

impl SelectItem for StatusOption {
    type Value = ReadingStatus;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &ReadingStatus {
        &self.status
    }
}

fn status_options() -> Vec<StatusOption> {
    ReadingStatus::ALL
        .into_iter()
        .map(|status| StatusOption {
            status,
            label: status_label(status).into(),
        })
        .collect()
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

/// The entry values the controls currently show.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    id: UserBookId,
    status: ReadingStatus,
    privacy: Option<Privacy>,
    /// What the progress field shows: a page, or "h:mm" for an audiobook.
    progress: Option<String>,
    audio: bool,
    started: Option<Date>,
    finished: Option<Date>,
}

impl Shown {
    fn of(entry: &LibraryEntry) -> Self {
        let read = entry.current_read();
        let progress = progress_summary(entry);
        Self {
            id: entry.user_book_id,
            status: entry.status,
            privacy: entry.privacy,
            progress: match progress.is_audio() {
                true => progress.seconds.map(position_text),
                false => progress.page.map(|p| p.to_string()),
            },
            audio: progress.is_audio(),
            started: read.and_then(|r| r.started_at),
            finished: read.and_then(|r| r.finished_at),
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
    status: Entity<SelectState<Vec<StatusOption>>>,
    add_status: Entity<SelectState<Vec<StatusOption>>>,
    page: Entity<InputState>,
    page_error: Option<SharedString>,
    started: Entity<DatePickerState>,
    finished: Entity<DatePickerState>,
    privacy: Entity<SelectState<Vec<PrivacyOption>>>,
    notes: Entity<TextareaState>,
    /// The entry and saved notes the notes field was last filled from.
    notes_base: Option<(UserBookId, Option<String>)>,
    /// Unsaved notes of entries no longer shown, restored when they are.
    notes_drafts: HashMap<UserBookId, String>,
    /// The entry whose notes were just saved, until the field changes again.
    notes_saved: Option<UserBookId>,
    /// One picker per book, so editions are fetched once per run.
    edition_pickers: HashMap<BookId, (Entity<EditionPicker>, Subscription)>,
    read_dialog: Option<(Entity<ReadDialog>, Subscription)>,
    /// The entry being added to the Owned list.
    owning: Option<UserBookId>,
    shown: Option<Shown>,
    /// The cover URL last requested and its image once loaded, released when
    /// the pane moves to another cover.
    cover: Option<(String, Option<Arc<RenderImage>>)>,
    similar: HashMap<BookId, Similar>,
    similar_loading: Option<BookId>,
    _similar_task: Option<Task<()>>,
    /// "Show all tags" was pressed for the book shown.
    tags_expanded: bool,
    _subscriptions: Vec<Subscription>,
}

impl BookDetailView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let status = cx.new(|cx| SelectState::new(status_options(), None, window, cx));
        let add_status =
            cx.new(|cx| SelectState::new(status_options(), Some(IndexPath::new(0)), window, cx));
        let page = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Page")
                .validate(|text, _| {
                    text.chars().all(|c| c.is_ascii_digit() || c == ':') && text.len() <= 8
                })
        });
        let privacy = cx.new(|cx| SelectState::new(privacy_options(), None, window, cx));
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Only you can see these notes")
                .auto_grow(3, 10)
        });
        let started = cx.new(|cx| DatePickerState::new(window, cx).date_format("%b %-d, %Y"));
        let finished = cx.new(|cx| DatePickerState::new(window, cx).date_format("%b %-d, %Y"));

        let subscriptions =
            vec![
                cx.observe_in(&model, window, |this, _, window, cx| {
                    this.sync_controls(window, cx);
                    cx.notify();
                }),
                cx.subscribe_in(
                    &status,
                    window,
                    |this, _, event: &SelectEvent<Vec<StatusOption>>, window, cx| {
                        if let SelectEvent::Confirm(Some(status)) = event {
                            this.change_status(*status, window, cx);
                        }
                    },
                ),
                cx.subscribe_in(
                    &page,
                    window,
                    |this, _, event: &InputEvent, window, cx| match event {
                        InputEvent::PressEnter { .. } => this.submit_progress(window, cx),
                        InputEvent::Change if this.page_error.take().is_some() => cx.notify(),
                        _ => {}
                    },
                ),
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
                cx.subscribe_in(
                    &started,
                    window,
                    |this, _, event: &DatePickerEvent, window, cx| {
                        let DatePickerEvent::Change(value) = event;
                        this.change_dates(Some(value.date().start()), None, window, cx);
                    },
                ),
                cx.subscribe_in(
                    &finished,
                    window,
                    |this, _, event: &DatePickerEvent, window, cx| {
                        let DatePickerEvent::Change(value) = event;
                        this.change_dates(None, Some(value.date().start()), window, cx);
                    },
                ),
            ];

        Self {
            model,
            subject: None,
            details: HashMap::new(),
            details_loading: None,
            details_fetching: None,
            _details_task: None,
            status,
            add_status,
            page,
            page_error: None,
            started,
            finished,
            privacy,
            notes,
            notes_base: None,
            notes_drafts: HashMap::new(),
            notes_saved: None,
            edition_pickers: HashMap::new(),
            read_dialog: None,
            owning: None,
            shown: None,
            cover: None,
            similar: HashMap::new(),
            similar_loading: None,
            _similar_task: None,
            tags_expanded: false,
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
        self.page_error = None;
        self.shown = None;
        self.sync_controls(window, cx);
        self.load_details(cx);
        self.tags_expanded = false;
        // Drops a pending similar-books fetch for the book left behind.
        self._similar_task = None;
        self.similar_loading = None;
        self.load_similar(cx);
        cx.notify();
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
        let Some(next) = next else {
            self.shown = None;
            return;
        };
        if self.shown.as_ref() == Some(&next) {
            return;
        }
        let previous = self.shown.replace(next.clone());
        self.status
            .update(cx, |s, cx| s.set_selected_value(&next.status, window, cx));
        self.privacy.update(cx, |s, cx| match &next.privacy {
            Some(p) => s.set_selected_value(p, window, cx),
            None => s.set_selected_index(None, window, cx),
        });
        let progress_changed = previous
            .as_ref()
            .is_none_or(|p| p.id != next.id || p.progress != next.progress);
        if progress_changed {
            let text = next.progress.clone().unwrap_or_default();
            self.page.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        if previous.as_ref().is_none_or(|p| p.audio != next.audio) {
            let placeholder = if next.audio { "h:mm" } else { "Page" };
            self.page
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        self.started.update(cx, |s, cx| {
            s.set_date(CalendarDate::Single(next.started), window, cx)
        });
        self.finished.update(cx, |s, cx| {
            s.set_date(CalendarDate::Single(next.finished), window, cx)
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
            if kind == WriteKind::Notes {
                window.push_notification(
                    Notification::info(
                        "Another change to this book is still being saved. Try again in a moment.",
                    )
                    .title("Notes not saved yet"),
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
                    Ok(Written {
                        entry: Some(entry), ..
                    }) => m.apply_written(entry.clone(), cx),
                    Ok(Written { entry: None, .. }) => m.apply_removed(id, cx),
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
                        cx.notify();
                    }
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(Services::error_message(
                            &error,
                            Some(WRITE_LIBRARY),
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

    fn submit_progress(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let text = self.page.read(cx).value();
        let update = match entry.length() {
            Some(Length::Seconds(total)) => match parse_position(&text) {
                None => Err("Enter the time you're at, like 2:05.".to_owned()),
                Some(s) if s > total => {
                    Err(format!("This edition is {} long.", duration_text(total)))
                }
                Some(s) => Ok(ProgressUpdate::Seconds(s)),
            },
            length => match text.trim().parse::<u32>() {
                Err(_) => Err("Enter the page you're on.".to_owned()),
                Ok(page) => match length {
                    Some(Length::Pages(total)) if page > total => {
                        Err(format!("This edition has {total} pages."))
                    }
                    _ => Ok(ProgressUpdate::Pages(page)),
                },
            },
        };
        let update = match update {
            Ok(update) => update,
            Err(error) => {
                self.page_error = Some(error.into());
                cx.notify();
                return;
            }
        };
        self.page_error = None;
        let id = entry.user_book_id;
        let read = entry.open_read().map(|r| r.id);
        self.run_write(
            id,
            WriteKind::Progress,
            "Couldn't update progress",
            move |client| client.update_progress(id, read, update).map(Some),
            window,
            cx,
        );
    }

    /// Changes one date of the current read; `None` leaves that date as it is
    /// in the model, `Some(None)` clears it.
    fn change_dates(
        &mut self,
        started: Option<Option<Date>>,
        finished: Option<Option<Date>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let Some(read) = entry.current_read().cloned() else {
            return;
        };
        // `set_read_dates` sets both dates, so the other one comes from the model.
        let started = started.unwrap_or(read.started_at);
        let finished = finished.unwrap_or(read.finished_at);
        if started == read.started_at && finished == read.finished_at {
            return;
        }
        if let (Some(s), Some(f)) = (started, finished)
            && f < s
        {
            window.push_notification(
                Notification::warning("The finish date can't be before the start date.")
                    .title("Dates not changed"),
                cx,
            );
            self.revert_controls(window, cx);
            return;
        }
        let id = entry.user_book_id;
        self.run_write(
            id,
            WriteKind::Dates,
            "Couldn't change the dates",
            move |client| client.set_read_dates(read.id, started, finished).map(Some),
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
        cx.notify();
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
                    WriteKind::Reads,
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
        window.open_alert_dialog(cx, move |alert, _, _| {
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

    fn confirm_mark_owned(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let Some(edition) = entry.edition.clone() else {
            return;
        };
        let id = entry.user_book_id;
        let title: SharedString = format!("Add “{}” to your Owned list?", entry.book.title).into();
        let which = edition
            .format
            .as_ref()
            .or(edition.reading_format.as_ref())
            .map_or_else(
                || "This edition".to_owned(),
                |format| format!("The {} edition", format.to_lowercase()),
            );
        let description: SharedString = format!(
            "{which} goes on your Owned list on Hardcover and appears in your activity, \
             which others may see even if this book is private. To take it off the list, \
             use the Hardcover website."
        )
        .into();
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let edition = edition.id;
            alert
                .title(title.clone())
                .description(description.clone())
                .confirm()
                .ok_text("Add")
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.mark_owned(id, edition, window, cx));
                    true
                })
        });
    }

    /// Not a library write: Hardcover keeps ownership on a list, and the
    /// entry does not change.
    fn mark_owned(
        &mut self,
        id: UserBookId,
        edition: hardcover_core::EditionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.owning.is_some() {
            return;
        }
        self.owning = Some(id);
        cx.notify();
        let client = Services::global(cx).client().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.mark_owned(edition) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.owning = None;
                let note = match result {
                    Ok(()) => Notification::success("Added to your Owned list on Hardcover."),
                    Err(e) => Notification::error(Services::error_message(&e, None, cx))
                        .title("Couldn't add it to your Owned list"),
                };
                window.push_notification(note, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        let id = entry.user_book_id;
        let title: SharedString = format!("Remove “{}”?", entry.book.title).into();
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
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

    fn add_to_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Subject::Book(book)) = self.subject.clone() else {
            return;
        };
        let status = self
            .add_status
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(ReadingStatus::WantToRead);
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

    // ----- rendering -----

    /// Loads the cover for `url`, releasing the previous cover if it changed.
    fn load_cover(
        &mut self,
        url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        if self.cover.as_ref().map(|(u, _)| u) != url.as_ref()
            && let Some((old, image)) = self.cover.take()
        {
            release_cover_image(&old, image, window, cx);
        }
        let url = url?;
        let image = cover_image(&url, window, cx);
        self.cover = Some((url, image.clone()));
        image
    }

    fn render_header(
        &mut self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        details: Option<&BookDetails>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let cover_url = entry
            .and_then(|e| e.cover())
            .or(book.cover.as_ref())
            .map(|c| c.url.clone());
        let cover = self.load_cover(cover_url, window, cx);

        let mut facts: Vec<String> = Vec::new();
        let series = details.and_then(|d| d.series.first());
        let series_link = series.and_then(|s| self.render_series_link(s));
        if let Some(series) = series.filter(|_| series_link.is_none()) {
            facts.push(match series.position {
                Some(p) => format!("{} {}", series.name, series_position_text(p)),
                None => series.name.clone(),
            });
        }
        if let Some(year) = book
            .release_year
            .or_else(|| details.and_then(|d| d.release_date.map(|d| chrono::Datelike::year(&d))))
        {
            facts.push(year.to_string());
        }
        match entry
            .and_then(LibraryEntry::length)
            .or(book.pages.map(Length::Pages))
        {
            Some(Length::Pages(pages)) => facts.push(format!("{pages} pages")),
            Some(Length::Seconds(seconds)) => facts.push(duration_text(seconds)),
            None => {}
        }
        let average = book
            .average_rating
            .or_else(|| details.and_then(|d| d.summary.average_rating))
            .filter(|a| *a > 0.0);

        // Wraps so a narrow pane puts the text under the cover instead of
        // squeezing it beside.
        h_flex()
            .items_start()
            .gap_2()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .flex_wrap()
                    .items_start()
                    .gap_6()
                    .child(
                        div()
                            .flex_shrink_0()
                            .w_32()
                            .h_48()
                            .rounded(cx.theme().radius_lg)
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .map(|this| match cover {
                                Some(image) => {
                                    this.child(img(image).size_full().object_fit(ObjectFit::Cover))
                                }
                                None => this
                                    .child(Icon::new(IconName::BookOpen).large().text_color(muted)),
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(rems(12.))
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(book.title.clone()),
                            )
                            .when_some(book.subtitle.clone(), |this, subtitle| {
                                this.child(div().text_lg().text_color(muted).child(subtitle))
                            })
                            .child(self.render_contributors(book, details, cx))
                            .when(series_link.is_some() || !facts.is_empty(), |this| {
                                this.child(
                                    h_flex()
                                        .flex_wrap()
                                        .gap_1()
                                        .text_sm()
                                        .text_color(muted)
                                        .when_some(series_link, |this, link| {
                                            this.child(link)
                                                .when(!facts.is_empty(), |this| this.child("·"))
                                        })
                                        .when(!facts.is_empty(), |this| {
                                            this.child(facts.join(" · "))
                                        }),
                                )
                            })
                            .when_some(average, |this, avg| {
                                this.child(
                                    h_flex()
                                        .gap_1()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(Icon::new(IconName::StarFill).xsmall())
                                        .child(format!(
                                            "{} average on Hardcover",
                                            average_rating_text(avg)
                                        )),
                                )
                            })
                            .when_some(book.web_url(), |this, url| {
                                this.child(
                                    div().pt_2().child(
                                        Link::new("open-on-hardcover")
                                            .href(url)
                                            .text_sm()
                                            .child("Open on Hardcover"),
                                    ),
                                )
                            }),
                    ),
            )
            .when_some(entry, |this, entry| {
                let busy = self
                    .model
                    .read(cx)
                    .pending_write(entry.user_book_id)
                    .is_some();
                let has_edition = entry.edition.is_some();
                let owning = self.owning.is_some();
                let view = cx.entity().downgrade();
                this.child(
                    Button::new("book-more")
                        .ghost()
                        .icon(IconName::Ellipsis)
                        .tooltip("More actions")
                        .disabled(busy)
                        .dropdown_menu(move |menu, _, _| {
                            let own_view = view.clone();
                            let remove_view = view.clone();
                            // Ownership is per edition on Hardcover.
                            let own_label = if has_edition {
                                "Add to Owned list…"
                            } else {
                                "Add to Owned list (choose an edition first)"
                            };
                            menu.item(
                                PopupMenuItem::new(own_label)
                                    .disabled(!has_edition || owning)
                                    .on_click(move |_, window, cx| {
                                        let _ = own_view.update(cx, |this, cx| {
                                            this.confirm_mark_owned(window, cx)
                                        });
                                    }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("Remove from library…")
                                    .icon(IconName::Delete)
                                    .on_click(move |_, window, cx| {
                                        let _ = remove_view
                                            .update(cx, |this, cx| this.confirm_remove(window, cx));
                                    }),
                            )
                        }),
                )
            })
    }

    fn render_your_book(
        &self,
        entry: &LibraryEntry,
        label_width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let busy = pending.is_some();
        let rating = entry.rating;
        section("Your book").child(
            Form::new()
                .label_layout(Axis::Horizontal)
                .label_width(label_width)
                .child(
                    Field::new().label("Status").child(
                        div()
                            .w_full()
                            .max_w_56()
                            .child(Select::new(&self.status).disabled(busy)),
                    ),
                )
                .child(
                    Field::new().label("Rating").child(
                        h_flex()
                            .gap_2()
                            .child(
                                StarRating::new("your-rating")
                                    .value(rating)
                                    .disabled(busy)
                                    .on_change(cx.listener(
                                        |this, rating: &Option<Rating>, window, cx| {
                                            this.change_rating(*rating, window, cx)
                                        },
                                    )),
                            )
                            .when_some(rating, |this, r| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::format::rating_text(r)),
                                )
                            })
                            .when(rating.is_some(), |this| {
                                this.child(
                                    Button::new("clear-rating")
                                        .ghost()
                                        .xsmall()
                                        .label("Clear")
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.change_rating(None, window, cx)
                                        })),
                                )
                            })
                            .when(pending == Some(WriteKind::Rating), |this| {
                                this.child(saving(cx))
                            }),
                    ),
                )
                .child(
                    Field::new()
                        .label("Edition")
                        .child(self.render_edition(entry, cx)),
                )
                .child(
                    Field::new().label("Privacy").child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div().w_full().max_w_56().child(
                                    Select::new(&self.privacy)
                                        .placeholder("Unknown")
                                        .disabled(busy),
                                ),
                            )
                            .when(pending == Some(WriteKind::Privacy), |this| {
                                this.child(saving(cx))
                            }),
                    ),
                )
                .child(
                    Field::new()
                        .label("Notes")
                        .child(self.render_notes(entry, cx)),
                ),
        )
    }

    fn render_edition(&self, entry: &LibraryEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let muted = cx.theme().muted_foreground;
        let (text, chosen) = match &entry.edition {
            Some(edition) => (
                edition_text(edition)
                    .or_else(|| edition.title.clone())
                    .unwrap_or_else(|| "Edition chosen".to_owned()),
                true,
            ),
            None => ("Not chosen".to_owned(), false),
        };
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .child(
                div()
                    .min_w_0()
                    .text_sm()
                    .when(!chosen, |this| this.text_color(muted))
                    .child(text),
            )
            .child(
                Button::new("change-edition")
                    .outline()
                    .small()
                    .label(if chosen { "Change…" } else { "Choose…" })
                    .loading(pending == Some(WriteKind::Edition))
                    .disabled(pending.is_some())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_edition_picker(window, cx)),
                    ),
            )
    }

    fn render_notes(&self, entry: &LibraryEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let saving_notes = pending == Some(WriteKind::Notes);
        let dirty = self.notes_dirty(cx);
        let saved = !dirty && self.notes_saved == Some(entry.user_book_id);
        let muted = cx.theme().muted_foreground;
        v_flex()
            .w_full()
            .gap_2()
            .child(Textarea::new(&self.notes).disabled(saving_notes))
            .when(dirty || saved || saving_notes, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .when(dirty, |this| {
                            this.child(
                                Button::new("save-notes")
                                    .small()
                                    .label("Save")
                                    .tooltip("Ctrl+Enter")
                                    .loading(saving_notes)
                                    .disabled(pending.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_notes(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("discard-notes")
                                    .ghost()
                                    .small()
                                    .label("Discard")
                                    .disabled(saving_notes)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.discard_notes(window, cx)
                                    })),
                            )
                            .when(!saving_notes, |this| {
                                this.child(div().text_xs().text_color(muted).child("Not saved"))
                            })
                        })
                        .when(saved, |this| {
                            this.child(
                                h_flex()
                                    .gap_1()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(Icon::new(IconName::Check).xsmall())
                                    .child("Saved"),
                            )
                        }),
                )
            })
    }

    fn render_reads(&self, entry: &LibraryEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let muted = cx.theme().muted_foreground;
        let current = entry.current_read().map(|r| r.id);
        let view = cx.entity().downgrade();
        let rows = entry.reads.iter().map(|read| {
            let mut details: Vec<String> = Vec::new();
            if !read.is_finished() {
                details.push("Not finished".to_owned());
            }
            if Some(read.id) == current && entry.reads.len() > 1 {
                details.push("Current read".to_owned());
            }
            if let Some(text) = read.edition.as_ref().and_then(edition_text) {
                details.push(text);
            }
            let edit_view = view.clone();
            let delete_view = view.clone();
            let session = read.clone();
            let read_id = read.id;
            h_flex()
                .gap_2()
                .py_1()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .child(read_dates_text(read.started_at, read.finished_at)),
                        )
                        .when(!details.is_empty(), |this| {
                            this.child(div().text_xs().text_color(muted).child(details.join(" · ")))
                        }),
                )
                .child(
                    Button::new(ElementId::Name(format!("read-{}", read.id).into()))
                        .ghost()
                        .small()
                        .icon(IconName::Ellipsis)
                        .tooltip("Read actions")
                        .disabled(pending.is_some())
                        .dropdown_menu(move |menu, _, _| {
                            let edit_view = edit_view.clone();
                            let delete_view = delete_view.clone();
                            let session = session.clone();
                            menu.item(
                                PopupMenuItem::new("Change dates…")
                                    .icon(IconName::Calendar)
                                    .on_click(move |_, window, cx| {
                                        let session = session.clone();
                                        let _ = edit_view.update(cx, |this, cx| {
                                            this.open_read_dialog(Some(session), window, cx)
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new("Delete read…")
                                    .icon(IconName::Delete)
                                    .on_click(move |_, window, cx| {
                                        let _ = delete_view.update(cx, |this, cx| {
                                            this.confirm_delete_read(read_id, window, cx)
                                        });
                                    }),
                            )
                        }),
                )
        });
        section("Reads")
            .child(
                v_flex()
                    .gap_1()
                    .children(rows)
                    .when(entry.reads.is_empty(), |this| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child("No reads recorded."),
                        )
                    }),
            )
            .child(
                h_flex().gap_2().child(
                    Button::new("add-read")
                        .small()
                        .icon(IconName::Plus)
                        .label("Add a read…")
                        .loading(pending == Some(WriteKind::Reads))
                        .disabled(pending.is_some())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_read_dialog(None, window, cx)
                        })),
                ),
            )
    }

    fn render_progress(&self, entry: &LibraryEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let progress = progress_summary(entry);
        let muted = cx.theme().muted_foreground;
        let mut summary = progress.text();
        if let (Some(pct), Some(_)) = (progress.percent, progress.page) {
            summary.push_str(&format!(" · {}%", pct.round() as u32));
        }
        section("Progress")
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child(summary))
                    .child(
                        Progress::new("reading-progress")
                            .value(progress.percent.unwrap_or(0.0))
                            .accessibility_label("Reading progress"),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Input::new(&self.page).w_24().disabled(pending.is_some()))
                    .when_some(progress.total_pages, |this, total| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(format!("of {total}")),
                        )
                    })
                    .child(
                        Button::new("update-progress")
                            .primary()
                            .label("Update progress")
                            .loading(pending == Some(WriteKind::Progress))
                            .disabled(pending.is_some())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit_progress(window, cx)),
                            ),
                    ),
            )
            .when_some(self.page_error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
    }

    fn render_dates(
        &self,
        entry: &LibraryEntry,
        label_width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self
            .model
            .read(cx)
            .pending_write(entry.user_book_id)
            .is_some();
        section("Reading dates").child(
            Form::new()
                .label_layout(Axis::Horizontal)
                .label_width(label_width)
                .child(
                    Field::new().label("Started").child(
                        div().w_full().max_w_56().child(
                            DatePicker::new(&self.started)
                                .placeholder("Not set")
                                .cleanable(true)
                                .disabled(busy),
                        ),
                    ),
                )
                .child(
                    Field::new().label("Finished").child(
                        div().w_full().max_w_56().child(
                            DatePicker::new(&self.finished)
                                .placeholder("Not set")
                                .cleanable(true)
                                .disabled(busy),
                        ),
                    ),
                ),
        )
    }

    fn render_add(&self, book: &BookSummary, cx: &mut Context<Self>) -> impl IntoElement {
        let adding = self.model.read(cx).is_adding(book.id);
        section("Not in your library").child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w_56()
                        .child(Select::new(&self.add_status).disabled(adding)),
                )
                .child(
                    Button::new("add-to-library")
                        .icon(IconName::Plus)
                        .label("Add to library")
                        .loading(adding)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_to_library(window, cx)),
                        ),
                ),
        )
    }

    fn render_description(&self, book: &BookSummary, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let body: AnyElement = match self.details.get(&book.id) {
            Some(Details::Loaded(details)) => match details.description.as_deref().map(str::trim) {
                Some(text) if !text.is_empty() => v_flex()
                    .gap_2()
                    .text_sm()
                    .children(
                        text.split('\n')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(|p| div().child(p.to_owned())),
                    )
                    .into_any_element(),
                _ => div()
                    .text_sm()
                    .text_color(muted)
                    .child("No description on Hardcover.")
                    .into_any_element(),
            },
            Some(Details::Failed(message)) => h_flex()
                .gap_2()
                .text_sm()
                .child(div().text_color(muted).child(message.clone()))
                .child(
                    Button::new("retry-details")
                        .small()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_details(cx))),
                )
                .into_any_element(),
            None if self.details_fetching == Some(book.id) => v_flex()
                .gap_2()
                .child(Skeleton::new().h_4().w_full())
                .child(Skeleton::new().h_4().w_full())
                .child(Skeleton::new().h_4().w_2_3())
                .into_any_element(),
            // Waiting out the delay: the same placeholder, still.
            None => {
                let bar = || div().h_4().bg(cx.theme().skeleton);
                v_flex()
                    .gap_2()
                    .child(bar().w_full())
                    .child(bar().w_full())
                    .child(bar().w_2_3())
                    .into_any_element()
            }
        };
        section("Description").child(body).into_any_element()
    }
}

/// "Saving…" beside a control whose write is in flight.
fn saving(cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child("Saving…")
}

/// A titled region of the pane.
/// The outcome of a write, for [`BookDetailView::run_write`]: the entry to
/// apply (`None` when it was removed) and a warning to show despite success.
struct Written {
    entry: Option<LibraryEntry>,
    warning: Option<(&'static str, String)>,
}

impl From<Option<LibraryEntry>> for Written {
    fn from(entry: Option<LibraryEntry>) -> Self {
        Self {
            entry,
            warning: None,
        }
    }
}

impl From<EditionChange> for Written {
    fn from(change: EditionChange) -> Self {
        Self {
            entry: Some(change.entry),
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

fn section(title: &'static str) -> gpui_kit::Div {
    v_flex()
        .gap_3()
        .child(div().text_sm().font_semibold().child(title))
}

impl Render for BookDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let entry = self.entry(model).cloned();
        let book = match (&self.subject, &entry) {
            (_, Some(entry)) => Some(entry.book.clone()),
            (Some(Subject::Book(book)), None) => Some(book.clone()),
            _ => None,
        };
        let Some(book) = book else {
            self.load_cover(None, window, cx);
            return v_flex()
                .id("book-detail-empty")
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::BookOpen).large())
                .child(div().text_sm().child("Select a book to see its details"))
                .into_any_element();
        };
        let details = match self.details.get(&book.id) {
            Some(Details::Loaded(d)) => Some(d.clone()),
            _ => None,
        };

        // Form label widths take pixels; derive them from the rem size.
        let label_width = window.rem_size() * 6.;
        let header = self.render_header(&book, entry.as_ref(), details.as_deref(), window, cx);
        v_flex()
            // Keyed by book, so another book (e.g. a similar one) opens at the top.
            .id(("book-detail", book.id.get() as u64))
            .size_full()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(48.))
                    .p_6()
                    .gap_6()
                    .child(header)
                    .map(|this| match &entry {
                        Some(entry) => this
                            .child(self.render_your_book(entry, label_width, cx))
                            .when(entry.status == ReadingStatus::CurrentlyReading, |this| {
                                this.child(self.render_progress(entry, cx))
                            })
                            .when(entry.current_read().is_some(), |this| {
                                this.child(self.render_dates(entry, label_width, cx))
                            })
                            .child(self.render_reads(entry, cx)),
                        None => this.child(self.render_add(&book, cx)),
                    })
                    .child(self.render_description(&book, cx))
                    .children(self.render_catalog_sections(&book, details.as_deref(), window, cx)),
            )
            .into_any_element()
    }
}
