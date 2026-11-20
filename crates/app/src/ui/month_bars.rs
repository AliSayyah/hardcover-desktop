//! Month bars: books finished in each month of a year, as twelve small
//! columns under a caption. Home's reading goal uses it ("Finished each
//! month") for a calendar-year goal counted in books.
//!
//! - Each month is a column 32 high: an `accent` bar from the baseline, as
//!   tall as its count against the year's best month, with the count over it
//!   as a tabular caption, so the figures read without a pointer
//!   (`hardcover-design`, States: hover is never the only cue). The month's
//!   initial sits under it; its full name and count are also its tooltip.
//! - A month with nothing finished is a 2 px tick on `track`. A month still
//!   to come is a hairline tick in `border`: a mark on the baseline, never a
//!   box, since it holds nothing yet.
//! - Bars have `r-cover-s` top corners, and sit 4 apart.
//!
//! ```ignore
//! MonthBars::new("home-months", counts).future_from(Some(today.month0() as usize + 1))
//! ```

use gpui_kit::component::h_flex;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
    relative,
};

use crate::cover::CoverSize;
use crate::design::{TypeRoles as _, colors, size};

const INITIALS: [&str; 12] = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
const NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Twelve month columns. See the module doc.
#[derive(IntoElement)]
pub struct MonthBars {
    id: SharedString,
    counts: [u32; 12],
    future_from: Option<usize>,
    caption: SharedString,
}

impl MonthBars {
    /// The months' `counts`, January first. `id` must be unique among its
    /// siblings.
    pub fn new(id: impl Into<SharedString>, counts: [u32; 12]) -> Self {
        Self {
            id: id.into(),
            counts,
            future_from: None,
            caption: "Finished each month".into(),
        }
    }

    /// The months from index `month0` on are still to come (none when the
    /// whole year is past; 0 for a year that hasn't started).
    pub fn future_from(mut self, month0: Option<usize>) -> Self {
        self.future_from = month0;
        self
    }
}

/// A month's tooltip: "June: 3 books".
pub fn month_text(month0: usize, count: u32) -> String {
    match count {
        1 => format!("{}: 1 book", NAMES[month0]),
        n => format!("{}: {n} books", NAMES[month0]),
    }
}

impl RenderOnce for MonthBars {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = colors(cx);
        let max = self.counts.iter().copied().max().unwrap_or(0).max(1);
        let future_from = self.future_from.unwrap_or(12);
        let id = self.id;
        let columns = self.counts.into_iter().enumerate().map(|(ix, n)| {
            let future = ix >= future_from;
            let tip: SharedString = month_text(ix, n).into();
            let bar = div().w_full().map(|this| {
                if future {
                    this.h(size::HAIRLINE).bg(c.border)
                } else if n == 0 {
                    this.h_0p5().bg(c.track)
                } else {
                    this.h(relative(n as f32 / max as f32))
                        .rounded_t(CoverSize::Sm.radius())
                        .bg(c.accent)
                }
            });
            div()
                .id((id.clone(), ix))
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                // The count over its bar; a line kept empty where there is
                // none, so the bars share a baseline.
                .child(div().caption(cx).tabular().child(if n > 0 && !future {
                    n.to_string()
                } else {
                    "\u{a0}".to_owned()
                }))
                .child(
                    div()
                        .h_8()
                        .w_full()
                        .flex()
                        .flex_col()
                        .justify_end()
                        .child(bar),
                )
                .child(div().caption(cx).child(INITIALS[ix]))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().caption(cx).child(self.caption))
            .child(h_flex().gap_1().items_end().children(columns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_month_names_its_count() {
        assert_eq!(month_text(5, 3), "June: 3 books");
        assert_eq!(month_text(0, 1), "January: 1 book");
        assert_eq!(month_text(11, 0), "December: 0 books");
    }
}
