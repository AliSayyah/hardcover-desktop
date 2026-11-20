//! What "Coming soon" shows, worked out from the upcoming books, the library
//! and the date: which series to ask about, which books to show, how to word
//! their dates and how to group them. Plain functions, so they can be tested.

use std::collections::HashSet;

use chrono::{Datelike as _, Months};
use hardcover_core::{BookId, Date, LibraryEntry, ReadingStatus, SeriesId, UpcomingBook};

use crate::format::series_position_text;

/// Rows of the Home card.
pub const CARD_ROWS: usize = 5;
/// A release at most this many days away also says how soon it is.
const SOON_DAYS: i64 = 13;

/// The series to ask about: those of every library entry the reader hasn't
/// ignored or given up on, sorted, each once.
pub fn watched_series(entries: &[LibraryEntry]) -> Vec<SeriesId> {
    let mut ids: Vec<SeriesId> = entries
        .iter()
        .filter(|e| {
            !matches!(
                e.status,
                ReadingStatus::Ignored | ReadingStatus::DidNotFinish
            )
        })
        .flat_map(|e| e.series.iter().filter_map(|s| s.id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The books to show, the soonest first: still to come after `today`, and
/// either not in the library or on Want to Read. A book the reader already
/// has on another shelf (an advance copy read, or one set aside) is left out.
pub fn shown(
    books: &[UpcomingBook],
    status: impl Fn(BookId) -> Option<ReadingStatus>,
    today: Date,
) -> Vec<&UpcomingBook> {
    let mut shown: Vec<&UpcomingBook> = books
        .iter()
        .filter(|b| b.release_date > today)
        .filter(|b| status(b.book.id).is_none_or(|s| s == ReadingStatus::WantToRead))
        .collect();
    shown.sort_by(|a, b| {
        a.release_date
            .cmp(&b.release_date)
            .then_with(|| a.book.title.cmp(&b.book.title))
    });
    shown
}

/// The ids of the books not out yet on `today`, whatever their shelf.
pub fn unreleased(books: &[UpcomingBook], today: Date) -> HashSet<BookId> {
    books
        .iter()
        .filter(|b| b.release_date > today)
        .map(|b| b.book.id)
        .collect()
}

/// The last day counted as "within the coming 12 months". Dates past it are
/// mostly placeholders (Jan 1 or Dec 1 of a far year), so only their year
/// is shown.
pub fn horizon(today: Date) -> Date {
    today.checked_add_months(Months::new(12)).unwrap_or(today)
}

/// The Home card's books: the first few within the coming 12 months.
pub fn next_releases<'a>(shown: &[&'a UpcomingBook], today: Date) -> Vec<&'a UpcomingBook> {
    let horizon = horizon(today);
    shown
        .iter()
        .copied()
        .filter(|b| b.release_date <= horizon)
        .take(CARD_ROWS)
        .collect()
}

/// "Dec 8" this year, "Jan 5, 2027" within the coming 12 months, and just
/// "2030" further off.
pub fn release_text(date: Date, today: Date) -> String {
    if date.year() == today.year() {
        date.format("%b %-d").to_string()
    } else if date <= horizon(today) {
        date.format("%b %-d, %Y").to_string()
    } else {
        date.year().to_string()
    }
}

/// "Tomorrow", "In 4 days": only for releases in the next two weeks.
pub fn soon_text(date: Date, today: Date) -> Option<String> {
    match (date - today).num_days() {
        0 => Some("Today".to_owned()),
        1 => Some("Tomorrow".to_owned()),
        n @ 2..=SOON_DAYS => Some(format!("In {n} days")),
        _ => None,
    }
}

