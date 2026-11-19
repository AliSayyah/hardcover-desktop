//! Search: a debounced query over books, authors or series, paged results
//! and the shared detail pane.
//!
//! Every query change bumps a revision; a response is applied only if its
//! revision is still current, so a slow page for an old query never replaces
//! newer results. Each kind keeps its own results and request: only the kind
//! shown is searched, and switching kinds searches the current query there.
//!
//! Books open in the detail pane beside the list. Authors and series open
//! their pages, on Enter or a click.

use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, h_resizable, resizable_panel, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{AuthorHit, BookId, BookSummary, Client, SearchHits, SeriesHit};

use crate::app_state::Services;
use crate::book::detail_view::{BookDetailView, Subject};
use crate::catalog::navigation::{self, Page};
use crate::format::{author_hit_details, series_hit_details};
use crate::library::book_row::BookRow;
use crate::library::library_view::empty_state;
use crate::library::model::LibraryModel;
use crate::search::results::{Hit, Phase, SearchResults};

const DEBOUNCE: Duration = Duration::from_millis(300);

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
        ("search-book", self.id.get() as u64)
    }

    fn name(&self) -> SharedString {
        self.title.clone().into()
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
        BookRow::for_hit(
            self.name.clone(),
            author_hit_details(self),
            self.photo.as_ref().map(|p| p.url.clone()),
            IconName::User,
        )
    }

    fn hint() -> &'static str {
        "Find authors by name or by their books."
    }

    fn row_id(&self) -> (&'static str, u64) {
        ("search-author", self.id.get() as u64)
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
        BookRow::for_hit(
            self.name.clone(),
            series_hit_details(self),
            None,
            IconName::GalleryVerticalEnd,
        )
    }

    fn hint() -> &'static str {
        "Find series by name, author or one of their books."
    }

    fn row_id(&self) -> (&'static str, u64) {
        ("search-series", self.id.get() as u64)
    }

    fn name(&self) -> SharedString {
        self.name.clone().into()
    }
}

pub struct ResultsDelegate<T: SearchHit> {
    results: SearchResults<T>,
    model: Entity<LibraryModel>,
    view: WeakEntity<SearchView>,
}

impl<T: SearchHit> ListDelegate for ResultsDelegate<T> {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.results.hits().len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let hit = self.results.hits().get(ix.row)?;
        Some(
            ListItem::new(hit.row_id())
                .accessibility_label(hit.name())
                .child(hit.row(self.model.read(cx))),
        )
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        // Book selection is owned by `SearchView`, keyed by book id; authors
        // and series open on confirm.
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let (icon, title, description): (IconName, SharedString, SharedString) =
            match self.results.phase() {
                Phase::Idle => (
                    IconName::Search,
                    "Search Hardcover".into(),
                    T::hint().into(),
                ),
                Phase::Failed(message) => (
                    IconName::CircleAlert,
                    "Search failed".into(),
                    message.clone().into(),
                ),
                _ => (
                    IconName::Search,
                    "No results".into(),
                    format!("Nothing on Hardcover matches “{}”.", self.results.query()).into(),
                ),
            };
        empty_state(icon, title, description)
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
    _subscriptions: Vec<Subscription>,
}

