//! The book: the pane beside a shelf, a list or search results, and the full
//! page over a destination (`hardcover-design`, Screens → Shelf and Full book
//! page). One view does both; [`BookDetailView::set_tools_in_bar`] makes it
//! the page.
//!
//! - **Pane:** its own object bar (no Back; "Lists" and "more" trailing),
//!   then, inset 24: the header (xl cover, the title in `display-24`), the
//!   progress or Not started panel, and the blocks, in one column.
//! - **Wide pane:** when the pane's own width fits a main column of at least
//!   `size::MAIN_MIN` beside the rail ([`pane_has_rail`]), it takes the
//!   page's columns (the same builder, [`BookDetailView::render_columns`])
//!   with the pane's header and inset. The pane measures itself as it is
//!   laid out (it sits beside a resizable list), so a change of layout
//!   shows from the next frame.
//! - **Page:** the shell draws the bar (Back and this view's
//!   [`ObjectTools`]). Inset 32: a main column (xxl cover, `display-30`,
//!   the panel, About this book, Your review, Reviews, Similar books) and the
//!   rail (On Hardcover, Details, Reads, Notes), which folds under the main
//!   column in a narrow window (`ui::Columns`).
//! - **Width:** the pane and the page fill their region, and the pane's
//!   tools sit at its trailing edge. Only reading text (the description,
//!   reviews) keeps its measure, `size::PROSE`. When the main column holds
//!   that text and a side column of `size::SIDE_MIN` ([`side_width`]), the
//!   blocks under the panel split, 32 apart: About this book, Your review
//!   and Reviews at `size::PROSE`; tags, other titles and Similar books (a
//!   grid of lg covers) in the side column, filling the rest.
//!
//! Shows a library entry (with your book's controls and its reads) or a book
//! that is not in the library (with "Add to library"). Controls reflect the
//! entry in the [`LibraryModel`]; a write disables them, and on success the
//! returned entry replaces the model's. On failure a notification explains it
//! and the controls are reset to the model's (unchanged) values. A notes or
//! review draft is the exception: it is kept until saved or discarded.
//!
//! What the view shows of the catalogue (details, similar books, other
//! readers' reviews, editions) and the entry's own review come from the
//! session's stores (`resource::Catalog`, `LibraryModel::reviews`), shared
//! with every other book view: the view asks for its book's ([`Demand`]),
//! waiting a moment after the book changes so arrowing through a list asks
//! for nothing, and keeps only UI state.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::ui::reveal::RevealScroll as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{ElementExt as _, IconName, WindowExt as _, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, RenderImage, SharedString, Styled as _,
    Subscription, Task, Window, div,
};
use hardcover_core::{
    BookDetails, BookId, BookSummary, LibraryEntry, Privacy, Rating, ReadId, ReadSession,
    ReadingStatus, UserBookId,
};

use self::community_reviews::CommunityReviews;
use self::review_section::ReviewEditor;
use super::edition_picker::{EditionChosen, EditionPicker};
use super::read_dialog::{ReadDatesChosen, ReadDialog, ReadDialogKind};
use crate::app_state::release_cover_medium;
use crate::design::size;
use crate::format::read_dates_text;
use crate::library::model::{
    LibraryModel, LibraryWrite, Removed, WriteKind, WriteOutcome, WriteTarget, Written,
};
use crate::library::writes::{self, Ratings};
use crate::lists::book_lists;
use crate::lists::model::ListsModel;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::kit::open_alert_dialog;
use crate::ui::{
    Columns, EmptyState, ObjectBar, ObjectTools, Tier, blocks, folds, main_column_width,
    region_width,
};

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

/// Whether a book pane `width` wide takes the page's rail: inside its inset,
/// a main column of at least `size::MAIN_MIN` fits beside the rail (the
/// test `ui::Columns` folds by). Pure, from the width the pane itself gets.
pub fn pane_has_rail(width: Pixels, rem_size: Pixels) -> bool {
    !folds(
        width - size::PANE_INSET.to_pixels(rem_size) * 2.,
        rem_size,
        false,
    )
}

/// Between the book's text and the side column beside it (32).
const SPLIT_GAP: gpui_kit::Rems = gpui_kit::rems(2.);

/// The side column's width beside the book's text in a main column `main`
/// wide, when the blocks split: the text keeps `size::PROSE` and the side
/// gets the rest past the gap, at least `size::SIDE_MIN`. `None`: one column.
pub fn side_width(main: Pixels, rem_size: Pixels) -> Option<Pixels> {
    let side = main - size::PROSE.to_pixels(rem_size) - SPLIT_GAP.to_pixels(rem_size);
    (side >= size::SIDE_MIN.to_pixels(rem_size)).then_some(side)
}

/// How the pane lays out at the width it was last measured at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PaneLayout {
    /// The page's columns, with the rail ([`pane_has_rail`]).
    rail: bool,
    /// The blocks split beside the text: the similar books the side holds.
    side: Option<catalog_sections::SideGrid>,
}

