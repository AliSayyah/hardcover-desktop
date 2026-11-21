//! The Stats destination: what the reader finished, rated and shelved,
//! computed from the library on this computer. Recomputed only when the
//! library or the date changes.
//!
//! A page (inset 32, filling the region): the page title, the headline
//! figures in `display-30` over muted words, then sections ("Finished per
//! year", "Your ratings", "Shelves") 32 apart. Charts draw their bars in
//! `accent` on a `track` grid with muted labels (the theme's chart colours);
//! each bar carries its count, so no figure is only in the hover tooltip.
//! Counts are tabular.
//!
//! A chart fills its column, its axis and grid across it; its bars widen
//! with it up to `size::CHART_BAR_MAX`, then the paper between them grows.
//! Finished per year gives each year at least `size::CHART_YEAR_MIN`, so a
//! wider chart reaches further back when the library has the years
//! ([`years_shown`]), never fewer than [`YEARS_AT_LEAST`].
//!
//! The figures sit in equal columns that wrap together: in a narrow window
//! two per row, each on the same spine as the one above.
//!
//! **Wide region** ([`two_columns`], from the window less the sidebar and
//! the insets): when two columns of at least 544 fit (a 1424 window), the
//! sections sit in two columns 32 apart, each filling its half: "Finished
//! per year" on the leading one, "Your ratings" and "Shelves" on the other.
//! The figures stay one row across both, 32 apart, so the third starts on
//! the second column's spine.

use std::rc::Rc;

use crate::ui::reveal::RevealScroll as _;
use gpui_kit::component::chart::BarChart;
use gpui_kit::component::{Icon, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::Date;

use super::reading_stats::{ReadingStats, YearTotals, compute};
use crate::chip::status_icon;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{books_count_text, count_text, status_label};
use crate::library::model::LibraryModel;
use crate::ui::{Lane, PageHeader, SectionHeader, empty_state, region_width};

pub struct StatsView {
    model: Entity<LibraryModel>,
    focus_handle: FocusHandle,
    /// The stats and the library revision and date they were computed for.
    computed: Option<(u64, Date, Rc<ReadingStats>)>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for StatsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl StatsView {
    pub fn new(model: Entity<LibraryModel>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&model, |_, _, cx| cx.notify())];
        Self {
            model,
            focus_handle: cx.focus_handle(),
            computed: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    fn stats(&mut self, cx: &App) -> Rc<ReadingStats> {
        let model = self.model.read(cx);
        let revision = model.revision();
        let today = crate::dates::today();
        match &self.computed {
            Some((r, d, stats)) if *r == revision && *d == today => stats.clone(),
            _ => {
                let stats = Rc::new(compute(model.entries(), today));
                self.computed = Some((revision, today, stats.clone()));
                stats
            }
        }
    }
}

impl Render for StatsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.model.read(cx).is_loaded();
        let stats = self.stats(cx);
        let root = div()
            .id("stats")
            .track_focus(&self.focus_handle)
            .size_full();
        if !loaded {
            return root.into_any_element();
        }
        if stats.is_empty() {
            return root
                .child(empty_state(
                    IconName::ChartPie,
                    "No stats yet".into(),
                    "Add books to your library and finish them, and your reading is counted here."
                        .into(),
                ))
                .into_any_element();
        }
        let rem = window.rem_size();
        let width = region_width(window, size::PAGE_INSET);
        let wide = two_columns(width, rem);
        let years = years_shown(chart_column(width, rem), rem);
        let bar = size::CHART_BAR_MAX.to_pixels(rem);
        let page = v_flex()
            .w_full()
            .p_8()
            .gap_8()
            .child(PageHeader::new("Stats").line("From your library on this computer."))
            .child(render_figures(&stats, wide, cx));
        let page = if wide {
            let column = || v_flex().flex_1().min_w_0().gap_8();
            page.child(
                h_flex()
                    .items_start()
                    .gap_8()
                    .child(column().child(render_years(&stats, years, bar, cx)))
                    .child(
                        column()
                            .child(render_ratings(&stats, bar, cx))
                            .child(render_shelves(&stats, cx)),
                    ),
            )
        } else {
            page.child(render_years(&stats, years, bar, cx))
                .child(render_ratings(&stats, bar, cx))
                .child(render_shelves(&stats, cx))
        };
        root.scroll_y_revealing_focus()
            .child(page)
            .into_any_element()
    }
}

/// Whether Stats' sections sit in two columns in content `width` wide (the
/// window less the sidebar and the insets): both columns get at least
/// [`size::CHART_COLUMN_MIN`], 32 apart.
fn two_columns(width: Pixels, rem: Pixels) -> bool {
    width >= size::CHART_COLUMN_MIN.to_pixels(rem) * 2. + rems(2.).to_pixels(rem)
}

/// How wide a chart's column is in content `width` wide: the whole width,
/// or half of it less the gap in [`two_columns`].
fn chart_column(width: Pixels, rem: Pixels) -> Pixels {
    if two_columns(width, rem) {
        (width - rems(2.).to_pixels(rem)) / 2.
    } else {
        width
    }
}

