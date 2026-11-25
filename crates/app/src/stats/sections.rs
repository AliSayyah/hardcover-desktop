//! The sections of the Stats page, each from the [`ReadingStats`] of the
//! period shown. A section with nothing to say returns `None` and isn't
//! drawn; one about a mix (pace, the genres) says when the period has too
//! few books.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::chart::BarChart;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ElementId, IntoElement, ParentElement as _, Pixels, Rems, SharedString,
    Styled as _, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::tags::Mood;
use hardcover_core::{BookSummary, ReadingStatus};

use super::reading_stats::{
    BookFigure, FEW, LENGTH_BUCKETS, OverTime, Period, ReadingStats, YearTotals,
};
use super::tag_stats::TagStats;
use crate::catalog::navigation::{self, Page};
use crate::chip::status_icon;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{books_count_text, count_text, status_label};
use crate::library::tag_filter::{GenrePick, TagPick};
use crate::ui::kit::SmallControl as _;
use crate::ui::{
    BarRow, BarRows, CoverShelf, Lane, LoadFailed, PropertyList, SectionHeader, Sequence, Skeleton,
    SplitCells, Stars, TextButton, percent_of,
};

/// How many genres and moods show before "Show N more".
pub const TAGS_SHOWN: usize = 8;
/// How many rereads show.
pub const REREADS_SHOWN: usize = 12;

/// What every section needs: the stats, the day, and the room it has.
pub struct Ctx<'a> {
    pub stats: &'a ReadingStats,
    /// The widest a chart's bar gets.
    pub bar: Pixels,
    /// How many years the per-year chart shows.
    pub years: usize,
    /// Two lists sit side by side (the column holds two of
    /// `size::STATS_PAIR_MIN`, 32 apart).
    pub pair: bool,
}

impl Ctx<'_> {
    fn period(&self) -> Period {
        self.stats.period
    }

    /// " in 2026", or nothing for all time.
    fn in_period(&self) -> String {
        match self.period() {
            Period::All => String::new(),
            Period::Year(year) => format!(" in {year}"),
        }
    }

    /// An element id for this period: a new period draws its meters at
    /// once rather than moving them.
    fn id(&self, name: &str) -> SharedString {
        match self.period() {
            Period::All => format!("stats-{name}-all").into(),
            Period::Year(year) => format!("stats-{name}-{year}").into(),
        }
    }

    fn too_few(&self, what: &str) -> AnyElement {
        let period = match self.period() {
            Period::All => "your library".to_owned(),
            Period::Year(year) => year.to_string(),
        };
        format!("Too few books in {period} {what}.").into_any()
    }
}

trait IntoAny {
    fn into_any(self) -> AnyElement;
}

impl IntoAny for String {
    fn into_any(self) -> AnyElement {
        div().child(self).into_any_element()
    }
}

/// A section: its `section` title, then its content 12 below.
fn section(title: &'static str) -> gpui_kit::Div {
    v_flex().w_full().gap_3().child(SectionHeader::new(title))
}

/// One muted line.
pub fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .body()
        .text_color(colors(cx).fg_muted)
        .child(text.into())
        .into_any_element()
}

/// A property's value as text that wraps in a narrow column.
fn wrapping(text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .tabular()
        .child(text.into())
        .into_any_element()
}

/// A footnote under a section's content: what it leaves out, in `caption`.
fn foot(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div().caption(cx).child(text.into()).into_any_element()
}

/// A muted line naming a book, whose title opens it: "The longest:
/// Infinite Jest, 1,104 pages." `after` follows the title with no space.
fn note_with_book(
    id: impl Into<ElementId>,
    before: impl Into<SharedString>,
    book: &BookSummary,
    after: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    h_flex()
        .flex_wrap()
        .gap_x_1()
        .body()
        .text_color(colors(cx).fg_muted)
        .child(before.into())
        .child(
            h_flex()
                .child(TextButton::book(id, book).body())
                .child(after.into()),
        )
        .into_any_element()
}

/// A book's title that opens it, then muted words after it: "Infinite
/// Jest · 250 days".
fn book_and(id: impl Into<ElementId>, book: &BookSummary, after: String, cx: &App) -> AnyElement {
    h_flex()
        .min_w_0()
        .gap_1()
        .child(TextButton::book(id, book).body())
        .child(
            div()
                .flex_none()
                .text_color(colors(cx).fg_muted)
                .child(after),
        )
        .into_any_element()
}

/// A bar of a chart: its band, its value, and its tooltip.
#[derive(Clone)]
pub struct Bar {
    pub band: SharedString,
    pub value: u32,
    pub title: SharedString,
    pub detail: SharedString,
}

/// A bar chart `height` high filling its column, the count on each bar.
fn chart(id: SharedString, height: Rems, bar: Pixels, bars: Vec<Bar>) -> AnyElement {
    div()
        .w_full()
        .h(height)
        .child(
            BarChart::new(bars)
                .id(ElementId::Name(id))
                .max_band_width(bar)
                .band(|b: &Bar| b.band.clone())
                .value(|b| f64::from(b.value))
                .label(|b| match b.value {
                    0 => String::new(),
                    n => count_text(n),
                })
                .tooltip_title(|b| b.title.clone())
                .tooltip_value(|b, _| b.detail.clone()),
        )
        .into_any_element()
}

