//! Search: a debounced query over books, authors or series, paged results
//! and the shared detail pane.
//!
//! Every query change bumps a revision; a response is applied only if its
//! revision is still current, so a slow page for an old query never replaces
//! newer results. Each kind keeps its own results and request: only the kind
//! shown is searched, and switching kinds searches the current query there.
//!
//! Book results start with the matching books of the library, found on this
//! computer as the query changes ("In your library"), followed by Hardcover's
//! results without them ("On Hardcover").
//!
//! Books open in the detail pane beside the list. Authors and series open
//! their pages, on Enter or a click.
//!
//! Down or Enter in the query field moves into the results, selecting the
//! first row when none is, so the focus is visible where it lands. The
//! results are a tab stop too, after the kind tabs.
//!
//! A failed search says why with "Try again": the first page runs the query
//! again; a later page asks for that page again, keeping the rows so far.
//!
//! In a narrow list a row's `StatusChip` shrinks to its icon
//! ([`BookRow::compact`]), so the title keeps its room.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ElementExt as _, Icon, IconName, IndexPath, Sizable as _, h_flex, h_resizable, resizable_panel,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, WeakEntity, Window,
    actions, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{AuthorHit, BookId, BookSummary, Client, SearchHits, SeriesHit};

use crate::app_state::Services;
use crate::book::detail_view::{BookDetailView, Subject};
use crate::catalog::navigation::{self, Page};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{author_hit_details, series_hit_details};
use crate::library::book_row::{BookRow, loading_rows};
use crate::library::model::LibraryModel;
use crate::search::library_matches::{counts_as_shelved, library_matches};
use crate::search::results::{Hit, Phase, SearchResults};
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::{
    ListRow as _, RowFocus, empty_state, empty_state_with_action, on_keyboard_entry,
    select_first_row,
};

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Below this list width (360) a row's `StatusChip` shrinks to its icon: the
/// widest chip ("Currently reading") would leave a title too little room.
const COMPACT_BELOW: f32 = 22.5;

/// Around the query field. `main` binds Down to [`EnterResults`] in
/// `SearchField > Input`, over the input's own Down (a no-op on one line).
pub const KEY_CONTEXT: &str = "SearchField";

actions!(search, [EnterResults]);

/// What the search looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Books,
    Authors,
    Series,
}

impl SearchKind {
    const ALL: [SearchKind; 3] = [SearchKind::Books, SearchKind::Authors, SearchKind::Series];

    fn label(self) -> &'static str {
        match self {
            SearchKind::Books => "Books",
            SearchKind::Authors => "Authors",
            SearchKind::Series => "Series",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            SearchKind::Books => "Search books",
            SearchKind::Authors => "Search authors",
            SearchKind::Series => "Search series",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

/// One kind of result: how to fetch a page, draw a row and find its list.
pub trait SearchHit: Hit + Clone + Send + 'static {
    const KIND: SearchKind;
    fn fetch(client: &Client, query: &str, page: u32) -> hardcover_core::Result<SearchHits<Self>>;
    fn list(view: &SearchView) -> &Entity<ListState<ResultsDelegate<Self>>>;
    fn row(&self, model: &LibraryModel) -> BookRow;
    /// The empty list's description before a query.
    fn hint() -> &'static str;
    fn row_id(&self) -> (&'static str, u64);
    fn name(&self) -> SharedString;
    /// The library's own matches for `query`, shown before Hardcover's.
    fn library_matches(_: &LibraryModel, _: &str) -> Vec<Self> {
        Vec::new()
    }
    /// Whether a library match is still in the library.
    fn in_library(&self, _: &LibraryModel) -> bool {
        false
    }
}

impl SearchHit for BookSummary {
    const KIND: SearchKind = SearchKind::Books;

    fn fetch(client: &Client, query: &str, page: u32) -> hardcover_core::Result<SearchHits<Self>> {
        client.search_books(query, page).map(SearchHits::from)
    }

    fn list(view: &SearchView) -> &Entity<ListState<ResultsDelegate<Self>>> {
        &view.books
    }

    fn row(&self, model: &LibraryModel) -> BookRow {
        let status = model.entry_for_book(self.id).map(|e| e.status);
        BookRow::for_search_result(self, status)
    }

    fn hint() -> &'static str {
        "Find books by title, author or series."
    }

    fn row_id(&self) -> (&'static str, u64) {
        ("search-book", self.id.get().cast_unsigned())
    }

    fn name(&self) -> SharedString {
        self.title.clone().into()
    }

    fn library_matches(model: &LibraryModel, query: &str) -> Vec<Self> {
        library_matches(model.entries(), query)
    }

    fn in_library(&self, model: &LibraryModel) -> bool {
        model
            .entry_for_book(self.id)
            .is_some_and(|e| counts_as_shelved(e.status))
    }
}

impl SearchHit for AuthorHit {
    const KIND: SearchKind = SearchKind::Authors;

