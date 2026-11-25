//! Reading stats computed from the local library alone: no requests. The
//! genres and moods, which need the books' tags, are `tag_stats`.
//!
//! Everything is about a [`Period`]: all time, or one calendar year.
//!
//! The rules, each pinned by a test below:
//! - **Reads.** Every finished read counts, so a re-read counts again, in
//!   the period of its finish date: the over-time charts, the pages and
//!   the pace count reads. A read with no finish date is not finished, and
//!   one dated after today is a mistake that counts nowhere: every total
//!   agrees with the years. On a Did Not Finish book, the latest read is the
//!   abandoned one and does not count, even if it has a finish date;
//!   earlier finished reads do.
//! - **Books.** What the reader read (formats, lengths, authors, series,
//!   publication years, the genres) counts each book finished in the period
//!   once, as its latest read there.
//! - A read's pages come from its own edition, then the entry's edition,
//!   then the book. An audiobook edition adds no pages; its running time is
//!   time listened.
//! - Days to finish need both dates, finish on or after start. A book
//!   started and finished on the same day took one day.
//! - **Ratings** have no date: all time counts every rating in the library;
//!   a year counts the ratings of the books finished in it.
//! - **Shelves** are today's; a year adds how many books joined each shelf
//!   in it (the date added).
//! - **The backlog** (how long Want to read would take) goes at the past
//!   365 days' pace, whatever the period: an all-time pace is dragged down
//!   by years of old reads.
//! - **Authors** listed are those with at least two books finished.
//! - With fewer than [`FEW`] books in a period, the sections that describe a
//!   mix (pace, formats, lengths, the genres) have too little to say.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use hardcover_core::{
    BookId, BookSummary, Date, EditionSummary, Length, LibraryEntry, ReadSession, ReadingStatus,
    SeriesId,
};

use crate::format::SIDEBAR_SHELVES;

/// The most years the stats keep for the per-year chart, ending with the
/// latest. The chart shows the latest of them that fit its width
/// ([`OverTime::recent_years`]).
pub const MAX_YEARS: i32 = 40;

/// A period needs this many books finished for the sections that describe
/// a mix (pace, formats, lengths, genres and moods) to say anything.
pub const FEW: u32 = 3;

/// How many authors, series and compared books each list shows.
pub const TOP: usize = 5;
/// How many books each side of "against Hardcover" shows.
pub const COMPARED: usize = 3;
/// A book's Hardcover average counts against the reader's rating only when
/// this many readers rated it.
pub const MIN_HARDCOVER_RATINGS: u32 = 20;
/// The least difference, in stars, that puts a book on either side.
pub const MIN_DIFFERENCE: f32 = 0.5;
/// Covers shown beside an author.
pub const AUTHOR_COVERS: usize = 5;
/// The first decade with a bar of its own; earlier years share "Earlier".
pub const FIRST_DECADE: i32 = 1950;
/// A release year before this is taken for another calendar (a Persian
/// book's 1388, say) and counts as no year.
pub const EARLIEST_YEAR: i32 = 1450;

/// The page-count buckets: 1–100 … 601–1,000, 1,000+ (the last bucket's
/// lower bound is exclusive).
pub const LENGTH_BUCKETS: [u32; 7] = [100, 200, 300, 400, 500, 600, 1000];

/// What the stats are about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Period {
    #[default]
    All,
    Year(i32),
}

impl Period {
    pub fn contains(self, day: Date) -> bool {
        match self {
            Self::All => true,
            Self::Year(year) => day.year() == year,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::All => "All time".to_owned(),
            Self::Year(year) => year.to_string(),
        }
    }
}

/// Books and pages finished in one year or month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct YearTotals {
    pub year: i32,
    pub books: u32,
    pub pages: u32,
}

