//! The book list on Discover, the series and author pages and a list's page:
//! catalogue rows (`hardcover-design`, Rows → Catalogue row) with each
//! book's library status, keyboard-navigable. Enter or a click opens the
//! book. Escape reaches the page's Back binding: the list's own Cancel lets
//! it through.
//!
//! **Beside a book pane:** when the page's region (the window less the
//! sidebar) fits the list at `size::PAGE_BESIDE_PANE` and a book pane of at
//! least `size::PANE_BESIDE_PAGE_MIN` ([`pane_beside_list`]), the page shows the
//! selected book in a [`ListPane`] beside the list, as a shelf or search
//! does. Selecting a row (a click, the arrow keys, the keyboard landing on
//! the list) shows its book there; Enter or a double click still opens the
//! full book page. Narrower, a click opens the page, as Enter does. The
//! pane is the shared book view (`book::detail_view`), so on a wide region
//! it takes its rail too.
//!
//! The pages share one frame: content filling the region inside the 32
//! inset ([`page_header`]; a description keeps the `size::PROSE` measure),
//! with the list's rows on the same spine ([`page_list`]: a row's own 8 of
//! padding inside a 24 inset). Beside the pane, the frame is
//! `size::PAGE_BESIDE_PANE` wide and the pane takes the rest ([`list_width`]).
//!
//! The list is a tab stop after the header's controls, and lands on the
//! first row when none is selected ([`follow_list`]).
//!
//! A header that can run long (a bio, a description) is a
//! [`page_header_scrolling`]: in a short window it shrinks and scrolls on its
//! own, and the list under it keeps room for three rows
//! (`size::PAGE_LIST_MIN`).

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::{IconName, IndexPath, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, ElementId, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use hardcover_core::{BookSummary, ReadingStatus};

use super::navigation::{self, Page};
use crate::book::detail_view::{BookDetailView, Subject};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::average_rating_text;
use crate::library::book_row::{BookRow, loading_rows};
use crate::library::model::LibraryModel;
use crate::ui::reveal::RevealScroll as _;
use crate::ui::{
    ListRow as _, RowFocus, empty_state, on_keyboard_entry, region_width, select_first_row,
};

/// The top of a page: inset 32 on three sides, 16 above the list (whose
/// rows add their own 8), filling the page's width.
pub fn page_header() -> Div {
    div().flex_none().w_full().px_8().pt_8().pb_4()
}

/// A [`page_header`] that gives way in a short window: it shrinks, down to
/// nothing if it must, and scrolls its content in `scroll` (the page's own
/// handle, for focus reveal and [`reset_scroll`]), so the list under it
/// keeps `size::PAGE_LIST_MIN`. GPUI's own scroll, not the kit's scrollbar
/// wrapper, which measures a clamped bio at its full height.
pub fn page_header_scrolling(
    id: impl Into<ElementId>,
    scroll: &ScrollHandle,
    content: impl IntoElement,
) -> impl IntoElement {
    page_header()
        .id(id)
        .flex_shrink_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .child(content)
        .reveals_focus(scroll)
}

/// Back to the top of a [`page_header_scrolling`], for another page.
pub fn reset_scroll(scroll: &ScrollHandle) {
    scroll.set_offset(gpui_kit::Point::default());
}

/// The region a page's list fills, under its header. It keeps
/// `size::PAGE_LIST_MIN` when a [`page_header_scrolling`] gives way.
pub fn page_body() -> Div {
    div().flex_1().min_h(size::PAGE_LIST_MIN).w_full()
}

/// A page's list, its rows' text on the header's spine.
pub fn page_list<D: ListDelegate>(list: &Entity<ListState<D>>) -> List<D> {
    List::new(list).size_full().px_6().pb_6()
}

/// Gives the list focus after a catalogue page loads, if the page's fallback
/// control still owns focus.
pub(super) fn focus_list_if_fallback_is_focused(
    list: &Entity<ListState<CatalogBooks>>,
    fallback: Option<FocusHandle>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(fallback) = fallback
        && fallback.is_focused(window)
    {
        crate::ui::enter_list(list, window, cx);
    }
}

/// A page that failed to load: what happened, in one muted sentence, and
/// `retry` ("Try again") beside it.
pub fn load_failed(message: SharedString, retry: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(div().body().text_color(colors(cx).fg_muted).child(message))
        .child(retry)
}

/// A page's rows while they load, on the list's spine.
pub fn page_loading() -> Div {
    div().px_6().child(loading_rows())
}

/// Whether a catalogue page whose region is `region` wide shows the book
/// pane beside its list: the list gets `size::PAGE_BESIDE_PANE` (a
/// description at its full measure) and the pane, past the hairline between
/// them, at least `size::PANE_BESIDE_PAGE_MIN`. Pure, from the width the
/// page's region gets (the window less the sidebar).
pub fn pane_beside_list(region: Pixels, rem_size: Pixels) -> bool {
    region
        >= size::PAGE_BESIDE_PANE.to_pixels(rem_size)
            + size::HAIRLINE
            + size::PANE_BESIDE_PAGE_MIN.to_pixels(rem_size)
}

/// How wide a catalogue page's list (its header and rows, insets included)
/// is in a region `region` wide: the whole region, or
/// `size::PAGE_BESIDE_PANE` when the pane sits beside it
/// ([`pane_beside_list`]), which takes the rest.
pub fn list_width(region: Pixels, rem_size: Pixels) -> Pixels {
    if pane_beside_list(region, rem_size) {
        size::PAGE_BESIDE_PANE.to_pixels(rem_size)
    } else {
        region
    }
}

/// Finds the book in a list's row, for [`ListPane`].
pub type BookAt<D> = fn(&D, usize) -> Option<BookSummary>;

/// The book pane beside a catalogue page's list, when the page is wide
/// ([`pane_beside_list`]). It shows the book at the list's selected row,
/// and asks Hardcover for nothing while it isn't beside the list. See the
/// module doc.
#[derive(Clone)]
pub struct ListPane {
    detail: Entity<BookDetailView>,
    /// Shown beside the list, as of the page's last layout. The list's rows
    /// read it to decide what a click does ([`select_on_click`]).
    beside: Rc<Cell<bool>>,
}

impl ListPane {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut App) -> Self {
        Self {
            detail: cx.new(|cx| BookDetailView::new(model, window, cx)),
            beside: Rc::new(Cell::new(false)),
        }
    }

    /// Whether the pane is beside the list, for a list's rows
    /// ([`select_on_click`]).
    pub fn beside(&self) -> &Rc<Cell<bool>> {
        &self.beside
    }

    /// Shows the book at `list`'s selected row in the pane, if the pane is
    /// beside the list.
    fn sync<D: ListDelegate>(
        &self,
        list: &Entity<ListState<D>>,
        book_at: BookAt<D>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.beside.get() {
            return;
        }
        let list = list.read(cx);
        let book = list
            .selected_index()
            .and_then(|ix| book_at(list.delegate(), ix.row));
        self.detail.update(cx, |detail, cx| {
            detail.set_subject(book.map(Subject::Book), window, cx);
        });
    }

    /// Fetches the pane's book again if it is stale ([`crate::freshness`]),
    /// when the pane is beside the list. `shown_again` allows changes that
    /// move what the reader is looking at (see [`BookDetailView::revalidate`]).
    pub fn revalidate(&self, shown_again: bool, cx: &mut App) {
        if self.beside.get() {
            self.detail
                .update(cx, |detail, cx| detail.revalidate(shown_again, cx));
        }
    }

    /// The page: `column` (its header and list) alone, filling the region,
    /// or at [`list_width`] beside the pane when the region is wide.
    pub fn layout<D: ListDelegate>(
        &self,
        list: &Entity<ListState<D>>,
        book_at: BookAt<D>,
        column: impl IntoElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let rem = window.rem_size();
        let region = region_width(window, gpui_kit::rems(0.));
        let beside = pane_beside_list(region, rem);
        if self.beside.replace(beside) != beside {
            // Outside this frame's drawing: grown past the threshold, the
            // pane catches up with the selection; shrunk below it, the
            // hidden pane lets go of its book, so nothing it shows (a
            // review after a sync) is fetched again.
            let (pane, list) = (self.clone(), list.clone());
            window.defer(cx, move |window, cx| {
                if beside {
                    pane.sync(&list, book_at, window, cx);
                } else {
                    pane.detail
                        .update(cx, |detail, cx| detail.set_subject(None, window, cx));
                }
            });
        }
        if !beside {
            return column.into_any_element();
        }
        h_flex()
            .size_full()
            .items_stretch()
            .child(
                div()
                    .flex_none()
                    .w(list_width(region, rem))
                    .h_full()
                    .child(column),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .border_l_1()
                    .border_color(colors(cx).border)
                    .child(self.detail.clone()),
            )
            .into_any_element()
    }
}