    fn fetch(client: &Client, query: &str, page: u32) -> hardcover_core::Result<SearchHits<Self>> {
        client.search_authors(query, page)
    }

    fn list(view: &SearchView) -> &Entity<ListState<ResultsDelegate<Self>>> {
        &view.authors
    }

    fn row(&self, _: &LibraryModel) -> BookRow {
        BookRow::for_author(
            self.name.clone(),
            author_hit_details(self),
            self.photo.as_ref().map(|p| p.url.clone()),
        )
    }

    fn hint() -> &'static str {
        "Find authors by name or by their books."
    }

    fn row_id(&self) -> (&'static str, u64) {
        ("search-author", self.id.get().cast_unsigned())
    }

    fn name(&self) -> SharedString {
        self.name.clone().into()
    }
}

impl SearchHit for SeriesHit {
    const KIND: SearchKind = SearchKind::Series;

    fn fetch(client: &Client, query: &str, page: u32) -> hardcover_core::Result<SearchHits<Self>> {
        client.search_series(query, page)
    }

    fn list(view: &SearchView) -> &Entity<ListState<ResultsDelegate<Self>>> {
        &view.series
    }

    fn row(&self, _: &LibraryModel) -> BookRow {
        BookRow::for_series(self.name.clone(), series_hit_details(self))
    }

    fn hint() -> &'static str {
        "Find series by name, author or one of their books."
    }

    fn row_id(&self) -> (&'static str, u64) {
        ("search-series", self.id.get().cast_unsigned())
    }

    fn name(&self) -> SharedString {
        self.name.clone().into()
    }
}

pub struct ResultsDelegate<T: SearchHit> {
    results: SearchResults<T>,
    /// The library's matches, the first section when there are any.
    library: Vec<T>,
    library_keys: HashSet<T::Key>,
    model: Entity<LibraryModel>,
    view: WeakEntity<SearchView>,
    focus: RowFocus,
    /// Whether the list is narrow, as last laid out; shared by the three lists.
    compact: Rc<Cell<bool>>,
}

impl<T: SearchHit> ResultsDelegate<T> {
    fn set_library(&mut self, hits: Vec<T>) {
        self.library_keys = hits.iter().map(Hit::key).collect();
        self.library = hits;
    }

    /// With library matches the list has two sections, else one: section 0
    /// always has the first row, as the list's own keyboard handling expects.
    fn has_library(&self) -> bool {
        !self.library.is_empty()
    }

    /// Hardcover's results, without the ones shown as library matches.
    fn remote(&self) -> impl Iterator<Item = &T> {
        self.results
            .hits()
            .iter()
            .filter(|h| !self.library_keys.contains(&h.key()))
    }

    fn hit_at(&self, ix: IndexPath) -> Option<&T> {
        if self.has_library() && ix.section == 0 {
            self.library.get(ix.row)
        } else {
            self.remote().nth(ix.row)
        }
    }

    fn index_of(&self, key: T::Key) -> Option<IndexPath> {
        if let Some(row) = self.library.iter().position(|h| h.key() == key) {
            return Some(IndexPath::new(row));
        }
        let section = usize::from(self.has_library());
        self.remote()
            .position(|h| h.key() == key)
            .map(|row| IndexPath::new(row).section(section))
    }

    fn first(&self) -> Option<&T> {
        self.hit_at(IndexPath::default())
    }

    fn is_empty(&self) -> bool {
        self.library.is_empty() && self.results.hits().is_empty()
    }

    /// Hardcover's results so far that are left out as library matches.
    fn hidden_duplicates(&self) -> u32 {
        u32::try_from(self.results.hits().len() - self.remote().count()).unwrap_or(u32::MAX)
    }
}

/// "2 in your library · 14 more on Hardcover": Hardcover's total less the
/// library matches among its results. One on a page not loaded yet is still
/// counted, so until it loads the number is a little high.
fn library_status(library: usize, total: u32, duplicates: u32) -> SharedString {
    match total.saturating_sub(duplicates) {
        0 => format!("{library} in your library").into(),
        more => format!("{library} in your library · {more} more on Hardcover").into(),
    }
}