/// Reading over time: per year for all time, per month for a year.
#[derive(Debug, Clone, PartialEq)]
pub enum OverTime {
    /// Consecutive years, oldest first, from the first finish (at most
    /// [`MAX_YEARS`] back) to this year, and the finished reads before the
    /// first (`year` is the first's). Empty without a finished read.
    Years {
        years: Vec<YearTotals>,
        earlier: YearTotals,
    },
    /// January to December; `year` is the period's.
    Months { year: i32, months: [YearTotals; 12] },
}

impl OverTime {
    /// The latest `count` years (all of them when there are fewer), and the
    /// books and pages finished before the first of those (its `year` is
    /// the first's). Empty for a year.
    pub fn recent_years(&self, count: usize) -> (&[YearTotals], YearTotals) {
        match self {
            Self::Years { years, earlier } => {
                let cut = years.len().saturating_sub(count);
                let (dropped, shown) = years.split_at(cut);
                let earlier = YearTotals {
                    year: shown.first().map_or(earlier.year, |y| y.year),
                    books: earlier.books + dropped.iter().map(|y| y.books).sum::<u32>(),
                    pages: earlier.pages + dropped.iter().map(|y| y.pages).sum::<u32>(),
                };
                (shown, earlier)
            }
            Self::Months { year, .. } => (
                &[],
                YearTotals {
                    year: *year,
                    ..Default::default()
                },
            ),
        }
    }
}

/// A book and a figure about it: days, pages, a year.
#[derive(Debug, Clone, PartialEq)]
pub struct BookFigure {
    pub book: BookSummary,
    pub value: u32,
}

/// The pace of a period.
#[derive(Debug, Clone, PartialEq)]
pub struct Pace {
    /// Days the period has run: from the first finished read (all time) or
    /// January 1 (a year) to today, or to December 31 of a past year.
    pub days: u32,
    pub reads: u32,
    pub pages: u32,
    /// Average days from starting a book to finishing it, over the reads
    /// with both dates.
    pub to_finish: Option<f64>,
    pub quickest: Option<BookFigure>,
    pub longest: Option<BookFigure>,
    /// This year only: the books finished by December 31 at this pace.
    pub forecast: Option<u32>,
}

impl Pace {
    /// Days per book finished.
    pub fn days_per_book(&self) -> f64 {
        f64::from(self.days) / f64::from(self.reads.max(1))
    }

    /// Books finished per month (30.44 days).
    pub fn books_per_month(&self) -> f64 {
        f64::from(self.reads) * 30.44 / f64::from(self.days.max(1))
    }

    /// Pages read per day.
    pub fn pages_per_day(&self) -> f64 {
        f64::from(self.pages) / f64::from(self.days.max(1))
    }
}

/// The ratings counted for the period.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ratings {
    pub count: u32,
    pub average: Option<f64>,
    /// Ratings per half star: index 0 is ½, index 9 is 5.
    pub counts: [u32; 10],
}

