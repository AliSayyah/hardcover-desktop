//! Counts as bars (`hardcover-design`, Bar rows and splits).
//!
//! - [`BarRows`]: a list of counts, one row each: the name in a 152 lane
//!   (text, or a [`super::TextButton`] that opens what it names), a 6 px
//!   meter as long as the row's count against the first row's (at most
//!   400), the count in a 48 lane and, with [`BarRows::share_of`], its share
//!   of a total in a 32 lane; rows 4 apart, at most 600 wide. A `label` line
//!   over them names the list and the count lane ("Genres" … "Books"). The
//!   reader's own counts, in accent (Stats' genres, moods, formats).
//! - [`SplitCells`]: a whole split a few ways (the Votes popover's pace and
//!   drive, Stats' fiction and non-fiction): equal cells 12 apart, in a
//!   fixed order, each its answer, optionally its count, its share on the
//!   trailing edge and a 6 px meter under them. The leading answer's name
//!   is ink at 500, the others muted. The shares add up to 100
//!   ([`shares`]).
//!
//! Every meter carries words beside it, and an accessible name the caller
//! gives (what it counts: "Fantasy: 177 books"). Meters keyed by `id`:
//! give an id that names what is counted (the period, the book), so a new
//! subject draws its bars at once rather than moving them.
//!
//! ```ignore
//! BarRows::new("stats-genres-2026")
//!     .heading("Genres", "Books")
//!     .share_of(349)
//!     .rows(genres.iter().map(|g| BarRow::new(name(g), g.books, label(g))))
//! SplitCells::new("stats-kind-2026")
//!     .cell("Fiction", 315, "Fiction: 315 books")
//!     .cell("Non-fiction", 14, "Non-fiction: 14 books")
//!     .with_counts()
//! ```

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::Meter;
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::count_text;

/// Each count's share of their total in whole percents, adding up to
/// exactly 100 (largest remainder: the floors first, then a point each to
/// the largest remainders, earlier first on a tie). All zeros with no
/// votes.
pub fn shares(counts: &[u32]) -> Vec<u32> {
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if total == 0 {
        return vec![0; counts.len()];
    }
    let mut out: Vec<u32> = Vec::with_capacity(counts.len());
    let mut remainders: Vec<(u64, usize)> = Vec::with_capacity(counts.len());
    for (ix, &count) in counts.iter().enumerate() {
        let scaled = u64::from(count) * 100;
        out.push(u32::try_from(scaled / total).unwrap_or(100));
        remainders.push((scaled % total, ix));
    }
    let left = 100 - out.iter().sum::<u32>();
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, ix) in remainders.iter().take(left as usize) {
        out[ix] += 1;
    }
    out
}

/// `count` as a whole percentage of `total`, rounded.
pub fn percent_of(count: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    u32::try_from((u64::from(count) * 200 + u64::from(total)) / (2 * u64::from(total)))
        .unwrap_or(100)
}

/// One row of [`BarRows`].
pub struct BarRow {
    name: AnyElement,
    count: u32,
    label: SharedString,
}

impl BarRow {
    /// A row: its name (text or a text button), its count, and its meter's
    /// accessible name ("Fantasy: 177 books").
    pub fn new(name: impl IntoElement, count: u32, label: impl Into<SharedString>) -> Self {
        Self {
            name: name.into_any_element(),
            count,
            label: label.into(),
        }
    }
}

/// Counts as rows of bars. See the module doc.
#[derive(IntoElement)]
pub struct BarRows {
    id: SharedString,
    heading: Option<(SharedString, SharedString)>,
    total: Option<u32>,
    rows: Vec<BarRow>,
}

impl BarRows {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            heading: None,
            total: None,
            rows: Vec::new(),
        }
    }

    /// The `label` line over the rows: the list's name, and what the count
    /// lane counts.
    pub fn heading(
        mut self,
        list: impl Into<SharedString>,
        counts: impl Into<SharedString>,
    ) -> Self {
        self.heading = Some((list.into(), counts.into()));
        self
    }

    /// Each row's share of `total`, after its count.
    pub fn share_of(mut self, total: u32) -> Self {
        self.total = Some(total);
        self
    }

    pub fn rows(mut self, rows: impl IntoIterator<Item = BarRow>) -> Self {
        self.rows.extend(rows);
        self
    }
}

