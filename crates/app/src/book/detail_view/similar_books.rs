//! Similar books (`hardcover-design`, Shelf → Book pane, and Full book page):
//! lg covers with their shadow, 16 apart, each with its title under it, a
//! text button in two lines at most; in one column the first
//! [`SIMILAR_IN_COLUMN`], in a side column ([`SideGrid`]) a grid of them.
//!
//! Its own view, [`SimilarBooks`]: it follows the catalogue for the book
//! shown, asks for its similar books once the details name them, and lets go
//! of the covers it showed when the book changes or it goes away (browsing
//! books would otherwise keep every lg cover it passed for the run).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::h_flex;
use gpui_kit::{
    App, Context, IntoElement, ParentElement as _, Pixels, Render, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::{BookId, BookSummary};

use crate::app_state::release_cover_medium;
use crate::catalog::navigation::Page;
use crate::cover::{Cover, CoverSize};
use crate::resource::catalog::SIMILAR_FETCHED;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::{Block, CoverShelf, LoadFailed, Skeleton, TextButton, books_that_fit};

/// Similar books shown in one column (the narrow pane, the page without a
/// side column).
const SIMILAR_IN_COLUMN: usize = 8;

/// The "Similar books" block of the book shown. See the module doc.
pub(super) struct SimilarBooks {
    /// The session's catalogue; `None` once signed out.
    catalog: Option<Catalog>,
    book: Option<BookId>,
    /// The book whose similar books are due: asked for once its details
    /// land, if they weren't in yet.
    due: Option<BookId>,
    /// The side column's grid, or `None` in one column: set by the book view
    /// as it lays its blocks out, just before this view renders.
    side: Rc<Cell<Option<SideGrid>>>,
    /// The cover URLs of the similar books shown, released when the view
    /// moves to another book or goes away.
    covers: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl SimilarBooks {
    pub(super) fn new(
        side: Rc<Cell<Option<SideGrid>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let catalog = Catalog::global(cx);
        let mut subscriptions = vec![cx.on_release_in(window, |this: &mut Self, window, cx| {
            for url in this.covers.drain(..) {
                release_cover_medium(&url, Some(window), cx);
            }
        })];
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe(
                &catalog.details,
                |this, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if this.book != Some(*id) {
                        return;
                    }
                    if this.due == Some(*id)
                        && let Some(catalog) = this.catalog.clone()
                    {
                        catalog.request_similar(*id, Demand::Active, cx);
                    }
                    cx.notify();
                },
            ));
            subscriptions.push(cx.subscribe(
                &catalog.similar,
                |this, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if this.book == Some(*id) {
                        cx.notify();
                    }
                },
            ));
        }
        Self {
            catalog,
            book: None,
            due: None,
            side,
            covers: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `book`'s similar books: what the catalogue has at once; the
    /// book view asks for them ([`Self::fetch`]) once the book has settled.
    /// The previous book's covers are let go at once, even when the new
    /// one shows no similar books.
    pub(super) fn set_book(
        &mut self,
        book: Option<BookId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.book == book {
            return;
        }
        self.book = book;
        self.due = None;
        self.keep_covers(None, window, cx);
        cx.notify();
    }

    /// Asks the catalogue for the book's similar books, if `demand` calls
    /// for it; when its details aren't in yet, once they land.
    pub(super) fn fetch(&mut self, demand: Demand, cx: &mut Context<Self>) {
        let (Some(book), Some(catalog)) = (self.book, self.catalog.clone()) else {
            return;
        };
        self.due = Some(book);
        catalog.request_similar(book, demand, cx);
    }

    /// The books Hardcover lists as similar to the book shown, if fetched.
    fn similar(&self, cx: &App) -> Option<Arc<Vec<BookSummary>>> {
        let book = self.book?;
        self.catalog
            .as_ref()?
            .similar
            .read(cx)
            .resource()
            .value(&book)
            .cloned()
    }

    /// Whether the block shows: the details name similar books, and they
    /// aren't known to be none. The book view leaves it out otherwise.
    pub(super) fn shows(&self, cx: &App) -> bool {
        let (Some(book), Some(catalog)) = (self.book, &self.catalog) else {
            return false;
        };
        let named = catalog
            .details
            .read(cx)
            .resource()
            .value(&book)
            .is_some_and(|details| !details.similar_book_ids.is_empty());
        named && self.similar(cx).is_none_or(|books| !books.is_empty())
    }

    /// Releases the covers of similar books no longer shown, now that
    /// `books` are.
    fn keep_covers(&mut self, books: Option<&[BookSummary]>, window: &mut Window, cx: &mut App) {
        let shown: Vec<String> = books
            .unwrap_or_default()
            .iter()
            .filter_map(|b| b.cover.as_ref().map(|c| c.url.clone()))
            .collect();
        if shown == self.covers {
            return;
        }
        for url in std::mem::replace(&mut self.covers, shown) {
            if !self.covers.contains(&url) {
                release_cover_medium(&url, Some(window), cx);
            }
        }
    }
}

