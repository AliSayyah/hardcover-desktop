//! Reading stats computed from the local library alone: no requests.
//!
//! The rules, each pinned by a test below:
//! - Every finished read counts, so a re-read counts again, in the year of its
//!   finish date. A read with no finish date is not finished, and one dated
//!   in a later year is a mistake that counts nowhere: every total agrees
//!   with the years.
//! - On a Did Not Finish book, the latest read is the abandoned one and does
//!   not count, even if it has a finish date; earlier finished reads do.
//! - A read's pages come from its own edition, then the entry's edition, then
//!   the book. An audiobook edition adds no pages.
//! - Days to finish need both dates, finish on or after start. A book started
//!   and finished on the same day took one day.

use std::collections::BTreeMap;

use chrono::Datelike;
use hardcover_core::{Date, EditionSummary, Length, LibraryEntry, ReadSession, ReadingStatus};

use crate::format::SIDEBAR_SHELVES;

/// The most years the stats keep for the per-year chart, ending with the
/// latest. The chart shows the latest of them that fit its width
/// ([`ReadingStats::recent_years`]).
pub const MAX_YEARS: i32 = 40;

/// Books and pages finished in one year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct YearTotals {
    pub year: i32,
    pub books: u32,
    pub pages: u32,
}

impl ReadingStats {
    /// The latest `count` of [`Self::years`] (all of them when there are
    /// fewer), and the books finished before the first of those.
    pub fn recent_years(&self, count: usize) -> (&[YearTotals], u32) {
        let cut = self.years.len().saturating_sub(count);
        let (dropped, shown) = self.years.split_at(cut);
        let earlier = self.earlier_books + dropped.iter().map(|y| y.books).sum::<u32>();
        (shown, earlier)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadingStats {
    /// Books in the library, Ignored included.
    pub books: usize,
    /// Consecutive years, oldest first, from the first finish (at most
    /// [`MAX_YEARS`] back) to this year. Empty without a finished read.
    pub years: Vec<YearTotals>,
    /// Finished reads before the first year in `years`.
    pub earlier_books: u32,
    pub this_year: YearTotals,
    pub finished_reads: u32,
    /// Books on the Read shelf with no finished read, which no year can count.
    pub read_without_date: u32,
    /// Ratings given, one per book.
    pub ratings: u32,
    pub average_rating: Option<f64>,
    /// Ratings per half star: index 0 is ½, index 9 is 5.
    pub rating_counts: [u32; 10],
    /// Books per shelf, in sidebar order.
    pub shelves: Vec<(ReadingStatus, usize)>,
    /// Reads with both dates, which the average below covers.
    pub timed_reads: u32,
    pub average_days: Option<f64>,
}

impl ReadingStats {
    pub fn is_empty(&self) -> bool {
        self.books == 0
    }
}

pub fn compute(entries: &[LibraryEntry], today: Date) -> ReadingStats {
    let mut per_year: BTreeMap<i32, YearTotals> = BTreeMap::new();
    let mut finished_reads = 0;
    let mut read_without_date = 0;
    let mut days_total = 0.0;
    let mut timed_reads = 0;
    for entry in entries {
        if entry.status == ReadingStatus::Read && counted_reads(entry).next().is_none() {
            read_without_date += 1;
        }
        let finished = counted_reads(entry)
            .filter(|r| r.finished_at.is_some_and(|d| d.year() <= today.year()));
        for read in finished {
            let Some(done) = read.finished_at else {
                continue;
            };
            finished_reads += 1;
            let year = per_year.entry(done.year()).or_insert(YearTotals {
                year: done.year(),
                ..Default::default()
            });
            year.books += 1;
            year.pages += read_pages(entry, read).unwrap_or(0);
            if let Some(started) = read.started_at
                && done >= started
            {
                days_total += f64::from(done.num_days_from_ce() - started.num_days_from_ce() + 1);
                timed_reads += 1;
            }
        }
    }

    let this_year = per_year.get(&today.year()).copied().unwrap_or(YearTotals {
        year: today.year(),
        ..Default::default()
    });
    let (years, earlier_books) = year_range(&per_year, today.year());

    let mut rating_counts = [0; 10];
    let mut rating_sum = 0.0;
    let mut ratings = 0;
    for rating in entries.iter().filter_map(|e| e.rating) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Rating permits exactly ten half-star values; doubling gives the bin number in 1..=10"
        )]
        let half_stars = (rating.value() * 2.0).round() as usize;
        rating_counts[half_stars.clamp(1, 10) - 1] += 1;
        rating_sum += f64::from(rating.value());
        ratings += 1;
    }

    ReadingStats {
        books: entries.len(),
        years,
        earlier_books,
        this_year,
        finished_reads,
        read_without_date,
        ratings,
        average_rating: (ratings > 0).then(|| rating_sum / f64::from(ratings)),
        rating_counts,
        shelves: SIDEBAR_SHELVES
            .iter()
            .map(|&s| (s, entries.iter().filter(|e| e.status == s).count()))
            .collect(),
        timed_reads,
        average_days: (timed_reads > 0).then(|| days_total / f64::from(timed_reads)),
    }
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

