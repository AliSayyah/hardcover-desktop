//! Rows (`hardcover-design`, Rows): how books and facts line up.
//!
//! - [`Sequence`]: read-only rows on Home or a page (Continue reading, Coming
//!   soon, reviews): no fill, a hairline between rows (none above the first),
//!   12 above and below each, 16 for reviews. Their titles are
//!   [`super::TextButton`]s.
//! - [`ListRow`]: the list row of a collection you select from (a shelf,
//!   search results, a list): padding 8, `r-box`, gap 12, the `hover` and
//!   `selected` fills, the focus ring and edge. On the kit's `ListItem`.
//! - [`Lane`]: a fixed, right-aligned lane for trailing figures or actions,
//!   kept even when a row has nothing for it, so columns stay aligned.
//! - [`PropertyList`]: key and value rows ("Edition", "Owned", "Privacy").

use gpui_kit::component::h_flex;
use gpui_kit::component::list::ListItem;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement, Rems, RenderOnce, SharedString, Styled, Window,
    div, prelude::FluentBuilder as _,
};

use super::focus::{FocusTreatment as _, box_radius};
use super::reveal::probe;
use crate::design::{TypeRoles as _, colors, size};

/// The space above and below each row of a [`Sequence`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowSpacing {
    /// 12: Continue reading, Coming soon, the other sequences.
    #[default]
    Rows,
    /// 16: other readers' reviews, and Home's rows of books being read.
    Reviews,
}

/// Read-only rows separated by hairlines. See the module doc.
///
/// ```ignore
/// Sequence::new().children(books.iter().map(|b| upcoming_row(b)))
/// Sequence::new().spacing(RowSpacing::Reviews).flush_top().children(reviews)
/// ```
#[derive(IntoElement, Default)]
pub struct Sequence {
    spacing: RowSpacing,
    flush_top: bool,
    rows: Vec<AnyElement>,
}

impl Sequence {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spacing(mut self, spacing: RowSpacing) -> Self {
        self.spacing = spacing;
        self
    }

    /// No space above the first row, for a sequence right under its heading
    /// (reviews under "Reviews").
    pub fn flush_top(mut self) -> Self {
        self.flush_top = true;
        self
    }
}

impl ParentElement for Sequence {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.rows.extend(elements);
    }
}

impl RenderOnce for Sequence {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let border = colors(cx).border;
        let spacing = self.spacing;
        let flush_top = self.flush_top;
        div()
            .flex()
            .flex_col()
            .w_full()
            .children(self.rows.into_iter().enumerate().map(move |(ix, row)| {
                probe(
                    div()
                        .w_full()
                        .map(|this| match spacing {
                            RowSpacing::Rows => this.py_3(),
                            RowSpacing::Reviews => this.py_4(),
                        })
                        .when(ix == 0 && flush_top, |this| this.pt_0())
                        .when(ix > 0, |this| this.border_t_1().border_color(border))
                        .child(row),
                )
            }))
    }
}

/// The list row treatment, on the kit's `ListItem` (what every collection's
/// `ListDelegate::render_item` returns):
/// - padding 8, `r-box` corners, children 12 apart, rows 2 apart;
/// - `hover` on the pointer and `selected` on the current row (the theme's
///   `list_hover` and `list_active`, which the list applies itself);
/// - with `focused`, the focus ring outside and the 1 px accent edge inside
///   ([`super::focus::FocusTreatment::focus_row`]), drawn over the list
///   that clips the row. Pass whether this row is the selected one and its
///   list shows keyboard focus ([`super::focus::focus_visible`] on the
///   list's focus handle).
///
/// ```ignore
/// ListItem::new(("library-book", id))
///     .list_row(selected == Some(ix) && focus_visible(&self.focus, window), cx)
///     .child(BookRow::for_entry(entry))
/// ```
pub trait ListRow: Styled + Sized {
    fn list_row(self, focused: bool, cx: &App) -> Self;
}