impl PaneLayout {
    fn at(width: Pixels, rem_size: Pixels) -> Self {
        let rail = pane_has_rail(width, rem_size);
        let side = rail
            .then(|| {
                let columns = width - size::PANE_INSET.to_pixels(rem_size) * 2.;
                side_width(main_column_width(columns, rem_size, false, false), rem_size)
            })
            .flatten()
            .map(|side| catalog_sections::similar_in_side(side, rem_size));
        Self { rail, side }
    }
}

/// How long a book must stay shown before each thing it shows is asked
/// for, from when it was shown: arrowing through a list asks for nothing
/// about the books passed over, and a book settled on costs one request at
/// a time. Similar books also wait for the details, which name them.
const SETTLE: [(Duration, Fetch); 4] = [
    (Duration::from_millis(250), Fetch::Details),
    (Duration::from_millis(400), Fetch::OwnReview),
    (Duration::from_millis(600), Fetch::Similar),
    (Duration::from_millis(700), Fetch::Reviews),
];

/// One thing a book view asks the stores for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fetch {
    Details,
    /// The entry's own review.
    OwnReview,
    Similar,
    /// Other readers' reviews.
    Reviews,
}

pub struct BookDetailView {
    model: Entity<LibraryModel>,
    subject: Option<Subject>,
    /// The session's catalogue; `None` once signed out.
    catalog: Option<Catalog>,
    /// Asks for what the book shows, a step at a time ([`SETTLE`]).
    settle: Option<Task<()>>,
    /// The book whose similar books are due: asked for once its details
    /// land, if they weren't in yet.
    similar_due: Option<BookId>,
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
    /// The cover URLs of the similar books shown, released when the view
    /// moves to another book or goes away: browsing books would otherwise
    /// keep every lg cover it passed for the run.
    similar_covers: Vec<String>,
    /// "Show all tags" was pressed for the book shown.
    tags_expanded: bool,
    /// The full book page, under the object bar the shell draws: the book's
    /// tools go to that bar, and the page has its own layout (xxl cover,
    /// `display-30`, a rail). Otherwise the pane, which draws its own bar.
    tools_in_bar: bool,
    /// The pane's layout, as measured when it was last laid out.
    layout: Rc<Cell<PaneLayout>>,
    review: ReviewEditor,
    /// Other readers' reviews, a view of their own.
    community_reviews: Entity<CommunityReviews>,
    /// Ratings asked for and not yet saved.
    ratings: Entity<Ratings>,
    /// The status menu's trigger, which its commands and their
    /// confirmations give focus back to.
    status_focus: FocusHandle,
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
        let ratings = cx.new(|cx| Ratings::new(model.clone(), window, cx));
        let community_reviews = cx.new(|cx| CommunityReviews::new(model.clone(), cx));
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
        subscriptions.push(cx.observe(&ratings, |_, _, cx| cx.notify()));
        subscriptions.push(cx.on_release_in(window, |this: &mut Self, window, cx| {
            for url in this.similar_covers.drain(..) {
                release_cover_medium(&url, Some(window), cx);
            }
        }));
        // Restarting into an update asks first while this view holds drafts.
        let this = cx.entity().downgrade();
        crate::drafts::register(
            move |cx| this.upgrade().map(|v| v.read(cx).has_unsaved_drafts(cx)),
            cx,
        );
        // The list menu and the Owned switch follow the user's lists.
        if let Some(lists) = ListsModel::global(cx) {
            subscriptions.push(cx.observe(&lists, |_, _, cx| cx.notify()));
        }
        let catalog = Catalog::global(cx);
        if let Some(catalog) = &catalog {
            subscriptions.extend(Self::follow_catalog(catalog, cx));
        }

        Self {
            model,
            subject: None,
            catalog,
            settle: None,
            similar_due: None,
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
            similar_covers: Vec::new(),
            tags_expanded: false,
            tools_in_bar: false,
            layout: Rc::new(Cell::new(PaneLayout::default())),
            review,
            community_reviews,
            ratings,
            status_focus: cx.focus_handle(),
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
        if let (Some(Subject::Book(shown)), Some(Subject::Book(book))) = (&self.subject, &subject)
            && shown.id == book.id
        {
            // The same book with newer facts (its list's page was fetched
            // again): everything the pane shows of it stays as it is.
            self.subject = subject;
            cx.notify();
            return;
        }
        self.subject = subject;
        self.shown = None;
        self.sync_controls(window, cx);
        if let Some(book) = self.book(cx) {
            book_lists::book_shown(book.id, cx);
        }
        self.tags_expanded = false;
        self.settle(cx);
        cx.notify();
    }