impl RenderOnce for BarRows {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let most = self.rows.iter().map(|r| r.count).max().unwrap_or(0);
        let total = self.total;
        let id = self.id;
        let lane = |width, text: SharedString| {
            div()
                .w(width)
                .flex_none()
                .caption(cx)
                .tabular()
                .text_right()
                .child(text)
        };
        v_flex()
            .w_full()
            .max_w(size::BAR_ROWS)
            .gap_1()
            .when_some(self.heading, |this, (list, counts)| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(div().flex_1().min_w_0().label(cx).child(list))
                        .child(
                            div()
                                .w(size::TRAILING_LANE)
                                .flex_none()
                                .text_right()
                                .label(cx)
                                .child(counts),
                        )
                        .when(total.is_some(), |this| {
                            this.child(div().w(size::SHARE_LANE).flex_none())
                        }),
                )
            })
            .children(self.rows.into_iter().enumerate().map(|(ix, row)| {
                #[expect(clippy::cast_precision_loss, reason = "a percentage")]
                let percent = if most == 0 {
                    0.
                } else {
                    row.count as f32 * 100. / most as f32
                };
                let meter = Meter::new(
                    ElementId::NamedInteger(id.clone(), ix as u64),
                    percent,
                    row.label,
                )
                .flex_1()
                .max_w(size::METER_MAX);
                h_flex()
                    .w_full()
                    .min_h(size::CONTROL_SMALL)
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .w(size::BAR_NAME)
                            .flex_none()
                            .min_w_0()
                            .truncate()
                            .body()
                            .child(row.name),
                    )
                    .child(meter)
                    .child(lane(size::TRAILING_LANE, count_text(row.count).into()))
                    .when_some(total, |this, total| {
                        this.child(lane(
                            size::SHARE_LANE,
                            format!("{}%", percent_of(row.count, total)).into(),
                        ))
                    })
            }))
    }
}

struct SplitCell {
    name: SharedString,
    count: u32,
    label: SharedString,
}

/// A whole split a few ways. See the module doc.
#[derive(IntoElement)]
pub struct SplitCells {
    id: SharedString,
    cells: Vec<SplitCell>,
    leader: Option<usize>,
    counts: bool,
    muted: bool,
}

impl SplitCells {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            cells: Vec::new(),
            leader: None,
            counts: false,
            muted: false,
        }
    }

    /// A cell: its answer, its count, and its meter's accessible name.
    pub fn cell(
        mut self,
        name: impl Into<SharedString>,
        count: u32,
        label: impl Into<SharedString>,
    ) -> Self {
        self.cells.push(SplitCell {
            name: name.into(),
            count,
            label: label.into(),
        });
        self
    }

    /// The leading answer, named in ink. By default, the largest count (the
    /// first on a tie).
    pub fn leader(mut self, ix: usize) -> Self {
        self.leader = Some(ix);
        self
    }

    /// Each cell's count beside its answer.
    pub fn with_counts(mut self) -> Self {
        self.counts = true;
        self
    }

    /// Hardcover's figures: muted meters.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }
}

impl RenderOnce for SplitCells {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = colors(cx);
        let counts: Vec<u32> = self.cells.iter().map(|c| c.count).collect();
        let percents = shares(&counts);
        let leader = self.leader.unwrap_or_else(|| {
            let most = counts.iter().copied().max().unwrap_or(0);
            counts.iter().position(|c| *c == most).unwrap_or(0)
        });
        let id = self.id;
        let muted = self.muted;
        let show_counts = self.counts;
        h_flex()
            .w_full()
            .gap_3()
            .children(self.cells.into_iter().zip(percents).enumerate().map(
                move |(ix, (cell, percent))| {
                    let lead = ix == leader;
                    #[expect(clippy::cast_precision_loss, reason = "a percentage")]
                    let meter = Meter::new(
                        ElementId::NamedInteger(id.clone(), ix as u64),
                        percent as f32,
                        cell.label,
                    );
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .body()
                                        .text_color(if lead {
                                            palette.fg
                                        } else {
                                            palette.fg_muted
                                        })
                                        .when(lead, |this| this.font_weight(FontWeight::MEDIUM))
                                        .child(cell.name),
                                )
                                .when(show_counts, |this| {
                                    this.child(
                                        div()
                                            .flex_none()
                                            .caption(cx)
                                            .baseline_on(TypeRole::Caption, TypeRole::Body)
                                            .tabular()
                                            .child(count_text(cell.count)),
                                    )
                                })
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .flex_none()
                                        .caption(cx)
                                        .tabular()
                                        .child(format!("{percent}%")),
                                ),
                        )
                        .child(if muted { meter.muted() } else { meter })
                },
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_add_up_to_a_hundred() {
        assert_eq!(shares(&[1, 1, 1]), vec![34, 33, 33]);
        assert_eq!(shares(&[1, 2, 0]), vec![33, 67, 0]);
        assert_eq!(shares(&[58, 259, 133]).iter().sum::<u32>(), 100);
        assert_eq!(shares(&[0, 0, 0]), vec![0, 0, 0]);
    }

    #[test]
    fn a_share_of_a_total_rounds() {
        assert_eq!(percent_of(177, 349), 51);
        assert_eq!(percent_of(1, 3), 33);
        assert_eq!(percent_of(2, 3), 67);
        assert_eq!(percent_of(5, 0), 0);
    }
}
