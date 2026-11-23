//! An author page: photo, bio, then their books or their series.
//!
//! The header puts the author's photo where a book has its cover, a large
//! `ui::Avatar` (initials on `accent-soft` without one), beside their name
//! in `display-24`, the facts, how many of their books the reader has read
//! and "Open on Hardcover". The bio follows in `prose`.
//!
//! Under the header, a small segmented "Books" | "Series" switches what the
//! page lists (an author without series gets the "Books" heading instead).
//! Books: the order, and the books as catalogue rows. Series: every series,
//! the most read first, as sequence rows on the book rows' spine, each with
//! the cover of their most-read book in it, the name as a text button that
//! opens the series page, and its facts ("7 books · Complete · 3 read"). The
//! choice goes back to Books for another author. The book pane beside a wide
//! page keeps the book selected in Books.
//!
//! The photo, name and bio scroll on their own in a short window
//! (`book_list::page_header_scrolling`); the switch and the list stay, with
//! room for at least three rows.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::ScrollHandle;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListState;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{IndexPath, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::{
    AuthorBook, AuthorDetails, AuthorId, AuthorSeries, LibraryEntry, ReadingStatus, SeriesId,
};

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, PageRows, demand, focus_list_if_fallback_is_focused,
    follow_list, new_list, page_body, page_header, page_header_scrolling, page_list, read_summary,
    reset_scroll, row_details,
};
use super::navigation::Page;
use crate::bidi::WrappingText;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::library::model::LibraryModel;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::kit::OnTextSpine as _;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::small_tab;
use crate::ui::reveal::RevealScroll as _;
use crate::ui::{
    Avatar, AvatarSize, ExternalLink, LoadFailed, SectionHeader, Sequence, Skeleton, TextButton,
};

/// How an author's books are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BookOrder {
    #[default]
    MostRead,
    Newest,
    Oldest,
}

impl BookOrder {
    const ALL: [BookOrder; 3] = [BookOrder::MostRead, BookOrder::Newest, BookOrder::Oldest];

    fn label(self) -> &'static str {
        match self {
            BookOrder::MostRead => "Most read",
            BookOrder::Newest => "Newest",
            BookOrder::Oldest => "Oldest",
        }
    }
}

#[derive(Clone)]
pub struct OrderOption(BookOrder, SharedString);

impl SelectItem for OrderOption {
    type Value = BookOrder;

    fn title(&self) -> SharedString {
        self.1.clone()
    }

    fn value(&self) -> &BookOrder {
        &self.0
    }
}

/// `books` (most read first, as fetched) in `order`. Books without a year go
/// last when ordering by year; ties keep the most-read first.
fn ordered(books: &[AuthorBook], order: BookOrder) -> Vec<&AuthorBook> {
    let mut out: Vec<&AuthorBook> = books.iter().collect();
    match order {
        BookOrder::MostRead => {}
        BookOrder::Newest => {
            out.sort_by_key(|b| std::cmp::Reverse(b.book.release_year.unwrap_or(i32::MIN)));
        }
        BookOrder::Oldest => out.sort_by_key(|b| b.book.release_year.unwrap_or(i32::MAX)),
    }
    out
}

/// "1920–1986", "Born 1947", or nothing.
fn life_years(author: &AuthorDetails) -> Option<String> {
    match (author.born_year, author.death_year) {
        (Some(b), Some(d)) => Some(format!("{b}–{d}")),
        (Some(b), None) => Some(format!("Born {b}")),
        (None, Some(d)) => Some(format!("Died {d}")),
        (None, None) => None,
    }
}

/// Bios longer than this start collapsed.
const LONG_BIO: usize = 400;

/// What the page lists under its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Books,
    Series,
}

impl Tab {
    const ALL: [Tab; 2] = [Tab::Books, Tab::Series];

    fn label(self) -> &'static str {
        match self {
            Tab::Books => "Books",
            Tab::Series => "Series",
        }
    }
}

/// How many books of each series the reader has read, from their library.
fn read_per_series(entries: &[LibraryEntry]) -> HashMap<SeriesId, u32> {
    let mut read: HashMap<SeriesId, u32> = HashMap::new();
    for entry in entries.iter().filter(|e| e.status == ReadingStatus::Read) {
        let mut seen: Vec<SeriesId> = Vec::new();
        for id in entry.series.iter().filter_map(|s| s.id) {
            if !seen.contains(&id) {
                seen.push(id);
                *read.entry(id).or_default() += 1;
            }
        }
    }
    read
}