    /// Asks for what the book shown shows, a step at a time ([`SETTLE`]);
    /// moving to another book first drops the steps not taken. What the
    /// stores already hold shows at once, and only what is missing or
    /// stale is fetched.
    fn settle(&mut self, cx: &mut Context<Self>) {
        self.similar_due = None;
        let Some(book) = self.book(cx).map(|b| b.id) else {
            self.settle = None;
            return;
        };
        self.settle = Some(cx.spawn(async move |this, cx| {
            let mut waited = Duration::ZERO;
            for (at, fetch) in SETTLE {
                cx.background_executor().timer(at - waited).await;
                waited = at;
                let asked = this.update(cx, |this, cx| this.fetch(book, fetch, Demand::Shown, cx));
                if asked.is_err() {
                    return;
                }
            }
        }));
    }

    /// Asks the stores for one thing `book` shows, if `demand` calls for it.
    fn fetch(&mut self, book: BookId, fetch: Fetch, demand: Demand, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        match fetch {
            Fetch::Details => catalog.request_details(book, demand, cx),
            Fetch::OwnReview => {
                self.review.asked = Some(book);
                if let Some(entry) = self.entry(self.model.read(cx)).map(|e| e.user_book_id) {
                    self.model
                        .update(cx, |m, cx| m.request_review(entry, demand, cx));
                }
            }
            Fetch::Similar => {
                self.similar_due = Some(book);
                catalog.request_similar(book, demand, cx);
            }
            Fetch::Reviews => self
                .community_reviews
                .update(cx, |reviews, cx| reviews.fetch(demand, cx)),
        }
    }

    /// Re-renders when the stores change for the book shown, and asks for
    /// its similar books once its details name them.
    fn follow_catalog(catalog: &Catalog, cx: &mut Context<Self>) -> Vec<Subscription> {
        vec![
            cx.subscribe(
                &catalog.details,
                |this, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if this.shows(*id, cx) {
                        if this.similar_due == Some(*id)
                            && let Some(catalog) = this.catalog.clone()
                        {
                            catalog.request_similar(*id, Demand::Active, cx);
                        }
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &catalog.similar,
                |this, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if this.shows(*id, cx) {
                        cx.notify();
                    }
                },
            ),
        ]
    }

    /// The "Reviews" block, left out without `read:social`.
    fn render_community_reviews(&self, cx: &App) -> Option<AnyElement> {
        CommunityReviews::visible(cx).then(|| self.community_reviews.clone().into_any_element())
    }

    /// The book shown is `book`.
    fn shows(&self, book: BookId, cx: &App) -> bool {
        self.book(cx).is_some_and(|b| b.id == book)
    }

    /// The book's details, if the catalogue has them.
    fn details(&self, book: BookId, cx: &App) -> Option<Arc<BookDetails>> {
        self.catalog
            .as_ref()?
            .details
            .read(cx)
            .resource()
            .value(&book)
            .cloned()
    }

