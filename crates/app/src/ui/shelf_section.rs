//! A shelf section (`hardcover-design`, Discover): Discover's "For you",
//! "Top picks", "Because you loved …" and "Trending this month".
//!
//! - Its [`SectionHeader`], with a small ghost "See all" on the trailing
//!   edge where there is more ([`ShelfSection::action`]).
//! - An optional muted line, 8 under the title ("Hardcover's
//!   recommendations from your library.").
//! - 12 below, its content, one of:
//!   - [`ShelfSection::books`]: a [`CoverShelf::one_row`] of as many books
//!     as fit ([`super::books_that_fit`]);
//!   - [`ShelfSection::loading`]: a row of skeleton covers in the shelf's
//!     place, at the shelf's size;
//!   - [`ShelfSection::failed`]: a [`LoadFailed`], one muted sentence with
//!     "Try again";
//!   - [`ShelfSection::note`]: one muted sentence and a link where the shelf
//!     would be (a vibe Hardcover hasn't built yet).
//!
//! The lead's covers (lg, with the cover shadow) and the others' (md, flat)
//! are the caller's: the section only places them.
//!
//! ```ignore
//! ShelfSection::new("Top picks")
//!     .action(see_all("top-picks-see-all", Some(35), on_click))
//!     .books(CoverShelf::new(CoverSize::Md).one_row().books(books))
//! ```

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    Window, div,
};

use super::{CoverShelf, ExternalLink, LoadFailed, SectionHeader, Skeleton};
use crate::cover::CoverSize;
use crate::design::{TypeRoles as _, colors};

/// A shelf section. See the module doc.
#[derive(IntoElement)]
pub struct ShelfSection {
    header: SectionHeader,
    intro: Option<SharedString>,
    body: Body,
}

enum Body {
    Empty,
    Books(CoverShelf),
    Loading { size: CoverSize, count: usize },
    Failed(LoadFailed),
    Note(SharedString, ExternalLink),
}

impl ShelfSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            header: SectionHeader::new(title),
            intro: None,
            body: Body::Empty,
        }
    }

    /// The small action on the title's trailing edge: "See all".
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.header = self.header.action(action);
        self
    }

    /// The muted line under the title.
    pub fn intro(mut self, intro: impl Into<SharedString>) -> Self {
        self.intro = Some(intro.into());
        self
    }

    /// The books: a one-row shelf of as many as fit.
    pub fn books(mut self, shelf: CoverShelf) -> Self {
        self.body = Body::Books(shelf);
        self
    }

    /// Still loading: `count` skeleton covers at the shelf's `size`.
    pub fn loading(mut self, size: CoverSize, count: usize) -> Self {
        self.body = Body::Loading { size, count };
        self
    }

    /// The load failed: one muted sentence with "Try again".
    pub fn failed(mut self, failed: LoadFailed) -> Self {
        self.body = Body::Failed(failed);
        self
    }

    /// One muted sentence and a link in the shelf's place.
    pub fn note(mut self, text: impl Into<SharedString>, link: ExternalLink) -> Self {
        self.body = Body::Note(text.into(), link);
        self
    }
}

impl RenderOnce for ShelfSection {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        let body: AnyElement = match self.body {
            Body::Empty => div().into_any_element(),
            Body::Books(shelf) => shelf.into_any_element(),
            Body::Loading { size, count } => h_flex()
                .gap_4()
                .children((0..count.max(1)).map(|_| Skeleton::cover(size)))
                .into_any_element(),
            Body::Failed(failed) => failed.into_any_element(),
            Body::Note(text, link) => v_flex()
                .gap_2()
                .items_start()
                .child(div().body().text_color(muted).child(text))
                .child(link)
                .into_any_element(),
        };
        let top = match self.intro {
            Some(intro) => v_flex()
                .gap_2()
                .child(self.header)
                .child(div().body().text_color(muted).child(intro))
                .into_any_element(),
            None => self.header.into_any_element(),
        };
        v_flex().w_full().gap_3().child(top).child(body)
    }
}