/// "7 books · Complete · 3 read". The count of books is Hardcover's count of
/// the main ones, and the read count the library's of any book in the series
/// (novellas, compilations), so the count is left out when it is lower than
/// the books read, rather than read as "2 books · 4 read".
fn series_facts(series: &AuthorSeries, read: u32) -> String {
    let mut facts: Vec<String> = Vec::new();
    match series.primary_books_count.filter(|n| *n >= read) {
        Some(1) => facts.push("1 book".into()),
        Some(n) => facts.push(format!("{} books", crate::format::count_text(n))),
        None => {}
    }
    match series.is_completed {
        Some(true) => facts.push("Complete".into()),
        Some(false) => facts.push("Ongoing".into()),
        None => {}
    }
    if read > 0 {
        facts.push(format!("{} read", crate::format::count_text(read)));
    }
    facts.join(" · ")
}

pub struct AuthorView {
    model: Entity<LibraryModel>,
    /// The session's catalogue, which holds the pages; `None` once signed
    /// out.
    catalog: Option<Catalog>,
    current: Option<AuthorId>,
    /// Holds focus while the page loads; see [`Self::focus`].
    focus_when_ready: Option<FocusHandle>,
    order: BookOrder,
    order_select: Entity<SelectState<Vec<OrderOption>>>,
    bio_expanded: bool,
    /// What the page lists: Books unless the reader picked Series.
    tab: Tab,
    /// The Series tab's list, which takes focus from the book list when the
    /// tab changes under it.
    series_focus: FocusHandle,
    list: Entity<ListState<CatalogBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// The author whose books the list holds, and each page's selection.
    rows: PageRows<AuthorId, AuthorDetails>,
    /// The order the list's rows are in.
    listed_order: BookOrder,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl AuthorView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = ListPane::new(model.clone(), window, cx);
        let list = new_list(
            model.clone(),
            "Hardcover lists no books by this author.",
            &pane,
            window,
            cx,
        );
        let options: Vec<OrderOption> = BookOrder::ALL
            .into_iter()
            .map(|o| OrderOption(o, o.label().into()))
            .collect();
        let order_select =
            cx.new(|cx| SelectState::new(options, Some(IndexPath::new(0)), window, cx));
        let mut subscriptions = follow_list(&list, &model, &pane, window, cx);
        let catalog = Catalog::global(cx);
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe_in(
                &catalog.authors,
                window,
                |this, _, SharedChanged(id): &SharedChanged<AuthorId>, window, cx| {
                    if this.current == Some(*id) {
                        this.page_changed(window, cx);
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &order_select,
            window,
            |this, _, event: &SelectEvent<Vec<OrderOption>>, window, cx| {
                if let SelectEvent::Confirm(Some(order)) = event {
                    this.order = *order;
                    this.fill_list(window, cx);
                }
            },
        ));
        Self {
            model,
            catalog,
            current: None,
            focus_when_ready: None,
            order: BookOrder::default(),
            order_select,
            bio_expanded: false,
            tab: Tab::default(),
            series_focus: cx.focus_handle(),
            list,
            pane,
            rows: PageRows::default(),
            listed_order: BookOrder::default(),
            header_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, id: AuthorId, window: &mut Window, cx: &mut Context<Self>) {
        if self.current != Some(id) {
            self.bio_expanded = false;
            self.tab = Tab::default();
            reset_scroll(&self.header_scroll);
        }
        self.current = Some(id);
        self.request(Demand::Shown, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list, which renders only once the page has loaded.
    /// Until then `fallback` holds focus, so Escape and Back keep working, and
    /// the list takes over when the page arrives if `fallback` still has it.
    pub fn focus(&mut self, fallback: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.page(cx);
        if page.as_deref().map(|author| self.tab_for(author)) == Some(Tab::Series) {
            self.focus_when_ready = None;
            window.focus(&self.series_focus, cx);
        } else if page.is_some() {
            self.focus_when_ready = None;
            crate::ui::enter_list(&self.list, window, cx);
        } else {
            self.focus_when_ready = Some(fallback.clone());
            window.focus(fallback, cx);
        }
    }

    /// The current page, once fetched.
    fn page(&self, cx: &App) -> Option<Arc<AuthorDetails>> {
        let id = self.current?;
        self.catalog
            .as_ref()?
            .authors
            .read(cx)
            .resource()
            .value(&id)
            .cloned()
    }

    fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if let (Some(id), Some(catalog)) = (self.current, &self.catalog) {
            catalog.request_author(id, demand, cx);
        }
    }

    /// The page shown changed in the catalogue: it loaded, failed, or was
    /// fetched again.
    fn page_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_list(window, cx);
        if self.page(cx).is_some() {
            focus_list_if_fallback_is_focused(&self.list, self.focus_when_ready.take(), window, cx);
        }
        cx.notify();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Now, cx);
        cx.notify();
    }