    /// Asks again for whatever the view shows that is stale or missing
    /// (`resource`): the details, the entry's own review, similar books and
    /// other readers' reviews, keeping them on screen meanwhile. The shell
    /// calls it for the visible views only, so a view out of sight asks for
    /// nothing. `shown_again` (Refresh, or the page shown again) also tries
    /// a failed fetch again, and lets reviews the reader paged through be
    /// replaced by a fresh first page; without it (the window became active,
    /// a sync finished) neither happens.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        let Some(book) = self.book(cx).map(|b| b.id) else {
            return;
        };
        let demand = if shown_again {
            Demand::Shown
        } else {
            Demand::Active
        };
        for (_, fetch) in SETTLE {
            self.fetch(book, fetch, demand, cx);
        }
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
        let book = self.book(cx).map(|b| b.id);
        self.community_reviews
            .update(cx, |reviews, cx| reviews.set_book(book, cx));
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

    fn retry_details(&mut self, cx: &mut Context<Self>) {
        if let Some(book) = self.book(cx) {
            self.fetch(book.id, Fetch::Details, Demand::Now, cx);
        }
    }

    // ----- writes -----

    /// Runs one write on an entry through the library model, which applies the
    /// result whether or not this view still shows the book, and shows a
    /// failure. `warning` names what to say despite success.
    fn run_write<T: Written>(
        &mut self,
        mut write: LibraryWrite<T>,
        warning: impl FnOnce(&T) -> Option<(&'static str, &'static str)> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (kind, target) = (write.kind(), write.target());
        let WriteTarget::Entry(id) = target else {
            return;
        };
        // A review write needs write:reviews and, for its status,
        // write:library: an unnamed refusal can't be pinned on either.
        if kind == WriteKind::Review {
            write = write.any_scope();
        }
        let outcome = self.model.update(cx, |m, cx| m.write(write, window, cx));
        cx.spawn_in(window, async move |this, cx| {
            let outcome = outcome.await;
            let _ = this.update_in(cx, |this, window, cx| match outcome {
                WriteOutcome::Saved(written) => {
                    if let Some((title, message)) = warning(&written) {
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
                WriteOutcome::Failed { .. } | WriteOutcome::Busy => {
                    this.revert_controls(window, cx);
                }
                WriteOutcome::InLibrary | WriteOutcome::Ended => {}
            });
        })
        .detach();
    }

    fn current_entry(&self, cx: &App) -> Option<LibraryEntry> {
        self.entry(self.model.read(cx)).cloned()
    }

    /// Moves the entry to `status`, asking first when that starts or
    /// finishes a read (`writes::change_status`).
    fn change_status(
        &mut self,
        status: ReadingStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.entry(self.model.read(cx)).map(|e| e.user_book_id) else {
            return;
        };
        writes::change_status(&self.model, id, status, window, cx);
    }

    /// Saves a rating, now or once the write running on the entry ends
    /// ([`Ratings`]); the stars show it meanwhile.
    fn change_rating(
        &mut self,
        rating: Option<Rating>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.entry(self.model.read(cx)).map(|e| e.user_book_id) else {
            return;
        };
        self.ratings
            .update(cx, |r, cx| r.rate(id, rating, window, cx));
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
            LibraryWrite::entry(
                id,
                WriteKind::Privacy,
                "Couldn't change who can see this book",
                move |client| client.set_privacy(id, privacy),
            ),
            |_| None,
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
            LibraryWrite::entry(
                id,
                WriteKind::Notes,
                "Couldn't save your notes",
                move |client| client.set_private_notes(id, &text),
            ),
            |_| None,
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

    /// Opens the notes editor on the entry shown, with the cursor at the end
    /// of the notes, ready to add a line.
    fn edit_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        self.notes_editing = Some(entry.user_book_id);
        self.notes_saved = None;
        self.notes.update(cx, |s, cx| {
            let end = s.value().len();
            s.set_selected_range(end..end, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Whether this view holds words not saved: in the notes or review
    /// editor, or set aside for another book (`crate::drafts`).
    pub fn has_unsaved_drafts(&self, cx: &App) -> bool {
        self.notes_dirty(cx) || !self.notes_drafts.is_empty() || self.review_unsaved(cx)
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
        let picker = if let Some((picker, _)) = self.edition_pickers.get(&book) {
            picker.clone()
        } else {
            let title: SharedString = entry.book.title.clone().into();
            let picker = cx.new(|cx| EditionPicker::new(book, title, cx));
            let subscription = cx.subscribe_in(
                &picker,
                window,
                |this, _, EditionChosen(edition), window, cx| {
                    this.change_edition(edition.id, window, cx);
                },
            );
            self.edition_pickers
                .insert(book, (picker.clone(), subscription));
            picker
        };
        let current = entry.edition.as_ref().map(|e| e.id);
        picker.update(cx, |p, cx| p.set_current(current, cx));
        EditionPicker::open(picker, window, cx);
    }

    fn change_edition(
        &mut self,
        edition: hardcover_core::EditionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.current_entry(cx) else {
            return;
        };
        if entry.edition.as_ref().map(|e| e.id) == Some(edition) {
            return;
        }
        writes::set_edition(&self.model, entry.user_book_id, edition, window, cx);
    }

    /// Opens the read dialog to add a read (`None`) or change `read`'s dates.
    fn open_read_dialog(
        &mut self,
        read: Option<&ReadSession>,
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
        let dialog = cx.new(|cx| ReadDialog::new(kind, read, window, cx));
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
                    LibraryWrite::entry(
                        id,
                        WriteKind::Reads,
                        "Couldn't add the read",
                        move |client| client.add_read(id, started, finished, edition),
                    ),
                    |_| None,
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
                    LibraryWrite::entry(
                        id,
                        WriteKind::Dates,
                        "Couldn't change the dates",
                        move |client| client.set_read_dates(read, started, finished),
                    ),
                    |_| None,
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
        let title: SharedString = format!("Delete this read of “{}”?", entry.book.title).into();
        let description: SharedString = format!(
            "{}. Its dates and progress are deleted from Hardcover.",
            read_dates_text(session.started_at, session.finished_at)
        )
        .into();
        let view = cx.entity().downgrade();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title(title.clone())
                .description(description.clone())
                .confirm()
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.run_write(
                            LibraryWrite::entry(
                                id,
                                WriteKind::Reads,
                                "Couldn't delete the read",
                                move |client| client.delete_read(id, read),
                            ),
                            |_| None,
                            window,
                            cx,
                        );
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
        let title: SharedString =
            format!("Remove “{}” from your library?", entry.book.title).into();
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
            LibraryWrite::entry(
                id,
                WriteKind::Remove,
                "Couldn't remove the book",
                move |client| client.remove_from_library(id).map(|()| Removed),
            ),
            |_| None,
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
        writes::add_to_library(&self.model, book.id, status, window, cx);
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
    /// and every block; or, when it is wide, over the page's columns.
    fn render_pane(
        &mut self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        details: Option<&BookDetails>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let layout = self.layout.get();
        let content = if layout.rail {
            let columns = self.render_columns(book, entry, details, false, layout.side, window, cx);
            // At least the pane high, so the rail's hairline runs to the
            // foot of a short book, as on the page.
            div()
                .flex()
                .flex_col()
                .w_full()
                .min_h_full()
                .px_6()
                .pb_6()
                .tab_group()
                .child(columns)
                .into_any_element()
        } else {
            div()
                .px_6()
                .pb_6()
                .child(self.render_column(book, entry, details, window, cx))
                .into_any_element()
        };
        let measured = self.layout.clone();
        v_flex()
            .size_full()
            // GPUI has no container queries, and the pane sits beside a
            // resizable list: measure the width the parent gives it.
            .on_prepaint(move |bounds, window, _| {
                let layout = PaneLayout::at(bounds.size.width, window.rem_size());
                if measured.replace(layout) != layout {
                    window.refresh();
                }
            })
            .child(ObjectBar::new().tools(self.render_tools(book, entry, cx)))
            .child(
                div()
                    // Keyed by book, so another book (a similar one) opens at
                    // the top.
                    .id(("book-detail", book.id.get().cast_unsigned()))
                    .flex_1()
                    .min_h_0()
                    .scroll_y_revealing_focus()
                    .child(content),
            )
            .into_any_element()
    }

    /// The narrow pane's one column: the header, the panel and every block.
    fn render_column(
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
            column.push(self.render_notes(entry, false, cx));
        }
        column.push(self.render_about(book, cx));
        column.extend(self.render_tags(details, cx));
        column.extend(Self::render_other_titles(details, cx));
        if let Some(entry) = entry {
            column.push(self.render_details(entry, false, cx));
            column.push(self.render_review(entry, cx).into_any_element());
            column.push(self.render_reads(entry, false, cx));
        }
        column.extend(Self::render_on_hardcover(book, details, false, cx));
        column.extend(self.render_community_reviews(cx));
        column.extend(self.render_similar(book, details, None, cx));

        v_flex()
            .w_full()
            .gap_6()
            .child(header)
            .children(entry.and_then(|entry| self.render_panel(entry, cx)))
            .child(blocks().children(column))
            .into_any_element()
    }

    /// The page's columns, on the full page and in a wide pane: a main column
    /// (the header, the panel, About this book, tags, Your review, Reviews,
    /// Similar books) beside the rail (On Hardcover, Details, Reads, Notes),
    /// both from the top. With `side` (the similar books it holds; see
    /// [`side_width`]), the blocks under the panel split into the text and
    /// a side column. The caller wraps them in its inset as a tab group.
    #[allow(clippy::too_many_arguments)]
    fn render_columns(
        &mut self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        details: Option<&BookDetails>,
        folded: bool,
        side: Option<catalog_sections::SideGrid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Columns {
        let header = self.render_header(book, entry, details, true, window, cx);
        let about = self.render_about(book, cx);
        let tags = self.render_tags(details, cx);
        let other_titles = Self::render_other_titles(details, cx);
        let review = entry.map(|entry| self.render_review(entry, cx).into_any_element());
        let reviews = self.render_community_reviews(cx);
        let similar = self.render_similar(book, details, side, cx);

        // Keyboard order follows the task, not the tree: the header and
        // panel, then the rail beside them (your book), then the main
        // column's blocks. The blocks are a later tab group inside the
        // caller's own group, so the rail's stops come before them.
        let blocks = if side.is_some() {
            // The text keeps its measure; the side column takes the rest.
            let text = blocks()
                .w(size::PROSE)
                .flex_none()
                .child(about)
                .children(review)
                .children(reviews);
            let side = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_6()
                .children(tags)
                .children(other_titles)
                .children(similar);
            div()
                .flex()
                .flex_row()
                .items_start()
                .w_full()
                .gap(SPLIT_GAP)
                .child(text)
                .child(side)
        } else {
            blocks()
                .child(about)
                .children(tags)
                .children(other_titles)
                .children(review)
                .children(reviews)
                .children(similar)
        };

        let mut rail: Vec<AnyElement> = Vec::new();
        rail.extend(Self::render_on_hardcover(book, details, true, cx));
        if let Some(entry) = entry {
            rail.push(self.render_details(entry, true, cx));
            rail.push(self.render_reads(entry, true, cx));
            rail.push(self.render_notes(entry, true, cx));
        }

        let main = v_flex()
            .w_full()
            .gap_6()
            .child(header)
            .children(entry.and_then(|entry| self.render_panel(entry, cx)))
            .child(blocks.tab_group().tab_index(1));
        Columns::new(Tier::rail(folded), main).rail(rail)
    }

    /// The full page: the columns in the page's inset; the rail folds under
    /// the main column when the page is too narrow.
    fn render_page(
        &mut self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        details: Option<&BookDetails>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The page is the window less the sidebar and its own insets: GPUI
        // has no container queries.
        let rem = window.rem_size();
        let width = region_width(window, size::PAGE_INSET);
        let folded = folds(width, rem, false);
        let side = side_width(main_column_width(width, rem, false, folded), rem)
            .map(|side| catalog_sections::similar_in_side(side, rem));
        let columns = self.render_columns(book, entry, details, folded, side, window, cx);
        div()
            .id(("book-detail", book.id.get().cast_unsigned()))
            .size_full()
            .scroll_y_revealing_focus()
            .child(
                // At least the window high, so the rail's hairline runs to
                // the foot of a short page.
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .min_h_full()
                    .p_8()
                    .tab_group()
                    .child(columns),
            )
            .into_any_element()
    }
}

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
        let shown = self.shown_book(cx);
        self.keep_similar_covers(shown.as_ref().map(|(book, _)| book.id), window, cx);
        let Some((book, entry)) = shown else {
            self.load_cover(None, window, cx);
            return EmptyState::new(IconName::BookOpen, "Select a book to see its details")
                .into_any_element();
        };
        let details = self.details(book.id, cx);
        if self.tools_in_bar {
            self.render_page(&book, entry.as_ref(), details.as_deref(), window, cx)
        } else {
            self.render_pane(&book, entry.as_ref(), details.as_deref(), window, cx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext, px};
    use hardcover_core::store::LibraryCache;

    use crate::session::SessionScope;

    /// Project Hail Mary in the demo catalogue: more than a page of reviews.
    pub(super) const BOOK: BookId = BookId::new(427_578);

    /// A book pane on the demo client, showing [`BOOK`] once its details,
    /// similar books and first page of reviews are in.
    pub(super) fn demo_pane(
        cx: &mut TestAppContext,
    ) -> (Entity<BookDetailView>, &mut VisualTestContext) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        pane_on(session, cx)
    }

    /// A book pane on `session`'s client, showing [`BOOK`] once everything
    /// it asks for has landed.
    fn pane_on(
        session: SessionScope,
        cx: &mut TestAppContext,
    ) -> (Entity<BookDetailView>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            session.install(cx);
            Catalog::init(&session, cx);
        });
        let model = cx.new(|_| LibraryModel::new(session));
        let (view, cx) = cx.add_window_view(|window, cx| BookDetailView::new(model, window, cx));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let book = BookSummary::new(BOOK, "Project Hail Mary");
                view.set_subject(Some(Subject::Book(book)), window, cx);
            });
        });
        settle(cx);
        (view, cx)
    }

    /// Past every delay, with every fetch landed.
    pub(super) fn settle(cx: &mut VisualTestContext) {
        for _ in 0..3 {
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
        }
    }

    fn catalog(view: &Entity<BookDetailView>, cx: &mut VisualTestContext) -> Catalog {
        view.read_with(cx, |view, _| view.catalog.clone().unwrap())
    }

    fn details_loading(view: &Entity<BookDetailView>, cx: &mut VisualTestContext) -> bool {
        let catalog = catalog(view, cx);
        catalog
            .details
            .read_with(cx, |s, _| s.resource().is_loading(&BOOK))
    }

    /// A book view inside the kit's root (dialogs need it), on the demo
    /// library, synced.
    pub(super) fn rooted_view(
        cx: &mut TestAppContext,
    ) -> (
        Entity<BookDetailView>,
        Entity<LibraryModel>,
        &mut VisualTestContext,
    ) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::kit::bind_keys(cx);
            session.install(cx);
            Catalog::init(&session, cx);
        });
        let model = cx.new(|_| LibraryModel::new(session));
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let view = std::rc::Rc::new(std::cell::RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let (model, view) = (model.clone(), view.clone());
            move |window, cx| {
                let detail = cx.new(|cx| BookDetailView::new(model.clone(), window, cx));
                *view.borrow_mut() = Some(detail.clone());
                gpui_kit::base::Root::new(detail, window, cx)
            }
        });
        cx.update(|window, _| window.activate_window());
        let view = view.borrow_mut().take().unwrap();
        (view, model, cx)
    }

    /// Shows the first entry with `status` that `matches`.
    pub(super) fn show_entry(
        view: &Entity<BookDetailView>,
        model: &Entity<LibraryModel>,
        cx: &mut VisualTestContext,
        matches: impl Fn(&LibraryEntry) -> bool,
    ) -> UserBookId {
        let id = model.read_with(cx, |m, _| {
            m.entries()
                .iter()
                .find(|e| matches(e))
                .map(|e| e.user_book_id)
                .expect("the demo has such an entry")
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.set_subject(Some(Subject::Entry(id)), window, cx);
            });
        });
        settle(cx);
        crate::ui::test_support::redraw(cx);
        id
    }

    fn dialog_open(cx: &mut VisualTestContext) -> bool {
        cx.update(gpui_kit::component::WindowExt::has_active_dialog)
    }

    /// D3: moving a paused book back to Currently reading asks first; Cancel
    /// changes nothing and gives focus back to the status menu's trigger.
    #[gpui_kit::test]
    fn a_status_change_that_starts_a_read_asks_first(cx: &mut TestAppContext) {
        use crate::ui::test_support::{press, redraw};

        let (view, model, cx) = rooted_view(cx);
        let id = show_entry(&view, &model, cx, |e| {
            e.status == ReadingStatus::Paused && e.open_read().is_some()
        });
        let trigger = view.read_with(cx, |v, _| v.status_focus.clone());
        cx.update(|window, cx| {
            window.focus(&trigger, cx);
            view.update(cx, |v, cx| {
                v.change_status(ReadingStatus::CurrentlyReading, window, cx);
            });
        });
        redraw(cx);
        redraw(cx);
        assert!(dialog_open(cx), "asked first");
        press(cx, "enter");
        cx.run_until_parked();
        redraw(cx);
        assert!(!dialog_open(cx), "Cancel, where focus opened");
        let status = model.read_with(cx, |m, _| m.entry(id).map(|e| e.status));
        assert_eq!(status, Some(ReadingStatus::Paused), "nothing changed");
        assert!(
            cx.update(|window, _| trigger.is_focused(window)),
            "focus is back on the status menu's trigger"
        );

        // A change that starts nothing goes at once.
        cx.update(|window, cx| {
            view.update(cx, |v, cx| {
                v.change_status(ReadingStatus::DidNotFinish, window, cx);
            });
        });
        redraw(cx);
        assert!(!dialog_open(cx));
        cx.run_until_parked();
        let status = model.read_with(cx, |m, _| m.entry(id).map(|e| e.status));
        assert_eq!(status, Some(ReadingStatus::DidNotFinish));
    }

    #[gpui_kit::test]
    fn stale_details_stay_on_screen_while_fetched_again(cx: &mut TestAppContext) {
        let (view, cx) = demo_pane(cx);
        let catalog = catalog(&view, cx);
        let shown = |cx: &mut VisualTestContext| {
            view.read_with(cx, |view, cx| view.details(BOOK, cx).is_some())
        };
        assert!(shown(cx));
        cx.update(|_, cx| crate::resource::refresh_all(cx));
        view.update(cx, |view, cx| view.revalidate(false, cx));
        assert!(details_loading(&view, cx), "fetched again");
        assert!(shown(cx), "and still shown meanwhile");
        settle(cx);
        assert!(!details_loading(&view, cx));
        let fresh = catalog
            .details
            .read_with(cx, |s, cx| !s.resource().is_stale(&BOOK, cx));
        assert!(fresh);
    }

    /// The book page opened from a pane shows what the pane showed at once:
    /// no request, no placeholder.
    #[gpui_kit::test]
    fn a_book_shown_in_another_view_opens_without_a_request(cx: &mut TestAppContext) {
        let (pane, cx) = demo_pane(cx);
        let catalog = catalog(&pane, cx);
        let started = |cx: &mut VisualTestContext| {
            let details = catalog.details.read_with(cx, |s, _| s.resource().started());
            let similar = catalog.similar.read_with(cx, |s, _| s.resource().started());
            let reviews = catalog.reviews.read_with(cx, |s, _| s.resource().started());
            (details, similar, reviews)
        };
        let before = started(cx);
        assert_eq!(before, (1, 1, 1));
        let model = pane.read_with(cx, |pane, _| pane.model.clone());
        let page = cx.update(|window, cx| {
            cx.new(|cx| {
                let mut page = BookDetailView::new(model, window, cx);
                page.set_tools_in_bar(true);
                page
            })
        });
        cx.update(|window, cx| {
            page.update(cx, |page, cx| {
                let book = BookSummary::new(BOOK, "Project Hail Mary");
                page.set_subject(Some(Subject::Book(book)), window, cx);
                // The shell asks the page shown to revalidate at once.
                page.revalidate(true, cx);
                assert!(page.details(BOOK, cx).is_some(), "shown at once");
            });
        });
        settle(cx);
        assert_eq!(started(cx), before, "nothing fetched again");
    }

    /// A sync makes the reader's own reviews stale, but only the view shown
    /// fetches its review again (the workspace asks it to): a hidden book
    /// view sends nothing.
    #[gpui_kit::test]
    fn a_sync_fetches_the_own_review_only_where_shown(cx: &mut TestAppContext) {
        let (view, cx) = demo_pane(cx);
        let model = view.read_with(cx, |view, _| view.model.clone());
        model.update(cx, |m, cx| m.start(cx));
        settle(cx);
        let entry = model.read_with(cx, |m, _| m.entries()[0].clone());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.set_subject(Some(Subject::Entry(entry.user_book_id)), window, cx);
            });
        });
        settle(cx);
        let asked = |cx: &mut VisualTestContext| model.read_with(cx, |m, _| m.reviews().started());
        let before = asked(cx);
        assert!(before >= 1, "the review was asked for when shown");

        model.update(cx, |m, cx| m.sync(cx));
        settle(cx);
        assert_eq!(asked(cx), before, "not while hidden");
        view.update(cx, |view, cx| view.revalidate(false, cx));
        assert_eq!(asked(cx), before + 1, "shown: fetched again");
    }

    /// D14: the Reviews block doesn't wait on the details; when they fail,
    /// the reviews still load.
    #[gpui_kit::test]
    fn reviews_load_when_the_details_fail(cx: &mut TestAppContext) {
        let faults = hardcover_core::demo::Faults::parse("refuse=read:catalog").unwrap();
        let session = SessionScope::for_tests(hardcover_core::demo::client_with(&faults), None);
        let (view, cx) = pane_on(session, cx);
        let catalog = catalog(&view, cx);
        let error = catalog
            .details
            .read_with(cx, |s, _| s.resource().error(&BOOK).is_some());
        assert!(error, "the details failed");
        let reviews = catalog
            .reviews
            .read_with(cx, |s, _| s.resource().value(&BOOK).is_some());
        assert!(reviews, "the reviews loaded anyway");
    }

    #[gpui_kit::test]
    fn a_failure_is_tried_again_when_shown_not_on_activation(cx: &mut TestAppContext) {
        // Not a refused scope: that counts the scope as missing, and nothing
        // needing it is asked for again until "Sign in again…".
        let faults = hardcover_core::demo::Faults::parse("500").unwrap();
        let session = SessionScope::for_tests(hardcover_core::demo::client_with(&faults), None);
        let (view, cx) = pane_on(session, cx);
        let catalog = catalog(&view, cx);
        let started = |cx: &mut VisualTestContext| {
            catalog.details.read_with(cx, |s, _| s.resource().started())
        };
        let before = started(cx);
        view.update(cx, |view, cx| view.revalidate(false, cx));
        assert_eq!(started(cx), before, "the window became active: left alone");
        view.update(cx, |view, cx| view.revalidate(true, cx));
        assert_eq!(
            started(cx),
            before + 1,
            "Refresh, or shown again: tried again"
        );
    }

    #[test]
    fn the_pane_takes_the_rail_only_when_a_main_column_fits_beside_it() {
        let rem = px(16.);
        // 24 + 400 + 24 + 272 + 24 = 744.
        assert!(pane_has_rail(px(744.), rem));
        assert!(!pane_has_rail(px(743.), rem));
        // The minimum window, the list at its narrowest: one column.
        assert!(!pane_has_rail(px(800. - 240. - 272.), rem));
        // The default window, with the list at its default and narrowest.
        assert!(!pane_has_rail(px(1200. - 240. - 384.), rem));
        assert!(!pane_has_rail(px(1200. - 240. - 272.), rem));
        // A 1600 laptop panel and a 4K monitor at 1.5, the list at its default.
        assert!(pane_has_rail(px(1600. - 240. - 384.), rem));
        assert!(pane_has_rail(px(2560. - 240. - 384.), rem));
    }

    #[test]
    fn the_blocks_split_only_when_the_text_and_a_side_column_fit() {
        let rem = px(16.);
        // 640 + 32 + 320 = 992.
        assert_eq!(side_width(px(992.), rem), Some(px(320.)));
        assert_eq!(side_width(px(991.), rem), None);
        // The full page: the window less the sidebar, the insets, 24 and the rail.
        let page = |window: f32| {
            let width = px(window - 240. - 64.);
            side_width(
                main_column_width(width, rem, false, folds(width, rem, false)),
                rem,
            )
        };
        assert_eq!(page(800.), None);
        assert_eq!(page(1200.), None);
        assert_eq!(page(1591.), None);
        assert_eq!(page(1592.), Some(px(320.)));
        assert_eq!(page(1600.), Some(px(328.)));
        assert_eq!(page(2560.), Some(px(1288.)));
        // The pane beside the shelf list at its default (384).
        let pane = |window: f32| PaneLayout::at(px(window - 240. - 384.), rem);
        assert_eq!(pane(800.), PaneLayout::default());
        assert_eq!(pane(1200.), PaneLayout::default());
        // 1600: the rail, but a main column of 632: one column.
        assert_eq!(
            pane(1600.),
            PaneLayout {
                rail: true,
                side: None
            }
        );
        // 2560: a main column of 1592 and a side of 920, nine a row.
        let wide = pane(2560.);
        assert!(wide.rail);
        assert_eq!(wide.side.map(|grid| grid.shown(16)), Some(9));
    }
}