/// The read's page count, from the first of its edition, the entry's edition
/// and the book that has a length. `None` for an audiobook or no length.
fn read_pages(entry: &LibraryEntry, read: &ReadSession) -> Option<u32> {
    let length = read
        .edition
        .as_ref()
        .and_then(EditionSummary::length)
        .or_else(|| entry.edition.as_ref().and_then(EditionSummary::length))
        .or_else(|| entry.book.pages.filter(|p| *p > 0).map(Length::Pages));
    match length {
        Some(Length::Pages(pages)) => Some(pages),
        _ => None,
    }
}

/// Consecutive years ending this year, at most [`MAX_YEARS`] of them, and the
/// books finished before the first one. `per_year` holds no later year.
fn year_range(per_year: &BTreeMap<i32, YearTotals>, this_year: i32) -> (Vec<YearTotals>, u32) {
    let Some(&first) = per_year.keys().next() else {
        return (Vec::new(), 0);
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
    let earlier = per_year.range(..first).map(|(_, y)| y.books).sum();
    (years, earlier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{BookId, BookSummary, EditionId, Rating, ReadId, UserBookId, UserId};

    fn date(s: &str) -> Date {
        Date::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    const TODAY: &str = "2026-11-19";

    fn entry(id: i64, status: ReadingStatus, pages: Option<u32>) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(100 + id), format!("Book {id}"));
        book.pages = pages;
        LibraryEntry::new(UserBookId::new(id), UserId::new(1), book, status)
    }

    fn read(id: i64, started: Option<&str>, finished: Option<&str>) -> ReadSession {
        let mut r = ReadSession::new(ReadId::new(id));
        r.started_at = started.map(date);
        r.finished_at = finished.map(date);
        r
    }

    fn edition(pages: Option<u32>, seconds: Option<u32>) -> EditionSummary {
        let mut e = EditionSummary::new(EditionId::new(7));
        e.pages = pages;
        e.audio_seconds = seconds;
        e
    }

    #[test]
    fn an_empty_library_has_no_stats() {
        let stats = compute(&[], date(TODAY));
        assert!(stats.is_empty());
        assert_eq!(stats.years, []);
        assert_eq!(
            stats.this_year,
            YearTotals {
                year: 2026,
                books: 0,
                pages: 0
            }
        );
        assert_eq!(stats.average_rating, None);
        assert_eq!(stats.average_days, None);
        assert!(stats.shelves.iter().all(|(_, n)| *n == 0));
    }

    #[test]
    fn rereads_count_in_the_year_they_finished() {
        let mut dune = entry(1, ReadingStatus::Read, Some(600));
        dune.reads = vec![
            read(10, Some("2026-01-01"), Some("2026-01-10")),
            read(11, Some("2024-03-01"), Some("2024-03-03")),
            read(12, Some("2026-05-01"), None),
        ];
        let stats = compute(&[dune], date(TODAY));
        assert_eq!(stats.finished_reads, 2);
        assert_eq!(
            stats.this_year,
            YearTotals {
                year: 2026,
                books: 1,
                pages: 600
            }
        );
        let years: Vec<(i32, u32)> = stats.years.iter().map(|y| (y.year, y.books)).collect();
        assert_eq!(years, vec![(2024, 1), (2025, 0), (2026, 1)], "gaps filled");
    }

    #[test]
    fn an_open_read_is_the_abandoned_one_for_a_dnf_book() {
        let mut e = entry(1, ReadingStatus::DidNotFinish, Some(100));
        e.reads = vec![
            read(2, None, None),
            read(1, Some("2020-01-01"), Some("2020-01-20")),
        ];
        let stats = compute(&[e], date(TODAY));
        assert_eq!(stats.finished_reads, 1);
    }

    #[test]
    fn a_finish_dated_in_a_later_year_does_not_move_the_chart() {
        let mut e = entry(1, ReadingStatus::Read, None);
        e.reads = vec![
            read(1, None, Some("2025-03-01")),
            read(2, None, Some("2062-03-01")),
        ];
        let stats = compute(&[e], date(TODAY));
        assert_eq!(stats.years.last().map(|y| y.year), Some(2026));
        assert_eq!(
            stats.years.first().map(|y| (y.year, y.books)),
            Some((2025, 1))
        );
        // Nor any total: they all agree with the years.
        assert_eq!(stats.finished_reads, 1);
        assert_eq!(stats.timed_reads, 0);
        assert_eq!(stats.read_without_date, 0);

        // A Read book finished only in a later year counts nowhere, and it
        // has a finish date, so it isn't one without.
        let mut later = entry(2, ReadingStatus::Read, Some(300));
        later.reads = vec![read(3, Some("2062-02-01"), Some("2062-03-01"))];
        let stats = compute(&[later], date(TODAY));
        assert_eq!(stats.finished_reads, 0);
        assert_eq!(stats.timed_reads, 0);
        assert_eq!(stats.read_without_date, 0);
        assert!(stats.years.is_empty());
    }

    #[test]
    fn a_finished_reread_on_another_shelf_still_counts() {
        let mut e = entry(1, ReadingStatus::CurrentlyReading, Some(100));
        e.reads = vec![
            read(2, Some("2026-10-01"), None),
            read(1, None, Some("2025-02-01")),
        ];
        let stats = compute(&[e], date(TODAY));
        assert_eq!(stats.finished_reads, 1);
        assert_eq!(stats.read_without_date, 0);
    }

    #[test]
    fn pages_come_from_the_read_then_the_entry_then_the_book() {
        let mut a = entry(1, ReadingStatus::Read, Some(300));
        let mut from_read = read(1, None, Some("2026-02-01"));
        from_read.edition = Some(edition(Some(250), None));
        a.edition = Some(edition(Some(400), None));
        a.reads = vec![from_read, read(2, None, Some("2026-03-01"))];

        let mut b = entry(2, ReadingStatus::Read, Some(320));
        b.reads = vec![read(3, None, Some("2026-04-01"))];

        // An audiobook read adds no pages, even though the book has some.
        let mut c = entry(3, ReadingStatus::Read, Some(500));
        let mut listened = read(4, None, Some("2026-05-01"));
        listened.edition = Some(edition(None, Some(36_000)));
        c.reads = vec![listened];

        let stats = compute(&[a, b, c], date(TODAY));
        assert_eq!(stats.this_year.books, 4);
        assert_eq!(stats.this_year.pages, 250 + 400 + 320);
    }

    #[test]
    fn days_to_finish_needs_both_dates_in_order() {
        let mut e = entry(1, ReadingStatus::Read, None);
        e.reads = vec![
            // 10 days, counting the first and the last.
            read(1, Some("2026-01-01"), Some("2026-01-10")),
            // The same day: one day.
            read(2, Some("2026-02-01"), Some("2026-02-01")),
            // Counted for its year, not for the average.
            read(3, None, Some("2001-06-01")),
            read(4, Some("2026-03-05"), Some("2026-03-01")),
        ];
        let stats = compute(&[e], date(TODAY));
        assert_eq!(stats.finished_reads, 4);
        assert_eq!(stats.timed_reads, 2);
        assert_eq!(stats.average_days, Some(5.5));
    }

    #[test]
    fn the_abandoned_read_of_a_did_not_finish_book_does_not_count() {
        let mut e = entry(1, ReadingStatus::DidNotFinish, Some(200));
        e.reads = vec![
            read(1, Some("2020-01-01"), Some("2020-01-20")),
            // Hardcover may date the day the reader gave up.
            read(2, Some("2026-04-01"), Some("2026-04-25")),
        ];
        let stats = compute(&[e], date(TODAY));
        assert_eq!(stats.finished_reads, 1);
        assert_eq!(stats.this_year.books, 0);
    }

    #[test]
    fn read_books_without_a_finish_date_are_reported() {
        let mut dated = entry(1, ReadingStatus::Read, None);
        dated.reads = vec![read(1, None, Some("2026-01-01"))];
        let mut undated = entry(2, ReadingStatus::Read, None);
        undated.reads = vec![read(2, Some("2026-01-01"), None)];
        let bare = entry(3, ReadingStatus::Read, None);
        let stats = compute(&[dated, undated, bare], date(TODAY));
        assert_eq!(stats.read_without_date, 2);
        assert_eq!(stats.finished_reads, 1);
        assert_eq!(stats.shelves[2], (ReadingStatus::Read, 3));
    }

    #[test]
    fn the_chart_keeps_the_latest_years() {
        let mut e = entry(1, ReadingStatus::Read, None);
        e.reads = vec![
            read(1, None, Some("2001-06-01")),
            read(2, None, Some("2012-12-20")),
            read(3, None, Some("2019-02-10")),
            read(4, None, Some("2026-10-12")),
        ];
        let stats = compute(&[e], date(TODAY));
        // Every year from the first finish is kept, 2001 to 2026.
        assert_eq!(stats.years.len(), 26);
        assert_eq!(stats.earlier_books, 0);
        // A chart with room for 12 shows 2015 to 2026, and counts the rest.
        let (years, earlier) = stats.recent_years(12);
        assert_eq!(years.len(), 12);
        assert_eq!(years.first().unwrap().year, 2015);
        assert_eq!(years.last().unwrap().year, 2026);
        assert_eq!(earlier, 2);
        let charted: u32 = years.iter().map(|y| y.books).sum();
        assert_eq!(charted + earlier, stats.finished_reads);
        // A wider one reaches back to 2010; one wider than the data, all.
        let (years, earlier) = stats.recent_years(17);
        assert_eq!((years.first().unwrap().year, earlier), (2010, 1));
        let (years, earlier) = stats.recent_years(40);
        assert_eq!((years.len(), earlier), (26, 0));
    }

    #[test]
    fn the_years_kept_stop_at_the_maximum() {
        let mut e = entry(1, ReadingStatus::Read, None);
        e.reads = vec![
            read(1, None, Some("1970-06-01")),
            read(2, None, Some("2026-10-12")),
        ];
        let stats = compute(&[e], date(TODAY));
        assert_eq!(stats.years.len(), usize::try_from(MAX_YEARS).unwrap());
        assert_eq!(stats.years.first().unwrap().year, 1987);
        assert_eq!(stats.earlier_books, 1);
        assert_eq!(stats.recent_years(100).1, 1);
    }

    #[test]
    fn the_range_reaches_this_year_without_a_finish_in_it() {
        let mut e = entry(1, ReadingStatus::Read, None);
        e.reads = vec![read(1, None, Some("2024-06-01"))];
        let stats = compute(&[e], date(TODAY));
        let years: Vec<i32> = stats.years.iter().map(|y| y.year).collect();
        assert_eq!(years, vec![2024, 2025, 2026]);
    }

    #[test]
    fn ratings_average_and_spread_by_half_star() {
        let rated = |id, value| {
            let mut e = entry(id, ReadingStatus::Read, None);
            e.rating = Rating::new(value);
            e
        };
        let stats = compute(
            &[
                rated(1, 5.0),
                rated(2, 4.5),
                rated(3, 4.5),
                rated(4, 0.5),
                entry(5, ReadingStatus::WantToRead, None),
            ],
            date(TODAY),
        );
        assert_eq!(stats.ratings, 4);
        assert_eq!(stats.average_rating, Some(3.625));
        assert_eq!(stats.rating_counts, [1, 0, 0, 0, 0, 0, 0, 0, 2, 1]);
        assert_eq!(stats.shelves[1], (ReadingStatus::WantToRead, 1));
        assert_eq!(stats.books, 5);
    }
}