fn chart_label(text: &'static str, cx: &App) -> AnyElement {
    div().label(cx).child(text).into_any_element()
}

fn totals_detail(t: &YearTotals) -> SharedString {
    let pages = match t.pages {
        1 => "1 page".to_owned(),
        n => format!("{} pages", count_text(n)),
    };
    format!("{} · {pages}", books_count_text(t.books)).into()
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_NAMES: [&str; 12] = [
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

// ----- Reading over time -----

pub fn over_time(c: &Ctx, cx: &App) -> Option<AnyElement> {
    let stats = c.stats;
    let mut body: Vec<AnyElement> = Vec::new();
    let mut notes: Vec<AnyElement> = Vec::new();
    let bars_of = |totals: &[YearTotals],
                   band: &dyn Fn(usize, &YearTotals) -> SharedString,
                   title: &dyn Fn(usize, &YearTotals) -> SharedString,
                   value: fn(&YearTotals) -> u32| {
        totals
            .iter()
            .enumerate()
            .map(|(ix, t)| Bar {
                band: band(ix, t),
                value: value(t),
                title: title(ix, t),
                detail: totals_detail(t),
            })
            .collect::<Vec<_>>()
    };
    match &stats.over_time {
        OverTime::Years { .. } => {
            let (years, earlier) = stats.over_time.recent_years(c.years);
            if years.is_empty() {
                body.push(note(
                    "Set a finish date on a book you've read and it's counted here.",
                    cx,
                ));
            } else {
                let band = |_: usize, y: &YearTotals| SharedString::from(y.year.to_string());
                // The books finished before the first year shown lead both
                // charts as one "Earlier" bar.
                let with_earlier = |value: fn(&YearTotals) -> u32| {
                    let mut bars = Vec::new();
                    if earlier.books > 0 {
                        bars.push(Bar {
                            band: "Earlier".into(),
                            value: value(&earlier),
                            title: format!("Before {}", earlier.year).into(),
                            detail: totals_detail(&earlier),
                        });
                    }
                    bars.extend(bars_of(years, &band, &band, value));
                    bars
                };
                body.push(chart_label("Books", cx));
                body.push(chart(
                    c.id("books"),
                    size::CHART,
                    c.bar,
                    with_earlier(|y| y.books),
                ));
                body.push(
                    div()
                        .pt_3()
                        .child(chart_label("Pages", cx))
                        .into_any_element(),
                );
                body.push(chart(
                    c.id("pages"),
                    size::CHART,
                    c.bar,
                    with_earlier(|y| y.pages),
                ));
            }
        }
        OverTime::Months { year, months } => {
            let year = *year;
            let band = |ix: usize, _: &YearTotals| SharedString::from(MONTHS[ix]);
            let title = move |ix: usize, _: &YearTotals| {
                SharedString::from(format!("{} {year}", MONTH_NAMES[ix]))
            };
            body.push(chart_label("Books", cx));
            body.push(chart(
                c.id("books"),
                size::CHART,
                c.bar,
                bars_of(months, &band, &title, |m| m.books),
            ));
            body.push(
                div()
                    .pt_3()
                    .child(chart_label("Pages", cx))
                    .into_any_element(),
            );
            body.push(chart(
                c.id("pages"),
                size::CHART,
                c.bar,
                bars_of(months, &band, &title, |m| m.pages),
            ));
        }
    }
    if stats.read_without_date > 0 && c.period() == Period::All {
        notes.push(foot(
            match stats.read_without_date {
                1 => {
                    "1 book on your Read shelf has no finish date, so no year counts it.".to_owned()
                }
                n => format!(
                    "{} books on your Read shelf have no finish date, so no year counts them.",
                    count_text(n)
                ),
            },
            cx,
        ));
    }
    Some(
        section("Reading over time")
            .children(body)
            .children(notes)
            .into_any_element(),
    )
}

// ----- Pace -----

/// A goal of books for the period's year, and where it stands today.
pub struct GoalPace {
    /// Books ahead (positive) or behind of steady progress.
    pub ahead: f64,
}

fn days_text(days: u32) -> String {
    match days {
        0 | 1 => "the same day".to_owned(),
        n => format!("{} days", count_text(n)),
    }
}

pub fn pace(c: &Ctx, goal: Option<GoalPace>, cx: &App) -> Option<AnyElement> {
    let section = section("Pace");
    if c.stats.reads == 0 {
        return None;
    }
    let Some(pace) = &c.stats.pace else {
        return Some(
            section
                .child(note_el(c.too_few("to show a pace"), cx))
                .into_any_element(),
        );
    };
    let mut list = PropertyList::new();
    if let Some(forecast) = pace.forecast {
        let goal = goal.map(|g| {
            #[expect(clippy::cast_possible_truncation, reason = "a small count of books")]
            let books = g.ahead.round() as i64;
            match books {
                0 => " · on pace for your goal".to_owned(),
                n if n > 0 => format!(" · {} ahead of your goal", count_text(n.unsigned_abs())),
                n => format!(" · {} behind your goal", count_text(n.unsigned_abs())),
            }
        });
        list = list.row(
            "Forecast",
            h_flex()
                .flex_wrap()
                .child(div().body_strong().child(books_count_text(forecast)))
                .child(div().flex_1().min_w_0().child(format!(
                    " by Dec 31 at this pace{}",
                    goal.unwrap_or_default()
                ))),
        );
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "small non-negative figures"
    )]
    let (every, per_day) = (
        pace.days_per_book().round().max(1.) as u32,
        pace.pages_per_day().round() as u32,
    );
    let mut overall = match every {
        1 => "A book a day".to_owned(),
        n if n <= 60 => format!("A book every {} days", count_text(n)),
        _ => format!("A book every {}", every_text(f64::from(every))),
    };
    let per_month = pace.books_per_month();
    if per_month >= 1.0 {
        overall.push_str(&format!(" · {per_month:.1} a month"));
    } else {
        overall.push_str(&format!(" · {:.1} a year", per_month * 12.0));
    }
    if per_day > 0 {
        overall.push_str(&format!(" · {} pages a day", count_text(per_day)));
    }
    list = list.row("Overall", wrapping(overall));
    if let Some(days) = pace.to_finish {
        list = list.row(
            "To finish",
            wrapping(format!("{days:.0} days on average, from starting a book")),
        );
    }
    if let Some(q) = &pace.quickest {
        list = list.row(
            "Quickest",
            book_and(
                "stats-quickest",
                &q.book,
                format!("· {}", days_text(q.value)),
                cx,
            ),
        );
    }
    if let Some(l) = pace
        .longest
        .as_ref()
        .filter(|l| Some(*l) != pace.quickest.as_ref())
    {
        list = list.row(
            "Longest",
            book_and(
                "stats-longest",
                &l.book,
                format!("· {}", days_text(l.value)),
                cx,
            ),
        );
    }
    Some(section.child(list).into_any_element())
}

fn note_el(text: AnyElement, cx: &App) -> AnyElement {
    div()
        .body()
        .text_color(colors(cx).fg_muted)
        .child(text)
        .into_any_element()
}

// ----- Genres and moods -----

/// Where the genres and moods stand.
pub enum TagState {
    /// Without `read:catalog`: no section.
    Hidden,
    Loading,
    Failed(SharedString),
    Ready {
        stats: Rc<TagStats>,
        failure: Option<SharedString>,
    },
}

/// A command a section's button runs.
pub type Action = Rc<dyn Fn(&mut gpui_kit::Window, &mut App)>;

/// The genres and moods' commands: "Try again", and "Show N more" on each
/// list.
pub struct TagActions {
    pub retry: Action,
    pub more_genres: Option<Action>,
    pub more_moods: Option<Action>,
}

fn pair(c: &Ctx, a: AnyElement, b: AnyElement) -> AnyElement {
    if c.pair {
        h_flex()
            .w_full()
            .items_start()
            .gap_8()
            .child(div().flex_1().min_w_0().child(a))
            .child(div().flex_1().min_w_0().child(b))
            .into_any_element()
    } else {
        v_flex()
            .w_full()
            .gap_6()
            .child(a)
            .child(b)
            .into_any_element()
    }
}

fn show_more(id: &'static str, hidden: usize, on: Option<Action>) -> Option<AnyElement> {
    let on = on?;
    (hidden > 0).then(|| {
        h_flex()
            .pt_1()
            .child(
                Button::new(id)
                    .ghost()
                    .small_control()
                    .label(format!("Show {hidden} more"))
                    .on_click(move |_, window, cx| on(window, cx)),
            )
            .into_any_element()
    })
}

pub fn genres_and_moods(
    c: &Ctx,
    state: TagState,
    expanded: (bool, bool),
    actions: TagActions,
    cx: &App,
) -> Option<AnyElement> {
    let section = section("Genres and moods");
    if c.stats.reads == 0 {
        return None;
    }
    let retry = actions.retry.clone();
    let failed = move |message: SharedString| {
        let retry = retry.clone();
        LoadFailed::new("stats-tags-retry", message)
            .on_retry(move |_, window, cx| retry(window, cx))
            .into_any_element()
    };
    match state {
        TagState::Hidden => None,
        _ if c.stats.is_few() => Some(
            section
                .child(note_el(c.too_few("to say"), cx))
                .into_any_element(),
        ),
        TagState::Loading => {
            let rows =
                |label: &'static str| {
                    v_flex()
                        .w_full()
                        .max_w(size::BAR_ROWS)
                        .gap_1()
                        .child(div().label(cx).child(label))
                        .children((0..5).map(|ix| {
                            h_flex()
                                .h(size::CONTROL_SMALL)
                                .gap_3()
                                .items_center()
                                .child(div().w(size::BAR_NAME).flex_none().child(
                                    Skeleton::line().w(rems([4.5, 6., 4., 5.5, 3.5][ix % 5])),
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .max_w(size::METER_MAX)
                                        .child(Skeleton::line().h(size::METER)),
                                )
                        }))
                        .into_any_element()
                };
            Some(
                section
                    .child(pair(c, rows("Genres"), rows("Moods")))
                    .into_any_element(),
            )
        }
        TagState::Failed(message) => Some(section.child(failed(message)).into_any_element()),
        TagState::Ready { stats, failure } => {
            let books = c.stats.distinct_books();
            let known = stats.known();
            let kinds = (known > 0).then(|| {
                div()
                    .w_full()
                    .max_w(size::BAR_ROWS)
                    .child(
                        SplitCells::new(c.id("kind"))
                            .cell(
                                "Fiction",
                                stats.fiction,
                                format!("Fiction: {}", books_count_text(stats.fiction)),
                            )
                            .cell(
                                "Non-fiction",
                                stats.non_fiction,
                                format!("Non-fiction: {}", books_count_text(stats.non_fiction)),
                            )
                            .when(stats.neither > 0, |this| {
                                this.cell(
                                    "Unknown",
                                    stats.neither,
                                    format!("Unknown: {}", books_count_text(stats.neither)),
                                )
                            })
                            .with_counts(),
                    )
                    .into_any_element()
            });
            let genre_rows = |all: bool| {
                stats
                    .genres
                    .iter()
                    .take(if all { usize::MAX } else { TAGS_SHOWN })
                    .enumerate()
                    .map(|(ix, g)| {
                        let pick = TagPick {
                            genre: Some(GenrePick {
                                key: g.genre.key.clone(),
                                name: g.genre.name.clone(),
                            }),
                            mood: None,
                        };
                        BarRow::new(
                            TextButton::new(("stats-genre", ix), g.genre.name.clone())
                                .body()
                                .on_click(move |_, _, cx| {
                                    navigation::open_shelf(ReadingStatus::Read, pick.clone(), cx);
                                }),
                            g.books,
                            format!("{}: {}", g.genre.name, books_count_text(g.books)),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let mood_rows = |all: bool| {
                stats
                    .moods
                    .iter()
                    .take(if all { usize::MAX } else { TAGS_SHOWN })
                    .map(|(mood, n)| {
                        let mood: Mood = *mood;
                        BarRow::new(
                            TextButton::new(
                                ("stats-mood", mood.id().get().cast_unsigned()),
                                mood.name(),
                            )
                            .body()
                            .on_click(move |_, _, cx| {
                                navigation::open_shelf(
                                    ReadingStatus::Read,
                                    TagPick {
                                        genre: None,
                                        mood: Some(mood),
                                    },
                                    cx,
                                );
                            }),
                            *n,
                            format!("{}: {}", mood.name(), books_count_text(*n)),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let list = |rows: Vec<BarRow>,
                        heading: &'static str,
                        id: &str,
                        more: Option<AnyElement>,
                        empty: &'static str|
             -> AnyElement {
                if rows.is_empty() {
                    return v_flex()
                        .gap_1()
                        .child(div().label(cx).child(heading))
                        .child(note(empty, cx))
                        .into_any_element();
                }
                v_flex()
                    .w_full()
                    .child(
                        BarRows::new(c.id(id))
                            .heading(heading, "Books")
                            .share_of(books)
                            .rows(rows),
                    )
                    .children(more)
                    .into_any_element()
            };
            let genres = list(
                genre_rows(expanded.0),
                "Genres",
                "genres",
                if expanded.0 {
                    None
                } else {
                    show_more(
                        "stats-more-genres",
                        stats.genres.len().saturating_sub(TAGS_SHOWN),
                        actions.more_genres,
                    )
                },
                "No genres yet.",
            );
            let moods = list(
                mood_rows(expanded.1),
                "Moods",
                "moods",
                if expanded.1 {
                    None
                } else {
                    show_more(
                        "stats-more-moods",
                        stats.moods.len().saturating_sub(TAGS_SHOWN),
                        actions.more_moods,
                    )
                },
                "None of these books has enough mood votes yet.",
            );
            let mut text = format!(
                "Shares are of the {} you finished{}; a book counts in each of its genres.",
                books_count_text(books),
                c.in_period()
            );
            if stats.without_genre > 0 {
                text.push_str(&format!(" {} no genre;", have(stats.without_genre)));
            }
            if stats.without_mood > 0 {
                text.push_str(&format!(
                    " {} too few mood votes for a mood;",
                    have(stats.without_mood)
                ));
            }
            if stats.unknown > 0 {
                text.push_str(&format!(
                    " {} tags that haven't loaded;",
                    have(stats.unknown)
                ));
            }
            let text =
                text.trim_end_matches(';').to_owned() + if text.ends_with(';') { "." } else { "" };
            Some(
                section
                    .children(kinds)
                    .child(div().pt_2().child(pair(c, genres, moods)))
                    .child(foot(text, cx))
                    .children(failure.map(failed))
                    .into_any_element(),
            )
        }
    }
}

/// "1 has", "20 have".
fn have(n: u32) -> String {
    match n {
        1 => "1 has".to_owned(),
        n => format!("{} have", count_text(n)),
    }
}

// ----- Formats -----

pub fn formats(c: &Ctx, cx: &App) -> Option<AnyElement> {
    let f = c.stats.formats;
    let known = f.physical + f.ebook + f.audiobook;
    if c.stats.is_few() || known == 0 {
        return None;
    }
    let rows: Vec<(&str, &str, u32)> = [
        ("Physical", "physical books", f.physical),
        ("Ebook", "ebooks", f.ebook),
        ("Audiobook", "audiobooks", f.audiobook),
    ]
    .into_iter()
    .filter(|(_, _, n)| *n > 0)
    .collect();
    let with_format = match known {
        1 => "1 book with a format".to_owned(),
        n => format!("{} books with a format", count_text(n)),
    };
    let most = rows.iter().map(|(_, _, n)| *n).max().unwrap_or(0);
    let leaders: Vec<_> = rows.iter().filter(|(_, _, n)| *n == most).collect();
    // "Mostly" only when one format holds most of them, and of enough.
    let line = match leaders.as_slice() {
        [(_, lead, _)] if known >= FEW && most * 2 > known => format!(
            "Mostly {lead}: {}% of the {with_format}{}.",
            percent_of(most, known),
            c.in_period()
        ),
        _ => format!("Of the {with_format}{}:", c.in_period()),
    };
    Some(
        section("Formats")
            .child(note(line, cx))
            .child(
                BarRows::new(c.id("formats"))
                    .share_of(known)
                    .rows(rows.iter().map(|(name, _, n)| {
                        BarRow::new(*name, *n, format!("{name}: {}", books_count_text(*n)))
                    })),
            )
            .when(f.unknown > 0, |this| {
                this.child(foot(
                    match f.unknown {
                        1 => "1 book has no edition with a format.".to_owned(),
                        n => format!("{} books have no edition with a format.", count_text(n)),
                    },
                    cx,
                ))
            })
            .into_any_element(),
    )
}

// ----- Book length -----

fn bucket_label(ix: usize) -> SharedString {
    let low = if ix == 0 {
        1
    } else {
        LENGTH_BUCKETS[ix - 1] + 1
    };
    match LENGTH_BUCKETS.get(ix) {
        Some(high) => format!("{}–{}", count_text(low), count_text(*high)).into(),
        None => format!("{}+", count_text(LENGTH_BUCKETS[LENGTH_BUCKETS.len() - 1])).into(),
    }
}

pub fn lengths(c: &Ctx, cx: &App) -> Option<AnyElement> {
    let l = &c.stats.lengths;
    let average = l.average?;
    if c.stats.is_few() {
        return None;
    }
    let most = l
        .buckets
        .iter()
        .enumerate()
        .max_by_key(|(ix, n)| (**n, std::cmp::Reverse(*ix)))
        .map_or(0, |(ix, _)| ix);
    let bars = l
        .buckets
        .iter()
        .enumerate()
        .map(|(ix, n)| Bar {
            band: bucket_label(ix),
            value: *n,
            title: format!("{} pages", bucket_label(ix)).into(),
            detail: books_count_text(*n).into(),
        })
        .collect();
    let mut notes: Vec<AnyElement> = Vec::new();
    if let Some(BookFigure { book, value }) = &l.longest {
        notes.push(note_with_book(
            "stats-longest-book",
            "The longest:",
            book,
            format!(", {} pages.", count_text(*value)),
            cx,
        ));
    }
    if l.without > 0 {
        notes.push(foot(
            match l.without {
                1 => "1 book has no page count (an audiobook, say), so it's left out.".to_owned(),
                n => format!(
                    "{} books have no page count (audiobooks, say), so they're left out.",
                    count_text(n)
                ),
            },
            cx,
        ));
    }
    Some(
        section("Book length")
            .child(note(
                format!(
                    "Most run {} pages; {} on average.",
                    bucket_label(most),
                    count_text(average)
                ),
                cx,
            ))
            .child(chart(c.id("lengths"), size::CHART_SHORT, c.bar, bars))
            .children(notes)
            .into_any_element(),
    )
}

// ----- Your ratings -----

/// "½", "1", "1½" … "5".
pub fn half_stars(index: usize) -> SharedString {
    let half_stars = index + 1;
    match (half_stars / 2, half_stars % 2) {
        (0, _) => "½".into(),
        (whole, 0) => whole.to_string().into(),
        (whole, _) => format!("{whole}½").into(),
    }
}

pub fn ratings(c: &Ctx, cx: &App) -> Option<AnyElement> {
    let r = &c.stats.ratings;
    let section = section("Your ratings");
    let Some(avg) = r.average else {
        return Some(
            section
                .child(note(
                    match c.period() {
                        Period::All => "Rate a book and it's counted here.".to_owned(),
                        Period::Year(year) => {
                            format!("None of the books you finished in {year} is rated.")
                        }
                    },
                    cx,
                ))
                .into_any_element(),
        );
    };
    let of = match c.period() {
        Period::All => String::new(),
        Period::Year(year) => format!(" of books finished in {year}"),
    };
    let summary = match r.count {
        1 => format!("{avg:.2} from 1 rating{of}."),
        n => format!("{avg:.2} on average from {} ratings{of}.", count_text(n)),
    };
    let bars = r
        .counts
        .iter()
        .enumerate()
        .map(|(ix, n)| {
            let label = half_stars(ix);
            let unit = if matches!(label.as_ref(), "1" | "½") {
                "star"
            } else {
                "stars"
            };
            Bar {
                band: label.clone(),
                value: *n,
                title: format!("{label} {unit}").into(),
                detail: books_count_text(*n).into(),
            }
        })
        .collect();
    let compared = |list: &[super::reading_stats::Compared],
                    id: &'static str,
                    heading: &'static str| {
        (!list.is_empty()).then(|| {
            v_flex()
                .w_full()
                .gap_2()
                .child(div().heading().child(heading))
                .child(
                    Sequence::new()
                        .flush_top()
                        .children(list.iter().enumerate().map(|(ix, b)| {
                            h_flex()
                                .w_full()
                                .gap_3()
                                .items_center()
                                .child(
                                    Cover::new(CoverSize::Sm, b.book.title.clone())
                                        .thumbnail(b.book.cover.as_ref().map(|c| c.url.clone()))
                                        .opens(
                                            SharedString::from(format!("{id}-cover-{ix}")),
                                            Page::Book(b.book.clone()),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(TextButton::book((id, ix), &b.book))
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .caption(cx)
                                                .child(
                                                    Stars::new(b.yours)
                                                        .small()
                                                        .own()
                                                        .accessibility_label(
                                                            SharedString::from(format!(
                                                                "{id}-yours-{ix}"
                                                            )),
                                                            format!("You rated it {}", b.yours),
                                                        ),
                                                )
                                                .child("You")
                                                .child("·")
                                                .child(
                                                    Icon::new(crate::star::icon())
                                                        .size_3()
                                                        .text_color(colors(cx).fg_muted),
                                                )
                                                .child(format!("{:.2} on Hardcover", b.hardcover)),
                                        ),
                                )
                        })),
                )
                .into_any_element()
        })
    };
    let higher = compared(&c.stats.higher, "stats-higher", "Higher than most readers");
    let lower = compared(&c.stats.lower, "stats-lower", "Lower than most readers");
    let against = match (higher, lower) {
        (Some(a), Some(b)) => Some(pair(c, a, b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    };
    Some(
        section
            .child(note(summary, cx))
            .child(chart(c.id("ratings"), size::CHART_SHORT, c.bar, bars))
            .children(against.map(|a| div().pt_3().child(a)))
            .into_any_element(),
    )
}

// ----- Authors -----

fn average_text(average: Option<f64>) -> String {
    average.map_or_else(String::new, |a| format!(" · {a:.1} average"))
}

pub fn authors(c: &Ctx, cx: &App) -> Option<AnyElement> {
    if c.stats.is_few() || c.stats.authors == 0 {
        return None;
    }
    let line = format!(
        "{} {}{}{}",
        count_text(c.stats.authors),
        if c.stats.authors == 1 {
            "author"
        } else {
            "authors"
        },
        c.in_period(),
        if c.stats.top_authors.is_empty() {
            ", none of them twice."
        } else {
            ". The ones you read most:"
        }
    );
    let rows = c.stats.top_authors.iter().enumerate().map(|(ix, a)| {
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().body_strong().truncate().child(a.name.clone()))
                    .child(
                        div()
                            .body()
                            .text_color(colors(cx).fg_muted)
                            .tabular()
                            .child(format!(
                                "{}{}",
                                books_count_text(a.books),
                                average_text(a.average)
                            )),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .children(a.books_shown.iter().enumerate().map(move |(bx, b)| {
                        Cover::new(CoverSize::Sm, b.title.clone())
                            .thumbnail(b.cover.as_ref().map(|c| c.url.clone()))
                            .opens(
                                SharedString::from(format!("stats-author-{ix}-{bx}")),
                                Page::Book(b.clone()),
                            )
                    })),
            )
    });
    Some(
        section("Authors")
            .child(note(line, cx))
            .when(!c.stats.top_authors.is_empty(), |this| {
                this.child(
                    div()
                        .w_full()
                        .max_w(size::BAR_ROWS)
                        .child(Sequence::new().children(rows)),
                )
            })
            .into_any_element(),
    )
}

// ----- Series -----

pub fn series(c: &Ctx, cx: &App) -> Option<AnyElement> {
    if c.stats.series.is_empty() {
        return None;
    }
    let rows = c.stats.series.iter().enumerate().map(|(ix, s)| {
        let name: AnyElement = match s.id {
            Some(id) => TextButton::new(("stats-series", ix), s.name.clone())
                .opens(Page::Series(id))
                .into_any_element(),
            None => div()
                .body_strong()
                .truncate()
                .child(s.name.clone())
                .into_any_element(),
        };
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .child(
                Cover::new(CoverSize::Sm, s.lead.title.clone())
                    .thumbnail(s.lead.cover.as_ref().map(|c| c.url.clone()))
                    .opens(
                        SharedString::from(format!("stats-series-cover-{ix}")),
                        Page::Book(s.lead.clone()),
                    ),
            )
            .child(
                v_flex().flex_1().min_w_0().child(name).child(
                    div()
                        .body()
                        .text_color(colors(cx).fg_muted)
                        .tabular()
                        .child(format!(
                            "{} · {} pages{}",
                            books_count_text(s.books),
                            count_text(s.pages),
                            average_text(s.average)
                        )),
                ),
            )
    });
    Some(
        section("Series")
            .child(note(
                format!("The series you read most{}.", c.in_period()),
                cx,
            ))
            .child(Sequence::new().children(rows))
            .into_any_element(),
    )
}

// ----- Rereads -----

fn times_text(times: u32) -> String {
    match times {
        2 => "Twice".to_owned(),
        n => format!("{n} times"),
    }
}

pub fn rereads(c: &Ctx, cx: &App) -> Option<AnyElement> {
    let rereads = &c.stats.rereads;
    if rereads.is_empty() {
        return None;
    }
    let line = match c.period() {
        Period::All => format!(
            "{} finished more than once: {} reads of {}.",
            books_count_text(rereads.len()),
            count_text(c.stats.reads),
            books_count_text(c.stats.distinct_books())
        ),
        Period::Year(year) => format!(
            "{} finished in {year} had been finished before.",
            books_count_text(rereads.len())
        ),
    };
    Some(
        section("Rereads")
            .child(note(line, cx))
            .child(
                CoverShelf::new(CoverSize::Md).books_captioned(
                    rereads
                        .iter()
                        .take(REREADS_SHOWN)
                        .enumerate()
                        .map(|(ix, r)| {
                            (
                                Cover::new(CoverSize::Md, r.book.title.clone())
                                    .thumbnail(r.book.cover.as_ref().map(|c| c.url.clone()))
                                    .opens(
                                        SharedString::from(format!("stats-reread-cover-{ix}")),
                                        Page::Book(r.book.clone()),
                                    ),
                                TextButton::book(("stats-reread", ix), &r.book),
                                times_text(r.times).into(),
                            )
                        }),
                ),
            )
            .into_any_element(),
    )
}

// ----- From adding to finishing -----

/// "42 days", "9 months", "2 years, 9 months".
pub fn duration_text(days: u32) -> String {
    if days == 0 {
        return "the same day".to_owned();
    }
    if days < 60 {
        return match days {
            1 => "1 day".to_owned(),
            n => format!("{n} days"),
        };
    }
    let months = (f64::from(days) / 30.44).round() as u32;
    if months < 24 {
        return format!("{months} months");
    }
    let (years, months) = (months / 12, months % 12);
    match months {
        0 => format!("{years} years"),
        1 => format!("{years} years, 1 month"),
        m => format!("{years} years, {m} months"),
    }
}

/// A span of `days` in months under a year, else in years to the half:
/// "a month", "2 months", "a year", "6½ years".
fn span_text(days: f64) -> String {
    let years = days / 365.25;
    if years < 1.0 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "under a year"
        )]
        let months = (days / 30.44).round().clamp(1.0, 12.0) as u32;
        match months {
            1 => "a month".to_owned(),
            12 => "a year".to_owned(),
            m => format!("{m} months"),
        }
    } else {
        let halves = (years * 2.0).round();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a few years"
        )]
        let whole = (halves / 2.0).floor() as u32;
        match (whole, halves % 2.0 >= 1.0) {
            (1, false) => "a year".to_owned(),
            (w, false) => format!("{w} years"),
            (w, true) => format!("{w}½ years"),
        }
    }
}

/// One book every `days`, past two months: "2 months", "month", "6½
/// years".
fn every_text(days: f64) -> String {
    let span = span_text(days);
    span.strip_prefix("a ").map(str::to_owned).unwrap_or(span)
}

/// How long `books` take at `per_day` books a day: "about 6½ years".
pub fn backlog_text(books: usize, per_day: f64) -> Option<String> {
    if books == 0 || per_day <= 0.0 {
        return None;
    }
    #[expect(clippy::cast_precision_loss, reason = "a count of books")]
    let days = books as f64 / per_day;
    Some(format!("about {}", span_text(days)))
}

pub fn wait(c: &Ctx, cx: &App) -> Option<AnyElement> {
    let w = c.stats.wait.as_ref()?;
    if c.stats.is_few() {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "days"
    )]
    let average = w.average_days.round() as u32;
    let mut list = PropertyList::new()
        .row("On average", wrapping(duration_text(average)))
        .row(
            "Quickest",
            book_and(
                "stats-wait-quickest",
                &w.quickest.book,
                format!("· {}", duration_text(w.quickest.value)),
                cx,
            ),
        );
    if w.longest.book.id != w.quickest.book.id {
        list = list.row(
            "Longest",
            book_and(
                "stats-wait-longest",
                &w.longest.book,
                format!("· {}", duration_text(w.longest.value)),
                cx,
            ),
        );
    }
    let waiting = c.stats.shelf(ReadingStatus::WantToRead);
    let per_day = f64::from(c.stats.past_year_reads) / 365.0;
    if let Some(backlog) = backlog_text(waiting, per_day) {
        list = list.row(
            "Still waiting",
            wrapping(format!(
                "{} on Want to read: {backlog} at this past year's pace",
                books_count_text(waiting)
            )),
        );
    }
    Some(
        section("From adding to finishing")
            .child(list)
            .child(foot(
                format!(
                    "From the day a book joined your library to the day you finished it. Counts the {} added before you finished them.",
                    books_count_text(w.measured)
                ),
                cx,
            ))
            .into_any_element(),
    )
}

// ----- Publication years -----

pub fn decades(c: &Ctx, cx: &App) -> Option<AnyElement> {
    let d = &c.stats.decades;
    if c.stats.is_few() || (d.decades.is_empty() && d.earlier == 0) {
        return None;
    }
    let mut bars: Vec<Bar> = Vec::new();
    if d.earlier > 0 {
        bars.push(Bar {
            band: "Earlier".into(),
            value: d.earlier,
            title: format!("Before {}", super::reading_stats::FIRST_DECADE).into(),
            detail: books_count_text(d.earlier).into(),
        });
    }
    bars.extend(d.decades.iter().map(|(decade, n)| Bar {
        band: format!("{decade}s").into(),
        value: *n,
        title: format!("{decade}s").into(),
        detail: books_count_text(*n).into(),
    }));
    let most = d
        .decades
        .iter()
        .max_by_key(|(decade, n)| (*n, *decade))
        .filter(|(_, n)| *n >= d.earlier)
        .map(|(decade, _)| format!("Mostly from the {decade}s."));
    let mut line: Vec<AnyElement> = Vec::new();
    let oldest = d.oldest.as_ref().map(|BookFigure { book, value }| {
        note_with_book(
            "stats-oldest",
            format!(
                "{}The oldest:",
                most.clone().map(|m| m + " ").unwrap_or_default()
            ),
            book,
            format!(", {value}."),
            cx,
        )
    });
    match oldest {
        Some(o) => line.push(o),
        None => line.extend(most.map(|m| note(m, cx))),
    }
    Some(
        section("Publication years")
            .children(line)
            .child(chart(c.id("decades"), size::CHART_SHORT, c.bar, bars))
            .when(d.no_year > 0, |this| {
                this.child(foot(
                    match d.no_year {
                        1 => "1 book has no year.".to_owned(),
                        n => format!("{} books have no year.", count_text(n)),
                    },
                    cx,
                ))
            })
            .into_any_element(),
    )
}

// ----- Shelves -----

/// Each shelf as the sidebar draws it: its icon, its name and its count on
/// a trailing lane, tabular, so the counts line up; for a year, what was
/// added in it before the count. They keep the shelf list's width
/// (`size::LIST`), not their column's: six short names whose counts would
/// otherwise sit a column's width away from them.
pub fn shelves(c: &Ctx, cx: &App) -> AnyElement {
    let muted = colors(cx).fg_muted;
    let year = match c.period() {
        Period::Year(year) => Some(year),
        Period::All => None,
    };
    let rows = c.stats.shelves.iter().map(|(status, count, added)| {
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
            .when_some(*added, |this, added| {
                this.child(Lane::new(rems(5.)).child(div().caption(cx).tabular().child(
                    if added > 0 {
                        format!("+{} added", count_text(added))
                    } else {
                        String::new()
                    },
                )))
            })
            .child(
                Lane::trailing().child(div().tabular().text_color(muted).child(count_text(*count))),
            )
    });
    section("Shelves")
        .when_some(year, |this, year| {
            this.child(note(
                format!("Your shelves today, and what you added in {year}."),
                cx,
            ))
        })
        .child(v_flex().max_w(size::LIST).children(rows))
        .into_any_element()
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
    fn length_buckets_are_labelled() {
        let labels: Vec<SharedString> = (0..8).map(bucket_label).collect();
        assert_eq!(
            labels,
            [
                "1–100",
                "101–200",
                "201–300",
                "301–400",
                "401–500",
                "501–600",
                "601–1,000",
                "1,000+"
            ]
            .map(SharedString::from)
        );
    }

    #[test]
    fn durations_read_in_days_months_and_years() {
        assert_eq!(duration_text(0), "the same day");
        assert_eq!(duration_text(1), "1 day");
        assert_eq!(duration_text(42), "42 days");
        assert_eq!(duration_text(61), "2 months");
        assert_eq!(duration_text(400), "13 months");
        assert_eq!(duration_text(1015), "2 years, 9 months");
        assert_eq!(duration_text(731), "2 years");
    }

    #[test]
    fn a_backlog_reads_in_half_years() {
        // 196 books at 30 a year: 6.53 years.
        assert_eq!(
            backlog_text(196, 30.0 / 365.25).as_deref(),
            Some("about 6½ years")
        );
        assert_eq!(
            backlog_text(30, 30.0 / 365.25).as_deref(),
            Some("about a year")
        );
        assert_eq!(
            backlog_text(5, 30.0 / 365.25).as_deref(),
            Some("about 2 months")
        );
        assert_eq!(backlog_text(0, 1.0), None);
        assert_eq!(backlog_text(5, 0.0), None);
    }

    #[test]
    fn a_slow_pace_reads_in_months_or_years() {
        assert_eq!(every_text(75.0), "2 months");
        assert_eq!(every_text(360.0), "year");
        assert_eq!(every_text(931.0), "2½ years");
    }
}