impl<T: SearchHit> ListDelegate for ResultsDelegate<T> {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        1 + usize::from(self.has_library())
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.focus
            .stop_when_rows(self.library.len() + self.remote().count());
        if self.has_library() && section == 0 {
            self.library.len()
        } else {
            self.remote().count()
        }
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        if !self.has_library() {
            return None;
        }
        // A `label` on the rows' text spine. Both headers are the same
        // height: the virtual list measures one and assumes it for the rest.
        Some(div().px_2().pt_3().pb_1().label(cx).child(if section == 0 {
            "In your library"
        } else {
            "On Hardcover"
        }))
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let focused = self.focus.shows_focus(ix, window);
        let selected = self.focus.is_selected(ix);
        let hit = self.hit_at(ix)?;
        let row = hit
            .row(self.model.read(cx))
            .selected(selected)
            .compact(self.compact.get());
        Some(
            ListItem::new(hit.row_id())
                .list_row(focused, cx)
                .accessibility_label(hit.name())
                .child(row),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        // Book selection is owned by `SearchView`, keyed by book id; authors
        // and series open on confirm. The rows only draw it.
        self.focus.select(ix);
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        match self.results.phase() {
            Phase::Idle => empty_state(
                IconName::Search,
                "Search Hardcover".into(),
                T::hint().into(),
            ),
            Phase::Failed(message) => empty_state_with_action(
                IconName::CircleAlert,
                "Couldn't search Hardcover".into(),
                message.clone().into(),
                retry_button::<T>(self.view.clone()),
            ),
            _ => empty_state(
                IconName::Search,
                "No results".into(),
                format!("Nothing on Hardcover matches “{}”.", self.results.query()).into(),
            ),
        }
    }

    /// Skeleton rows while the first page is on its way and the library
    /// has nothing to show meanwhile.
    fn loading(&self, _: &App) -> bool {
        matches!(self.results.phase(), Phase::Searching) && self.is_empty()
    }

    fn render_loading(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        loading_rows()
    }

    fn has_more(&self, _: &App) -> bool {
        self.results.has_more()
    }

    fn load_more_threshold(&self) -> usize {
        5
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        if !self.results.has_more() {
            return;
        }
        let view = self.view.clone();
        // Deferred: the view updates this list when it starts the request.
        window.defer(cx, move |window, cx| {
            let _ = view.update(cx, |view, cx| view.load_next_page::<T>(window, cx));
        });
    }
}

