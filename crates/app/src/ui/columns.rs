//! Columns (`hardcover-design`, Screens → Home, Full book page and Wide
//! windows; Layout → "As the window narrows"): a page's main column beside a
//! [`Rail`], which folds under the main column when the region is too narrow
//! for both, and an optional second content column for a wide region.
//!
//! - **Beside the rail:** the main column takes the surplus (`flex_1`,
//!   `min_w_0`); the rail is 272 with its leading hairline, starting at the
//!   top. The gap is 24 on the book page and 32 on Home
//!   ([`Columns::wide_gap`]).
//! - **Three columns** ([`Tier::Three`] with [`Columns::second`]): the main
//!   column and the second share the surplus equally, then the rail. They
//!   fill the width they are given; [`content_column_width`] says how wide
//!   each gets, for content that adds items as it widens (Home's shelf of
//!   covers). In any other tier a second column follows the main one inside
//!   it: the tier, not the caller, decides how many columns there are.
//! - **Folded:** the second column (if any) follows the main one, then the
//!   rail's sections, full width, under a hairline (the column gap above it,
//!   24 below), separated by hairlines like the rail's (24 above and below).
//!   Nothing is hidden.
//! - The caller decides the tier from the width the columns get (the
//!   window less the sidebar, any list beside them and the page's insets;
//!   never the raw window): GPUI has no container queries. [`region_width`]
//!   is that width for a page beside the sidebar. [`folds`] folds when the
//!   main column would get less than [`size::MAIN_MIN`]; [`tier`] also says
//!   when two content columns of at least [`size::WIDE_COLUMN_MIN`] fit
//!   beside the rail.
//!
//! ```ignore
//! let width = region_width(window, size::PAGE_INSET);
//! Columns::new(tier(width, window.rem_size(), true), main)
//!     .second(coming_soon)
//!     .wide_gap()
//!     .rail(sections)
//! ```

use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, Pixels, Rems, RenderOnce, Styled as _,
    Window, div, prelude::FluentBuilder as _, rems,
};

use super::Rail;
use super::reveal::probe;
use crate::design::{colors, size};

/// The width of the content region beside the sidebar, less `inset` on
/// each side (`size::PAGE_INSET` for a page, `rems(0.)` for the whole
/// region): what a screen picks its tier from. The one place that knows
/// the sidebar's width is all there is beside it.
pub fn region_width(window: &Window, inset: Rems) -> Pixels {
    region_width_in(window.viewport_size().width, window.rem_size(), inset)
}

/// [`region_width`] for a window `viewport` wide.
pub fn region_width_in(viewport: Pixels, rem_size: Pixels, inset: Rems) -> Pixels {
    (viewport - size::SIDEBAR.to_pixels(rem_size) - inset.to_pixels(rem_size) * 2.)
        .max(Pixels::ZERO)
}

/// Between the columns on the book page (24).
const GAP: Rems = rems(1.5);
/// Between the columns on Home (32).
const WIDE_GAP: Rems = rems(2.);

/// A main column and its rail. See the module doc.
#[derive(IntoElement)]
pub struct Columns {
    tier: Tier,
    main: AnyElement,
    second: Option<AnyElement>,
    rail: Vec<AnyElement>,
    wide_gap: bool,
}

impl Columns {
    /// `main` laid out as `tier` says ([`tier`] or [`folds`] picks it).
    pub fn new(tier: Tier, main: impl IntoElement) -> Self {
        Self {
            tier,
            main: main.into_any_element(),
            second: None,
            rail: Vec::new(),
            wide_gap: false,
        }
    }

    /// Home's 32 between the columns, instead of the book page's 24.
    pub fn wide_gap(mut self) -> Self {
        self.wide_gap = true;
        self
    }

    /// A second content column, between the main column and the rail in
    /// [`Tier::Three`], the two sharing the width equally. In any other
    /// tier it follows the main column, inside it.
    pub fn second(mut self, column: impl IntoElement) -> Self {
        self.second = Some(column.into_any_element());
        self
    }

    /// The rail's sections, in order ([`super::RailSection`]s). With none,
    /// the main column takes the whole width.
    pub fn rail(mut self, sections: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.rail
            .extend(sections.into_iter().map(IntoElement::into_any_element));
        self
    }
}