/// The books finished in the period by the format of their latest read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Formats {
    pub physical: u32,
    pub ebook: u32,
    pub audiobook: u32,
    /// No edition chosen, or one that doesn't say.
    pub unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lengths {
    /// Books per [`LENGTH_BUCKETS`] bucket, then 1,000+.
    pub buckets: [u32; 8],
    /// Average pages over the books with a page count.
    pub average: Option<u32>,
    pub longest: Option<BookFigure>,
    /// Books with no page count: audiobooks, and books with no length.
    pub without: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthorStats {
    pub name: String,
    pub books: u32,
    /// The reader's own ratings of their books, on average.
    pub average: Option<f64>,
    /// Up to [`AUTHOR_COVERS`] of their books, the most recently finished
    /// first.
    pub books_shown: Vec<BookSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesStats {
    pub id: Option<SeriesId>,
    pub name: String,
    pub books: u32,
    pub pages: u32,
    pub average: Option<f64>,
    /// The book of the series with the lowest position: its cover.
    pub lead: BookSummary,
}

/// A book the reader rated well away from Hardcover's average.
#[derive(Debug, Clone, PartialEq)]
pub struct Compared {
    pub book: BookSummary,
    pub yours: f32,
    pub hardcover: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reread {
    pub book: BookSummary,
    /// Finished reads up to the end of the period.
    pub times: u32,
}

/// From adding a book to the library to finishing it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wait {
    /// The books measured: added before their first finish since.
    pub measured: u32,
    pub average_days: f64,
    pub quickest: BookFigure,
    pub longest: BookFigure,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decades {
    /// Books before [`FIRST_DECADE`].
    pub earlier: u32,
    /// Each decade from the first with a book (at least [`FIRST_DECADE`])
    /// to the last, gaps included: (1990, 31) is the 1990s.
    pub decades: Vec<(i32, u32)>,
    /// No release year, or one taken for another calendar.
    pub no_year: u32,
    pub oldest: Option<BookFigure>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadingStats {
    pub period: Period,
    /// Books in the library, Ignored included.
    pub library_books: usize,
    /// Finished reads in the period.
    pub reads: u32,
    /// The books finished in the period, each once, the latest finish first.
    pub books: Vec<BookId>,
    pub pages: u32,
    pub seconds_listened: u64,
    pub over_time: OverTime,
    /// Books on the Read shelf with no finished read, which no year can count.
    pub read_without_date: u32,
    pub ratings: Ratings,
    /// `None` with fewer than [`FEW`] books.
    pub pace: Option<Pace>,
    pub formats: Formats,
    pub lengths: Lengths,
    /// Distinct authors of the books finished.
    pub authors: usize,
    pub top_authors: Vec<AuthorStats>,
    /// The series with at least two books finished, most first.
    pub series: Vec<SeriesStats>,
    pub higher: Vec<Compared>,
    pub lower: Vec<Compared>,
    pub rereads: Vec<Reread>,
    pub wait: Option<Wait>,
    /// Reads finished in the 365 days to today, whatever the period: the
    /// pace a backlog is measured at.
    pub past_year_reads: u32,
    pub decades: Decades,
    /// Books per shelf today, in sidebar order, and for a year how many of
    /// them were added in it.
    pub shelves: Vec<(ReadingStatus, usize, Option<usize>)>,
}

impl ReadingStats {
    pub fn is_empty(&self) -> bool {
        self.library_books == 0
    }

    /// Fewer than [`FEW`] books finished in the period.
    pub fn is_few(&self) -> bool {
        self.distinct_books() < FEW
    }

    pub fn distinct_books(&self) -> u32 {
        u32::try_from(self.books.len()).unwrap_or(u32::MAX)
    }

    pub fn hours_listened(&self) -> u64 {
        (self.seconds_listened + 1800) / 3600
    }

    pub fn shelf(&self, status: ReadingStatus) -> usize {
        self.shelves
            .iter()
            .find(|(s, _, _)| *s == status)
            .map_or(0, |(_, n, _)| *n)
    }
}

/// The years with a finished read, newest first: the period select's years.
pub fn finish_years(entries: &[LibraryEntry], today: Date) -> Vec<i32> {
    let mut years: Vec<i32> = entries
        .iter()
        .flat_map(counted_reads)
        .filter_map(|r| r.finished_at)
        .filter(|d| *d <= today)
        .map(|d| d.year())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    years.sort_unstable_by_key(|y| Reverse(*y));
    years
}

/// One finished read in the period, with its entry.
struct Finish<'a> {
    entry: &'a LibraryEntry,
    read: &'a ReadSession,
    done: Date,
}

pub fn compute(entries: &[LibraryEntry], today: Date, period: Period) -> ReadingStats {
    // Every counted finished read up to today, in any period.
    let all: Vec<Finish> = entries
        .iter()
        .flat_map(|entry| {
            counted_reads(entry).filter_map(move |read| {
                read.finished_at
                    .filter(|d| *d <= today)
                    .map(|done| Finish { entry, read, done })
            })
        })
        .collect();
    let finishes: Vec<&Finish> = all.iter().filter(|f| period.contains(f.done)).collect();

    // Each book once, as its latest finish in the period, the latest first.
    let mut latest: HashMap<BookId, &Finish> = HashMap::new();
    for f in &finishes {
        let slot = latest.entry(f.entry.book.id).or_insert(f);
        if (f.done, f.read.id) > (slot.done, slot.read.id) {
            *slot = f;
        }
    }
    let mut books: Vec<&Finish> = latest.into_values().collect();
    books.sort_by(|a, b| {
        (b.done, b.read.id)
            .cmp(&(a.done, a.read.id))
            .then(a.entry.book.id.get().cmp(&b.entry.book.id.get()))
    });

    let reads = u32::try_from(finishes.len()).unwrap_or(u32::MAX);
    let pages = finishes
        .iter()
        .map(|f| read_pages(f.entry, f.read).unwrap_or(0))
        .sum();
    let seconds_listened = finishes
        .iter()
        .filter_map(|f| read_seconds(f.entry, f.read))
        .map(u64::from)
        .sum();
    let read_without_date = u32::try_from(
        entries
            .iter()
            .filter(|e| e.status == ReadingStatus::Read && counted_reads(e).next().is_none())
            .count(),
    )
    .unwrap_or(u32::MAX);

    let few = books.len() < FEW as usize;
    ReadingStats {
        period,
        library_books: entries.len(),
        reads,
        books: books.iter().map(|f| f.entry.book.id).collect(),
        pages,
        seconds_listened,
        over_time: over_time(&all, today, period),
        read_without_date,
        ratings: ratings(entries, &books, period),
        pace: (!few).then(|| pace(&all, &finishes, today, period, pages)),
        formats: formats(&books),
        lengths: lengths(&books),
        authors: books
            .iter()
            .flat_map(|f| f.entry.book.authors.iter())
            .collect::<HashSet<_>>()
            .len(),
        top_authors: top_authors(&books),
        series: series(&books),
        higher: compared(&books, true),
        lower: compared(&books, false),
        rereads: rereads(&all, &books),
        wait: wait(&all, &books, period),
        past_year_reads: u32::try_from(
            all.iter()
                .filter(|f| days_between(f.done, today) < 365)
                .count(),
        )
        .unwrap_or(u32::MAX),
        decades: decades(&books, today),
        shelves: SIDEBAR_SHELVES
            .iter()
            .map(|&s| {
                let on = entries.iter().filter(|e| e.status == s);
                let added = match period {
                    Period::All => None,
                    Period::Year(_) => Some(
                        on.clone()
                            .filter(|e| e.date_added.is_some_and(|d| period.contains(d)))
                            .count(),
                    ),
                };
                (s, on.count(), added)
            })
            .collect(),
    }
}

fn over_time(all: &[Finish], today: Date, period: Period) -> OverTime {
    match period {
        Period::All => {
            let mut per_year: BTreeMap<i32, YearTotals> = BTreeMap::new();
            for f in all {
                let year = per_year.entry(f.done.year()).or_insert(YearTotals {
                    year: f.done.year(),
                    ..Default::default()
                });
                year.books += 1;
                year.pages += read_pages(f.entry, f.read).unwrap_or(0);
            }
            let (years, earlier) = year_range(&per_year, today.year());
            OverTime::Years { years, earlier }
        }
        Period::Year(year) => {
            let mut months = [YearTotals {
                year,
                ..Default::default()
            }; 12];
            for f in all.iter().filter(|f| f.done.year() == year) {
                let month = &mut months[f.done.month0() as usize];
                month.books += 1;
                month.pages += read_pages(f.entry, f.read).unwrap_or(0);
            }
            OverTime::Months { year, months }
        }
    }
}

fn ratings(entries: &[LibraryEntry], books: &[&Finish], period: Period) -> Ratings {
    let rated: Vec<f32> = match period {
        Period::All => entries
            .iter()
            .filter_map(|e| e.rating)
            .map(|r| r.value())
            .collect(),
        Period::Year(_) => books
            .iter()
            .filter_map(|f| f.entry.rating)
            .map(|r| r.value())
            .collect(),
    };
    let mut counts = [0; 10];
    for value in &rated {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Rating permits exactly ten half-star values; doubling gives the bin number in 1..=10"
        )]
        let half_stars = (value * 2.0).round() as usize;
        counts[half_stars.clamp(1, 10) - 1] += 1;
    }
    let count = u32::try_from(rated.len()).unwrap_or(u32::MAX);
    Ratings {
        count,
        average: (count > 0)
            .then(|| rated.iter().map(|v| f64::from(*v)).sum::<f64>() / f64::from(count)),
        counts,
    }
}

fn days_between(from: Date, to: Date) -> i32 {
    to.num_days_from_ce() - from.num_days_from_ce()
}

fn pace(all: &[Finish], finishes: &[&Finish], today: Date, period: Period, pages: u32) -> Pace {
    let (from, to) = match period {
        Period::All => {
            let first = all
                .iter()
                .map(|f| f.read.started_at.filter(|s| *s <= f.done).unwrap_or(f.done))
                .min()
                .unwrap_or(today);
            (first, today)
        }
        Period::Year(year) => {
            let jan1 = NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(today);
            let dec31 = NaiveDate::from_ymd_opt(year, 12, 31).unwrap_or(today);
            (jan1, dec31.min(today))
        }
    };
    let days = u32::try_from(days_between(from, to) + 1)
        .unwrap_or(1)
        .max(1);
    let reads = u32::try_from(finishes.len()).unwrap_or(u32::MAX);

    let mut timed: Vec<(&Finish, u32)> = finishes
        .iter()
        .filter_map(|f| {
            let started = f.read.started_at?;
            let took = days_between(started, f.done);
            (took >= 0).then(|| (*f, u32::try_from(took + 1).unwrap_or(1)))
        })
        .collect();
    let to_finish = (!timed.is_empty())
        .then(|| timed.iter().map(|(_, d)| f64::from(*d)).sum::<f64>() / timed.len() as f64);
    // Ties go to the latest finish.
    timed.sort_by_key(|(f, d)| (*d, Reverse(f.done), Reverse(f.read.id)));
    let figure = |(f, d): &(&Finish, u32)| BookFigure {
        book: f.entry.book.clone(),
        value: *d,
    };
    let quickest = timed.first().map(figure);
    let longest = timed
        .iter()
        .max_by_key(|(f, d)| (*d, f.done, f.read.id))
        .map(figure);
    let forecast = match period {
        Period::Year(year) if year == today.year() => {
            let dec31 = NaiveDate::from_ymd_opt(year, 12, 31).unwrap_or(today);
            let in_year = f64::from(dec31.ordinal());
            (today < dec31).then(|| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a small non-negative count of books"
                )]
                let books = (f64::from(reads) * in_year / f64::from(days)).round() as u32;
                books
            })
        }
        _ => None,
    };
    Pace {
        days,
        reads,
        pages,
        to_finish,
        quickest,
        longest,
        forecast,
    }
}

