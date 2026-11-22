//! Meter (`hardcover-design`, Meters): progress as an `accent` fill on
//! `track`, with pill ends.
//!
//! - 6 px high when words sit beside it ("Page 312 of 476", "66%"); 2 px
//!   inside a row ([`Meter::thin`]). Words always come with a meter: the bar
//!   is never the only carrier.
//! - A goal meter adds the pace tick ([`Meter::pace`]): where steady progress
//!   would be today, 2 px wide in `fg`, 3 px proud of the meter on each side.
//! - Hardcover's figures (the book's ratings breakdown) take the muted fill
//!   ([`Meter::muted`]): `fg-muted` on `track`, since the accent is for
//!   progress. Their counts sit beside them.
//! - It fills its width; set one where the system names it
//!   (`size::METER_IN_ROW`, at most `size::METER_MAX`).
//! - A new value moves the fill there on a spring (`design::motion::METER`),
//!   so the change shows. A meter shown for the first time starts at its
//!   value, so `id` must name what the meter measures (the entry, the goal),
//!   not its place: a pane showing another book then draws its meter at
//!   once instead of moving it from the last book's. Muted meters
//!   (Hardcover's figures) don't move: they change only with the book shown.
//! - Exposed as a progress indicator with its value and an accessible name,
//!   which every meter must give: what it measures ("Reading progress",
//!   "2026 reading goal", "5-star ratings").
//!
//! ```ignore
//! Meter::new(("home-progress", id), percent, "Reading progress")
//!     .thin()
//!     .w(size::METER_IN_ROW)
//! ```

use std::sync::Arc;

use gpui_kit::base::Progress;
use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    AnimationExt as _, App, ElementId, IntoElement, ParentElement as _, RenderOnce, SharedString,
    SpringAnimation, StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _, relative,
};

use crate::design::{colors, motion, size};

/// A progress meter. Values are percentages, 0 to 100, like the kit's.
#[derive(IntoElement)]
pub struct Meter {
    id: ElementId,
    percent: f32,
    thin: bool,
    muted: bool,
    pace: Option<f32>,
    label: SharedString,
    style: StyleRefinement,
}

impl Meter {
    /// A meter at `percent` (clamped to 0..=100), named `label` for a
    /// screen reader ("Reading progress"). `id` must be unique among its
    /// siblings.
    pub fn new(id: impl Into<ElementId>, percent: f32, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            percent,
            thin: false,
            muted: false,
            pace: None,
            label: label.into(),
            style: StyleRefinement::default(),
        }
    }

    /// The 2 px meter that sits inside a row.
    pub fn thin(mut self) -> Self {
        self.thin = true;
        self
    }

    /// The `fg-muted` fill of a figure that isn't the reader's progress:
    /// Hardcover's ratings breakdown.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// The pace tick, at `percent` of the way: where steady progress would
    /// be today. `None` draws none.
    pub fn pace(mut self, percent: Option<f32>) -> Self {
        self.pace = percent;
        self
    }
}

impl Styled for Meter {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// `percent` as a fraction of the bar, clamped; anything that isn't a number
/// is an empty bar.
pub fn fraction(percent: f32) -> f32 {
    if percent.is_nan() {
        0.
    } else {
        (percent / 100.).clamp(0., 1.)
    }
}

impl RenderOnce for Meter {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let filled = fraction(self.percent);
        let height = if self.thin {
            size::METER_THIN
        } else {
            size::METER
        };
        let fill = if self.muted {
            colors.fg_muted
        } else {
            colors.accent
        };
        let tick = self.pace.map(fraction);
        let overhang = size::PACE_TICK_OVERHANG;
        let bar = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .rounded_full()
            .bg(fill);
        let bar = if self.muted {
            bar.w(relative(filled)).into_any_element()
        } else {
            let id = ElementId::NamedChild(Arc::new(self.id.clone()), "fill".into());
            bar.with_spring(
                id,
                SpringAnimation::new(motion::METER).to(filled),
                |bar, filled| bar.w(relative(filled.clamp(0., 1.))),
            )
            .into_any_element()
        };
        Progress::new(self.id)
            .value(filled * 100.)
            .accessibility_label(self.label)
            .relative()
            .flex_none()
            .w_full()
            .h(height)
            .rounded_full()
            .bg(colors.track)
            .refine_style(&self.style)
            .child(bar)
            .when_some(tick, |this, tick| {
                // Centred on its position: half its width back.
                this.child(
                    div()
                        .absolute()
                        .top(-overhang)
                        .bottom(-overhang)
                        .left(relative(tick))
                        .ml(-size::PACE_TICK_WIDTH / 2.)
                        .w(size::PACE_TICK_WIDTH)
                        .rounded_full()
                        .bg(colors.fg),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_stays_within_its_bar() {
        assert_eq!(fraction(0.), 0.);
        assert_eq!(fraction(66.), 0.66);
        assert_eq!(fraction(100.), 1.);
        assert_eq!(fraction(140.), 1., "a goal past its target");
        assert_eq!(fraction(-5.), 0.);
        assert_eq!(fraction(f32::NAN), 0.);
    }
}
