//! What Home shows, worked out from the library on this computer: the books
//! being read with their pace, a paused book, books finished lately with no
//! rating, the next book of a series the reader has started, and the top of
//! the Want to Read shelf. Plain functions
//! of the entries and the date, so they can be tested; nothing here asks
//! Hardcover for anything.

use std::collections::HashSet;

use chrono::Duration;
use hardcover_core::{
    BookId, Date, Goal, GoalMetric, Length, LibraryEntry, ReadingStatus, SeriesId, UserBookId,
};

/// A read younger than this, in days, has no pace yet: one evening's reading
/// would predict nonsense.
const MIN_PACE_DAYS: i64 = 3;
/// A finish further away than this is not guessed at.
const MAX_ESTIMATE_DAYS: i64 = 365;
/// A book being read but not updated for this many days says so.
pub const IDLE_DAYS: i64 = 7;
/// Want to Read books listed under Up next, besides the series pick.
const UP_NEXT_ROWS: usize = 3;
/// Read books finished at most this many days ago are offered for rating.
/// Older ones were likely left unrated on purpose.
pub const RATE_DAYS: i64 = 60;
/// Rows of "Rate what you finished".
const RATE_ROWS: usize = 4;

#[derive(Debug, Default)]
pub struct Digest {
    /// Currently Reading, the most recently updated first.
    pub reading: Vec<Reading>,
    /// The most recently updated Paused book.
    pub paused: Option<UserBookId>,
    pub paused_count: usize,
    pub series_next: Option<SeriesNext>,
    /// The most recently added Want to Read books, without the series pick.
    pub up_next: Vec<UserBookId>,
    pub want_to_read: usize,
}

impl Digest {
    pub fn is_empty(&self) -> bool {
        self.reading.is_empty() && self.paused.is_none() && self.want_to_read == 0
    }
}

/// A book being read.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub id: UserBookId,
    pub pace: Option<Pace>,
    /// Days since the entry last changed, once that is at least [`IDLE_DAYS`].
    pub idle_days: Option<i64>,
}

/// How fast a book is being read, from its start date and the page reached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    pub pages_per_day: f64,
    /// When the last page comes at this pace; `None` past [`MAX_ESTIMATE_DAYS`]
    /// or once the last page is reached.
    pub finish: Option<Date>,
}

/// A Want to Read book that comes right after one the reader finished.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesNext {
    pub id: UserBookId,
    pub series: String,
    pub position: f32,
    /// The finished book it follows.
    pub after: UserBookId,
    pub after_position: f32,
}

/// `unreleased` holds the books not out yet (see [`is_released`]): "Up next"
/// suggests only books the reader can start.
pub fn compute(entries: &[LibraryEntry], today: Date, unreleased: &HashSet<BookId>) -> Digest {
    let shelf = |status: ReadingStatus| entries.iter().filter(move |e| e.status == status);
    let released = |e: &LibraryEntry| is_released(e, today, unreleased);

    let mut reading: Vec<&LibraryEntry> = shelf(ReadingStatus::CurrentlyReading).collect();
    reading.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then(b.user_book_id.cmp(&a.user_book_id))
    });
    let reading = reading
        .into_iter()
        .map(|e| Reading {
            id: e.user_book_id,
            pace: pace(e, today),
            idle_days: idle_days(e, today),
        })
        .collect();

    let paused = shelf(ReadingStatus::Paused)
        .max_by(|a, b| {
            a.updated_at
                .cmp(&b.updated_at)
                .then(a.user_book_id.cmp(&b.user_book_id))
        })
        .map(|e| e.user_book_id);

    let series_next = series_next(entries, &released);
    let mut want: Vec<&LibraryEntry> = shelf(ReadingStatus::WantToRead).collect();
    want.sort_by(|a, b| {
        b.date_added
            .cmp(&a.date_added)
            .then(b.user_book_id.cmp(&a.user_book_id))
    });
    let up_next = want
        .iter()
        .filter(|e| series_next.as_ref().is_none_or(|n| n.id != e.user_book_id))
        .filter(|e| released(e))
        .take(UP_NEXT_ROWS)
        .map(|e| e.user_book_id)
        .collect();

    Digest {
        reading,
        paused,
        paused_count: shelf(ReadingStatus::Paused).count(),
        series_next,
        up_next,
        want_to_read: want.len(),
    }
}

