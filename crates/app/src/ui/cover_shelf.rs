//! Cover shelf (`hardcover-design`, Screens → Home's "Newest on Want to read"
//! and the full book page's "Similar books"): a few books in a row, each a
//! cover over its title, 16 apart.
//!
//! - Each book is as wide as its cover; the title under it, 8 below, is a
//!   [`TextButton`] at 12/500 in ink, at most two lines, so the cover is
//!   never the only keyboard path to the book.
//! - The covers take a shadow only where they lead their block (Similar
//!   books), never in a rail; that is the [`Cover`]'s own `shadow`.
//!
//! ```ignore
//! CoverShelf::new(CoverSize::Md).books(entries.iter().map(|e| {
//!     (cover(e, CoverSize::Md), TextButton::book(("want-title", e.id), &e.book))
//! }))
//! ```

use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, RenderOnce, Styled as _, Window, div,
};

use super::TextButton;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors};

/// Lines a title may take under its cover.
const TITLE_LINES: usize = 2;

/// Books in a row. See the module doc.
#[derive(IntoElement)]
pub struct CoverShelf {
    size: CoverSize,
    books: Vec<(Cover, TextButton)>,
}

impl CoverShelf {
    /// A shelf of covers at `size`; give each book's [`Cover`] the same.
    pub fn new(size: CoverSize) -> Self {
        Self {
            size,
            books: Vec::new(),
        }
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
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_4()
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
                                    .wrap()
                                    .label(cx)
                                    .text_color(fg)
                                    .line_clamp(TITLE_LINES)
                                    .text_ellipsis(),
                            )
                            .into_any_element()
                    }),
            )
    }
}
