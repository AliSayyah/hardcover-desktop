//! A series as a row of books (`hardcover-design`, Screens → Series), and the
//! empty slot that stands in for a book the reader doesn't have.
//!
//! - [`CoverSlot`]: a dashed outline in `border-strong` the size and shape of
//!   a cover at a system size, empty for a book not in the library, or with
//!   a 12 px icon (the clock for a book not out yet). It is not a cover: no
//!   fill, no edge, nothing to click.
//! - [`SeriesStrip`]: the books of a series in order, 4 apart, each 40 wide:
//!   its sm cover (or a slot), a 2 px line under it (a meter for the book
//!   being read, else empty, so every position sits on one line), then its
//!   shelf's 12 px icon and its position in a caption. The next book's
//!   position is in ink at weight 600. State never relies on colour: the
//!   icon, the slot and the clock carry it. A strip that can't show every
//!   book ends with a "+N" caption for the rest
//!   (`series::progress::strip_window` picks which).
//! - Each book is named for a screen reader with everything the eye reads
//!   from it ([`strip_label`]): "#2, Dune Messiah: Currently reading, 40%,
//!   next"; a slot says "not in your library" or "not out yet".
//!
//! ```ignore
//! SeriesStrip::new()
//!     .item(StripItem::cover(("strip", id), cover).shelf(ReadingStatus::Read).position("1"))
//!     .item(StripItem::missing(("strip", id2), "Dune Messiah").position("2").next(true))
//!     .item(StripItem::unreleased(("strip", id3), "Untitled Dune #3").position("3"))
//!     .more(4)
//! ```

use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::ReadingStatus;

use super::Meter;
use crate::chip::status_icon;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::status_label;

/// Between the books, and between a book's cover, line and caption (4).
const GAP: gpui_kit::Rems = gpui_kit::rems(0.25);

/// An empty place for a book. See the module doc.
#[derive(IntoElement)]
pub struct CoverSlot {
    size: CoverSize,
    icon: Option<Icon>,
}

impl CoverSlot {
    pub fn new(size: CoverSize) -> Self {
        Self { size, icon: None }
    }

    /// A 12 px icon in the middle, in `fg-muted`.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

impl RenderOnce for CoverSlot {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(self.size.width())
            .h(self.size.height())
            .rounded(self.size.radius())
            .border(size::HAIRLINE)
            .border_dashed()
            .border_color(colors.border_strong)
            .text_color(colors.fg_muted)
            .children(self.icon.map(gpui_kit::Styled::size_3))
    }
}

/// What stands for a book in the strip.
enum Body {
    Cover(Cover),
    Missing,
    Unreleased,
}

/// What a book's place in the strip says, for [`strip_label`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StripState {
    /// On a shelf.
    Shelf(ReadingStatus),
    /// Out, but not in the library.
    Missing,
    /// Not out yet.
    Unreleased,
}

/// A strip book's accessible name: its position, its title and what its
/// place shows (the shelf, the meter's percentage, a slot), then whether
/// it is the series' next book.
pub fn strip_label(
    position: &str,
    title: &str,
    state: StripState,
    progress: Option<f32>,
    next: bool,
) -> String {
    let mut parts = vec![match state {
        StripState::Shelf(status) => status_label(status).to_owned(),
        StripState::Missing => "not in your library".to_owned(),
        StripState::Unreleased => "not out yet".to_owned(),
    }];
    if let Some(percent) = progress {
        parts.push(format!("{:.0}%", super::meter::fraction(percent) * 100.));
    }
    if next {
        parts.push("next".to_owned());
    }
    let place = if position.is_empty() {
        String::new()
    } else {
        format!("#{position}, ")
    };
    format!("{place}{title}: {}", parts.join(", "))
}

/// One book of a [`SeriesStrip`].
pub struct StripItem {
    id: ElementId,
    title: SharedString,
    body: Body,
    shelf: Option<ReadingStatus>,
    position: SharedString,
    progress: Option<f32>,
    next: bool,
}

impl StripItem {
    fn new(id: impl Into<ElementId>, title: SharedString, body: Body) -> Self {
        Self {
            id: id.into(),
            title,
            body,
            shelf: None,
            position: SharedString::default(),
            progress: None,
            next: false,
        }
    }

    /// A book in the library: its sm cover. Give its shelf with
    /// [`StripItem::shelf`].
    pub fn cover(id: impl Into<ElementId>, cover: Cover) -> Self {
        let title = cover.title().clone();
        Self::new(id, title, Body::Cover(cover))
    }

    /// A book not in the library: an empty slot.
    pub fn missing(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self::new(id, title.into(), Body::Missing)
    }

    /// A book not out yet: a slot with the clock.
    pub fn unreleased(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self::new(id, title.into(), Body::Unreleased)
    }

    /// The book's shelf: its icon before the position.
    pub fn shelf(mut self, status: ReadingStatus) -> Self {
        self.shelf = Some(status);
        self
    }

    /// "3", "3.5".
    pub fn position(mut self, position: impl Into<SharedString>) -> Self {
        self.position = position.into();
        self
    }