impl SearchView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(SearchKind::Books.placeholder()));
        let view = cx.entity().downgrade();
        let books = new_results_list(&model, &view, window, cx);
        let authors = new_results_list(&model, &view, window, cx);
        let series = new_results_list(&model, &view, window, cx);
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
                    InputEvent::PressEnter { .. } => this.focus_results(window, cx),
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
                    let book = list.read(cx).delegate().results.hits().get(ix.row).cloned();
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
                        let author = list.read(cx).delegate().results.hits().get(ix.row).cloned();
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
                        let series = list.read(cx).delegate().results.hits().get(ix.row).cloned();
                        if let Some(series) = series {
                            navigation::open(Page::Series(series.id), cx);
                        }
                    }
                },
            ),
            // Status badges follow library changes.
            cx.observe(&model, |this, _, cx| {
                this.books.update(cx, |_, cx| cx.notify())
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
            _subscriptions: subscriptions,
        }
    }

    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    fn focus_results(&self, window: &mut Window, cx: &mut App) {
        match self.kind {
            SearchKind::Books => focus_list(&self.books, window, cx),
            SearchKind::Authors => focus_list(&self.authors, window, cx),
            SearchKind::Series => focus_list(&self.series, window, cx),
        }
    }

    fn select(&mut self, book: BookSummary, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(book.id);
        self.sync_list_selection(window, cx);
        self.detail.update(cx, |d, cx| {
            d.set_subject(Some(Subject::Book(book)), window, cx)
        });
    }

    fn set_kind(&mut self, kind: SearchKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        self.input.update(cx, |input, cx| {
            input.set_placeholder(kind.placeholder(), window, cx)
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
            let revision = list.delegate_mut().results.start(query.trim().into());
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
                this.receive(revision, result, window, cx)
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
                this.receive(revision, result, window, cx)
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
            let ix = selected
                .and_then(|id| {
                    list.delegate()
                        .results
                        .hits()
                        .iter()
                        .position(|b| b.id == id)
                })
                .map(IndexPath::new);
            if list.selected_index() != ix {
                list.set_selected_index(ix, window, cx);
            }
        });
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, busy, list) = match self.kind {
            SearchKind::Books => results_status(&self.books, cx),
            SearchKind::Authors => results_status(&self.authors, cx),
            SearchKind::Series => results_status(&self.series, cx),
        };
        let view = cx.entity().downgrade();
        v_flex()
            .size_full()
            .min_w_0()
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Input::new(&self.input)
                            .cleanable(true)
                            .prefix(
                                Icon::new(IconName::Search).text_color(cx.theme().muted_foreground),
                            )
                            .when(busy, |this| this.suffix(Spinner::new().small())),
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
                        div()
                            .h_4()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .children(status),
                    ),
            )
            .child(div().flex_1().min_h_0().child(list))
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
    window: &mut Window,
    cx: &mut App,
) -> Entity<ListState<ResultsDelegate<T>>> {
    cx.new(|cx| {
        ListState::new(
            ResultsDelegate {
                results: SearchResults::default(),
                model: model.clone(),
                view: view.clone(),
            },
            window,
            cx,
        )
    })
}

fn focus_list<T: SearchHit>(
    list: &Entity<ListState<ResultsDelegate<T>>>,
    window: &mut Window,
    cx: &mut App,
) {
    if !list.read(cx).delegate().results.hits().is_empty() {
        list.update(cx, |list, cx| list.focus(window, cx));
    }
}

/// The status line under the input, whether a request is running, and the list.
fn results_status<T: SearchHit>(
    list: &Entity<ListState<ResultsDelegate<T>>>,
    cx: &App,
) -> (Option<SharedString>, bool, AnyElement) {
    let results = &list.read(cx).delegate().results;
    let status: Option<SharedString> = match results.phase() {
        Phase::Searching => Some("Searching…".into()),
        Phase::LoadingMore => Some("Loading more…".into()),
        Phase::Loaded => results.total().map(|t| match t {
            1 => "1 result".into(),
            n => format!("{n} results").into(),
        }),
        Phase::Failed(message) if !results.hits().is_empty() => Some(message.clone().into()),
        _ => None,
    };
    let busy = matches!(results.phase(), Phase::Searching | Phase::LoadingMore);
    (
        status,
        busy,
        List::new(list).size_full().p_1().into_any_element(),
    )
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        h_resizable("search-split")
            .child(
                resizable_panel()
                    .size(rem * 24.)
                    .size_range(rem * 16.0..rem * 40.)
                    .child(self.render_results(cx)),
            )
            .child(
                resizable_panel()
                    // With the sidebar, 16rem + 18rem fits the 800px minimum window.
                    .size_range(rem * 18.0..gpui_kit::Pixels::MAX)
                    .child(self.render_detail()),
            )
    }
}