/// Keeps `pane` on the book at `list`'s selected row, however the selection
/// moves: a click, the arrows, the keyboard landing on the list, a new
/// page of rows.
pub fn follow_pane<V: 'static, D: ListDelegate>(
    list: &Entity<ListState<D>>,
    pane: &ListPane,
    book_at: BookAt<D>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Subscription {
    let pane = pane.clone();
    cx.observe_in(list, window, move |_, list, window, cx| {
        pane.sync(&list, book_at, window, cx);
    })
}

/// Makes a single click on `item` (row `ix`) select it while the book pane
/// is beside the list (`beside`), instead of opening the book: the list's
/// own click handler, which confirms, doesn't run. A double click, and any
/// click without the pane, falls through to it and opens the book.
pub fn select_on_click<D: ListDelegate>(
    item: ListItem,
    ix: IndexPath,
    beside: &Rc<Cell<bool>>,
    cx: &mut Context<ListState<D>>,
) -> ListItem {
    let beside = beside.clone();
    let list = cx.entity().downgrade();
    item.on_click(move |event, window, cx| {
        if !beside.get() || event.click_count() > 1 {
            return;
        }
        cx.stop_propagation();
        let _ = list.update(cx, |list, cx| {
            list.set_selected_index(Some(ix), window, cx);
            cx.emit(ListEvent::Select(ix));
            cx.notify();
        });
    })
}