/// The fewest years Finished per year shows, when the library has them.
const YEARS_AT_LEAST: usize = 12;

/// How many years Finished per year shows in a column `column` wide: one
/// per `size::CHART_YEAR_MIN`, at least [`YEARS_AT_LEAST`]. Fewer when the
/// library has fewer ([`ReadingStats::recent_years`]).
fn years_shown(column: Pixels, rem: Pixels) -> usize {
    let fits = (column / size::CHART_YEAR_MIN.to_pixels(rem)).floor();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a floored width ratio; the saturating cast bounds it"
    )]
    let fits = if fits.is_finite() && fits > 0. {
        fits as usize
    } else {
        0
    };
    fits.max(YEARS_AT_LEAST)
}

/// A headline figure in `display-30` over what it counts, muted. Each takes
/// an equal share of its row, at least `size::FIGURE` wide, so figures that
/// wrap keep their columns.
fn figure(value: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .flex_basis(size::FIGURE)
        .min_w_0()
        .child(div().display_30().tabular().child(value.into()))
        .child(
            div()
                .body()
                .text_color(colors(cx).fg_muted)
                .child(label.into()),
        )
        .into_any_element()
}

/// The headline figures; `wide`, 32 apart (the columns' gap), so they line
/// up with the two columns under them.
fn render_figures(stats: &ReadingStats, wide: bool, cx: &App) -> impl IntoElement {
    let year = stats.this_year.year;
    let mut figures = vec![
        figure(
            count_text(stats.this_year.books),
            match stats.this_year.books {
                1 => format!("book finished in {year}"),
                _ => format!("books finished in {year}"),
            },
            cx,
        ),
        figure(
            count_text(stats.this_year.pages),
            format!("pages read in {year}"),
            cx,
        ),
    ];
    if let Some(days) = stats.average_days {
        figures.push(figure(
            format!("{days:.0}"),
            "days to finish, on average",
            cx,
        ));
    }
    if let Some(avg) = stats.average_rating {
        figures.push(figure(format!("{avg:.1}"), "average rating", cx));
    }
    h_flex()
        .flex_wrap()
        .items_start()
        .map(|this| if wide { this.gap_x_8() } else { this.gap_x_6() })
        .gap_y_6()
        .children(figures)
}

/// A section: its `section` title, then its content 12 below.
fn section(title: &'static str) -> gpui_kit::Div {
    v_flex().gap_3().child(SectionHeader::new(title))
}

/// A chart's name over it: "Books", "Pages".
fn chart_label(text: &'static str, cx: &App) -> AnyElement {
    div().label(cx).child(text).into_any_element()
}

fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .body()
        .text_color(colors(cx).fg_muted)
        .child(text.into())
        .into_any_element()
}

/// A bar chart of `value` per year, with the count on each bar.
fn year_chart(
    id: &'static str,
    years: &[YearTotals],
    bar: Pixels,
    value: fn(&YearTotals) -> u32,
    tooltip: fn(&YearTotals) -> String,
) -> AnyElement {
    div()
        .w_full()
        .h(size::CHART)
        .child(
            BarChart::new(years.to_vec())
                .id(ElementId::Name(id.into()))
                .max_band_width(bar)
                .band(|y: &YearTotals| SharedString::from(y.year.to_string()))
                .value(move |y| f64::from(value(y)))
                .label(move |y| match value(y) {
                    0 => String::new(),
                    n => count_text(n),
                })
                .tooltip_title(|y| y.year.to_string().into())
                .tooltip_value(move |y, _| tooltip(y).into()),
        )
        .into_any_element()
}

fn year_tooltip(y: &YearTotals) -> String {
    let pages = match y.pages {
        1 => "1 page".to_owned(),
        n => format!("{} pages", count_text(n)),
    };
    format!("{} · {pages}", books_count_text(y.books))
}

/// Finished per year: the latest `count` years (as many as the library
/// has, if fewer), bars at most `bar` wide.
fn render_years(stats: &ReadingStats, count: usize, bar: Pixels, cx: &App) -> impl IntoElement {
    let (years, earlier) = stats.recent_years(count);
    let mut notes = Vec::new();
    if let Some(first) = years.first().filter(|_| earlier > 0) {
        notes.push(note(
            format!(
                "Plus {} finished before {}.",
                books_count_text(earlier),
                first.year
            ),
            cx,
        ));
    }
    if stats.read_without_date > 0 {
        let books = match stats.read_without_date {
            1 => "1 book on your Read shelf has no finish date, so no year counts it.".to_owned(),
            n => {
                format!(
                    "{} books on your Read shelf have no finish date, so no year counts them.",
                    count_text(n)
                )
            }
        };
        notes.push(note(books, cx));
    }
    let body: Vec<AnyElement> = if years.is_empty() {
        vec![note(
            "Set a finish date on a book you've read and it's counted here.",
            cx,
        )]
    } else {
        vec![
            chart_label("Books", cx),
            year_chart("books-per-year", years, bar, |y| y.books, year_tooltip),
            div()
                .pt_3()
                .child(chart_label("Pages", cx))
                .into_any_element(),
            year_chart("pages-per-year", years, bar, |y| y.pages, year_tooltip),
        ]
    };
    section("Finished per year").children(body).children(notes)
}