    fn fill_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        let page = self.page(cx);
        let order = self.order;
        let reorder = std::mem::replace(&mut self.listed_order, order) != order;
        self.rows.show(
            id,
            page.as_ref(),
            |author| Self::rows(author, order),
            reorder,
            &self.list,
            window,
            cx,
        );
    }

    /// The list's rows for a page, in `order`.
    fn rows(author: &AuthorDetails, order: BookOrder) -> Vec<CatalogRow> {
        ordered(&author.books, order)
            .into_iter()
            .map(|b| CatalogRow {
                details: row_details(&[b.role.clone().unwrap_or_default()], &b.book),
                book: b.book.clone(),
            })
            .collect()
    }

    /// Asks for the page again if it is stale (`resource`; a failed one only
    /// when `shown_again`), and for the book in the pane beside the list.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.request(demand(shown_again), cx);
        self.pane.revalidate(shown_again, cx);
    }

    fn render_header(&self, author: &AuthorDetails, cx: &mut Context<Self>) -> AnyElement {
        let muted = colors(cx).fg_muted;
        let model = self.model.read(cx);
        let mut facts: Vec<String> = Vec::new();
        if let Some(years) = life_years(author) {
            facts.push(years);
        }
        match author.books_count.filter(|n| *n > 0) {
            Some(1) => facts.push("1 book on Hardcover".into()),
            Some(n) => facts.push(format!(
                "{} books on Hardcover",
                crate::format::count_text(n)
            )),
            None => {}
        }
        let summary = read_summary(
            author
                .books
                .iter()
                .filter(|b| b.role.is_none())
                .map(|b| model.entry_for_book(b.book.id).map(|e| e.status)),
        );
        let long_bio = author
            .bio
            .as_ref()
            .is_some_and(|b| b.chars().count() > LONG_BIO);
        v_flex()
            .gap_6()
            .child(
                h_flex()
                    .gap_6()
                    .items_start()
                    .child(
                        // Scaled on the cover-decoder thread like a row
                        // thumbnail, not decoded full size on the shared pool.
                        Avatar::new(author.name.clone())
                            .size(AvatarSize::Xl)
                            .image(author.photo.as_ref().map(|p| p.url.clone())),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .display_24()
                                    .child(WrappingText::new(author.name.clone())),
                            )
                            .when(!facts.is_empty(), |this| {
                                this.child(
                                    div()
                                        .body()
                                        .tabular()
                                        .text_color(muted)
                                        .child(facts.join(" · ")),
                                )
                            })
                            .when_some(summary, |this, summary| {
                                this.child(div().body().tabular().child(summary))
                            })
                            .when_some(author.web_url(), |this, url| {
                                this.child(div().pt_3().child(ExternalLink::new(
                                    "author-on-hardcover",
                                    "Open on Hardcover",
                                    url,
                                )))
                            }),
                    ),
            )
            .when_some(author.bio.clone(), |this, bio| {
                let collapsed = long_bio && !self.bio_expanded;
                this.child(
                    v_flex()
                        .gap_2()
                        .items_start()
                        .child(div().max_w(size::PROSE).prose().child({
                            let bio = WrappingText::new(bio);
                            if collapsed { bio.line_clamp(4) } else { bio }
                        }))
                        .when(long_bio, |this| {
                            this.child(
                                Button::new("toggle-bio")
                                    .ghost()
                                    .small_control()
                                    // The label on the bio's spine, not the
                                    // button's edge.
                                    .on_text_spine(true)
                                    .label(if self.bio_expanded {
                                        "Show less"
                                    } else {
                                        "Read more"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.bio_expanded = !this.bio_expanded;
                                        cx.notify();
                                    })),
                            )
                        }),
                )
            })
            .into_any_element()
    }

    /// The tab shown for `author`: Books when they have no series.
    fn tab_for(&self, author: &AuthorDetails) -> Tab {
        if author.series.is_empty() {
            Tab::Books
        } else {
            self.tab
        }
    }

    /// Switches the list under the header. Focus on the list that goes moves
    /// to the one that comes, so Escape and the keys keep working.
    fn set_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        let list_focused = self
            .list
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let series_focused = self.series_focus.contains_focused(window, cx);
        self.tab = tab;
        match tab {
            Tab::Series if list_focused => window.focus(&self.series_focus, cx),
            Tab::Books if series_focused => crate::ui::enter_list(&self.list, window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Every series, as sequence rows on the book rows' spine, scrolling in
    /// the list's place.
    fn render_series(&self, series: &[AuthorSeries], cx: &mut Context<Self>) -> AnyElement {
        let muted = colors(cx).fg_muted;
        let read = read_per_series(self.model.read(cx).entries());
        let rows = series.iter().map(|s| {
            let id = s.id.get();
            let page = Page::Series(s.id);
            let facts = series_facts(s, read.get(&s.id).copied().unwrap_or(0));
            h_flex()
                .w_full()
                .gap_3()
                .child(
                    Cover::new(CoverSize::Sm, s.lead.title.clone())
                        .thumbnail(s.lead.cover.as_ref().map(|c| c.url.clone()))
                        .opens(
                            SharedString::from(format!("author-series-cover-{id}")),
                            page.clone(),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            TextButton::new(
                                SharedString::from(format!("author-series-{id}")),
                                s.name.clone(),
                            )
                            .opens(page),
                        )
                        .when(!facts.is_empty(), |this| {
                            this.child(div().body().text_color(muted).truncate().child(facts))
                        }),
                )
        });
        div()
            .id("author-series")
            .track_focus(&self.series_focus)
            .size_full()
            .scroll_y_revealing_focus()
            // The page's inset: the list's 24 and a row's own 8.
            .child(
                div()
                    .px_8()
                    .pb_6()
                    .child(Sequence::new().flush_top().children(rows)),
            )
            .into_any_element()
    }

    /// The switch between books and series (or "Books" alone) and, for the
    /// books, the order, over the list: 24 under the header (whose own 16 of
    /// foot this completes).
    fn render_list_bar(
        &self,
        tab: Tab,
        has_series: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let order = (tab == Tab::Books).then(|| {
            div().w(size::SELECT_SMALL).child(
                Select::new(&self.order_select)
                    .small_control()
                    .raised_field(false, cx),
            )
        });
        let bar = if has_series {
            let view = cx.entity().downgrade();
            let selected = Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
            h_flex()
                .w_full()
                .min_h(size::CONTROL_SMALL)
                .gap_2()
                .justify_between()
                .child(
                    TabBar::new("author-tab")
                        .segmented()
                        .small_control()
                        .selected_index(selected)
                        .children(Tab::ALL.map(|t| small_tab(t.label())))
                        .on_click(move |ix, window, cx| {
                            let tab = Tab::ALL[*ix];
                            let _ = view.update(cx, |this, cx| this.set_tab(tab, window, cx));
                        }),
                )
                .children(order)
                .into_any_element()
        } else {
            SectionHeader::new("Books")
                .when_some(order, |this, order| this.action(order))
                .into_any_element()
        };
        page_header().pt_2().child(bar)
    }
}

impl AuthorView {
    /// The name of a page already loaded this session, for Back's label.
    pub fn name<'a>(&self, id: AuthorId, cx: &'a App) -> Option<&'a str> {
        let author = self
            .catalog
            .as_ref()?
            .authors
            .read(cx)
            .resource()
            .value(&id)?;
        Some(&author.name)
    }
}

