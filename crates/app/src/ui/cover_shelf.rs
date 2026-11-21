//! Cover shelf (`hardcover-design`, Screens → Home's "Newest on Want to read"
//! and the full book page's "Similar books"): a few books in a row, each a
//! cover over its title, 16 apart.
//!
//! - Books that don't fit wrap onto another row, 16 below (Similar books in
//!   a narrow pane).
//! - [`CoverShelf::one_row`]: a single row that never wraps (Home's "Newest
//!   on Want to read" in its second column). Give it as many books as
//!   [`books_that_fit`] says for its width. It doesn't clip (that would cut
//!   the titles' focus rings), so a book too many runs past its edge.
//! - [`CoverShelf::grid`]: rows 24 apart, so each title clears the covers
//!   under it (Similar books beside the book's text).
//! - Each book is as wide as its cover; the title under it, 8 below, is a
//!   [`TextButton`] at 12/500 in ink, at most two lines broken between
//!   words (the second ends in "…" when the title is longer), so the cover
//!   is never the only keyboard path to the book.
//! - The covers take a shadow only where they lead their block (Similar
//!   books), never in a rail; that is the [`Cover`]'s own `shadow`.
//!
//! ```ignore
//! CoverShelf::new(CoverSize::Md).books(entries.iter().map(|e| {
//!     (cover(e, CoverSize::Md), TextButton::book(("want-title", e.id), &e.book))
//! }))
//! ```

use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, Pixels, RenderOnce, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use super::TextButton;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors};

/// Lines a title may take under its cover.
const TITLE_LINES: usize = 2;

/// How many books at `size` fit one row `width` wide, 16 apart.
pub fn books_that_fit(width: Pixels, size: CoverSize, rem_size: Pixels) -> usize {
    let gap = rem_size; // `gap_4`
    let book = size.width().to_pixels(rem_size);
    let fit = ((width + gap) / (book + gap)).floor();
    if fit > 0. { fit as usize } else { 0 }
}

/// How the books flow.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// Rows 16 apart.
    Wrap,
    /// One row, never wrapped.
    Row,
    /// Rows 24 apart.
    Grid,
}

/// Books in a row. See the module doc.
#[derive(IntoElement)]
pub struct CoverShelf {
    size: CoverSize,
    books: Vec<(Cover, TextButton)>,
    flow: Flow,
}

impl CoverShelf {
    /// A shelf of covers at `size`; give each book's [`Cover`] the same.
    pub fn new(size: CoverSize) -> Self {
        Self {
            size,
            books: Vec::new(),
            flow: Flow::Wrap,
        }
    }

    /// One row that never wraps: give it only the books that fit.
    pub fn one_row(mut self) -> Self {
        self.flow = Flow::Row;
        self
    }

    /// Wrapping rows 24 apart, for a grid of covers.
    pub fn grid(mut self) -> Self {
        self.flow = Flow::Grid;
        self
    }

    /// The books, each its cover and its title.
    pub fn books(mut self, books: impl IntoIterator<Item = (Cover, TextButton)>) -> Self {
        self.books.extend(books);
        self
    }
}

impl RenderOnce for CoverShelf {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fg = colors(cx).fg;
        let width = self.size.width();
        // The title breaks its own lines at the cover's width: GPUI's
        // wrapping would start the second line with the space it broke at.
        let width_px = px(self.size.width_px());
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_4()
            .map(|this| match self.flow {
                Flow::Wrap => this.flex_wrap(),
                Flow::Row => this,
                Flow::Grid => this.flex_wrap().gap_y_6(),
            })
            .children(
                self.books
                    .into_iter()
                    .map(move |(cover, title)| -> AnyElement {
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .w(width)
                            .gap_2()
                            .child(cover)
                            .child(
                                title
                                    .wrap_width(width_px)
                                    .line_clamp(TITLE_LINES)
                                    .label(cx)
                                    .text_color(fg),
                            )
                            .into_any_element()
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_holds_as_many_books_as_fit_16_apart() {
        let rem = px(16.);
        // md covers are 64: n books take 64n + 16(n - 1).
        assert_eq!(books_that_fit(px(64.), CoverSize::Md, rem), 1);
        assert_eq!(books_that_fit(px(143.), CoverSize::Md, rem), 1);
        assert_eq!(books_that_fit(px(144.), CoverSize::Md, rem), 2);
        assert_eq!(books_that_fit(px(63.), CoverSize::Md, rem), 0);
        assert_eq!(books_that_fit(px(0.), CoverSize::Md, rem), 0);
        // Home's second column in a 2560 window (960): 12 md covers.
        assert_eq!(books_that_fit(px(960.), CoverSize::Md, rem), 12);
        // lg covers are 80.
        assert_eq!(books_that_fit(px(320.), CoverSize::Lg, rem), 3);
    }
}