fn formats(books: &[&Finish]) -> Formats {
    let mut formats = Formats::default();
    for f in books {
        let edition = f.read.edition.as_ref().or(f.entry.edition.as_ref());
        match edition {
            Some(e) if e.is_audiobook() => formats.audiobook += 1,
            Some(e) => match e.reading_format_id {
                Some(1) => formats.physical += 1,
                Some(4) => formats.ebook += 1,
                _ => formats.unknown += 1,
            },
            None => formats.unknown += 1,
        }
    }
    formats
}

fn lengths(books: &[&Finish]) -> Lengths {
    let mut lengths = Lengths::default();
    let mut total = 0u64;
    let mut counted = 0u64;
    let mut longest: Option<(&Finish, u32)> = None;
    for f in books {
        let Some(pages) = read_pages(f.entry, f.read) else {
            lengths.without += 1;
            continue;
        };
        let bucket = LENGTH_BUCKETS
            .iter()
            .position(|top| pages <= *top)
            .unwrap_or(LENGTH_BUCKETS.len());
        lengths.buckets[bucket] += 1;
        total += u64::from(pages);
        counted += 1;
        if longest.is_none_or(|(_, p)| pages > p) {
            longest = Some((f, pages));
        }
    }
    lengths.average =
        (counted > 0).then(|| u32::try_from((total + counted / 2) / counted).unwrap_or(u32::MAX));
    lengths.longest = longest.map(|(f, pages)| BookFigure {
        book: f.entry.book.clone(),
        value: pages,
    });
    lengths
}