pub struct SearchView {
    input: Entity<InputState>,
    kind: SearchKind,
    books: Entity<ListState<ResultsDelegate<BookSummary>>>,
    authors: Entity<ListState<ResultsDelegate<AuthorHit>>>,
    series: Entity<ListState<ResultsDelegate<SeriesHit>>>,
    detail: Entity<BookDetailView>,
    selected: Option<BookId>,
    /// One request per kind, indexed by [`SearchKind::index`]. Replacing one
    /// drops a pending debounce or page for that kind only.
    tasks: [Option<Task<()>>; 3],
    /// Shared with the lists' delegates: whether the list is narrow.
    compact: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

impl SearchView {
    pub fn new(model: &Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(SearchKind::Books.placeholder()));
        let view = cx.entity().downgrade();
        let compact = Rc::new(Cell::new(false));
        let books = new_results_list(model, &view, &compact, window, cx);
        let authors = new_results_list(model, &view, &compact, window, cx);
        let series = new_results_list(model, &view, &compact, window, cx);
        let detail = cx.new(|cx| BookDetailView::new(model.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &input,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let query = input.read(cx).value();
                        this.query_changed(query, window, cx);
                    }
                    InputEvent::PressEnter { .. } => this.enter_results(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &books,
                window,
                |this, list, event: &ListEvent, window, cx| {
                    let ix = match event {
                        ListEvent::Select(ix) | ListEvent::Confirm(ix) => *ix,
                        ListEvent::Cancel => return,
                    };
                    let book = list.read(cx).delegate().hit_at(ix).cloned();
                    if let Some(book) = book {
                        this.select(book, window, cx);
                    }
                },
            ),
            // Arrowing only moves the selection; Enter or a click opens the page.
            cx.subscribe(
                &authors,
                |_, list: Entity<ListState<ResultsDelegate<AuthorHit>>>, event: &ListEvent, cx| {
                    if let ListEvent::Confirm(ix) = event {
                        let author = list.read(cx).delegate().hit_at(*ix).cloned();
                        if let Some(author) = author {
                            navigation::open(Page::Author(author.id), cx);
                        }
                    }
                },
            ),
            cx.subscribe(
                &series,
                |_, list: Entity<ListState<ResultsDelegate<SeriesHit>>>, event: &ListEvent, cx| {
                    if let ListEvent::Confirm(ix) = event {
                        let series = list.read(cx).delegate().hit_at(*ix).cloned();
                        if let Some(series) = series {
                            navigation::open(Page::Series(series.id), cx);
                        }
                    }
                },
            ),
            // Status badges follow library changes, and a book that left the
            // library leaves its matches. New matches wait for the next query:
            // adding the selected book mustn't move it under the pointer.
            cx.observe_in(model, window, |this, model, window, cx| {
                this.books.update(cx, |list, cx| {
                    let library = model.read(cx);
                    let delegate = list.delegate_mut();
                    if delegate.library.iter().any(|b| !b.in_library(library)) {
                        let kept = delegate
                            .library
                            .iter()
                            .filter(|b| b.in_library(library))
                            .cloned()
                            .collect();
                        delegate.set_library(kept);
                    }
                    cx.notify();
                });
                this.sync_list_selection(window, cx);
            }),
            // Tab into the results lands on a row: the first if none is.
            on_keyboard_entry(&books, window, cx, |this, window, cx| {
                let first = this.books.read(cx).delegate().first().cloned();
                if let Some(book) = first {
                    this.select(book, window, cx);
                }
            }),
            on_keyboard_entry(&authors, window, cx, |this, window, cx| {
                select_first_row(&this.authors, window, cx);
            }),
            on_keyboard_entry(&series, window, cx, |this, window, cx| {
                select_first_row(&this.series, window, cx);
            }),
        ];
        Self {
            input,
            kind: SearchKind::Books,
            books,
            authors,
            series,
            detail,
            selected: None,
            tasks: [None, None, None],
            compact,
            _subscriptions: subscriptions,
        }
    }