/// Replaces the list's rows after its page was fetched again, keeping the
/// selected book selected at its new row, so the pane beside the list keeps
/// showing it; nothing stays selected if the book is gone.
pub fn replace_rows(
    list: &Entity<ListState<CatalogBooks>>,
    rows: Vec<CatalogRow>,
    window: &mut Window,
    cx: &mut App,
) {
    list.update(cx, |list, cx| {
        let selected = list
            .selected_index()
            .and_then(|ix| list.delegate().rows.get(ix.row))
            .map(|row| row.book.id);
        let row = selected.and_then(|id| rows.iter().position(|r| r.book.id == id));
        list.delegate_mut().set_rows(rows);
        list.set_selected_index(row.map(IndexPath::new), window, cx);
        cx.notify();
    });
}

/// One book and its secondary line.
#[derive(Clone)]
pub struct CatalogRow {
    pub book: BookSummary,
    pub details: SharedString,
}

pub struct CatalogBooks {
    rows: Vec<CatalogRow>,
    model: Entity<LibraryModel>,
    empty: SharedString,
    focus: RowFocus,
    beside: Rc<Cell<bool>>,
}

impl CatalogBooks {
    pub fn set_rows(&mut self, rows: Vec<CatalogRow>) {
        self.rows = rows;
    }

    /// The book in row `ix`, for the [`ListPane`].
    pub fn book_at(&self, ix: usize) -> Option<BookSummary> {
        self.rows.get(ix).map(|row| row.book.clone())
    }
}

impl ListDelegate for CatalogBooks {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.focus.stop_when_rows(self.rows.len());
        self.rows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let focused = self.focus.shows_focus(ix, window);
        let row = self.rows.get(ix.row)?;
        let status = self
            .model
            .read(cx)
            .entry_for_book(row.book.id)
            .map(|e| e.status);
        let item = ListItem::new(("catalog-book", row.book.id.get().cast_unsigned()))
            .list_row(focused, cx)
            .accessibility_label(row.book.title.clone())
            .child(BookRow::for_catalog(&row.book, row.details.clone(), status));
        Some(select_on_click(item, ix, &self.beside, cx))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.focus.select(ix);
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        empty_state(IconName::BookOpen, "No books".into(), self.empty.clone())
    }
}

