//! The book list on series and author pages: keyboard-navigable rows with each
//! book's library status. Enter or a click opens the book. Escape reaches the
//! page's Back binding: the list's own Cancel lets it through.

use gpui_kit::component::list::{ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::{IconName, IndexPath};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, SharedString,
    Subscription, Window,
};
use hardcover_core::{BookSummary, ReadingStatus};

use super::navigation::{self, Page};
use crate::format::average_rating_text;
use crate::library::book_row::BookRow;
use crate::library::model::LibraryModel;
use crate::ui::empty_state;

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
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let row = self.rows.get(ix.row)?;
        let status = self
            .model
            .read(cx)
            .entry_for_book(row.book.id)
            .map(|e| e.status);
        Some(
            ListItem::new(("catalog-book", row.book.id.get() as u64))
                .accessibility_label(row.book.title.clone())
                .child(BookRow::for_catalog(&row.book, row.details.clone(), status)),
        )
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
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
    cx.new(|cx| {
        ListState::new(
            CatalogBooks {
                rows: Vec::new(),
                model: model.clone(),
                empty,
            },
            window,
            cx,
        )
    })
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