/// Whether columns `width` wide fold their rail: the main column would get
/// less than [`size::MAIN_MIN`] beside it.
pub fn folds(width: Pixels, rem_size: Pixels, wide_gap: bool) -> bool {
    let gap = if wide_gap { WIDE_GAP } else { GAP };
    let needed = size::MAIN_MIN.to_pixels(rem_size)
        + gap.to_pixels(rem_size)
        + size::RAIL.to_pixels(rem_size);
    width < needed
}

/// How a page's columns lay out in the width they get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// The rail's sections under the main column.
    Folded,
    /// The main column beside the rail.
    Rail,
    /// The main column, a second content column and the rail.
    Three,
}

impl Tier {
    /// [`Tier::Folded`] or [`Tier::Rail`], for a page with no second column
    /// (the book page), from [`folds`].
    pub fn rail(folded: bool) -> Self {
        if folded { Self::Folded } else { Self::Rail }
    }
}

/// The tier for columns `width` wide: [`Tier::Three`] when two content
/// columns of at least [`size::WIDE_COLUMN_MIN`] fit beside the rail, else
/// as [`folds`] says.
pub fn tier(width: Pixels, rem_size: Pixels, wide_gap: bool) -> Tier {
    let gap = if wide_gap { WIDE_GAP } else { GAP };
    let three = size::WIDE_COLUMN_MIN.to_pixels(rem_size) * 2.
        + gap.to_pixels(rem_size) * 2.
        + size::RAIL.to_pixels(rem_size);
    if width >= three {
        Tier::Three
    } else if folds(width, rem_size, wide_gap) {
        Tier::Folded
    } else {
        Tier::Rail
    }
}

/// How wide the main column is in columns `width` wide with a rail: beside
/// it (less the gap and the rail), or the whole width when `folded`.
pub fn main_column_width(width: Pixels, rem_size: Pixels, wide_gap: bool, folded: bool) -> Pixels {
    if folded {
        return width;
    }
    let gap = if wide_gap { WIDE_GAP } else { GAP }.to_pixels(rem_size);
    (width - gap - size::RAIL.to_pixels(rem_size)).max(Pixels::ZERO)
}

/// How wide each of the two content columns is in [`Tier::Three`] columns
/// `width` wide: they share what the rail (if `has_rail`) and the gaps leave.
pub fn content_column_width(
    width: Pixels,
    rem_size: Pixels,
    wide_gap: bool,
    has_rail: bool,
) -> Pixels {
    let gap = if wide_gap { WIDE_GAP } else { GAP }.to_pixels(rem_size);
    let beside = if has_rail {
        gap + size::RAIL.to_pixels(rem_size)
    } else {
        Pixels::ZERO
    };
    ((width - gap - beside) / 2.).max(Pixels::ZERO)
}