/// A list for a catalogue page, beside `pane` when the page is wide.
/// `empty` explains an empty list.
pub fn new_list(
    model: Entity<LibraryModel>,
    empty: impl Into<SharedString>,
    pane: &ListPane,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ListState<CatalogBooks>> {
    let empty = empty.into();
    let list = cx.new(|cx| {
        ListState::new(
            CatalogBooks {
                rows: Vec::new(),
                model,
                empty,
                focus: RowFocus::default(),
                beside: pane.beside.clone(),
            },
            window,
            cx,
        )
    });
    RowFocus::attach(&list, |d| &mut d.focus, cx);
    list
}

/// Opens the confirmed book. Arrowing only moves the selection (and shows
/// the book in `pane` when it is beside the list); the keyboard bringing
/// focus to the list selects its first row if none is.
pub fn follow_list<V: 'static>(
    list: &Entity<ListState<CatalogBooks>>,
    model: &Entity<LibraryModel>,
    pane: &ListPane,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Vec<Subscription> {
    let entered = list.clone();
    vec![
        follow_pane(list, pane, CatalogBooks::book_at, window, cx),
        on_keyboard_entry(list, window, cx, move |_, window, cx| {
            select_first_row(&entered, window, cx);
        }),
        cx.subscribe(list, |_, list, event: &ListEvent, cx| match event {
            ListEvent::Confirm(ix) => {
                let book = list.read(cx).delegate().rows.get(ix.row).cloned();
                if let Some(row) = book {
                    navigation::open(Page::Book(row.book), cx);
                }
            }
            ListEvent::Select(_) | ListEvent::Cancel => {}
        }),
        // Status tags follow library changes.
        cx.observe(model, {
            let list = list.clone();
            move |_, _, cx| list.update(cx, |_, cx| cx.notify())
        }),
    ]
}

/// The secondary line: the parts given, then the year and the average rating.
pub fn row_details(lead: &[String], book: &BookSummary) -> SharedString {
    let mut parts: Vec<String> = lead.iter().filter(|p| !p.is_empty()).cloned().collect();
    if let Some(year) = book.release_year {
        parts.push(year.to_string());
    }
    if let Some(avg) = book.average_rating.filter(|a| *a > 0.0) {
        parts.push(format!("{} average", average_rating_text(avg)));
    }
    parts.join(" · ").into()
}

/// "You've read 3 of 7", over the books' library statuses; `None` for no books.
pub fn read_summary(statuses: impl IntoIterator<Item = Option<ReadingStatus>>) -> Option<String> {
    let mut total = 0;
    let mut read = 0;
    for status in statuses {
        total += 1;
        if status == Some(ReadingStatus::Read) {
            read += 1;
        }
    }
    (total > 0).then(|| format!("You've read {read} of {total}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::BookId;

    #[test]
    fn read_summary_counts_read_books_only() {
        assert_eq!(read_summary([]), None);
        assert_eq!(
            read_summary([
                Some(ReadingStatus::Read),
                Some(ReadingStatus::CurrentlyReading),
                None,
                Some(ReadingStatus::Read),
            ])
            .as_deref(),
            Some("You've read 2 of 4")
        );
    }

    #[test]
    fn row_details_skips_missing_parts() {
        let mut book = BookSummary::new(BookId::new(1), "A Book");
        assert_eq!(row_details(&[], &book).as_ref(), "");
        book.release_year = Some(1965);
        book.average_rating = Some(4.234);
        assert_eq!(
            row_details(&["#2".into(), String::new()], &book).as_ref(),
            "#2 · 1965 · 4.23 average"
        );
    }

    #[test]
    fn the_pane_shows_beside_the_list_only_when_both_fit() {
        let rem = gpui_kit::px(16.);
        let region = |window: f32| gpui_kit::px(window - 240.);
        // 704 + 1 + 392 = 1097, a 1337 window.
        assert!(pane_beside_list(gpui_kit::px(1097.), rem));
        assert!(!pane_beside_list(gpui_kit::px(1096.), rem));
        assert!(pane_beside_list(region(1337.), rem));
        assert!(!pane_beside_list(region(1336.), rem));
        // The minimum and default windows: the page opens books, as before.
        assert!(!pane_beside_list(region(800.), rem));
        assert!(!pane_beside_list(region(1200.), rem));
        // A 1600 laptop panel and a 4K monitor at 1.5.
        assert!(pane_beside_list(region(1600.), rem));
        assert!(pane_beside_list(region(2560.), rem));
    }

    #[test]
    fn the_list_fills_the_region_unless_the_pane_is_beside_it() {
        let rem = gpui_kit::px(16.);
        let region = |window: f32| gpui_kit::px(window - 240.);
        // Alone, the list (and its header) fills the region.
        assert_eq!(list_width(region(800.), rem), gpui_kit::px(560.));
        assert_eq!(list_width(region(1200.), rem), gpui_kit::px(960.));
        assert_eq!(list_width(region(1336.), rem), gpui_kit::px(1096.));
        // Beside the pane it keeps the prose measure and its insets; the
        // pane, past the hairline, takes the rest.
        for window in [1337., 1600., 2560.] {
            assert_eq!(list_width(region(window), rem), gpui_kit::px(704.));
        }
        let pane = |window: f32| region(window) - list_width(region(window), rem) - size::HAIRLINE;
        assert_eq!(pane(1337.), gpui_kit::px(392.));
        assert_eq!(pane(1600.), gpui_kit::px(655.));
        assert_eq!(pane(2560.), gpui_kit::px(1615.));
    }
}

#[cfg(test)]
mod click_tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gpui_kit::component::IndexPath;
    use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent,
        MouseUpEvent, ParentElement as _, Styled as _, TestAppContext, VisualTestContext, Window,
        div, point, px,
    };

    use super::select_on_click;
    use crate::ui::test_support::{host, press, redraw};

    struct Rows {
        beside: Rc<Cell<bool>>,
    }

    impl ListDelegate for Rows {
        type Item = ListItem;

        fn items_count(&self, _: usize, _: &gpui_kit::App) -> usize {
            3
        }

        fn render_item(
            &mut self,
            ix: IndexPath,
            _: &mut Window,
            cx: &mut Context<ListState<Self>>,
        ) -> Option<ListItem> {
            let item = ListItem::new(("row", ix.row)).h_10().child("A book");
            Some(select_on_click(item, ix, &self.beside, cx))
        }

        fn set_selected_index(
            &mut self,
            _: Option<IndexPath>,
            _: &mut Window,
            _: &mut Context<ListState<Self>>,
        ) {
        }
    }

    /// The list, with the pane `beside` it or not; and the rows it selected
    /// and confirmed (opened), in order.
    struct Fixture<'a> {
        cx: &'a mut VisualTestContext,
        list: Entity<ListState<Rows>>,
        selected: Rc<RefCell<Vec<usize>>>,
        opened: Rc<RefCell<Vec<usize>>>,
    }

    fn fixture(cx: &mut TestAppContext, beside: bool) -> Fixture<'_> {
        let beside = Rc::new(Cell::new(beside));
        let slot: Rc<RefCell<Option<Entity<ListState<Rows>>>>> = Rc::default();
        let cx = host(cx, {
            let slot = slot.clone();
            move |window, cx| {
                let list = slot
                    .borrow_mut()
                    .get_or_insert_with(|| {
                        let beside = beside.clone();
                        cx.new(|cx| ListState::new(Rows { beside }, window, cx))
                    })
                    .clone();
                div()
                    .size_full()
                    .child(div().h_64().w_64().child(List::new(&list).size_full()))
                    .into_any_element()
            }
        });
        let list = slot.borrow().clone().expect("drawn");
        let (selected, opened) = (
            Rc::<RefCell<Vec<usize>>>::default(),
            Rc::<RefCell<Vec<usize>>>::default(),
        );
        cx.update(|_, cx| {
            let (selected, opened) = (selected.clone(), opened.clone());
            cx.subscribe(&list, move |_, event: &ListEvent, _| match event {
                ListEvent::Select(ix) => selected.borrow_mut().push(ix.row),
                ListEvent::Confirm(ix) => opened.borrow_mut().push(ix.row),
                ListEvent::Cancel => {}
            })
            .detach();
        });
        Fixture {
            cx,
            list,
            selected,
            opened,
        }
    }

    /// Row `row`'s middle: the rows are 40 high from the top.
    fn on_row(row: usize) -> gpui_kit::Point<gpui_kit::Pixels> {
        #[expect(clippy::cast_precision_loss, reason = "a row index in a test")]
        point(px(40.), px(20. + 40. * row as f32))
    }

    fn click(cx: &mut VisualTestContext, row: usize, count: usize) {
        let position = on_row(row);
        let modifiers = Modifiers::default();
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: count,
        });
        redraw(cx);
    }

    #[gpui_kit::test]
    fn beside_the_pane_a_click_selects_and_opens_nothing(cx: &mut TestAppContext) {
        let f = fixture(cx, true);
        click(f.cx, 1, 1);
        assert_eq!(*f.selected.borrow(), vec![1]);
        assert!(
            f.opened.borrow().is_empty(),
            "the list's own click is held back"
        );
        let selected = f.cx.update(|_, cx| f.list.read(cx).selected_index());
        assert_eq!(selected, Some(IndexPath::new(1)));
    }

    #[gpui_kit::test]
    fn beside_the_pane_a_double_click_opens_the_book(cx: &mut TestAppContext) {
        let f = fixture(cx, true);
        click(f.cx, 2, 1);
        click(f.cx, 2, 2);
        assert_eq!(*f.selected.borrow(), vec![2]);
        assert_eq!(*f.opened.borrow(), vec![2]);
    }

    #[gpui_kit::test]
    fn beside_the_pane_enter_opens_the_book(cx: &mut TestAppContext) {
        let f = fixture(cx, true);
        click(f.cx, 0, 1);
        f.cx.update(|window, cx| f.list.update(cx, |list, cx| list.focus(window, cx)));
        redraw(f.cx);
        press(f.cx, "enter");
        assert_eq!(*f.opened.borrow(), vec![0]);
    }

    #[gpui_kit::test]
    fn without_the_pane_a_click_opens_the_book(cx: &mut TestAppContext) {
        let f = fixture(cx, false);
        click(f.cx, 1, 1);
        assert!(f.selected.borrow().is_empty());
        assert_eq!(*f.opened.borrow(), vec![1]);
    }
}
