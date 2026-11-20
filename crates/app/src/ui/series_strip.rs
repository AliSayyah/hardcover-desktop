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
//!
//! ```ignore
//! SeriesStrip::new()
//!     .item(StripItem::cover(("strip", id), cover).icon(IconName::CircleCheck).position("1"))
//!     .item(StripItem::missing(("strip", id2)).position("2").next(true))
//!     .item(StripItem::unreleased(("strip", id3)).position("3"))
//!     .more(4)
//! ```

use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::Meter;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors, size};

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
            .children(self.icon.map(|icon| icon.size_3()))
    }
}

/// What stands for a book in the strip.
enum Body {
    Cover(Cover),
    Missing,
    Unreleased(Box<Icon>),
}

/// One book of a [`SeriesStrip`].
pub struct StripItem {
    id: ElementId,
    body: Body,
    icon: Option<Icon>,
    position: SharedString,
    progress: Option<f32>,
    next: bool,
}

impl StripItem {
    fn new(id: impl Into<ElementId>, body: Body) -> Self {
        Self {
            id: id.into(),
            body,
            icon: None,
            position: SharedString::default(),
            progress: None,
            next: false,
        }
    }

    /// A book in the library: its sm cover.
    pub fn cover(id: impl Into<ElementId>, cover: Cover) -> Self {
        Self::new(id, Body::Cover(cover))
    }

    /// A book not in the library: an empty slot.
    pub fn missing(id: impl Into<ElementId>) -> Self {
        Self::new(id, Body::Missing)
    }

    /// A book not out yet: a slot with `clock`.
    pub fn unreleased(id: impl Into<ElementId>, clock: impl Into<Icon>) -> Self {
        Self::new(id, Body::Unreleased(Box::new(clock.into())))
    }

    /// The shelf's icon, before the position.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
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

    fn render(self, cx: &App) -> AnyElement {
        let colors = colors(cx);
        let body = match self.body {
            Body::Cover(cover) => cover.into_any_element(),
            Body::Missing => CoverSlot::new(CoverSize::Sm).into_any_element(),
            Body::Unreleased(clock) => CoverSlot::new(CoverSize::Sm)
                .icon(*clock)
                .into_any_element(),
        };
        let line = match self.progress {
            Some(percent) => Meter::new(self.id.clone(), percent)
                .thin()
                .w(CoverSize::Sm.width())
                .into_any_element(),
            None => div().flex_none().h(size::METER_THIN).into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1()
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
                    .children(self.icon.map(|icon| icon.size_3().flex_none()))
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

    /// How many items fit in `width`, counting the gaps between them.
    pub fn capacity(width: gpui_kit::Pixels, rem: gpui_kit::Pixels) -> usize {
        let item = size::STRIP_ITEM.to_pixels(rem);
        let gap = gpui_kit::rems(0.25).to_pixels(rem);
        let fits = ((width + gap) / (item + gap)).floor();
        if fits.is_finite() && fits > 0. {
            fits as usize
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
            .gap_1()
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