impl Render for SimilarBooks {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let books = self.similar(cx);
        self.keep_covers(books.as_deref().map(Vec::as_slice), window, cx);
        let Some(book) = self.book.filter(|_| self.shows(cx)) else {
            return div().into_any_element();
        };
        let (error, loading) = match &self.catalog {
            Some(catalog) => {
                let store = catalog.similar.read(cx).resource();
                (store.error(&book).cloned(), store.is_loading(&book))
            }
            None => (None, false),
        };
        let side = self.side.get();
        let body = match (books, error) {
            (Some(books), _) => CoverShelf::new(CoverSize::Lg)
                .when(side.is_some(), CoverShelf::grid)
                .books(
                    books
                        .iter()
                        .take(side.map_or(SIMILAR_IN_COLUMN, |grid| grid.shown(books.len())))
                        .map(|book| {
                            let id = book.id.get().cast_unsigned();
                            (
                                Cover::new(CoverSize::Lg, book.title.clone())
                                    .author(book.authors.first().cloned())
                                    .thumbnail(book.cover.as_ref().map(|c| c.url.clone()))
                                    .shadow(true)
                                    .opens(("similar-cover", id), Page::Book(book.clone())),
                                TextButton::book(("similar-title", id), book),
                            )
                        }),
                )
                .into_any_element(),
            (None, Some(message)) if !loading => LoadFailed::new("retry-similar", message)
                .on_retry(cx.listener(|this, _, _, cx| this.fetch(Demand::Now, cx)))
                .into_any_element(),
            // Waiting out the delay or loading: the strip's shape, still.
            _ => h_flex()
                .gap_4()
                .children((0..4).map(|_| Skeleton::cover(CoverSize::Lg).still()))
                .into_any_element(),
        };
        Block::new("Similar books").child(body).into_any_element()
    }
}

/// The similar books' grid in a side column: how many lg covers a row holds
/// and how many it shows at most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SideGrid {
    per_row: usize,
    max: usize,
}

impl SideGrid {
    /// How many of `available` books the grid shows: at most [`Self::max`],
    /// and past one row only whole rows, so no cover sits alone on the last.
    pub(super) fn shown(self, available: usize) -> usize {
        let n = available.min(self.max);
        if n <= self.per_row {
            n
        } else {
            n - n % self.per_row
        }
    }
}

/// The similar books' grid in a side column `width` wide: two rows of lg
/// covers (at least the eight one column shows, at most those fetched), cut
/// to whole rows by [`SideGrid::shown`].
pub(super) fn similar_in_side(width: Pixels, rem_size: Pixels) -> SideGrid {
    let per_row = books_that_fit(width, CoverSize::Lg, rem_size).max(1);
    SideGrid {
        per_row,
        max: (per_row * 2).clamp(SIMILAR_IN_COLUMN, SIMILAR_FETCHED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_side_column_shows_two_rows_of_similar_books() {
        let rem = gpui_kit::px(16.);
        // The side at its narrowest holds three lg covers a row: still the
        // eight one column shows.
        assert_eq!(similar_in_side(gpui_kit::px(320.), rem).max, 8);
        // Five a row (464): two rows.
        assert_eq!(similar_in_side(gpui_kit::px(464.), rem).max, 10);
        // A 2560 pane's side (920) holds nine a row: all that were fetched.
        assert_eq!(similar_in_side(gpui_kit::px(920.), rem).max, 16);
    }

    #[test]
    fn the_side_grid_shows_only_whole_rows() {
        let nine = SideGrid {
            per_row: 9,
            max: 16,
        };
        // Ten books, nine a row: one full row, not nine and one alone.
        assert_eq!(nine.shown(10), 9);
        // All sixteen fetched: still one full row.
        assert_eq!(nine.shown(16), 9);
        // Fewer than a row: all of them.
        assert_eq!(nine.shown(4), 4);
        let three = SideGrid { per_row: 3, max: 8 };
        // Eight at most, three a row: two full rows.
        assert_eq!(three.shown(16), 6);
        assert_eq!(three.shown(3), 3);
    }
}