fn average(ratings: impl Iterator<Item = f32>) -> Option<f64> {
    let (sum, n) = ratings.fold((0.0, 0u32), |(s, n), r| (s + f64::from(r), n + 1));
    (n > 0).then(|| sum / f64::from(n))
}

/// The authors with the most books finished, at least two each; ties by
/// the reader's average rating, then by name.
fn top_authors(books: &[&Finish]) -> Vec<AuthorStats> {
    let mut by_author: HashMap<&str, Vec<&Finish>> = HashMap::new();
    for f in books {
        for author in &f.entry.book.authors {
            by_author.entry(author.as_str()).or_default().push(f);
        }
    }
    let mut authors: Vec<AuthorStats> = by_author
        .into_iter()
        .map(|(name, books)| AuthorStats {
            name: name.to_owned(),
            books: u32::try_from(books.len()).unwrap_or(u32::MAX),
            average: average(
                books
                    .iter()
                    .filter_map(|f| f.entry.rating)
                    .map(|r| r.value()),
            ),
            books_shown: books
                .iter()
                .take(AUTHOR_COVERS)
                .map(|f| f.entry.book.clone())
                .collect(),
        })
        .filter(|a| a.books >= 2)
        .collect();
    authors.sort_by(|a, b| {
        b.books
            .cmp(&a.books)
            .then(
                b.average
                    .unwrap_or(0.0)
                    .total_cmp(&a.average.unwrap_or(0.0)),
            )
            .then(a.name.cmp(&b.name))
    });
    authors.truncate(TOP);
    authors
}

