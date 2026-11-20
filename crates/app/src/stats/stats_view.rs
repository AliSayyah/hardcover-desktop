//! The Stats destination: what the reader finished, rated and shelved,
//! computed from the library on this computer. Recomputed only when the
//! library or the date changes.
//!
//! A page (inset 32, at most 768 wide): the page title, the headline figures
//! in `display-30` over muted words, then sections ("Finished per year",
//! "Your ratings", "Shelves") 32 apart. Charts draw their bars in `accent`
//! on a `track` grid with muted labels (the theme's chart colours); counts
//! are tabular.

use std::rc::Rc;

use gpui_kit::component::chart::BarChart;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    rems,
};
use hardcover_core::Date;

use super::reading_stats::{ReadingStats, YearTotals, compute};
use crate::catalog::book_list::PAGE_WIDTH;
use crate::chip::status_icon;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{books_count_text, count_text, status_label};
use crate::library::model::LibraryModel;
use crate::ui::{Lane, PageHeader, SectionHeader, empty_state};

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
        let today = chrono::Local::now().date_naive();
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        root.overflow_y_scrollbar()
            .child(
                v_flex()
                    .w_full()
                    .max_w(PAGE_WIDTH)
                    .p_8()
                    .gap_8()
                    .child(PageHeader::new("Stats").line("From your library on this computer."))
                    .child(render_figures(&stats, cx))
                    .child(render_years(&stats, cx))
                    .child(render_ratings(&stats, cx))
                    .child(render_shelves(&stats, cx)),
            )
            .into_any_element()
    }
}

/// A headline figure in `display-30` over what it counts, muted.
fn figure(value: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> AnyElement {
    v_flex()
        .child(div().display_30().child(value.into()))
        .child(
            div()
                .body()
                .text_color(colors(cx).fg_muted)
                .child(label.into()),
        )
        .into_any_element()
}

fn render_figures(stats: &ReadingStats, cx: &App) -> impl IntoElement {
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
    h_flex().flex_wrap().gap_x_12().gap_y_6().children(figures)
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
    value: fn(&YearTotals) -> u32,
    tooltip: fn(&YearTotals) -> String,
) -> AnyElement {
    div()
        .w_full()
        .h(rems(12.))
        .child(
            BarChart::new(years.to_vec())
                .id(ElementId::Name(id.into()))
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

fn render_years(stats: &ReadingStats, cx: &App) -> impl IntoElement {
    let mut notes = Vec::new();
    if let Some(first) = stats.years.first().filter(|_| stats.earlier_books > 0) {
        notes.push(note(
            format!(
                "Plus {} finished before {}.",
                books_count_text(stats.earlier_books),
                first.year
            ),
            cx,
        ));
    }
    if stats.read_without_date > 0 {
        let books = match stats.read_without_date {
            1 => "1 book on your Read shelf has no finish date, so no year counts it.".to_owned(),
            n => {
                format!("{n} books on your Read shelf have no finish date, so no year counts them.")
            }
        };
        notes.push(note(books, cx));
    }
    let body: Vec<AnyElement> = if stats.years.is_empty() {
        vec![note(
            "Set a finish date on a book you've read and it's counted here.",
            cx,
        )]
    } else {
        vec![
            chart_label("Books", cx),
            year_chart("books-per-year", &stats.years, |y| y.books, year_tooltip),
            div()
                .pt_3()
                .child(chart_label("Pages", cx))
                .into_any_element(),
            year_chart("pages-per-year", &stats.years, |y| y.pages, year_tooltip),
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

fn render_ratings(stats: &ReadingStats, cx: &App) -> impl IntoElement {
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
        div().w_full().h(rems(10.)).child(
            BarChart::new(buckets)
                .id("rating-spread")
                .band(|(ix, _): &(usize, u32)| half_stars(*ix))
                .value(|(_, n)| f64::from(*n))
                .label(|(_, n)| match n {
                    0 => String::new(),
                    n => n.to_string(),
                })
                .tooltip_title(|(ix, _)| {
                    let stars = half_stars(*ix);
                    let unit = if matches!(stars.as_ref(), "1" | "½") {
                        "star"
                    } else {
                        "stars"
                    };
                    format!("{stars} {unit}").into()
                })
                .tooltip_value(|(_, n), _| books_count_text(*n).into()),
        ),
    )
}

/// Each shelf as the sidebar draws it: its icon, its name and its count on
/// a trailing lane, tabular, so the counts line up.
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
}
