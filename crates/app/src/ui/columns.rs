//! Columns (`hardcover-design`, Screens → Home, Full book page and Wide
//! windows; Layout → "As the window narrows"): a page's main column beside a
//! [`Rail`], which folds under the main column when the region is too narrow
//! for both, and an optional second content column for a wide region.
//!
//! - **Beside the rail:** the main column takes the surplus (`flex_1`,
//!   `min_w_0`); the rail is 272 with its leading hairline, starting at the
//!   top. The gap is 24 on the book page and 32 on Home
//!   ([`Columns::wide_gap`]).
//! - **Three columns** ([`Columns::second`]): the main column and the second
//!   share the surplus equally, each at most [`size::COLUMN_MAX`], then the
//!   rail. Surplus width holds more content, never wider rows: the page's
//!   own maximum stops the columns at 720.
//! - **Folded:** the second column (if any) follows the main one, then the
//!   rail's sections, full width, under a hairline (the column gap above it,
//!   24 below), separated by hairlines like the rail's (24 above and below).
//!   Nothing is hidden.
//! - The caller decides from the width the columns get (the window less the
//!   sidebar, any list beside them and the page's insets; never the raw
//!   window): GPUI has no container queries. [`folds`] folds when the main
//!   column would get less than [`size::MAIN_MIN`]; [`tier`] also says when
//!   two content columns of at least [`size::WIDE_COLUMN_MIN`] fit beside
//!   the rail. Pass a second
//!   column only in [`Tier::Three`].
//!
//! ```ignore
//! let width = window.viewport_size().width - size::SIDEBAR.to_pixels(window.rem_size());
//! Columns::new(main)
//!     .wide_gap()
//!     .rail(sections)
//!     .folded(folds(width - inset, window.rem_size(), true))
//! ```

use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, Pixels, Rems, RenderOnce, Styled as _,
    Window, div, prelude::FluentBuilder as _, rems,
};

use super::Rail;
use super::reveal::probe;
use crate::design::{colors, size};

/// Between the columns on the book page (24).
const GAP: Rems = rems(1.5);
/// Between the columns on Home (32).
const WIDE_GAP: Rems = rems(2.);

/// A main column and its rail. See the module doc.
#[derive(IntoElement)]
pub struct Columns {
    main: AnyElement,
    second: Option<AnyElement>,
    rail: Vec<AnyElement>,
    wide_gap: bool,
    folded: bool,
}

impl Columns {
    pub fn new(main: impl IntoElement) -> Self {
        Self {
            main: main.into_any_element(),
            second: None,
            rail: Vec::new(),
            wide_gap: false,
            folded: false,
        }
    }

    /// Home's 32 between the columns, instead of the book page's 24.
    pub fn wide_gap(mut self) -> Self {
        self.wide_gap = true;
        self
    }

    /// A second content column, between the main column and the rail. The
    /// two share the width equally, each at most [`size::COLUMN_MAX`]. For
    /// [`Tier::Three`]; folded, it follows the main column.
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

    /// Puts the rail's sections under the main column.
    pub fn folded(mut self, folded: bool) -> Self {
        self.folded = folded;
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

impl RenderOnce for Columns {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let gap = if self.wide_gap { WIDE_GAP } else { GAP };
        let paired = self.second.is_some() && !self.folded;
        let content = |child: AnyElement| {
            probe(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .when(paired, |this| this.max_w(size::COLUMN_MAX))
                    .child(child),
            )
        };
        let main = content(self.main);
        let second = self.second.map(content);
        if self.rail.is_empty() && second.is_none() {
            return div().flex().w_full().child(main).into_any_element();
        }
        if !self.folded {
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
            .children(second)
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
    fn home_at_its_widest_holds_two_full_columns_and_the_rail() {
        let rem = px(16.);
        let content = size::HOME_WIDE_MAX.to_pixels(rem) - size::PAGE_INSET.to_pixels(rem) * 2.;
        let columns = size::COLUMN_MAX.to_pixels(rem) * 2.
            + WIDE_GAP.to_pixels(rem) * 2.
            + size::RAIL.to_pixels(rem);
        assert_eq!(content, columns);
        assert_eq!(size::HOME_WIDE_MAX.to_pixels(rem), px(1840.));
    }
}