/// "½", "1", "1½" … "5".
fn half_stars(index: usize) -> SharedString {
    let half_stars = index + 1;
    match (half_stars / 2, half_stars % 2) {
        (0, _) => "½".into(),
        (whole, 0) => whole.to_string().into(),
        (whole, _) => format!("{whole}½").into(),
    }
}

fn render_ratings(stats: &ReadingStats, bar: Pixels, cx: &App) -> impl IntoElement {
    let section = section("Your ratings");
    let Some(avg) = stats.average_rating else {
        return section.child(note("Rate a book and it's counted here.", cx));
    };
    let buckets: Vec<(usize, u32)> = stats.rating_counts.iter().copied().enumerate().collect();
    let summary = match stats.ratings {
        1 => format!("{avg:.1} from 1 rating"),
        n => format!("{avg:.1} on average from {n} ratings"),
    };
    section.child(note(summary, cx)).child(
        div().w_full().h(size::CHART_SHORT).child(
            BarChart::new(buckets)
                .id("rating-spread")
                .max_band_width(bar)
                .band(|(ix, _): &(usize, u32)| half_stars(*ix))
                .value(|(_, n)| f64::from(*n))
                .label(|(_, n)| match n {
                    0 => String::new(),
                    n => n.to_string(),
                })
                .tooltip_title(|(ix, _)| {
                    let label = half_stars(*ix);
                    let unit = if matches!(label.as_ref(), "1" | "½") {
                        "star"
                    } else {
                        "stars"
                    };
                    format!("{label} {unit}").into()
                })
                .tooltip_value(|(_, n), _| books_count_text(*n).into()),
        ),
    )
}

/// Each shelf as the sidebar draws it: its icon, its name and its count on
/// a trailing lane, tabular, so the counts line up. Unlike other rows, they
/// keep the shelf list's width (`size::LIST`), not their column's: six short
/// names whose counts would otherwise sit a column's width away from them.
fn render_shelves(stats: &ReadingStats, cx: &App) -> impl IntoElement {
    let muted = colors(cx).fg_muted;
    let rows = stats.shelves.iter().map(|(status, count)| {
        h_flex()
            .h(size::CONTROL)
            .gap_2()
            .body()
            .child(Icon::new(status_icon(*status)).text_color(muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(status_label(*status)),
            )
            .child(
                Lane::trailing().child(div().tabular().text_color(muted).child(count_text(*count))),
            )
    });
    section("Shelves").child(v_flex().max_w(size::LIST).children(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_star_labels() {
        let labels: Vec<SharedString> = (0..10).map(half_stars).collect();
        assert_eq!(
            labels,
            ["½", "1", "1½", "2", "2½", "3", "3½", "4", "4½", "5"].map(SharedString::from)
        );
    }

    #[test]
    fn stats_splits_into_two_columns_only_on_a_wide_region() {
        let rem = gpui_kit::px(16.);
        let at = |window: f32| two_columns(gpui_kit::px(window - 240. - 64.), rem);
        // 544 + 32 + 544 = 1120 of content: a 1424 window.
        assert!(two_columns(gpui_kit::px(1120.), rem));
        assert!(!two_columns(gpui_kit::px(1119.), rem));
        assert!(!at(800.));
        assert!(!at(1200.));
        assert!(!at(1423.));
        assert!(at(1424.));
        assert!(at(1600.));
        assert!(at(2560.));
    }

    #[test]
    fn each_column_fills_its_share_of_the_page() {
        let rem = gpui_kit::px(16.);
        let content = |window: f32| gpui_kit::px(window - 240. - 64.);
        // One column: the whole content.
        assert_eq!(chart_column(content(800.), rem), gpui_kit::px(496.));
        assert_eq!(chart_column(content(1200.), rem), gpui_kit::px(896.));
        assert_eq!(chart_column(content(1423.), rem), gpui_kit::px(1119.));
        // Two: each half, less half the 32 gap.
        assert_eq!(chart_column(content(1424.), rem), gpui_kit::px(544.));
        assert_eq!(chart_column(content(1600.), rem), gpui_kit::px(632.));
        assert_eq!(chart_column(content(2560.), rem), gpui_kit::px(1112.));
    }

    #[test]
    fn a_wider_chart_shows_more_years() {
        let rem = gpui_kit::px(16.);
        let years =
            |window: f32| years_shown(chart_column(gpui_kit::px(window - 240. - 64.), rem), rem);
        // Never fewer than twelve, as before.
        assert_eq!(years(800.), 12);
        assert_eq!(years(1600.), 12);
        // One per 64: 896 at the default window, 1112 per column at 2560.
        assert_eq!(years(1200.), 14);
        assert_eq!(years(2560.), 17);
        assert_eq!(years_shown(gpui_kit::px(64. * 20.), rem), 20);
        assert_eq!(years_shown(gpui_kit::px(64. * 20. - 1.), rem), 19);
        assert_eq!(years_shown(gpui_kit::px(0.), rem), 12);
    }
}