    /// "Try again" after a failure: the failed page again if earlier pages
    /// loaded, else the whole query.
    fn retry<T: SearchHit>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (page, query) = T::list(self).update(cx, |list, cx| {
            let results = &mut list.delegate_mut().results;
            let page = results.retry_page();
            cx.notify();
            (page, SharedString::from(results.query().to_owned()))
        });
        if page {
            self.load_next_page::<T>(window, cx);
        } else {
            self.search::<T>(query, false, window, cx);
        }
        cx.notify();
    }

    /// Fetches the pane's book again where it is stale ([`crate::freshness`]).
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.revalidate(shown_again, cx));
    }

    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    /// Moves focus from the query field into the results. With no row
    /// selected, selects the first (for books, showing it in the detail pane):
    /// a focused list with no selection shows nothing where the focus went.
    fn enter_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.kind {
            SearchKind::Books => {
                let list = &self.books.read(cx);
                let first = match list.selected_index() {
                    None => list.delegate().first().cloned(),
                    Some(_) => None,
                };
                if let Some(book) = first {
                    self.select(book, window, cx);
                }
                focus_list(&self.books, window, cx);
            }
            SearchKind::Authors => focus_list(&self.authors, window, cx),
            SearchKind::Series => focus_list(&self.series, window, cx),
        }
    }

    fn select(&mut self, book: BookSummary, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(book.id);
        self.sync_list_selection(window, cx);
        self.detail.update(cx, |d, cx| {
            d.set_subject(Some(Subject::Book(book)), window, cx);
        });
    }

    fn set_kind(&mut self, kind: SearchKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        self.input.update(cx, |input, cx| {
            input.set_placeholder(kind.placeholder(), window, cx);
        });
        let query = self.input.read(cx).value();
        // Search this kind for the current query, unless it already has.
        match kind {
            SearchKind::Books => self.search_if_stale::<BookSummary>(query, window, cx),
            SearchKind::Authors => self.search_if_stale::<AuthorHit>(query, window, cx),
            SearchKind::Series => self.search_if_stale::<SeriesHit>(query, window, cx),
        }
        // Keep the query as typed: the next key edits it rather than replacing it.
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn search_if_stale<T: SearchHit>(
        &mut self,
        query: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let results = &T::list(self).read(cx).delegate().results;
        let failed = matches!(results.phase(), Phase::Failed(_)) && results.hits().is_empty();
        if results.query() != query.trim() || failed {
            self.search::<T>(query, false, window, cx);
        }
    }

    fn query_changed(&mut self, query: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        match self.kind {
            SearchKind::Books => self.search::<BookSummary>(query, true, window, cx),
            SearchKind::Authors => self.search::<AuthorHit>(query, true, window, cx),
            SearchKind::Series => self.search::<SeriesHit>(query, true, window, cx),
        }
    }

    /// Starts `query` for `T`'s kind, after the typing pause if `debounce`.
    fn search<T: SearchHit>(
        &mut self,
        query: SharedString,
        debounce: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = T::list(self).update(cx, |list, cx| {
            let delegate = list.delegate_mut();
            let revision = delegate.results.start(query.trim().into());
            // The library's matches show at once; Hardcover's follow.
            let library = T::library_matches(delegate.model.read(cx), query.trim());
            delegate.set_library(library);
            cx.notify();
            revision
        });
        if T::KIND == SearchKind::Books {
            self.sync_list_selection(window, cx);
        } else {
            // New results: nothing is chosen in them yet.
            T::list(self).update(cx, |list, cx| list.set_selected_index(None, window, cx));
        }
        let slot = T::KIND.index();
        let Some(revision) = revision else {
            // Blank query: nothing to fetch.
            self.tasks[slot] = None;
            return;
        };
        let client = Services::global(cx).client().clone();
        self.tasks[slot] = Some(cx.spawn_in(window, async move |this, cx| {
            if debounce {
                cx.background_executor().timer(DEBOUNCE).await;
            }
            let q = query.to_string();
            let result = cx
                .background_spawn(async move { T::fetch(&client, &q, 1) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.receive(revision, result, window, cx);
            });
        }));
    }

    fn load_next_page<T: SearchHit>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = T::list(self).update(cx, |list, cx| {
            let request = list.delegate_mut().results.next_page_request();
            cx.notify();
            request
        });
        let Some((revision, query, page)) = request else {
            return;
        };
        let client = Services::global(cx).client().clone();
        self.tasks[T::KIND.index()] = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { T::fetch(&client, &query, page) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.receive(revision, result, window, cx);
            });
        }));
    }

    fn receive<T: SearchHit>(
        &mut self,
        revision: u64,
        result: hardcover_core::Result<SearchHits<T>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = result.map_err(|e| Services::error_message(&e, Some(READ_CATALOG), cx));
        T::list(self).update(cx, |list, cx| {
            if list.delegate_mut().results.receive(revision, result) {
                cx.notify();
            }
        });
        self.sync_list_selection(window, cx);
        cx.notify();
    }

    /// Points the book list's positional selection at the selected book, if shown.
    fn sync_list_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected;
        self.books.update(cx, |list, cx| {
            let ix = selected.and_then(|id| list.delegate().index_of(id));
            if list.selected_index() != ix {
                list.set_selected_index(ix, window, cx);
            }
        });
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, busy, failed, list) = match self.kind {
            SearchKind::Books => results_status(&self.books, cx),
            SearchKind::Authors => results_status(&self.authors, cx),
            SearchKind::Series => results_status(&self.series, cx),
        };
        let retry = failed.then(|| match self.kind {
            SearchKind::Books => retry_button::<BookSummary>(cx.entity().downgrade()),
            SearchKind::Authors => retry_button::<AuthorHit>(cx.entity().downgrade()),
            SearchKind::Series => retry_button::<SeriesHit>(cx.entity().downgrade()),
        });
        let compact = self.compact.clone();
        let view = cx.entity().downgrade();
        let colors = colors(cx);
        // The shelf's header: padding 12/16 and a hairline under it.
        v_flex()
            .size_full()
            .min_w_0()
            .child(
                v_flex()
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .key_context(KEY_CONTEXT)
                            .on_action(cx.listener(|this, _: &EnterResults, window, cx| {
                                this.enter_results(window, cx);
                            }))
                            .child(
                                Input::new(&self.input)
                                    .raised_field(false, cx)
                                    .cleanable(true)
                                    .prefix(Icon::new(IconName::Search).text_color(colors.fg_muted))
                                    .when(busy, |this| this.suffix(Spinner::new().small())),
                            ),
                    )
                    .child(
                        TabBar::new("search-kind")
                            .segmented()
                            .small()
                            .selected_index(self.kind.index())
                            .children(SearchKind::ALL.map(|k| Tab::new().label(k.label())))
                            .on_click(move |ix, window, cx| {
                                let kind = SearchKind::ALL[*ix];
                                let _ = view.update(cx, |this, cx| this.set_kind(kind, window, cx));
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .caption(cx)
                                    .tabular()
                                    .truncate()
                                    // A blank line keeps the line's height.
                                    .child(status.unwrap_or_else(|| "\u{a0}".into())),
                            )
                            .children(retry),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .on_prepaint(move |bounds, window, _| {
                        let narrow = bounds.size.width < window.rem_size() * COMPACT_BELOW;
                        if compact.replace(narrow) != narrow {
                            // The rows were drawn for the other width.
                            window.refresh();
                        }
                    })
                    .child(list),
            )
    }

    /// Beside the list: the selected book, or what opening a result does.
    fn render_detail(&self) -> AnyElement {
        match self.kind {
            SearchKind::Books => self.detail.clone().into_any_element(),
            SearchKind::Authors => empty_state(
                IconName::User,
                "Authors".into(),
                "Open an author to see their books.".into(),
            )
            .into_any_element(),
            SearchKind::Series => empty_state(
                IconName::GalleryVerticalEnd,
                "Series".into(),
                "Open a series to see its books in order.".into(),
            )
            .into_any_element(),
        }
    }
}

