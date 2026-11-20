//! The Goodreads library export's CSV, the de facto format other services
//! (StoryGraph, BookWyrm and more) import.
//!
//! The columns are Goodreads' own, in its order, up to Owned Copies, with its
//! conventions as seen in its exports: ISBNs written as `="…"` (so a
//! spreadsheet keeps the leading zeros), an empty ISBN as `=""`, dates as
//! `YYYY/MM/DD`, `0` for no rating, the average to two decimals. Hardcover's
//! own ids and values fill them:
//!
//! - `Book Id` is the Hardcover book id.
//! - `My Rating` is rounded to whole stars, halves up: Goodreads has no
//!   half stars, and a rated book must not read as unrated (0).
//! - `Exclusive Shelf` is `to-read`, `currently-reading` or `read`; Paused and
//!   Did Not Finish become the custom shelves `paused` and `did-not-finish`.
//!   Ignored books are left out.
//! - `Bookshelves` holds the exclusive shelf (except `read`, as Goodreads
//!   does) and the user's lists the book is on, the Owned list aside, each as a
//!   shelf name: lower case, words joined by hyphens, never a comma.
//!   `Bookshelves with positions` adds each one's position: the order added
//!   for a status shelf, the list's own order for a list.
//! - `Read Count` counts finished reads; `Owned Copies` is 1 for a book on the
//!   Owned list.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use chrono::Datelike as _;

use super::Backup;
use crate::api::{BookId, Date, LibraryEntry, ReadingStatus, UserList};

/// The columns, in Goodreads' order.
pub(super) const COLUMNS: [&str; 24] = [
    "Book Id",
    "Title",
    "Author",
    "Author l-f",
    "Additional Authors",
    "ISBN",
    "ISBN13",
    "My Rating",
    "Average Rating",
    "Publisher",
    "Binding",
    "Number of Pages",
    "Year Published",
    "Original Publication Year",
    "Date Read",
    "Date Added",
    "Bookshelves",
    "Bookshelves with positions",
    "Exclusive Shelf",
    "My Review",
    "Spoiler",
    "Private Notes",
    "Read Count",
    "Owned Copies",
];

/// The library as a Goodreads export: a header row, then one row per entry
/// (Ignored ones left out), by entry id. Lines end in `\n`, as Goodreads'
/// do; fields with a comma, quote or line break are quoted.
pub fn goodreads_csv(backup: &Backup) -> String {
    let shelves = ListShelves::new(backup.lists.as_deref().unwrap_or_default());
    let positions = status_positions(&backup.entries);
    let mut entries: Vec<&LibraryEntry> = backup
        .entries
        .iter()
        .filter(|e| exclusive_shelf(e.status).is_some())
        .collect();
    entries.sort_by_key(|e| e.user_book_id);

    let mut out = String::new();
    write_row(&mut out, COLUMNS.iter().map(|c| Cow::Borrowed(*c)));
    for entry in entries {
        let row = row(backup, entry, &shelves, &positions);
        write_row(&mut out, row.into_iter());
    }
    out
}

/// The shelf for a status; `None` for Ignored, which is left out.
pub(super) fn exclusive_shelf(status: ReadingStatus) -> Option<&'static str> {
    Some(match status {
        ReadingStatus::WantToRead => "to-read",
        ReadingStatus::CurrentlyReading => "currently-reading",
        ReadingStatus::Read => "read",
        ReadingStatus::Paused => "paused",
        ReadingStatus::DidNotFinish => "did-not-finish",
        ReadingStatus::Ignored => return None,
    })
}

/// Goodreads has whole stars only: halves round up, so 0.5 is 1 and no
/// rating is 0.
pub(super) fn whole_stars(rating: Option<f32>) -> u8 {
    rating.map_or(0, |r| r.round().clamp(1.0, 5.0) as u8)
}