impl Render for AuthorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.page(cx);
        let error = match (self.current, &self.catalog) {
            (Some(id), Some(catalog)) => {
                let store = catalog.authors.read(cx).resource();
                store.error(&id).filter(|_| !store.is_loading(&id)).cloned()
            }
            _ => None,
        };
        let listing = page.as_ref().map(|author| {
            let tab = self.tab_for(author);
            let body = match tab {
                Tab::Books => page_list(&self.list).into_any_element(),
                Tab::Series => self.render_series(&author.series, cx),
            };
            (tab, !author.series.is_empty(), body)
        });
        let header = match (page, error) {
            (Some(author), _) => self.render_header(&author, cx),
            (None, Some(message)) => LoadFailed::new("retry-author", message)
                .on_retry(cx.listener(|this, _, _, cx| this.retry(cx)))
                .into_any_element(),
            (None, None) => h_flex()
                .gap_6()
                .items_start()
                .child(Skeleton::circle().size(size::AVATAR_LARGE))
                .child(
                    v_flex()
                        .gap_2()
                        .child(Skeleton::line_in(TypeRole::Display24).w_64())
                        .child(Skeleton::line().w_48())
                        .child(Skeleton::line().w_32()),
                )
                .into_any_element(),
        };
        let column = v_flex().size_full().min_w_0().map(|this| {
            if let Some((tab, has_series, body)) = listing {
                // The header scrolls on its own when the window is short;
                // the switch and the list keep their room.
                this.child(page_header_scrolling(
                    "author-header",
                    &self.header_scroll,
                    header,
                ))
                .child(self.render_list_bar(tab, has_series, cx))
                .child(page_body().child(body))
            } else {
                this.child(page_header().child(header))
            }
        });
        self.pane
            .layout(&self.list, CatalogBooks::book_at, column, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{BookId, BookSummary};

    fn book(id: i64, year: Option<i32>) -> AuthorBook {
        let mut b = BookSummary::new(BookId::new(id), format!("Book {id}"));
        b.release_year = year;
        AuthorBook::new(b)
    }

    #[test]
    fn orders_by_year_with_unknown_years_last() {
        let books = vec![
            book(1, Some(1990)),
            book(2, None),
            book(3, Some(2001)),
            book(4, Some(1990)),
        ];
        let ids = |order| -> Vec<i64> {
            ordered(&books, order)
                .iter()
                .map(|b| b.book.id.get())
                .collect()
        };
        assert_eq!(ids(BookOrder::MostRead), vec![1, 2, 3, 4]);
        assert_eq!(ids(BookOrder::Newest), vec![3, 1, 4, 2]);
        assert_eq!(ids(BookOrder::Oldest), vec![1, 4, 3, 2]);
    }

    #[test]
    fn series_facts_leave_out_a_count_below_the_books_read() {
        let mut s = AuthorSeries::new(
            SeriesId::new(1),
            "A Series",
            BookSummary::new(BookId::new(1), "Book 1"),
        );
        assert_eq!(series_facts(&s, 0), "");
        s.primary_books_count = Some(7);
        s.is_completed = Some(true);
        assert_eq!(series_facts(&s, 0), "7 books · Complete");
        assert_eq!(series_facts(&s, 3), "7 books · Complete · 3 read");
        s.primary_books_count = Some(1);
        s.is_completed = Some(false);
        assert_eq!(series_facts(&s, 1), "1 book · Ongoing · 1 read");
        // Novellas and compilations read past Hardcover's count of main books.
        s.primary_books_count = Some(2);
        s.is_completed = None;
        assert_eq!(series_facts(&s, 4), "4 read");
    }

    #[test]
    fn read_per_series_counts_read_books_once_each() {
        use hardcover_core::{SeriesRef, UserBookId, UserId};
        let entry = |id: i64, status, series: &[i64]| {
            let mut e = LibraryEntry::new(
                UserBookId::new(id),
                UserId::new(1),
                BookSummary::new(BookId::new(id), format!("Book {id}")),
                status,
            );
            e.series = series
                .iter()
                .map(|s| {
                    let mut r = SeriesRef::new(format!("Series {s}"));
                    r.id = Some(SeriesId::new(*s));
                    r
                })
                .collect();
            e
        };
        let read = read_per_series(&[
            entry(1, ReadingStatus::Read, &[10, 20, 10]),
            entry(2, ReadingStatus::Read, &[10]),
            entry(3, ReadingStatus::CurrentlyReading, &[10, 20]),
        ]);
        assert_eq!(read.get(&SeriesId::new(10)), Some(&2));
        assert_eq!(read.get(&SeriesId::new(20)), Some(&1));
    }

    #[test]
    fn life_years_text() {
        let mut a = AuthorDetails::new(AuthorId::new(1), "A. Writer");
        assert_eq!(life_years(&a), None);
        a.born_year = Some(1920);
        assert_eq!(life_years(&a).as_deref(), Some("Born 1920"));
        a.death_year = Some(1986);
        assert_eq!(life_years(&a).as_deref(), Some("1920–1986"));
    }
}