/// Whether the entry's book is out: not among the upcoming books (`unreleased`,
/// which know the day), and not dated to a later year. The library knows only
/// the year, which also covers the time before the upcoming books load.
pub fn is_released(entry: &LibraryEntry, today: Date, unreleased: &HashSet<BookId>) -> bool {
    use chrono::Datelike as _;
    !unreleased.contains(&entry.book.id)
        && entry.book.release_year.is_none_or(|y| y <= today.year())
}

/// Pages per day since the open read started, and the finish that implies.
/// Only for books being read (Currently Reading), counted in pages, with a
/// start date and a page reached. A paused read has no pace: the days it
/// sits on the Paused shelf would drag the figure down, and the finish date
/// would promise what nobody is doing.
pub fn pace(entry: &LibraryEntry, today: Date) -> Option<Pace> {
    if entry.status != ReadingStatus::CurrentlyReading {
        return None;
    }
    let read = entry.open_read()?;
    let Some(Length::Pages(total)) = entry.length() else {
        return None;
    };
    let page = read.progress_pages.filter(|p| *p > 0)?;
    let started = read.started_at.filter(|s| *s <= today)?;
    // The start day counts: a book started yesterday has been read two days.
    let days = (today - started).num_days() + 1;
    if days < MIN_PACE_DAYS {
        return None;
    }
    let pages_per_day = f64::from(page) / days as f64;
    let finish = (page < total)
        .then(|| (f64::from(total - page) / pages_per_day).ceil() as i64)
        .filter(|d| *d <= MAX_ESTIMATE_DAYS)
        .map(|d| today + Duration::days(d));
    Some(Pace {
        pages_per_day,
        finish,
    })
}

/// Days since the entry last changed, if that is at least [`IDLE_DAYS`].
/// A progress update changes the entry, and so does any other edit.
fn idle_days(entry: &LibraryEntry, today: Date) -> Option<i64> {
    let updated = entry.updated_at?.with_timezone(&chrono::Local).date_naive();
    Some((today - updated).num_days()).filter(|d| *d >= IDLE_DAYS)
}

/// The Want to Read book that follows a finished book of the same series,
/// with no unread book of the series between them. The one after the most
/// recently finished book wins.
///
/// Only books in the library count: the series page would know of more, but
/// it would cost a request per series. So a book counts as next only if the
/// finished one is at most one position before it (2 after 1, or after 1.5),
/// in case the books between are missing from the library.
///
/// A book not out yet is never next, and doesn't hold back a later one that
/// is (a novella announced after the next novel came out).
fn series_next(
    entries: &[LibraryEntry],
    released: &dyn Fn(&LibraryEntry) -> bool,
) -> Option<SeriesNext> {
    let in_series = |e: &LibraryEntry, series: SeriesId| {
        e.series
            .iter()
            .find(|s| s.id == Some(series))
            .and_then(|s| s.position)
    };
    let mut best: Option<(Option<Date>, SeriesNext)> = None;
    for next in entries
        .iter()
        .filter(|e| e.status == ReadingStatus::WantToRead && released(e))
    {
        for series in &next.series {
            let (Some(id), Some(position)) = (series.id, series.position) else {
                continue;
            };
            let after = entries
                .iter()
                .filter(|e| e.status == ReadingStatus::Read)
                .filter_map(|e| in_series(e, id).map(|p| (e, p)))
                .filter(|(_, p)| *p < position && *p >= position - 1.0)
                .max_by(|(_, a), (_, b)| a.total_cmp(b));
            let Some((after, after_position)) = after else {
                continue;
            };
            // A book between them still to be read comes first. One the
            // reader ignored or gave up on doesn't hold the next one back.
            let unread_between = entries.iter().any(|e| {
                e.user_book_id != next.user_book_id
                    && released(e)
                    && matches!(
                        e.status,
                        ReadingStatus::WantToRead
                            | ReadingStatus::CurrentlyReading
                            | ReadingStatus::Paused
                    )
                    && in_series(e, id).is_some_and(|p| p > after_position && p < position)
            });
            if unread_between {
                continue;
            }
            let finished = after.reads.iter().filter_map(|r| r.finished_at).max();
            if best.as_ref().is_some_and(|(f, _)| *f >= finished) {
                continue;
            }
            best = Some((
                finished,
                SeriesNext {
                    id: next.user_book_id,
                    series: series.name.clone(),
                    position,
                    after: after.user_book_id,
                    after_position,
                },
            ));
        }
    }
    best.map(|(_, next)| next)
}