/// The series with at least two books finished, the most books first, then
/// the most pages, then by name. A compilation counts for none.
fn series(books: &[&Finish]) -> Vec<SeriesStats> {
    type Books<'a> = Vec<(&'a Finish<'a>, Option<f32>)>;
    let mut by_series: HashMap<(Option<SeriesId>, &str), Books> = HashMap::new();
    for f in books {
        for s in f.entry.series.iter().filter(|s| s.counts_in_series()) {
            by_series
                .entry((s.id, s.name.as_str()))
                .or_default()
                .push((f, s.position));
        }
    }
    let mut series: Vec<SeriesStats> = by_series
        .into_iter()
        .filter(|(_, books)| books.len() >= 2)
        .map(|((id, name), books)| {
            let lead = books
                .iter()
                .min_by(|(_, a), (_, b)| a.unwrap_or(f32::MAX).total_cmp(&b.unwrap_or(f32::MAX)))
                .map(|(f, _)| f.entry.book.clone())
                .unwrap_or_else(|| books[0].0.entry.book.clone());
            SeriesStats {
                id,
                name: name.to_owned(),
                books: u32::try_from(books.len()).unwrap_or(u32::MAX),
                pages: books
                    .iter()
                    .map(|(f, _)| read_pages(f.entry, f.read).unwrap_or(0))
                    .sum(),
                average: average(
                    books
                        .iter()
                        .filter_map(|(f, _)| f.entry.rating)
                        .map(|r| r.value()),
                ),
                lead,
            }
        })
        .collect();
    series.sort_by(|a, b| {
        b.books
            .cmp(&a.books)
            .then(b.pages.cmp(&a.pages))
            .then(a.name.cmp(&b.name))
    });
    series.truncate(TOP);
    series
}