    /// The book being read: a 2 px meter under the cover.
    pub fn progress(mut self, percent: Option<f32>) -> Self {
        self.progress = percent;
        self
    }

    /// The series' next book: its position in ink at weight 600.
    pub fn next(mut self, next: bool) -> Self {
        self.next = next;
        self
    }

    fn state(&self) -> StripState {
        match (&self.body, self.shelf) {
            (Body::Missing, _) => StripState::Missing,
            (Body::Unreleased, _) => StripState::Unreleased,
            (Body::Cover(_), Some(status)) => StripState::Shelf(status),
            // A cover with no shelf given: in the library, as a cover means.
            (Body::Cover(_), None) => StripState::Shelf(ReadingStatus::WantToRead),
        }
    }

    fn render(self, cx: &App) -> AnyElement {
        let colors = colors(cx);
        let label = strip_label(
            &self.position,
            &self.title,
            self.state(),
            self.progress,
            self.next,
        );
        let body = match self.body {
            Body::Cover(cover) => cover.into_any_element(),
            Body::Missing => CoverSlot::new(CoverSize::Sm).into_any_element(),
            Body::Unreleased => CoverSlot::new(CoverSize::Sm)
                .icon(crate::icons::clock())
                .into_any_element(),
        };
        let line = match self.progress {
            Some(percent) => Meter::new(self.id.clone(), percent, "Reading progress")
                .thin()
                .w(CoverSize::Sm.width())
                .into_any_element(),
            None => div().flex_none().h(size::METER_THIN).into_any_element(),
        };
        div()
            .id(self.id)
            .role(Role::ListItem)
            .aria_label(label)
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap(GAP)
            .w(size::STRIP_ITEM)
            .child(body)
            .child(line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .caption(cx)
                    .tabular()
                    .whitespace_nowrap()
                    .when(self.next, |this| {
                        this.text_color(colors.fg).font_weight(FontWeight::SEMIBOLD)
                    })
                    .children(
                        self.shelf
                            .map(|status| Icon::new(status_icon(status)).size_3().flex_none()),
                    )
                    .child(self.position),
            )
            .into_any_element()
    }
}

/// A series' books in a row. See the module doc.
#[derive(IntoElement, Default)]
pub struct SeriesStrip {
    items: Vec<StripItem>,
    more: usize,
}

impl SeriesStrip {
    pub fn new() -> Self {
        Self::default()
    }

    // One at a time is how the gallery builds its strips; screens pass
    // their items at once.
    #[cfg_attr(not(feature = "demo"), allow(dead_code))]
    pub fn item(mut self, item: StripItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = StripItem>) -> Self {
        self.items.extend(items);
        self
    }

    /// Books left out after the last one shown: a "+N" caption.
    pub fn more(mut self, more: usize) -> Self {
        self.more = more;
        self
    }

    /// How tall a strip is: an sm cover, the line and the caption, [`GAP`]
    /// apart. The tracker's virtual list sizes its rows from it.
    pub fn height(rem: gpui_kit::Pixels) -> gpui_kit::Pixels {
        (CoverSize::Sm.height() + GAP * 2. + size::METER_THIN).to_pixels(rem)
            + gpui_kit::px(TypeRole::Caption.scale().1)
    }

    /// How many items fit in `width`, counting the gaps between them.
    pub fn capacity(width: gpui_kit::Pixels, rem: gpui_kit::Pixels) -> usize {
        let item = size::STRIP_ITEM.to_pixels(rem);
        let gap = GAP.to_pixels(rem);
        let fits = ((width + gap) / (item + gap)).floor();
        if fits.is_finite() && fits > 0. {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "fits is a finite positive floor; the saturating cast bounds the slot count"
            )]
            let slots = fits as usize;
            slots
        } else {
            1
        }
    }
}

impl RenderOnce for SeriesStrip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let more = self.more;
        div()
            .flex()
            .items_start()
            .gap(GAP)
            .children(self.items.into_iter().map(|item| item.render(cx)))
            .when(more > 0, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .w(size::STRIP_ITEM)
                        .h(CoverSize::Sm.height())
                        .caption(cx)
                        .tabular()
                        .child(format!("+{more}")),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_book_says_what_its_place_shows() {
        assert_eq!(
            strip_label(
                "1",
                "Dune",
                StripState::Shelf(ReadingStatus::Read),
                None,
                false
            ),
            "#1, Dune: Read"
        );
        assert_eq!(
            strip_label(
                "2",
                "Dune Messiah",
                StripState::Shelf(ReadingStatus::CurrentlyReading),
                Some(40.),
                true
            ),
            "#2, Dune Messiah: Currently reading, 40%, next"
        );
        assert_eq!(
            strip_label("3", "Children of Dune", StripState::Missing, None, true),
            "#3, Children of Dune: not in your library, next"
        );
        assert_eq!(
            strip_label(
                "6.5",
                "Untitled Dune #7",
                StripState::Unreleased,
                None,
                false
            ),
            "#6.5, Untitled Dune #7: not out yet"
        );
    }
}