/// A Read book finished lately, offered for rating.
#[derive(Debug, Clone, PartialEq)]
pub struct ToRate {
    pub id: UserBookId,
    pub finished: Date,
}

/// The rows of "Rate what you finished" and how many books still need a
/// rating: Read books finished in the last [`RATE_DAYS`] days with no rating,
/// the most recently finished first.
///
/// The books in `keep` (rated on Home since it was opened) stay among the
/// rows, so a row doesn't vanish under the pointer and a slip can be undone.
/// The cap applies to both together, so rating a book never pulls another
/// one up into its place.
pub fn to_rate(
    entries: &[LibraryEntry],
    today: Date,
    keep: &HashSet<UserBookId>,
) -> (Vec<ToRate>, usize) {
    let mut unrated = 0;
    let mut rows: Vec<ToRate> = Vec::new();
    for e in entries.iter().filter(|e| e.status == ReadingStatus::Read) {
        if e.rating.is_some() && !keep.contains(&e.user_book_id) {
            continue;
        }
        let Some(finished) = e.reads.iter().filter_map(|r| r.finished_at).max() else {
            continue;
        };
        if !(0..=RATE_DAYS).contains(&(today - finished).num_days()) {
            continue;
        }
        if e.rating.is_none() {
            unrated += 1;
        }
        rows.push(ToRate {
            id: e.user_book_id,
            finished,
        });
    }
    rows.sort_by(|a, b| b.finished.cmp(&a.finished).then(b.id.cmp(&a.id)));
    rows.truncate(RATE_ROWS);
    (rows, unrated)
}

/// What the goal can expect from the books being read: the ones that should
/// finish by its end at their pace, the most recently updated first, so Home
/// can name them. A book left idle doesn't count, as Home shows no finish
/// date for it. Only for a goal counted in books.
pub fn finishing_by(goal: &Goal, digest: &Digest, today: Date) -> Vec<UserBookId> {
    if goal.metric != GoalMetric::Books || today > goal.end {
        return Vec::new();
    }
    digest
        .reading
        .iter()
        .filter(|r| r.idle_days.is_none())
        .filter(|r| r.pace.and_then(|p| p.finish).is_some_and(|f| f <= goal.end))
        .map(|r| r.id)
        .collect()
}