impl ListRow for ListItem {
    fn list_row(self, focused: bool, cx: &App) -> Self {
        self.p_2()
            .gap_3()
            .mb_0p5()
            .rounded(box_radius(cx))
            .text_color(colors(cx).fg)
            .when(focused, |this| this.focus_row(box_radius(cx), cx))
    }
}

/// A trailing lane: fixed width, its content on the trailing edge. A row
/// keeps the lane when it has nothing to put in it.
///
/// ```ignore
/// row.child(Lane::trailing().child(percent))      // a list row's 48 lane
///    .child(Lane::action().child(want_to_read))   // an upcoming row's 136 lane
/// ```
#[derive(IntoElement)]
pub struct Lane {
    width: Option<Rems>,
    stacked: bool,
    children: Vec<AnyElement>,
}

impl Lane {
    /// A lane `width` wide.
    pub fn new(width: Rems) -> Self {
        Self {
            width: Some(width),
            stacked: false,
            children: Vec::new(),
        }
    }

    /// A list row's figures: 48 wide (tabular caption figures, or a 2 px
    /// meter under a percentage).
    pub fn trailing() -> Self {
        Self::new(size::TRAILING_LANE)
    }

    /// An upcoming row's action: 136 wide ("Want to read", or the
    /// StatusChip).
    pub fn action() -> Self {
        Self::new(size::ACTION_LANE)
    }

    /// As wide as its content, still right-aligned: a lane whose widest
    /// entry is known only per row, such as the date on an upcoming row.
    pub fn fit() -> Self {
        Self {
            width: None,
            stacked: false,
            children: Vec::new(),
        }
    }

    /// Stacks the content, right-aligned: a date over its caption, a
    /// percentage over its meter. Children are 4 apart.
    pub fn stacked(mut self) -> Self {
        self.stacked = true;
        self
    }
}

impl ParentElement for Lane {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Lane {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        probe(
            div()
                .flex()
                .flex_none()
                .when_some(self.width, |this, width| this.w(width))
                .map(|this| {
                    if self.stacked {
                        this.flex_col().items_end().gap_1()
                    } else {
                        this.items_center().justify_end().gap_2()
                    }
                })
                .text_right()
                .children(self.children),
        )
    }
}

/// Key and value rows: each at least 24 high, 8 apart. The key column is 96
/// (72 in a rail) in `fg-muted`; the value is ink, with small controls
/// inline. A value that runs to a second line (a hint under a switch in a
/// rail) keeps its key on the first line, which is 24 high.
///
/// ```ignore
/// PropertyList::new()
///     .row("Edition", h_flex().gap_2().child("Not chosen").child(choose))
///     .row("Added", "Nov 19, 2026")
/// ```
#[derive(IntoElement, Default)]
pub struct PropertyList {
    in_rail: bool,
    rows: Vec<(SharedString, AnyElement)>,
}

impl PropertyList {
    pub fn new() -> Self {
        Self::default()
    }

    /// The narrower key column of a rail.
    pub fn in_rail(mut self, in_rail: bool) -> Self {
        self.in_rail = in_rail;
        self
    }

    /// A row: `key` and its value.
    pub fn row(mut self, key: impl Into<SharedString>, value: impl IntoElement) -> Self {
        self.rows.push((key.into(), value.into_any_element()));
        self
    }
}

impl RenderOnce for PropertyList {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        let key_width = if self.in_rail {
            size::KEY_IN_RAIL
        } else {
            size::KEY
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .body()
            .children(self.rows.into_iter().map(move |(key, value)| {
                probe(
                    h_flex()
                        .w_full()
                        .items_start()
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .w(key_width)
                                .h(size::CONTROL_SMALL)
                                .pr_2()
                                .text_color(muted)
                                .child(div().min_w_0().truncate().child(key)),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .min_h(size::CONTROL_SMALL)
                                .gap_2()
                                .child(value),
                        ),
                )
            }))
    }
}