/// The books the reader rated at least [`MIN_DIFFERENCE`] above (`higher`)
/// or below Hardcover's average, among those at least
/// [`MIN_HARDCOVER_RATINGS`] readers rated; the widest gap first.
fn compared(books: &[&Finish], higher: bool) -> Vec<Compared> {
    let mut found: Vec<Compared> = books
        .iter()
        .filter_map(|f| {
            let yours = f.entry.rating?.value();
            let book = &f.entry.book;
            let hardcover = book.average_rating.filter(|r| *r > 0.0)?;
            if book.ratings_count.unwrap_or(0) < MIN_HARDCOVER_RATINGS {
                return None;
            }
            let gap = yours - hardcover;
            let side = if higher { gap } else { -gap };
            (side >= MIN_DIFFERENCE).then(|| Compared {
                book: book.clone(),
                yours,
                hardcover,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        let gap = |c: &Compared| (c.yours - c.hardcover).abs();
        gap(b).total_cmp(&gap(a))
    });
    found.truncate(COMPARED);
    found
}

/// The books finished in the period that were finished before too; how many
/// times, up to the period's end. The most times first, then the latest.
fn rereads(all: &[Finish], books: &[&Finish]) -> Vec<Reread> {
    let mut found: Vec<Reread> = books
        .iter()
        .filter_map(|f| {
            let times = all
                .iter()
                .filter(|o| o.entry.book.id == f.entry.book.id && o.done <= f.done)
                .count();
            (times >= 2).then(|| Reread {
                book: f.entry.book.clone(),
                times: u32::try_from(times).unwrap_or(u32::MAX),
            })
        })
        .collect();
    // `books` is the latest first already: a stable sort keeps that.
    found.sort_by_key(|r| Reverse(r.times));
    found
}

/// From the day each book finished in the period joined the library to its
/// first finish since, when that finish is in the period. A book added only
/// once it had been read doesn't count.
fn wait(all: &[Finish], books: &[&Finish], period: Period) -> Option<Wait> {
    let mut waits: Vec<(&Finish, u32)> = books
        .iter()
        .filter_map(|f| {
            let added = f.entry.date_added?;
            let first = all
                .iter()
                .filter(|o| o.entry.book.id == f.entry.book.id && o.done >= added)
                .min_by_key(|o| (o.done, o.read.id))?;
            period.contains(first.done).then(|| {
                (
                    first,
                    u32::try_from(days_between(added, first.done)).unwrap_or(0),
                )
            })
        })
        .collect();
    if waits.is_empty() {
        return None;
    }
    let measured = u32::try_from(waits.len()).unwrap_or(u32::MAX);
    let average_days = waits.iter().map(|(_, d)| f64::from(*d)).sum::<f64>() / f64::from(measured);
    waits.sort_by_key(|(f, d)| (*d, Reverse(f.done)));
    let figure = |(f, d): &(&Finish, u32)| BookFigure {
        book: f.entry.book.clone(),
        value: *d,
    };
    let quickest = figure(&waits[0]);
    let longest = waits
        .iter()
        .max_by_key(|(f, d)| (*d, f.done))
        .map(figure)
        .unwrap_or_else(|| quickest.clone());
    Some(Wait {
        measured,
        average_days,
        quickest,
        longest,
    })
}

fn decades(books: &[&Finish], today: Date) -> Decades {
    let mut decades = Decades::default();
    let mut per: BTreeMap<i32, u32> = BTreeMap::new();
    let mut oldest: Option<(&Finish, i32)> = None;
    for f in books {
        let Some(year) = f
            .entry
            .book
            .release_year
            .filter(|y| (EARLIEST_YEAR..=today.year() + 1).contains(y))
        else {
            decades.no_year += 1;
            continue;
        };
        if year < FIRST_DECADE {
            decades.earlier += 1;
        } else {
            *per.entry(year - year.rem_euclid(10)).or_default() += 1;
        }
        if oldest.is_none_or(|(_, y)| year < y) {
            oldest = Some((f, year));
        }
    }
    if let (Some(first), Some(last)) = (per.keys().next().copied(), per.keys().last().copied()) {
        decades.decades = (first..=last)
            .step_by(10)
            .map(|d| (d, per.get(&d).copied().unwrap_or(0)))
            .collect();
    }
    decades.oldest = oldest.map(|(f, year)| BookFigure {
        book: f.entry.book.clone(),
        value: u32::try_from(year).unwrap_or(0),
    });
    decades
}

/// The entry's finished reads that count: all of them, except the latest read
/// of a book the reader did not finish. An open read of such a book is the
/// abandoned one, and isn't finished anyway.
fn counted_reads(entry: &LibraryEntry) -> impl Iterator<Item = &ReadSession> {
    let abandoned = (entry.status == ReadingStatus::DidNotFinish
        && entry.reads.iter().all(ReadSession::is_finished))
    .then(|| {
        entry
            .reads
            .iter()
            .max_by(|a, b| a.date().cmp(&b.date()).then(a.id.cmp(&b.id)))
            .map(|r| r.id)
    })
    .flatten();
    entry
        .reads
        .iter()
        .filter(move |r| r.is_finished() && Some(r.id) != abandoned)
}

/// The read's length, from the first of its edition, the entry's edition
/// and the book that has one.
fn read_length(entry: &LibraryEntry, read: &ReadSession) -> Option<Length> {
    read.edition
        .as_ref()
        .and_then(EditionSummary::length)
        .or_else(|| entry.edition.as_ref().and_then(EditionSummary::length))
        .or_else(|| entry.book.pages.filter(|p| *p > 0).map(Length::Pages))
}

/// The read's page count. `None` for an audiobook or no length.
fn read_pages(entry: &LibraryEntry, read: &ReadSession) -> Option<u32> {
    match read_length(entry, read) {
        Some(Length::Pages(pages)) => Some(pages),
        _ => None,
    }
}

/// An audiobook read's running time.
fn read_seconds(entry: &LibraryEntry, read: &ReadSession) -> Option<u32> {
    match read_length(entry, read) {
        Some(Length::Seconds(seconds)) => Some(seconds),
        _ => None,
    }
}

/// Consecutive years ending this year, at most [`MAX_YEARS`] of them, and the
/// books and pages finished before the first one. `per_year` holds no later
/// year.
fn year_range(
    per_year: &BTreeMap<i32, YearTotals>,
    this_year: i32,
) -> (Vec<YearTotals>, YearTotals) {
    let Some(&first) = per_year.keys().next() else {
        return (
            Vec::new(),
            YearTotals {
                year: this_year,
                ..Default::default()
            },
        );
    };
    let last = this_year;
    let first = first.max(last - MAX_YEARS + 1);
    let years = (first..=last)
        .map(|year| {
            per_year.get(&year).copied().unwrap_or(YearTotals {
                year,
                ..Default::default()
            })
        })
        .collect();
    let earlier = per_year.range(..first).fold(
        YearTotals {
            year: first,
            ..Default::default()
        },
        |acc, (_, y)| YearTotals {
            books: acc.books + y.books,
            pages: acc.pages + y.pages,
            ..acc
        },
    );
    (years, earlier)
}

#[cfg(test)]
mod tests;
