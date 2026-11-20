//! The book list on Discover and the series and author pages: catalogue
//! rows (`hardcover-design`, Rows → Catalogue row) with each book's library
//! status, keyboard-navigable. Enter or a click opens the book. Escape
//! reaches the page's Back binding: the list's own Cancel lets it through.
//!
//! The pages share one frame: content at most [`PAGE_WIDTH`] wide, inset 32
//! ([`page_header`]), with the list's rows on the same spine
//! ([`page_list`]: a row's own 8 of padding inside a 24 inset).

use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::{IconName, IndexPath};
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, IntoElement, ParentElement as _, Rems,
    SharedString, Styled as _, Subscription, Window, div, rems,
};
use hardcover_core::{BookSummary, ReadingStatus};

use super::navigation::{self, Page};
use crate::design::{TypeRoles as _, colors};
use crate::format::average_rating_text;
use crate::library::book_row::{BookRow, loading_rows};
use crate::library::model::LibraryModel;
use crate::ui::{ListRow as _, RowFocus, empty_state};

/// The widest a catalogue page's content gets (768), so rows and lines
/// stay readable in a wide window.
pub const PAGE_WIDTH: Rems = rems(48.);

/// The top of a page: inset 32 on three sides, 16 above the list (whose
/// rows add their own 8).
pub fn page_header() -> Div {
    div()
        .flex_none()
        .w_full()
        .max_w(PAGE_WIDTH)
        .px_8()
        .pt_8()
        .pb_4()
}

/// The region a page's list fills, under its header.
pub fn page_body() -> Div {
    div().flex_1().min_h_0().w_full().max_w(PAGE_WIDTH)
}

/// A page's list, its rows' text on the header's spine.
pub fn page_list<D: ListDelegate>(list: &Entity<ListState<D>>) -> List<D> {
    List::new(list).size_full().px_6().pb_6()
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
}

impl CatalogBooks {
    pub fn set_rows(&mut self, rows: Vec<CatalogRow>) {
        self.rows = rows;
    }
}

impl ListDelegate for CatalogBooks {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
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
        Some(
            ListItem::new(("catalog-book", row.book.id.get() as u64))
                .list_row(focused, cx)
                .accessibility_label(row.book.title.clone())
                .child(BookRow::for_catalog(&row.book, row.details.clone(), status)),
        )
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

/// A list for a catalogue page. `empty` explains an empty list.
pub fn new_list(
    model: Entity<LibraryModel>,
    empty: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ListState<CatalogBooks>> {
    let empty = empty.into();
    let list = cx.new(|cx| {
        ListState::new(
            CatalogBooks {
                rows: Vec::new(),
                model: model.clone(),
                empty,
                focus: RowFocus::default(),
            },
            window,
            cx,
        )
    });
    RowFocus::attach(&list, |d| &mut d.focus, cx);
    list
}

/// Opens the confirmed book. Arrowing only moves the selection.
pub fn follow_list<V: 'static>(
    list: &Entity<ListState<CatalogBooks>>,
    model: &Entity<LibraryModel>,
    cx: &mut Context<V>,
) -> Vec<Subscription> {
    vec![
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
}