/// Every series a book is in, with its position: "Lantern Cycle #2.5 ·
/// Companion Tales".
pub fn series_text(book: &UpcomingBook) -> String {
    book.series
        .iter()
        .map(|s| match s.position {
            Some(p) => format!("{} {}", s.name, series_position_text(p)),
            None => s.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A heading of the full list: a month within the coming 12 months, then
/// a year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Month { year: i32, month: u32 },
    Year(i32),
}

impl Group {
    pub fn of(date: Date, today: Date) -> Self {
        if date <= horizon(today) {
            Group::Month {
                year: date.year(),
                month: date.month(),
            }
        } else {
            Group::Year(date.year())
        }
    }

    /// "December", "January 2027", "Later in 2027" (after that year's
    /// months), "2030".
    pub fn label(self, today: Date) -> String {
        match self {
            Group::Month { year, month } => {
                let first = Date::from_ymd_opt(year, month, 1).unwrap_or(today);
                if year == today.year() {
                    first.format("%B").to_string()
                } else {
                    first.format("%B %Y").to_string()
                }
            }
            Group::Year(year) if year <= horizon(today).year() => format!("Later in {year}"),
            Group::Year(year) => year.to_string(),
        }
    }
}

/// The books under their headings, in date order. `shown` is sorted by date.
pub fn grouped<'a>(shown: &[&'a UpcomingBook], today: Date) -> Vec<(Group, Vec<&'a UpcomingBook>)> {
    let mut groups: Vec<(Group, Vec<&UpcomingBook>)> = Vec::new();
    for book in shown {
        let group = Group::of(book.release_date, today);
        match groups.last_mut() {
            Some((g, books)) if *g == group => books.push(book),
            _ => groups.push((group, vec![book])),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{BookSummary, SeriesRef, UserBookId, UserId};

    fn day(s: &str) -> Date {
        s.parse().unwrap()
    }

    fn upcoming(id: i64, date: &str) -> UpcomingBook {
        UpcomingBook::new(
            BookSummary::new(BookId::new(id), format!("Book {id}")),
            day(date),
        )
    }

    fn entry(id: i64, status: ReadingStatus, series: &[i64]) -> LibraryEntry {
        let book = BookSummary::new(BookId::new(id), format!("Book {id}"));
        let mut e = LibraryEntry::new(UserBookId::new(id), UserId::new(1), book, status);
        for s in series {
            let mut r = SeriesRef::new(format!("Series {s}"));
            r.id = Some(SeriesId::new(*s));
            e.series.push(r);
        }
        e
    }

    #[test]
    fn series_of_ignored_and_abandoned_books_are_not_watched() {
        let mut no_id = entry(9, ReadingStatus::Read, &[]);
        no_id.series.push(SeriesRef::new("No id"));
        let entries = [
            entry(1, ReadingStatus::Read, &[30, 10]),
            entry(2, ReadingStatus::WantToRead, &[10]),
            entry(3, ReadingStatus::Ignored, &[40]),
            entry(4, ReadingStatus::DidNotFinish, &[50]),
            entry(5, ReadingStatus::Paused, &[20]),
            // A series counts if any of its books is still followed.
            entry(6, ReadingStatus::DidNotFinish, &[30]),
            no_id,
        ];
        let ids: Vec<i64> = watched_series(&entries).iter().map(|s| s.get()).collect();
        assert_eq!(ids, [10, 20, 30]);
    }

    #[test]
    fn books_already_on_a_shelf_other_than_want_to_read_are_left_out() {
        let today = day("2026-11-20");
        let books = [
            upcoming(1, "2027-01-05"),
            upcoming(2, "2026-12-08"),
            upcoming(3, "2026-12-01"),
            upcoming(4, "2026-12-02"),
            upcoming(5, "2026-11-20"), // out today
            upcoming(6, "2026-12-08"),
            upcoming(7, "2026-12-03"),
            upcoming(8, "2026-12-04"),
        ];
        let status = |id: BookId| match id.get() {
            2 => Some(ReadingStatus::WantToRead),
            3 => Some(ReadingStatus::Read),
            4 => Some(ReadingStatus::CurrentlyReading),
            7 => Some(ReadingStatus::Paused),
            8 => Some(ReadingStatus::Ignored),
            _ => None,
        };
        let ids: Vec<i64> = shown(&books, status, today)
            .iter()
            .map(|b| b.book.id.get())
            .collect();
        // By date, then title.
        assert_eq!(ids, [2, 6, 1]);
        let out: Vec<i64> = {
            let mut v: Vec<i64> = unreleased(&books, today).iter().map(|b| b.get()).collect();
            v.sort();
            v
        };
        assert_eq!(out, [1, 2, 3, 4, 6, 7, 8]);
    }

    #[test]
    fn dates_lose_detail_with_distance() {
        let today = day("2026-11-20");
        assert_eq!(release_text(day("2026-12-08"), today), "Dec 8");
        assert_eq!(release_text(day("2027-01-05"), today), "Jan 5, 2027");
        assert_eq!(release_text(day("2027-11-20"), today), "Nov 20, 2027");
        // Next year, but past the coming 12 months: a placeholder, likely.
        assert_eq!(release_text(day("2027-12-01"), today), "2027");
        assert_eq!(release_text(day("2030-01-01"), today), "2030");
    }

    #[test]
    fn soon_releases_say_how_soon() {
        let today = day("2026-11-20");
        assert_eq!(
            soon_text(day("2026-11-20"), today).as_deref(),
            Some("Today")
        );
        assert_eq!(
            soon_text(day("2026-11-21"), today).as_deref(),
            Some("Tomorrow")
        );
        assert_eq!(
            soon_text(day("2026-11-24"), today).as_deref(),
            Some("In 4 days")
        );
        assert_eq!(
            soon_text(day("2026-12-03"), today).as_deref(),
            Some("In 13 days")
        );
        assert_eq!(soon_text(day("2026-12-04"), today), None);
    }

    #[test]
    fn the_card_shows_the_next_five_within_a_year() {
        let today = day("2026-11-20");
        let books: Vec<UpcomingBook> = [
            "2026-11-24",
            "2026-12-08",
            "2027-01-05",
            "2027-03-01",
            "2027-11-20",
            "2027-12-01",
        ]
        .iter()
        .enumerate()
        .map(|(i, d)| upcoming(i as i64 + 1, d))
        .collect();
        let refs: Vec<&UpcomingBook> = books.iter().collect();
        let ids: Vec<i64> = next_releases(&refs, today)
            .iter()
            .map(|b| b.book.id.get())
            .collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
        let far = [&books[5]];
        assert!(next_releases(&far, today).is_empty());
    }

    #[test]
    fn the_full_list_groups_by_month_then_by_year() {
        let today = day("2026-11-20");
        let books: Vec<UpcomingBook> = [
            "2026-11-24",
            "2026-11-30",
            "2026-12-08",
            "2027-01-05",
            "2027-11-20",
            "2027-12-01",
            "2028-06-01",
            "2030-01-01",
            "2030-12-01",
        ]
        .iter()
        .enumerate()
        .map(|(i, d)| upcoming(i as i64 + 1, d))
        .collect();
        let refs: Vec<&UpcomingBook> = books.iter().collect();
        let groups: Vec<(String, usize)> = grouped(&refs, today)
            .into_iter()
            .map(|(g, b)| (g.label(today), b.len()))
            .collect();
        let expected = [
            ("November", 2),
            ("December", 1),
            ("January 2027", 1),
            ("November 2027", 1),
            // 2027-12-01 is next year, but past the coming 12 months.
            ("Later in 2027", 1),
            ("2028", 1),
            ("2030", 2),
        ];
        let expected: Vec<(String, usize)> =
            expected.iter().map(|(l, n)| (l.to_string(), *n)).collect();
        assert_eq!(groups, expected);
    }

    #[test]
    fn every_series_is_named() {
        let mut book = upcoming(1, "2026-12-08");
        let mut a = SeriesRef::new("Lantern Cycle");
        a.position = Some(2.5);
        book.series.push(a);
        book.series.push(SeriesRef::new("Companion Tales"));
        assert_eq!(series_text(&book), "Lantern Cycle #2.5 · Companion Tales");
    }
}