impl RenderOnce for Columns {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let gap = if self.wide_gap { WIDE_GAP } else { GAP };
        let content =
            |child: AnyElement| probe(div().flex().flex_col().flex_1().min_w_0().child(child));
        let beside = self.tier == Tier::Three && self.second.is_some();
        // Outside three columns, the second follows the main one inside it.
        let (main, second) = if beside {
            (content(self.main), self.second.map(content))
        } else {
            let main = div()
                .flex()
                .flex_col()
                .gap(gap)
                .child(self.main)
                .children(self.second)
                .into_any_element();
            (content(main), None)
        };
        if self.rail.is_empty() && second.is_none() {
            return div().flex().w_full().child(main).into_any_element();
        }
        if self.tier != Tier::Folded {
            let rail = (!self.rail.is_empty()).then(|| Rail::new().sections(self.rail));
            // Stretched, so the rail's hairline runs the columns' height;
            // growing, so in a page that fills the window (a flex column
            // at least the window high) it runs to the page's foot.
            return div()
                .flex()
                .flex_row()
                .flex_grow(1.)
                .w_full()
                .gap(gap)
                .child(main)
                .children(second)
                .children(rail)
                .into_any_element();
        }
        let border = colors(cx).border;
        let has_rail = !self.rail.is_empty();
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(gap)
            .child(main)
            .when(has_rail, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .pt_6()
                        .border_t_1()
                        .border_color(border)
                        .children(self.rail.into_iter().enumerate().map(move |(ix, section)| {
                            probe(
                                div()
                                    .when(ix > 0, |this| {
                                        this.mt_6().pt_6().border_t_1().border_color(border)
                                    })
                                    .child(section),
                            )
                        })),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::px;

    #[test]
    fn the_rail_folds_when_the_main_column_would_be_too_narrow() {
        let rem = px(16.);
        // 400 + 32 + 272 = 704 on Home; 400 + 24 + 272 = 696 on the book page.
        assert!(!folds(px(704.), rem, true));
        assert!(folds(px(703.), rem, true));
        assert!(!folds(px(696.), rem, false));
        assert!(folds(px(695.), rem, false));
        // The minimum window: 800 less the sidebar (240) and Home's insets.
        assert!(folds(px(800. - 240. - 64.), rem, true));
        // The default window: 1200 less the same.
        assert!(!folds(px(1200. - 240. - 64.), rem, true));
    }

    #[test]
    fn home_gains_a_third_column_only_on_a_wide_region() {
        let rem = px(16.);
        let home = |window: f32| tier(px(window - 240. - 64.), rem, true);
        // 448 + 32 + 448 + 32 + 272 = 1232: a 1536 window.
        assert_eq!(tier(px(1232.), rem, true), Tier::Three);
        assert_eq!(tier(px(1231.), rem, true), Tier::Rail);
        assert_eq!(tier(px(703.), rem, true), Tier::Folded);
        // The minimum and default windows lay out as they always have.
        assert_eq!(home(800.), Tier::Folded);
        assert_eq!(home(1200.), Tier::Rail);
        assert_eq!(home(1440.), Tier::Rail);
        assert_eq!(home(1535.), Tier::Rail);
        assert_eq!(home(1536.), Tier::Three);
        assert_eq!(home(1600.), Tier::Three);
        assert_eq!(home(2560.), Tier::Three);
        // On the book page's 24 gap: 448 + 24 + 448 + 24 + 272 = 1216.
        assert_eq!(tier(px(1216.), rem, false), Tier::Three);
        assert_eq!(tier(px(1215.), rem, false), Tier::Rail);
    }

    #[test]
    fn the_main_column_takes_what_the_rail_leaves() {
        let rem = px(16.);
        // The book page in a 1600 window: 1600 - 240 - 64 = 1296, less 24 and 272.
        assert_eq!(main_column_width(px(1296.), rem, false, false), px(1000.));
        assert_eq!(main_column_width(px(1296.), rem, true, false), px(992.));
        assert_eq!(main_column_width(px(600.), rem, false, true), px(600.));
    }

    #[test]
    fn the_region_is_the_window_less_the_sidebar_and_insets() {
        let rem = px(16.);
        assert_eq!(region_width_in(px(1200.), rem, size::PAGE_INSET), px(896.));
        assert_eq!(region_width_in(px(1200.), rem, rems(0.)), px(960.));
        assert_eq!(region_width_in(px(200.), rem, size::PAGE_INSET), px(0.));
    }

    /// A second column sits beside the main one only in three columns.
    #[gpui_kit::test]
    fn a_second_column_is_beside_the_main_one_only_in_three_columns(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use crate::ui::test_support::host;
        use gpui_kit::{InteractiveElement as _, Styled as _};

        for (tier, beside) in [
            (Tier::Rail, false),
            (Tier::Folded, false),
            (Tier::Three, true),
        ] {
            let cx = host(cx, move |_, _| {
                Columns::new(tier, div().h_8().debug_selector(|| "main".into()))
                    .second(div().h_8().debug_selector(|| "second".into()))
                    .into_any_element()
            });
            let main = cx.debug_bounds("main").expect("main is drawn");
            let second = cx.debug_bounds("second").expect("second is drawn");
            assert_eq!(
                main.top() == second.top(),
                beside,
                "{tier:?}: main {main:?}, second {second:?}"
            );
        }
    }

    #[test]
    fn two_content_columns_share_what_the_rail_leaves() {
        let rem = px(16.);
        // A 2560 window: 2560 - 240 - 64 = 2256; (2256 - 32 - 32 - 272) / 2.
        assert_eq!(content_column_width(px(2256.), rem, true, true), px(960.));
        // At the three-column threshold each gets its minimum.
        assert_eq!(content_column_width(px(1232.), rem, true, true), px(448.));
        // With nothing in the rail, the rail and its gap go to the columns.
        assert_eq!(content_column_width(px(2256.), rem, true, false), px(1112.));
    }
}
