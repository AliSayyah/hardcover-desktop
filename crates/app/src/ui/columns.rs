//! Columns (`hardcover-design`, Screens → Home and Full book page; Layout →
//! "As the window narrows"): a page's main column beside a [`Rail`], which
//! folds under the main column when the region is too narrow for both.
//!
//! - **Wide:** the main column takes the surplus (`flex_1`, `min_w_0`); the
//!   rail is 272 with its leading hairline, starting at the top. The gap is
//!   24 on the book page and 32 on Home ([`Columns::wide_gap`]).
//! - **Folded:** the rail's sections follow the main column, full width,
//!   under a hairline (the column gap above it, 24 below), separated by
//!   hairlines like the rail's (24 above and below). Nothing is hidden.
//! - The caller decides with [`folds`], from the width the columns get: GPUI
//!   has no container queries. Fold when the main column would get less than
//!   [`size::MAIN_MIN`].
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
    rail: Vec<AnyElement>,
    wide_gap: bool,
    folded: bool,
}

impl Columns {
    pub fn new(main: impl IntoElement) -> Self {
        Self {
            main: main.into_any_element(),
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

impl RenderOnce for Columns {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let gap = if self.wide_gap { WIDE_GAP } else { GAP };
        let main = probe(div().flex().flex_col().flex_1().min_w_0().child(self.main));
        if self.rail.is_empty() {
            return div().flex().w_full().child(main).into_any_element();
        }
        if !self.folded {
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
                .child(Rail::new().sections(self.rail))
                .into_any_element();
        }
        let border = colors(cx).border;
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(gap)
            .child(main)
            .child(
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
}