/// Which lists each book is on, as shelves, and which books are owned.
struct ListShelves {
    /// Per book: (shelf name, 1-based position on the list), in list order.
    by_book: HashMap<BookId, Vec<(String, usize)>>,
    owned: HashSet<BookId>,
}

impl ListShelves {
    fn new(lists: &[super::ListContents]) -> Self {
        let mut by_book: HashMap<BookId, Vec<(String, usize)>> = HashMap::new();
        let mut owned = HashSet::new();
        let mut ordered: Vec<&super::ListContents> = lists.iter().collect();
        ordered.sort_by(|a, b| {
            a.list
                .name
                .cmp(&b.list.name)
                .then(a.list.id.cmp(&b.list.id))
        });
        for contents in ordered {
            if contents.list.is_owned() {
                owned.extend(contents.books.iter().map(|b| b.book.id));
                continue;
            }
            let name = shelf_name(&contents.list);
            let mut seen = HashSet::new();
            for (ix, book) in contents.books.iter().enumerate() {
                // A book on a list in two editions is one shelf entry.
                if seen.insert(book.book.id) {
                    by_book
                        .entry(book.book.id)
                        .or_default()
                        .push((name.clone(), ix + 1));
                }
            }
        }
        Self { by_book, owned }
    }
}

/// A list's name as a Goodreads shelf name: lower case, runs of anything but
/// letters and digits turned into one hyphen (so no comma survives). Persian
/// and other scripts are kept, the zero-width non-joiner of Persian words too.
pub(super) fn shelf_name(list: &UserList) -> String {
    let slug = slugify(&list.name);
    if !slug.is_empty() {
        return slug;
    }
    match list.slug.as_deref().map(slugify) {
        Some(slug) if !slug.is_empty() => slug,
        _ => format!("list-{}", list.id),
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_alphanumeric() || c == '\u{200c}' {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Each entry's position on its status shelf: the order the books were
/// added, as Goodreads numbers its shelves.
fn status_positions(entries: &[LibraryEntry]) -> HashMap<crate::api::UserBookId, usize> {
    let mut sorted: Vec<&LibraryEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| (e.date_added, e.user_book_id));
    let mut counts: HashMap<ReadingStatus, usize> = HashMap::new();
    sorted
        .into_iter()
        .map(|e| {
            let n = counts.entry(e.status).or_default();
            *n += 1;
            (e.user_book_id, *n)
        })
        .collect()
}

fn row(
    backup: &Backup,
    entry: &LibraryEntry,
    shelves: &ListShelves,
    positions: &HashMap<crate::api::UserBookId, usize>,
) -> Vec<Cow<'static, str>> {
    let book = &entry.book;
    let edition = entry.edition.as_ref();
    let exclusive = exclusive_shelf(entry.status).unwrap_or_default();
    let review = backup.reviews.get(&entry.user_book_id);
    let review_text = review.and_then(|r| r.text());
    let isbn_13: Option<String> = edition
        .and_then(|e| e.isbn_13.as_deref())
        .map(|s| s.chars().filter(|c| !matches!(c, '-' | ' ')).collect())
        .filter(|s: &String| !s.is_empty());
    let isbn_13 = isbn_13.as_deref();

    let mut names: Vec<String> = Vec::new();
    let mut with_positions: Vec<String> = Vec::new();
    if entry.status != ReadingStatus::Read {
        names.push(exclusive.to_owned());
        let position = positions.get(&entry.user_book_id).copied().unwrap_or(1);
        with_positions.push(format!("{exclusive} (#{position})"));
    }
    for (name, position) in shelves.by_book.get(&book.id).into_iter().flatten() {
        if !names.contains(name) {
            names.push(name.clone());
            with_positions.push(format!("{name} (#{position})"));
        }
    }

    let author = book.authors.first().map(|a| a.trim()).unwrap_or_default();
    let finished: Vec<Date> = entry.reads.iter().filter_map(|r| r.finished_at).collect();

    let s = |v: String| Cow::Owned(v);
    let opt = |v: Option<String>| Cow::Owned(v.unwrap_or_default());
    vec![
        s(book.id.to_string()),
        s(book.title.clone()),
        s(author.to_owned()),
        s(last_first(author)),
        s(book
            .authors
            .iter()
            .skip(1)
            .map(|a| a.trim())
            .collect::<Vec<_>>()
            .join(", ")),
        s(excel_text(isbn_13.and_then(isbn_10).as_deref())),
        s(excel_text(isbn_13)),
        s(whole_stars(entry.rating.map(|r| r.value())).to_string()),
        opt(book.average_rating.map(|r| format!("{r:.2}"))),
        opt(edition.and_then(|e| e.publisher.clone())),
        opt(edition.and_then(|e| e.format.clone().or_else(|| e.reading_format.clone()))),
        opt(edition
            .and_then(|e| e.pages)
            .or(book.pages)
            .map(|p| p.to_string())),
        opt(edition
            .and_then(|e| e.release_date)
            .map(|d| d.year().to_string())),
        opt(book.release_year.map(|y| y.to_string())),
        opt(finished.iter().max().map(|d| date(*d))),
        opt(entry.date_added.map(date)),
        s(names.join(", ")),
        s(with_positions.join(", ")),
        Cow::Borrowed(exclusive),
        opt(review_text.map(str::to_owned)),
        Cow::Borrowed(
            if review_text.is_some() && review.is_some_and(|r| r.has_spoilers) {
                "true"
            } else {
                ""
            },
        ),
        opt(entry.private_notes.clone()),
        s(finished.len().to_string()),
        Cow::Borrowed(if shelves.owned.contains(&book.id) {
            "1"
        } else {
            "0"
        }),
    ]
}

/// `YYYY/MM/DD`.
pub(super) fn date(d: Date) -> String {
    d.format("%Y/%m/%d").to_string()
}

/// "Last, First": split at the last space, since Hardcover keeps no separate
/// surname ("Aaron A. Reed" → "Reed, Aaron A.").
pub(super) fn last_first(name: &str) -> String {
    match name.trim().rsplit_once(' ') {
        Some((first, last)) => format!("{last}, {}", first.trim_end()),
        None => name.trim().to_owned(),
    }
}

/// `="…"`: a formula a spreadsheet shows as text, keeping leading zeros.
pub(super) fn excel_text(value: Option<&str>) -> String {
    format!("=\"{}\"", value.unwrap_or_default())
}

/// The ISBN-10 of a `978` ISBN-13; `None` for any other.
pub(super) fn isbn_10(isbn_13: &str) -> Option<String> {
    let digits: Vec<u32> = isbn_13
        .chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_digit(10))
        .collect::<Option<_>>()?;
    if digits.len() != 13 || digits[..3] != [9, 7, 8] {
        return None;
    }
    let body = &digits[3..12];
    let sum: u32 = body.iter().zip((2..=10).rev()).map(|(d, w)| d * w).sum();
    let check = match (11 - sum % 11) % 11 {
        10 => 'X',
        n => char::from_digit(n, 10)?,
    };
    let mut out: String = body
        .iter()
        .filter_map(|d| char::from_digit(*d, 10))
        .collect();
    out.push(check);
    Some(out)
}

fn write_row<'a>(out: &mut String, fields: impl Iterator<Item = Cow<'a, str>>) {
    for (ix, field) in fields.enumerate() {
        if ix > 0 {
            out.push(',');
        }
        write_field(out, &field);
    }
    out.push('\n');
}

/// One CSV field (RFC 4180): quoted when it holds a comma, a quote or a line
/// break, with quotes doubled.
pub(super) fn write_field(out: &mut String, field: &str) {
    if field.contains([',', '"', '\n', '\r']) {
        let _ = write!(out, "\"{}\"", field.replace('"', "\"\""));
    } else {
        out.push_str(field);
    }
}