/// Books finished in each month of a calendar-year goal, January first, as
/// [`crate::goals::model::GoalsModel::finished_in`] counts them. `None` for a
/// goal over other dates.
pub fn finished_per_month(goal: &Goal, entries: &[LibraryEntry]) -> Option<[u32; 12]> {
    use chrono::Datelike as _;
    let year = crate::format::goal_year(goal.start, goal.end)?;
    let mut months = [0; 12];
    // A goal that ends on the next New Year's Day counts that day too; it
    // has no bar of its own.
    for (_, finished) in crate::goals::model::GoalsModel::finished_in(goal, entries) {
        if finished.year() == year {
            months[finished.month0() as usize] += 1;
        }
    }
    Some(months)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{
        BookId, BookSummary, GoalId, ReadId, ReadSession, SeriesRef, Timestamp, UserId,
    };

    fn day(s: &str) -> Date {
        s.parse().unwrap()
    }

    /// Midday UTC, so the local date is the same in any time zone up to ±11h.
    fn at(s: &str) -> Timestamp {
        format!("{s}T12:00:00Z").parse().unwrap()
    }

    fn entry(id: i64, status: ReadingStatus, pages: Option<u32>) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(id), format!("Book {id}"));
        book.pages = pages;
        LibraryEntry::new(UserBookId::new(id), UserId::new(1), book, status)
    }

    fn reading(id: i64, pages: u32, started: Option<&str>, page: Option<u32>) -> LibraryEntry {
        let mut e = entry(id, ReadingStatus::CurrentlyReading, Some(pages));
        let mut read = ReadSession::new(ReadId::new(id));
        read.started_at = started.map(day);
        read.progress_pages = page;
        e.reads.push(read);
        e
    }

    fn finished(mut e: LibraryEntry, on: &str) -> LibraryEntry {
        let mut read = ReadSession::new(ReadId::new(e.user_book_id.get() * 10));
        read.finished_at = Some(day(on));
        e.reads.push(read);
        e
    }

    fn in_series(mut e: LibraryEntry, series: i64, position: Option<f32>) -> LibraryEntry {
        let mut s = SeriesRef::new(format!("Series {series}"));
        s.id = Some(SeriesId::new(series));
        s.position = position;
        e.series.push(s);
        e
    }

    #[test]
    fn pace_is_pages_per_day_since_the_start_day() {
        let today = day("2026-11-20");
        // 312 pages over 20 days (Nov 1 to 20) is 15.6 a day: 164 left take 11 days.
        let p = pace(&reading(1, 476, Some("2026-11-01"), Some(312)), today).unwrap();
        assert!((p.pages_per_day - 15.6).abs() < 1e-9);
        assert_eq!(p.finish, Some(day("2026-12-01")));
    }

    #[test]
    fn no_pace_without_enough_to_go_on() {
        let today = day("2026-11-20");
        // No start date, no page, started today or yesterday, or in the future.
        assert_eq!(pace(&reading(1, 300, None, Some(50)), today), None);
        assert_eq!(
            pace(&reading(1, 300, Some("2026-11-01"), None), today),
            None
        );
        assert_eq!(
            pace(&reading(1, 300, Some("2026-11-01"), Some(0)), today),
            None
        );
        assert_eq!(
            pace(&reading(1, 300, Some("2026-11-19"), Some(80)), today),
            None
        );
        assert_eq!(
            pace(&reading(1, 300, Some("2026-11-25"), Some(80)), today),
            None
        );
        assert!(pace(&reading(1, 300, Some("2026-11-18"), Some(80)), today).is_some());
        // No page count, or an audiobook.
        let mut unknown = reading(1, 300, Some("2026-11-01"), Some(80));
        unknown.book.pages = None;
        assert_eq!(pace(&unknown, today), None);
        let mut audio = reading(1, 300, Some("2026-11-01"), Some(80));
        let mut edition = hardcover_core::EditionSummary::new(hardcover_core::EditionId::new(5));
        edition.audio_seconds = Some(36_000);
        audio.edition = Some(edition);
        assert_eq!(pace(&audio, today), None);
    }

    #[test]
    fn a_book_not_being_read_has_no_pace() {
        let today = day("2026-11-20");
        let reading_now = reading(1, 476, Some("2026-11-01"), Some(312));
        assert!(pace(&reading_now, today).is_some());
        for status in [
            ReadingStatus::Paused,
            ReadingStatus::DidNotFinish,
            ReadingStatus::Read,
        ] {
            let mut e = reading_now.clone();
            e.status = status;
            assert_eq!(pace(&e, today), None, "{status:?}");
        }
    }

    #[test]
    fn a_far_or_finished_book_gets_no_finish_date() {
        let today = day("2026-11-20");
        // One page a day, 900 to go.
        let slow = pace(&reading(1, 1000, Some("2026-08-13"), Some(100)), today).unwrap();
        assert_eq!(slow.finish, None);
        // On the last page, or past it (a longer edition read than recorded).
        let done = pace(&reading(1, 300, Some("2026-11-01"), Some(300)), today).unwrap();
        assert_eq!(done.finish, None);
        let past = pace(&reading(1, 300, Some("2026-11-01"), Some(320)), today).unwrap();
        assert_eq!(past.finish, None);
    }

    #[test]
    fn reading_comes_most_recently_updated_first_and_flags_idle_books() {
        let today = day("2026-11-20");
        let mut a = reading(1, 300, Some("2026-11-01"), Some(50));
        a.updated_at = Some(at("2026-11-12"));
        let mut b = reading(2, 300, Some("2026-11-01"), Some(50));
        b.updated_at = Some(at("2026-11-19"));
        let c = reading(3, 300, None, None);
        let d = compute(&[a, b, c], today, &HashSet::new());
        let ids: Vec<i64> = d.reading.iter().map(|r| r.id.get()).collect();
        assert_eq!(ids, [2, 1, 3]);
        assert_eq!(d.reading[0].idle_days, None);
        assert_eq!(d.reading[1].idle_days, Some(8));
        assert_eq!(d.reading[2].idle_days, None, "never updated says nothing");
    }

    #[test]
    fn the_next_book_follows_a_finished_one() {
        let one = finished(
            in_series(entry(1, ReadingStatus::Read, Some(400)), 7, Some(1.0)),
            "2026-10-12",
        );
        let two = in_series(entry(2, ReadingStatus::WantToRead, Some(300)), 7, Some(2.0));
        let four = in_series(entry(4, ReadingStatus::WantToRead, Some(300)), 7, Some(4.0));
        let d = compute(
            &[one.clone(), two.clone(), four.clone()],
            day("2026-11-20"),
            &HashSet::new(),
        );
        let next = d.series_next.expect("book 2 is next");
        assert_eq!((next.id.get(), next.after.get()), (2, 1));
        assert_eq!((next.position, next.after_position), (2.0, 1.0));
        assert_eq!(next.series, "Series 7");
        // The pick isn't repeated in the list below it.
        assert!(!d.up_next.contains(&next.id));

        // Book 4 is not next after book 1: 2 and 3 may be missing.
        let d = compute(
            &[one.clone(), four.clone()],
            day("2026-11-20"),
            &HashSet::new(),
        );
        assert_eq!(d.series_next, None);

        // A novella at 1.5 comes first, and then 2 follows it.
        let novella = in_series(entry(3, ReadingStatus::WantToRead, Some(90)), 7, Some(1.5));
        let d = compute(
            &[one.clone(), two.clone(), novella.clone()],
            day("2026-11-20"),
            &HashSet::new(),
        );
        assert_eq!(d.series_next.unwrap().id.get(), 3);
        let read_novella = finished(
            in_series(entry(3, ReadingStatus::Read, Some(90)), 7, Some(1.5)),
            "2026-11-01",
        );
        let d = compute(
            &[one.clone(), two.clone(), read_novella],
            day("2026-11-20"),
            &HashSet::new(),
        );
        let next = d.series_next.unwrap();
        assert_eq!((next.id.get(), next.after_position), (2, 1.5));

        // An ignored novella between them doesn't hold book 2 back.
        let ignored = in_series(entry(3, ReadingStatus::Ignored, Some(90)), 7, Some(1.5));
        let d = compute(
            &[one.clone(), two.clone(), ignored],
            day("2026-11-20"),
            &HashSet::new(),
        );
        assert_eq!(d.series_next.unwrap().id.get(), 2);

        // Book 2 being read (or paused) leaves nothing to suggest.
        let mut reading_two = two.clone();
        reading_two.status = ReadingStatus::CurrentlyReading;
        let d = compute(
            &[one.clone(), reading_two],
            day("2026-11-20"),
            &HashSet::new(),
        );
        assert_eq!(d.series_next, None);
    }

    #[test]
    fn a_book_not_out_yet_is_not_up_next() {
        let today = day("2026-11-20");
        let one = finished(
            in_series(entry(1, ReadingStatus::Read, Some(400)), 7, Some(1.0)),
            "2026-10-12",
        );
        let mut two = in_series(entry(2, ReadingStatus::WantToRead, None), 7, Some(2.0));
        two.date_added = Some(day("2026-11-01"));
        let mut other = entry(5, ReadingStatus::WantToRead, Some(200));
        other.date_added = Some(day("2026-10-01"));
        let ids = |v: &[UserBookId]| v.iter().map(|i| i.get()).collect::<Vec<_>>();

        // Out on a day the upcoming books know: neither the pick nor a row.
        let unreleased = HashSet::from([BookId::new(2)]);
        let d = compute(
            &[one.clone(), two.clone(), other.clone()],
            today,
            &unreleased,
        );
        assert_eq!(d.series_next, None);
        assert_eq!(ids(&d.up_next), [5]);
        assert_eq!(d.want_to_read, 2, "the shelf still counts it");

        // Dated to a later year, before the upcoming books load.
        let mut next_year = two.clone();
        next_year.book.release_year = Some(2027);
        let d = compute(
            &[one.clone(), next_year, other.clone()],
            today,
            &HashSet::new(),
        );
        assert_eq!(d.series_next, None);
        let mut this_year = two.clone();
        this_year.book.release_year = Some(2026);
        let d = compute(&[one.clone(), this_year], today, &HashSet::new());
        assert_eq!(d.series_next.unwrap().id.get(), 2);

        // A novella still to come doesn't hold back the next book.
        let novella = in_series(entry(3, ReadingStatus::WantToRead, None), 7, Some(1.5));
        let unreleased = HashSet::from([BookId::new(3)]);
        let d = compute(&[one.clone(), two.clone(), novella], today, &unreleased);
        assert_eq!(d.series_next.unwrap().id.get(), 2);
    }

    #[test]
    fn series_without_ids_or_positions_suggest_nothing() {
        let one = finished(
            in_series(entry(1, ReadingStatus::Read, None), 7, None),
            "2026-10-12",
        );
        let two = in_series(entry(2, ReadingStatus::WantToRead, None), 7, Some(2.0));
        assert_eq!(
            compute(&[one, two], day("2026-11-20"), &HashSet::new()).series_next,
            None
        );

        let mut one = finished(entry(1, ReadingStatus::Read, None), "2026-10-12");
        let mut two = entry(2, ReadingStatus::WantToRead, None);
        let mut s = SeriesRef::new("No id");
        s.position = Some(1.0);
        one.series.push(s.clone());
        s.position = Some(2.0);
        two.series.push(s);
        assert_eq!(
            compute(&[one, two], day("2026-11-20"), &HashSet::new()).series_next,
            None
        );
    }

    #[test]
    fn the_most_recently_finished_series_wins() {
        let a1 = finished(
            in_series(entry(1, ReadingStatus::Read, None), 7, Some(1.0)),
            "2026-05-01",
        );
        let a2 = in_series(entry(2, ReadingStatus::WantToRead, None), 7, Some(2.0));
        let b1 = finished(
            in_series(entry(3, ReadingStatus::Read, None), 8, Some(1.0)),
            "2026-09-01",
        );
        let b2 = in_series(entry(4, ReadingStatus::WantToRead, None), 8, Some(2.0));
        let d = compute(&[a1, a2, b1, b2], day("2026-11-20"), &HashSet::new());
        assert_eq!(d.series_next.unwrap().id.get(), 4);
    }

    #[test]
    fn up_next_is_the_newest_want_to_read() {
        let books: Vec<LibraryEntry> = [
            (1, "2026-01-01", Some(500)),
            (2, "2026-03-01", Some(200)),
            (3, "2026-02-01", None),
            (4, "2026-04-01", Some(350)),
            (5, "2025-12-01", Some(120)),
        ]
        .into_iter()
        .map(|(id, added, pages)| {
            let mut e = entry(id, ReadingStatus::WantToRead, pages);
            e.date_added = Some(day(added));
            e
        })
        .collect();
        let d = compute(&books, day("2026-11-20"), &HashSet::new());
        let ids = |v: &[UserBookId]| v.iter().map(|i| i.get()).collect::<Vec<_>>();
        assert_eq!(ids(&d.up_next), [4, 2, 3]);
        assert_eq!(d.want_to_read, 5);
    }

    #[test]
    fn paused_is_the_latest_one() {
        let mut a = entry(1, ReadingStatus::Paused, None);
        a.updated_at = Some(at("2026-10-01"));
        let mut b = entry(2, ReadingStatus::Paused, None);
        b.updated_at = Some(at("2026-09-01"));
        let d = compute(&[a, b], day("2026-11-20"), &HashSet::new());
        assert_eq!(d.paused.map(|i| i.get()), Some(1));
        assert_eq!(d.paused_count, 2);
        assert!(!d.is_empty());
        assert!(compute(&[], day("2026-11-20"), &HashSet::new()).is_empty());
    }

    fn rated(mut e: LibraryEntry, stars: f32) -> LibraryEntry {
        e.rating = hardcover_core::Rating::new(stars);
        e
    }

    #[test]
    fn books_finished_lately_without_a_rating_are_offered() {
        let today = day("2026-11-20");
        let read = |id, on| finished(entry(id, ReadingStatus::Read, None), on);
        let books = [
            read(1, "2026-11-12"),
            read(2, "2026-09-21"), // 60 days ago: still in
            read(3, "2026-09-20"), // 61 days ago: out
            rated(read(4, "2026-11-01"), 4.0),
            finished(entry(5, ReadingStatus::DidNotFinish, None), "2026-11-10"),
            entry(6, ReadingStatus::Read, None), // no finish date
            read(7, "2026-11-25"),               // in the future
            read(8, "2026-10-30"),
        ];
        let (rows, unrated) = to_rate(&books, today, &HashSet::new());
        let ids: Vec<i64> = rows.iter().map(|r| r.id.get()).collect();
        assert_eq!(ids, [1, 8, 2]);
        assert_eq!(rows[0].finished, day("2026-11-12"));
        assert_eq!(unrated, 3);

        // A later read's finish counts, not the first one's.
        let reread = finished(read(9, "2019-02-10"), "2026-11-15");
        let (rows, _) = to_rate(&[reread], today, &HashSet::new());
        assert_eq!(rows[0].finished, day("2026-11-15"));
    }

    #[test]
    fn a_book_rated_on_home_keeps_its_row() {
        let today = day("2026-11-20");
        let read = |id, on| finished(entry(id, ReadingStatus::Read, None), on);
        let mut books = vec![
            read(1, "2026-11-15"),
            read(2, "2026-11-10"),
            read(3, "2026-11-05"),
            read(4, "2026-11-01"),
            read(5, "2026-10-28"),
        ];
        let ids = |rows: &[ToRate]| rows.iter().map(|r| r.id.get()).collect::<Vec<_>>();
        let (rows, unrated) = to_rate(&books, today, &HashSet::new());
        assert_eq!(ids(&rows), [1, 2, 3, 4]);
        assert_eq!(unrated, 5);

        // Rating book 2 keeps it in place, and book 5 doesn't move up.
        books[1] = rated(books[1].clone(), 3.5);
        let keep = HashSet::from([UserBookId::new(2)]);
        let (rows, unrated) = to_rate(&books, today, &keep);
        assert_eq!(ids(&rows), [1, 2, 3, 4]);
        assert_eq!(unrated, 4);

        // Next time Home opens, it's gone.
        let (rows, _) = to_rate(&books, today, &HashSet::new());
        assert_eq!(ids(&rows), [1, 3, 4, 5]);

        // Cleared again on Home: still kept, and counted as unrated.
        books[1].rating = None;
        let (rows, unrated) = to_rate(&books, today, &keep);
        assert_eq!(ids(&rows), [1, 2, 3, 4]);
        assert_eq!(unrated, 5);
    }

    fn goal(metric: GoalMetric, start: &str, end: &str) -> Goal {
        Goal::new(GoalId::new(1), metric, 12, day(start), day(end))
    }

    #[test]
    fn books_on_pace_to_finish_count_toward_a_book_goal() {
        let today = day("2026-11-20");
        let soon = reading(1, 476, Some("2026-11-01"), Some(312)); // Dec 1
        let late = reading(2, 1007, Some("2026-10-20"), Some(418)); // January
        let d = compute(&[soon, late], today, &HashSet::new());
        let year = goal(GoalMetric::Books, "2026-01-01", "2026-12-31");
        assert_eq!(finishing_by(&year, &d, today), [UserBookId::new(1)]);
        let november = goal(GoalMetric::Books, "2026-01-01", "2026-11-30");
        assert!(finishing_by(&november, &d, today).is_empty());
        let pages = goal(GoalMetric::Pages, "2026-01-01", "2026-12-31");
        assert!(finishing_by(&pages, &d, today).is_empty());
        assert!(finishing_by(&year, &d, day("2027-01-02")).is_empty());
        let next_year = goal(GoalMetric::Books, "2026-01-01", "2027-12-31");
        assert_eq!(
            finishing_by(&next_year, &d, today),
            [UserBookId::new(2), UserBookId::new(1)],
            "both, in the order Home lists them"
        );

        // An idle book's finish date isn't trusted.
        let mut idle = reading(3, 476, Some("2026-11-01"), Some(312));
        idle.updated_at = Some(at("2026-11-10"));
        let d = compute(&[idle], today, &HashSet::new());
        assert!(d.reading[0].pace.and_then(|p| p.finish).is_some());
        assert!(finishing_by(&year, &d, today).is_empty());
    }

    #[test]
    fn months_are_counted_for_a_calendar_year_only() {
        let books = [
            finished(entry(1, ReadingStatus::Read, None), "2026-03-01"),
            finished(entry(2, ReadingStatus::Read, None), "2026-06-08"),
            finished(entry(3, ReadingStatus::Read, None), "2026-06-30"),
            finished(entry(4, ReadingStatus::Read, None), "2025-12-31"),
        ];
        let months =
            finished_per_month(&goal(GoalMetric::Books, "2026-01-01", "2026-12-31"), &books)
                .unwrap();
        assert_eq!(months, [0, 0, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            finished_per_month(&goal(GoalMetric::Books, "2026-01-01", "2026-06-30"), &books),
            None
        );
    }

    #[test]
    fn a_year_goal_may_end_on_the_next_new_years_day() {
        // How Hardcover dates its own year goals. The last day has no bar.
        let books = [
            finished(entry(1, ReadingStatus::Read, None), "2026-01-01"),
            finished(entry(2, ReadingStatus::Read, None), "2026-12-31"),
            finished(entry(3, ReadingStatus::Read, None), "2027-01-01"),
        ];
        let months =
            finished_per_month(&goal(GoalMetric::Books, "2026-01-01", "2027-01-01"), &books)
                .unwrap();
        assert_eq!(months, [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    }
}