fn new_results_list<T: SearchHit>(
    model: &Entity<LibraryModel>,
    view: &WeakEntity<SearchView>,
    compact: &Rc<Cell<bool>>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ListState<ResultsDelegate<T>>> {
    let list = cx.new(|cx| {
        ListState::new(
            ResultsDelegate {
                results: SearchResults::default(),
                library: Vec::new(),
                library_keys: HashSet::new(),
                model: model.clone(),
                view: view.clone(),
                focus: RowFocus::default(),
                compact: compact.clone(),
            },
            window,
            cx,
        )
    });
    RowFocus::attach(&list, |d| &mut d.focus, cx);
    list
}

/// Focuses `list` if it has results, selecting its first row if none is.
fn focus_list<T: SearchHit>(
    list: &Entity<ListState<ResultsDelegate<T>>>,
    window: &mut Window,
    cx: &mut App,
) {
    if list.read(cx).delegate().is_empty() {
        return;
    }
    list.update(cx, |list, cx| {
        if list.selected_index().is_none() {
            list.set_selected_index(Some(IndexPath::default()), window, cx);
            list.scroll_to_selected_item(window, cx);
        }
        list.focus(window, cx);
    });
}

/// "Try again" after a failed search, for `T`'s list.
fn retry_button<T: SearchHit>(view: WeakEntity<SearchView>) -> Button {
    Button::new(("retry-search", T::KIND.index()))
        .ghost()
        .small_control()
        .label("Try again")
        .on_click(move |_, window, cx| {
            let _ = view.update(cx, |view, cx| view.retry::<T>(window, cx));
        })
}

/// The status line under the input, whether a request is running, whether a
/// later page failed (the status line then offers "Try again"), and the list.
fn results_status<T: SearchHit>(
    list: &Entity<ListState<ResultsDelegate<T>>>,
    cx: &App,
) -> (Option<SharedString>, bool, bool, AnyElement) {
    let delegate = list.read(cx).delegate();
    let results = &delegate.results;
    let status: Option<SharedString> = match results.phase() {
        Phase::Searching => Some("Searching…".into()),
        Phase::LoadingMore => Some("Loading more…".into()),
        Phase::Loaded => results.total().map(|total| {
            if delegate.has_library() {
                library_status(delegate.library.len(), total, delegate.hidden_duplicates())
            } else if total == 1 {
                "1 result".into()
            } else {
                format!("{total} results").into()
            }
        }),
        // Shown above any rows; with none, the empty list says it.
        Phase::Failed(message) if !delegate.is_empty() => Some(message.clone().into()),
        _ => None,
    };
    let busy = matches!(results.phase(), Phase::Searching | Phase::LoadingMore);
    let failed = matches!(results.phase(), Phase::Failed(_)) && !delegate.is_empty();
    (
        status,
        busy,
        failed,
        List::new(list).size_full().p_2().into_any_element(),
    )
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        h_resizable("search-split")
            .child(
                resizable_panel()
                    .size(rem * size::LIST.0)
                    .size_range(rem * size::LIST_MIN.0..rem * size::LIST_MAX.0)
                    .child(self.render_results(cx)),
            )
            .child(
                resizable_panel()
                    .size_range(rem * size::PANE_MIN.0..gpui_kit::Pixels::MAX)
                    .child(self.render_detail()),
            )
    }
}
