//! Skeletons (`hardcover-design`, States → Loading): `skeleton` shapes where
//! content will be, pulsing between 100% and 50% opacity while it genuinely
//! loads. Every skeleton on screen pulses in step, and none moves when the
//! desktop asks for reduced motion (GPUI's animations honour it; the kit reads
//! the setting from the desktop portal).
//!
//! - [`Skeleton::line`]: a bar of text, 12 high with `r-control` corners. Set
//!   its width (`.w_2_3()`); it fills the row by default.
//! - [`Skeleton::line_in`]: a line in a larger type role (a page title, a
//!   name in `display-24`): a bar as high as the role's type, in a box as high
//!   as its line, so the text takes the same room when it lands.
//! - [`Skeleton::circle`]: an avatar.
//! - [`Skeleton::cover`]: a cover at a system size.
//! - [`SkeletonText`]: a paragraph: lines 8 apart, the last one shorter.
//! - [`SkeletonRow`]: a list row while it loads: a cover and two lines.
//!
//! [`Skeleton::still`] keeps a shape from pulsing, for a placeholder that
//! holds a place before anything is asked for (the book pane waits a moment
//! before fetching its details): a skeleton only pulses while content is
//! really on its way.
//!
//! Covers show their own skeleton until the image lands; don't put one over
//! a [`crate::cover::Cover`].

use std::time::Duration;

use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    Animation, AnimationExt as _, App, IntoElement, ParentElement as _, RenderOnce,
    StyleRefinement, Styled, Window, bounce, div, ease_in_out, prelude::FluentBuilder as _, rems,
};

use super::focus::{box_radius, control_radius};
use crate::cover::CoverSize;
use crate::design::{TypeRole, colors, size};

/// One full pulse, there and back.
const PULSE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
enum Shape {
    Line,
    Text(TypeRole),
    Circle,
    Cover(CoverSize),
}

/// One skeleton shape. See the module doc.
#[derive(IntoElement)]
pub struct Skeleton {
    shape: Shape,
    still: bool,
    style: StyleRefinement,
}

impl Skeleton {
    fn new(shape: Shape) -> Self {
        Self {
            shape,
            still: false,
            style: StyleRefinement::default(),
        }
    }

    /// A line of text, 12 high, filling its row unless given a width.
    pub fn line() -> Self {
        Self::new(Shape::Line)
    }

    /// A line of text in `role`; see the module doc.
    pub fn line_in(role: TypeRole) -> Self {
        Self::new(Shape::Text(role))
    }

    /// A 32 px avatar circle.
    pub fn circle() -> Self {
        Self::new(Shape::Circle)
    }

    /// A cover at `size`, 2:3, with the cover's corners.
    pub fn cover(size: CoverSize) -> Self {
        Self::new(Shape::Cover(size))
    }

    /// Holds still: a placeholder for content not yet asked for.
    pub fn still(mut self) -> Self {
        self.still = true;
        self
    }
}

impl Styled for Skeleton {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Skeleton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let shape = div()
            .flex_none()
            .bg(colors(cx).skeleton)
            .map(|this| match self.shape {
                // 12 high: a 16 line's glyphs, not its leading.
                Shape::Line => this.w_full().h_3().rounded(control_radius(cx)),
                Shape::Text(role) => this
                    .w_full()
                    .h(rems(role.scale().0 / 16.))
                    .rounded(control_radius(cx)),
                Shape::Circle => this.size(size::AVATAR).rounded_full(),
                Shape::Cover(cover) => this
                    .w(cover.width())
                    .h(cover.height())
                    .rounded(cover.radius()),
            })
            .refine_style(&self.style);
        let shape = if self.still {
            shape.into_any_element()
        } else {
            shape
                .with_animation(
                    "skeleton-pulse",
                    Animation::new(PULSE)
                        .repeat_synced()
                        .with_easing(bounce(ease_in_out)),
                    |this, delta| this.opacity(1. - delta * 0.5),
                )
                .into_any_element()
        };
        match self.shape {
            // The role's line box around its bar, as the text will have.
            Shape::Text(role) => div()
                .flex()
                .items_center()
                .h(rems(role.scale().1 / 16.))
                .child(shape)
                .into_any_element(),
            _ => shape,
        }
    }
}

/// A paragraph's skeleton: `lines` lines 8 apart, the last one two thirds
/// long.
#[derive(IntoElement)]
pub struct SkeletonText {
    lines: usize,
    still: bool,
}

impl SkeletonText {
    pub fn new(lines: usize) -> Self {
        Self {
            lines,
            still: false,
        }
    }

    /// Holds still, as [`Skeleton::still`].
    pub fn still(mut self) -> Self {
        self.still = true;
        self
    }
}

impl RenderOnce for SkeletonText {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let still = self.still;
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .children((0..self.lines).map(move |ix| {
                Skeleton::line()
                    .when(ix + 1 == self.lines && self.lines > 1, |this| this.w_2_3())
                    .when(still, Skeleton::still)
            }))
    }
}

/// A list row while it loads: a cover at `size` (sm by default) beside a
/// title line and a shorter second line, with a list row's padding.
#[derive(IntoElement)]
pub struct SkeletonRow {
    cover: CoverSize,
}

impl SkeletonRow {
    pub fn new() -> Self {
        Self {
            cover: CoverSize::Sm,
        }
    }
}

impl RenderOnce for SkeletonRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .w_full()
            .p_2()
            .gap_3()
            .rounded(box_radius(cx))
            .child(Skeleton::cover(self.cover))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_2()
                    .child(Skeleton::line().w_3_5())
                    .child(Skeleton::line().w_2_5()),
            )
    }
}
