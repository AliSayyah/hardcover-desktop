//! The Stats destination: what the reader finished, rated and shelved,
//! computed from the library on this computer. Recomputed only when the
//! library or the date changes.

use std::rc::Rc;

use gpui_kit::component::chart::BarChart;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, rems,
};
use hardcover_core::Date;

use super::reading_stats::{ReadingStats, YearTotals, compute};
use crate::format::{books_count_text, count_text, status_label};
use crate::library::library_view::empty_state;
use crate::library::model::LibraryModel;

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
                    .max_w(rems(48.))
                    .px_6()
                    .py_6()
                    .gap_8()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Stats"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("From your library on this computer."),
                            ),
                    )
                    .child(render_figures(&stats, cx))
                    .child(render_years(&stats, cx))
                    .child(render_ratings(&stats, cx))
                    .child(render_shelves(&stats, cx)),
            )
            .into_any_element()
    }
}

/// A large number over what it counts.
fn figure(value: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> AnyElement {
    v_flex()
        .gap_0p5()
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(value.into()),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
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
    h_flex().flex_wrap().gap_x_12().gap_y_4().children(figures)
}

fn section(title: &'static str, cx: &App) -> gpui_kit::Div {
    v_flex().gap_3().child(
        div()
            .pb_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_base()
            .font_weight(FontWeight::SEMIBOLD)
            .child(title),
    )
}

fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
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
    format!(
        "{} · {} pages",
        books_count_text(y.books),
        count_text(y.pages)
    )
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
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("Books")
                .into_any_element(),
            year_chart("books-per-year", &stats.years, |y| y.books, year_tooltip),
            div()
                .pt_2()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("Pages")
                .into_any_element(),
            year_chart("pages-per-year", &stats.years, |y| y.pages, year_tooltip),
        ]
    };
    section("Finished per year", cx)
        .children(body)
        .children(notes)
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
    let section = section("Your ratings", cx);
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
                .tooltip_title(|(ix, _)| format!("{} stars", half_stars(*ix)).into())
                .tooltip_value(|(_, n), _| books_count_text(*n).into()),
        ),
    )
}

fn render_shelves(stats: &ReadingStats, cx: &App) -> impl IntoElement {
    let rows = stats.shelves.iter().map(|(status, count)| {
        h_flex()
            .justify_between()
            .text_sm()
            .child(status_label(*status))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(count_text(*count)),
            )
    });
    section("Shelves", cx).child(v_flex().gap_2().children(rows))
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
