//! What the Series destination shows, worked out from the library, the
//! series' books and the date: which series count as started, which books
//! of each show and in what state, the one next step, the group and the
//! order. Plain functions, so every rule is pinned by a test.
//!
//! - **Started:** a series of a library entry (not Ignored) that is Read
//!   with a finished read, Currently reading, Paused or Did not finish. Only
//!   Want to read books don't start a series.
//! - **Books shown:** whole-numbered ones, plus fractional ones (novellas)
//!   only when they're in the library or not out yet, and split parts
//!   (`is_partial_book`) only when they're in the library. Of several at one position, the one in the library, else
//!   the most-read. A library entry the series doesn't list by id (an
//!   unmerged duplicate) still takes its position from the entry's own
//!   series.
//! - **Out yet:** not after today by Hardcover's date or the upcoming books
//!   (`UpcomingModel`), and not dated to a later year. A book the reader has
//!   read, is reading, paused or gave up on counts as out for them.
//! - **The next step:** the first book that is out and isn't Read or Did not
//!   finish; with none, the first book not out yet.
//! - **Groups:** "In progress" (a book is being read, or the next book is out
//!   and the series was touched within a year), "Waiting for the next book",
//!   "Not touched in a year" and "Finished"; within each the most recent
//!   activity first, and the waiting ones by the release date.

use std::collections::HashMap;
use std::ops::Range;

use chrono::Datelike as _;
use hardcover_core::{
    BookId, BookSummary, Date, LibraryEntry, ReadingStatus, SeriesId, TrackedSeries, UserBookId,
};

use crate::format::{ProgressSummary, count_text, percent_label, series_position_text, short_date};
use crate::text::{indexed_words, search_words};

/// A series not touched for longer than this is "Not touched in a year".
const UNTOUCHED_DAYS: i64 = 365;

// ----- started series -----

/// Whether an entry starts its series: read through, being read, paused or
/// given up on.
fn starts_series(entry: &LibraryEntry) -> bool {
    match entry.status {
        ReadingStatus::Read => entry
            .reads
            .iter()
            .any(hardcover_core::ReadSession::is_finished),
        ReadingStatus::CurrentlyReading | ReadingStatus::Paused | ReadingStatus::DidNotFinish => {
            true
        }
        ReadingStatus::WantToRead | ReadingStatus::Ignored => false,
    }
}

/// The series to ask about, sorted, each once.
pub fn started_series(entries: &[LibraryEntry]) -> Vec<SeriesId> {
    let mut ids: Vec<SeriesId> = entries
        .iter()
        .filter(|e| starts_series(e))
        .flat_map(|e| e.series.iter().filter_map(|s| s.id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

// ----- one series -----

/// One book of a series as the reader sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesBook {
    /// With the cover the library shows, when the book is in it.
    pub book: BookSummary,
    pub position: f32,
    /// The library entry, never an Ignored one.
    pub entry: Option<UserBookId>,
    pub status: Option<ReadingStatus>,
    /// Out, or as good as out for this reader (see the module doc).
    pub out: bool,
    pub release_date: Option<Date>,
    /// The open read's progress, 0..=100, for a book being read.
    pub progress: Option<f32>,
}

impl SeriesBook {
    /// Read or given up on: the next step is past it.
    pub fn is_done(&self) -> bool {
        matches!(
            self.status,
            Some(ReadingStatus::Read | ReadingStatus::DidNotFinish)
        )
    }
}

/// What a book looks like in the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Its cover, with its shelf's icon.
    Shelved(ReadingStatus),
    /// Not in the library: an empty slot.
    Missing,
    /// Not out yet: a slot with a clock.
    Unreleased,
}

impl SeriesBook {
    pub fn mark(&self) -> Mark {
        match self.status {
            Some(status) if status != ReadingStatus::WantToRead => Mark::Shelved(status),
            _ if !self.out => Mark::Unreleased,
            Some(status) => Mark::Shelved(status),
            None => Mark::Missing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    InProgress,
    Waiting,
    Untouched,
    Finished,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::InProgress => "In progress",
            Group::Waiting => "Waiting for the next book",
            Group::Untouched => "Not touched in a year",
            Group::Finished => "Finished",
        }
    }

    pub fn is_finished(self) -> bool {
        self == Group::Finished
    }
}

/// The next step's kind, which decides the lane's words and command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextKind {
    /// Currently reading: its progress and "Update progress…".
    Reading,
    /// Paused: where it stopped and "Update progress…".
    Paused,
    /// On Want to read: "Start reading".
    Start,
    /// Out but not in the library: "+ Want to read".
    Add,
    /// Not out yet, every book that is out done: its date.
    Waiting,
}

/// The line about what last happened in the series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    ReadingNow,
    PausedSince(Date),
    CaughtUpSince(Date),
    FinishedOn(Date),
    LastFinished(Date),
    Started(Date),
    None,
}

impl Activity {
    /// "Reading now", "Last finished Sep 28", "Caught up since Feb 2",
    /// "Paused since Jun 2, 2024", "Finished Jan 12, 2025".
    pub fn text(self, today: Date) -> Option<String> {
        let date = |d| short_date(d, today);
        Some(match self {
            Activity::ReadingNow => "Reading now".to_owned(),
            Activity::PausedSince(d) => format!("Paused since {}", date(d)),
            Activity::CaughtUpSince(d) => format!("Caught up since {}", date(d)),
            Activity::FinishedOn(d) => format!("Finished {}", date(d)),
            Activity::LastFinished(d) => format!("Last finished {}", date(d)),
            Activity::Started(d) => format!("Started {}", date(d)),
            Activity::None => return None,
        })
    }
}

/// A started series, ready to show.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesSummary {
    pub id: SeriesId,
    pub name: String,
    pub author: Option<String>,
    /// In reading order.
    pub books: Vec<SeriesBook>,
    /// Index into `books`.
    pub next: Option<usize>,
    pub group: Group,
    pub activity: Activity,
    /// The latest finish, start or update of the series' entries.
    pub last_activity: Option<Date>,
}

impl SeriesSummary {
    pub fn next_book(&self) -> Option<&SeriesBook> {
        self.next.map(|ix| &self.books[ix])
    }

    pub fn next_kind(&self) -> Option<NextKind> {
        let book = self.next_book()?;
        Some(match book.status {
            Some(ReadingStatus::CurrentlyReading) => NextKind::Reading,
            Some(ReadingStatus::Paused) => NextKind::Paused,
            _ if !book.out => NextKind::Waiting,
            Some(_) => NextKind::Start,
            None => NextKind::Add,
        })
    }

    pub fn read_count(&self) -> usize {
        self.books
            .iter()
            .filter(|b| b.status == Some(ReadingStatus::Read))
            .count()
    }

    pub fn out_count(&self) -> usize {
        self.books.iter().filter(|b| b.out).count()
    }

    /// The first book not out yet.
    pub fn first_unreleased(&self) -> Option<&SeriesBook> {
        self.books.iter().find(|b| !b.out)
    }

    /// "2 of 6 read · #6 not out yet · Last finished Sep 28": the facts after
    /// the author. "Not out yet" is left out where the next step already
    /// says it.
    pub fn facts(&self, today: Date) -> String {
        let mut parts = vec![format!(
            "{} of {} read",
            count_text(self.read_count()),
            count_text(self.out_count())
        )];
        if self.group != Group::Waiting
            && let Some(book) = self.first_unreleased()
        {
            parts.push(format!(
                "{} not out yet",
                series_position_text(book.position)
            ));
        }
        parts.extend(self.activity.text(today));
        parts.join(" · ")
    }

    /// The release date of the next step, when it is still to come.
    fn waiting_date(&self) -> Option<Date> {
        self.next_book()
            .filter(|b| !b.out)
            .and_then(|b| b.release_date)
    }
}

/// The library, indexed for the series.
pub struct LibraryIndex<'a> {
    by_book: HashMap<BookId, &'a LibraryEntry>,
    by_series: HashMap<SeriesId, Vec<&'a LibraryEntry>>,
}

impl<'a> LibraryIndex<'a> {
    /// Every entry but the Ignored ones, which count for nothing.
    pub fn new(entries: &'a [LibraryEntry]) -> Self {
        let mut by_book = HashMap::new();
        let mut by_series: HashMap<SeriesId, Vec<&LibraryEntry>> = HashMap::new();
        for e in entries
            .iter()
            .filter(|e| e.status != ReadingStatus::Ignored)
        {
            by_book.insert(e.book.id, e);
            for id in e.series.iter().filter_map(|s| s.id) {
                by_series.entry(id).or_default().push(e);
            }
        }
        Self { by_book, by_series }
    }
}

/// Whether a book is out on `today`: not dated after it by Hardcover or by
/// the upcoming books, and not dated to a later year.
fn released(
    release_date: Option<Date>,
    release_year: Option<i32>,
    upcoming: Option<Date>,
    today: Date,
) -> bool {
    match upcoming.or(release_date) {
        Some(date) => date <= today,
        None => release_year.is_none_or(|y| y <= today.year()),
    }
}

/// How far along an entry's status is, to pick between two entries at one
/// position.
fn shelf_rank(status: ReadingStatus) -> u8 {
    match status {
        ReadingStatus::Read => 0,
        ReadingStatus::CurrentlyReading => 1,
        ReadingStatus::Paused => 2,
        ReadingStatus::DidNotFinish => 3,
        ReadingStatus::WantToRead => 4,
        ReadingStatus::Ignored => 5,
    }
}

/// Every date in an entry that says the reader did something.
fn entry_dates(entry: &LibraryEntry) -> impl Iterator<Item = Date> + '_ {
    entry
        .reads
        .iter()
        .flat_map(|r| [r.started_at, r.finished_at])
        .chain([entry.updated_at.map(|t| t.date_naive())])
        .flatten()
}

fn latest_finish<'a>(entries: impl Iterator<Item = &'a LibraryEntry>) -> Option<Date> {
    entries
        .flat_map(|e| e.reads.iter().filter_map(|r| r.finished_at))
        .max()
}

fn series_books(
    series: &TrackedSeries,
    library: &LibraryIndex,
    series_entries: &[&LibraryEntry],
    upcoming: &HashMap<BookId, Date>,
    today: Date,
) -> Vec<SeriesBook> {
    struct Candidate<'a> {
        book: BookSummary,
        position: f32,
        release_date: Option<Date>,
        partial: bool,
        entry: Option<&'a LibraryEntry>,
    }
    let mut candidates: Vec<Candidate> = series
        .books
        .iter()
        .map(|b| Candidate {
            book: b.book.clone(),
            position: b.position,
            release_date: b.release_date,
            partial: b.partial,
            entry: library.by_book.get(&b.book.id).copied(),
        })
        .collect();
    // An entry the series doesn't list by id keeps its own position.
    for entry in series_entries {
        if candidates.iter().any(|c| c.book.id == entry.book.id) {
            continue;
        }
        let position = entry
            .series
            .iter()
            .find(|s| s.id == Some(series.id))
            .and_then(|s| s.position);
        if let Some(position) = position {
            candidates.push(Candidate {
                book: entry.book.clone(),
                position,
                release_date: None,
                partial: false,
                entry: Some(entry),
            });
        }
    }

    let mut books: Vec<SeriesBook> = Vec::new();
    let mut chosen: Vec<(u8, u32)> = Vec::new();
    for c in candidates {
        let status = c.entry.map(|e| e.status);
        let out = released(
            c.release_date,
            c.book.release_year,
            upcoming.get(&c.book.id).copied(),
            today,
        ) || status.is_some_and(|s| s != ReadingStatus::WantToRead);
        let in_library = c.entry.is_some();
        let whole = c.position.fract() == 0.0;
        if c.partial && !in_library {
            continue;
        }
        if !whole && !in_library && out {
            continue;
        }
        // In the library first, the furthest shelf first; then the most-read.
        let rank = (
            status.map_or(u8::MAX, shelf_rank),
            u32::MAX - c.book.users_count.unwrap_or(0),
        );
        let mut book = c.book;
        if let Some(cover) = c.entry.and_then(|e| e.cover()) {
            book.cover = Some(cover.clone());
        }
        let progress = c
            .entry
            .filter(|e| e.status == ReadingStatus::CurrentlyReading)
            .and_then(|e| crate::format::progress_summary(e).percent);
        let release_date = upcoming.get(&book.id).copied().or(c.release_date);
        let candidate = SeriesBook {
            book,
            position: c.position,
            entry: c.entry.map(|e| e.user_book_id),
            status,
            out,
            release_date,
            progress,
        };
        #[expect(
            clippy::float_cmp,
            reason = "positions are catalogue keys; nearby volumes must remain distinct"
        )]
        match books.iter().position(|b| b.position == c.position) {
            Some(ix) if rank < chosen[ix] => {
                books[ix] = candidate;
                chosen[ix] = rank;
            }
            Some(_) => {}
            None => {
                books.push(candidate);
                chosen.push(rank);
            }
        }
    }
    books.sort_by(|a, b| a.position.total_cmp(&b.position));

    books
}

/// The series as the reader sees it. `upcoming` holds the release dates of
/// the books not out yet that `UpcomingModel` knows.
pub fn summarize(
    series: &TrackedSeries,
    library: &LibraryIndex,
    upcoming: &HashMap<BookId, Date>,
    today: Date,
) -> SeriesSummary {
    let series_entries: &[&LibraryEntry] = library
        .by_series
        .get(&series.id)
        .map_or_default(Vec::as_slice);
    let books = series_books(series, library, series_entries, upcoming, today);
    let next = books
        .iter()
        .position(|b| b.out && !b.is_done())
        .or_else(|| books.iter().position(|b| !b.out && !b.is_done()));
    let reading = books
        .iter()
        .any(|b| b.status == Some(ReadingStatus::CurrentlyReading));
    let entries = || series_entries.iter().copied();
    let last_activity = entries().flat_map(entry_dates).max();
    let touched_lately = last_activity.is_none_or(|d| (today - d).num_days() <= UNTOUCHED_DAYS);
    let next_out = next.map(|ix| books[ix].out);
    let group = match next_out {
        _ if reading => Group::InProgress,
        Some(true) if touched_lately => Group::InProgress,
        Some(true) => Group::Untouched,
        Some(false) => Group::Waiting,
        None => Group::Finished,
    };
    let finished = latest_finish(entries());
    let paused_since = next
        .map(|ix| &books[ix])
        .filter(|b| b.status == Some(ReadingStatus::Paused))
        .and_then(|b| entries().find(|e| Some(e.user_book_id) == b.entry))
        .and_then(|e| {
            e.updated_at
                .map(|t| t.date_naive())
                .or_else(|| e.open_read().and_then(|r| r.started_at))
        });
    let started = entries()
        .flat_map(|e| e.reads.iter().filter_map(|r| r.started_at))
        .max();
    let activity = if reading {
        Activity::ReadingNow
    } else if let Some(since) = paused_since {
        Activity::PausedSince(since)
    } else {
        match (group, finished.or(last_activity)) {
            (Group::Waiting, Some(d)) => Activity::CaughtUpSince(d),
            (Group::Finished, Some(d)) => Activity::FinishedOn(d),
            _ => match (finished, started) {
                (Some(d), _) => Activity::LastFinished(d),
                (None, Some(d)) => Activity::Started(d),
                (None, None) => Activity::None,
            },
        }
    };

    SeriesSummary {
        id: series.id,
        name: series.name.clone(),
        author: series
            .author
            .as_ref()
            .map(|a| a.name.clone())
            .or_else(|| books.iter().find_map(|b| b.book.authors.first().cloned())),
        books,
        next,
        group,
        activity,
        last_activity,
    }
}

/// Every started series in `series`, in group order, each group sorted.
/// A series none of whose books has a position (a publisher's line such as
/// "Grokking", where Hardcover numbers nothing) has no order to show and is
/// left out; it would otherwise read as finished, with an empty strip.
pub fn summaries(
    series: &[TrackedSeries],
    entries: &[LibraryEntry],
    upcoming: &HashMap<BookId, Date>,
    today: Date,
) -> Vec<SeriesSummary> {
    let started = started_series(entries);
    let library = LibraryIndex::new(entries);
    let mut out: Vec<SeriesSummary> = series
        .iter()
        .filter(|s| started.binary_search(&s.id).is_ok())
        .map(|s| summarize(s, &library, upcoming, today))
        .filter(|s| !s.books.is_empty())
        .collect();
    out.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| match a.group {
                // The soonest first; a book with no date last.
                Group::Waiting => match (a.waiting_date(), b.waiting_date()) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (x, y) => x.is_none().cmp(&y.is_none()),
                },
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| b.last_activity.cmp(&a.last_activity))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.cmp(&b.id))
    });
    out
}

// ----- the filters -----

/// The header's select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Filter {
    #[default]
    NotFinished,
    Finished,
    All,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::NotFinished, Filter::Finished, Filter::All];

    pub fn label(self) -> &'static str {
        match self {
            Filter::NotFinished => "Not finished",
            Filter::Finished => "Finished",
            Filter::All => "All series",
        }
    }

    pub fn admits(self, group: Group) -> bool {
        match self {
            Filter::NotFinished => !group.is_finished(),
            Filter::Finished => group.is_finished(),
            Filter::All => true,
        }
    }
}

/// Whether every word of `query` starts a word of the series' name or
/// author. A blank query matches everything.
pub fn matches_query(series: &SeriesSummary, query: &[String]) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut haystack = indexed_words(&series.name);
    if let Some(author) = &series.author {
        haystack.extend(indexed_words(author));
    }
    query
        .iter()
        .all(|q| haystack.iter().any(|w| w.starts_with(q.as_str())))
}

/// How many series each filter shows, given the text filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub not_finished: usize,
    pub finished: usize,
}

impl Counts {
    pub fn of(&self, filter: Filter) -> usize {
        match filter {
            Filter::NotFinished => self.not_finished,
            Filter::Finished => self.finished,
            Filter::All => self.not_finished + self.finished,
        }
    }
}

pub fn counts<'a>(series: impl IntoIterator<Item = &'a SeriesSummary>, query: &str) -> Counts {
    let query = search_words(query);
    let mut counts = Counts::default();
    for s in series.into_iter().filter(|s| matches_query(s, &query)) {
        if s.group.is_finished() {
            counts.finished += 1;
        } else {
            counts.not_finished += 1;
        }
    }
    counts
}

/// The series to show, under their groups, in order.
pub fn grouped<'a>(
    series: &'a [SeriesSummary],
    filter: Filter,
    query: &str,
) -> Vec<(Group, Vec<&'a SeriesSummary>)> {
    let query = search_words(query);
    let mut groups: Vec<(Group, Vec<&SeriesSummary>)> = Vec::new();
    for s in series
        .iter()
        .filter(|s| filter.admits(s.group) && matches_query(s, &query))
    {
        match groups.last_mut() {
            Some((g, rows)) if *g == s.group => rows.push(s),
            _ => groups.push((s.group, vec![s])),
        }
    }
    groups
}

/// "1 finished series isn't shown.", "3 finished series aren't shown."
pub fn hidden_line(finished: usize) -> String {
    match finished {
        1 => "1 finished series isn't shown.".to_owned(),
        n => format!("{} finished series aren't shown.", count_text(n)),
    }
}

// ----- the strip and the lane -----

/// The books a strip with room for `capacity` slots shows, and how many
/// after them it leaves out (its "+N"). A strip that can't show every book
/// keeps the next one in view, with the book after it, and keeps a slot's
/// room for the "+N".
pub fn strip_window(len: usize, next: Option<usize>, capacity: usize) -> (Range<usize>, usize) {
    let capacity = capacity.max(1);
    if len <= capacity {
        return (0..len, 0);
    }
    let slots = (capacity - 1).max(1);
    let next = next.unwrap_or(0).min(len - 1);
    if next < slots {
        return (0..slots, len - slots);
    }
    let end = (next + 2).min(len);
    if end == len {
        return (len - capacity..len, 0);
    }
    (end - slots..end, len - end)
}

/// The lane's caption: "Reading · #1", "Paused · #3", "Next · #3".
pub fn lane_caption(kind: NextKind, position: f32) -> String {
    let head = match kind {
        NextKind::Reading => "Reading",
        NextKind::Paused => "Paused",
        NextKind::Start | NextKind::Add | NextKind::Waiting => "Next",
    };
    format!("{head} · {}", series_position_text(position))
}

/// A book being read: "Page 418 of 1,007 · 42%". `None` with nothing to
/// say.
pub fn reading_line(progress: &ProgressSummary) -> Option<String> {
    let position =
        (progress.page.is_some() || progress.seconds.is_some()).then(|| progress.text());
    let percent = progress.percent.map(percent_label);
    match (position, percent) {
        (Some(a), Some(b)) => Some(format!("{a} · {b}")),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{
        AuthorId, AuthorRef, ReadId, ReadSession, SeriesRef, TrackedBook, UserId,
    };

    fn day(s: &str) -> Date {
        s.parse().unwrap()
    }

    const TODAY: &str = "2026-11-20";

    fn book(id: i64, users: u32) -> BookSummary {
        let mut b = BookSummary::new(BookId::new(id), format!("Book {id}"));
        b.users_count = Some(users);
        b.release_year = Some(2020);
        b
    }

    fn tracked(id: i64, name: &str, books: &[(i64, f32)]) -> TrackedSeries {
        let mut s = TrackedSeries::new(SeriesId::new(id), name);
        s.author = Some(AuthorRef::new(AuthorId::new(1), "Ada Quill"));
        for (book_id, position) in books {
            let mut b = TrackedBook::new(book(*book_id, 100), *position);
            b.release_date = Some(day("2020-01-01"));
            s.books.push(b);
        }
        s
    }

    fn entry(book_id: i64, status: ReadingStatus, series: &[(i64, f32)]) -> LibraryEntry {
        let mut e = LibraryEntry::new(
            UserBookId::new(book_id * 10),
            UserId::new(1),
            book(book_id, 100),
            status,
        );
        for (id, position) in series {
            let mut r = SeriesRef::new(format!("Series {id}"));
            r.id = Some(SeriesId::new(*id));
            r.position = Some(*position);
            e.series.push(r);
        }
        e
    }

    fn finished(mut e: LibraryEntry, on: &str) -> LibraryEntry {
        let mut r = ReadSession::new(ReadId::new(e.user_book_id.get()));
        r.started_at = Some(day(on) - chrono::Days::new(10));
        r.finished_at = Some(day(on));
        e.reads.push(r);
        e
    }

    fn reading(mut e: LibraryEntry, started: &str, page: u32) -> LibraryEntry {
        let mut r = ReadSession::new(ReadId::new(e.user_book_id.get()));
        r.started_at = Some(day(started));
        r.progress_pages = Some(page);
        e.reads.push(r);
        e.book.pages = Some(400);
        e
    }

    fn summary(series: &TrackedSeries, entries: &[LibraryEntry]) -> SeriesSummary {
        summarize(
            series,
            &LibraryIndex::new(entries),
            &HashMap::new(),
            day(TODAY),
        )
    }

    fn positions(s: &SeriesSummary) -> Vec<f32> {
        s.books.iter().map(|b| b.position).collect()
    }

    #[test]
    fn a_series_starts_with_a_finished_read_a_current_one_a_pause_or_a_dnf() {
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-01-01"),
            // Read, but no finished read: doesn't start it.
            entry(2, ReadingStatus::Read, &[(20, 1.0)]),
            entry(3, ReadingStatus::CurrentlyReading, &[(30, 1.0)]),
            entry(4, ReadingStatus::Paused, &[(40, 1.0)]),
            entry(5, ReadingStatus::DidNotFinish, &[(50, 1.0)]),
            entry(6, ReadingStatus::WantToRead, &[(60, 1.0)]),
            finished(entry(7, ReadingStatus::Ignored, &[(70, 1.0)]), "2026-01-01"),
            entry(8, ReadingStatus::CurrentlyReading, &[(10, 2.0), (80, 1.0)]),
        ];
        let ids: Vec<i64> = started_series(&entries).iter().map(|s| s.get()).collect();
        assert_eq!(ids, [10, 30, 40, 50, 80]);
    }

    #[test]
    fn a_series_with_no_numbered_book_is_left_out() {
        // Hardcover lists no positioned book, and the reader's own has no position.
        let unnumbered = tracked(90, "Grokking", &[]);
        let mut e = finished(entry(9, ReadingStatus::Read, &[(90, 1.0)]), "2026-03-01");
        e.series[0].position = None;
        let numbered = tracked(10, "Lantern", &[(1, 1.0)]);
        let entries = [
            e,
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-03-01"),
        ];
        let out = summaries(
            &[unnumbered, numbered],
            &entries,
            &HashMap::new(),
            day(TODAY),
        );
        let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Lantern"]);
    }

    #[test]
    fn the_next_step_skips_read_and_dnf_books_and_stops_at_a_gap() {
        let series = tracked(10, "Lantern", &[(1, 1.0), (2, 2.0), (3, 3.0), (4, 4.0)]);
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-10-01"),
            entry(2, ReadingStatus::DidNotFinish, &[(10, 2.0)]),
            // #3 isn't in the library: a gap.
            finished(entry(4, ReadingStatus::Read, &[(10, 4.0)]), "2026-10-05"),
        ];
        let s = summary(&series, &entries);
        assert_eq!(s.next, Some(2));
        assert_eq!(s.next_kind(), Some(NextKind::Add));
        assert_eq!(s.books[2].mark(), Mark::Missing);
        assert_eq!(
            s.books[1].mark(),
            Mark::Shelved(ReadingStatus::DidNotFinish)
        );
        assert_eq!(s.group, Group::InProgress);
        // A DNF book is done but not read.
        assert_eq!(s.read_count(), 2);
        assert_eq!(s.out_count(), 4);
        assert_eq!(s.facts(day(TODAY)), "2 of 4 read · Last finished Oct 5");
    }

    #[test]
    fn reading_past_a_gap_is_in_progress_but_the_gap_is_next() {
        let series = tracked(10, "Lantern", &[(1, 1.0), (2, 2.0), (3, 3.0)]);
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-10-01"),
            reading(
                entry(3, ReadingStatus::CurrentlyReading, &[(10, 3.0)]),
                "2026-11-01",
                100,
            ),
        ];
        let s = summary(&series, &entries);
        assert_eq!(s.next, Some(1));
        assert_eq!(s.next_kind(), Some(NextKind::Add));
        assert_eq!(s.group, Group::InProgress);
        assert_eq!(s.activity, Activity::ReadingNow);
        assert_eq!(s.books[2].progress, Some(25.0));
    }

    #[test]
    fn novellas_and_split_parts_show_only_in_the_library_or_before_release() {
        let mut series = tracked(
            10,
            "Lantern",
            &[
                (1, 1.0),
                (15, 1.5),
                (2, 2.0),
                (25, 2.5),
                (3, 3.0),
                (35, 3.5),
            ],
        );
        // #3.5 isn't out yet.
        series.books[5].release_date = Some(day("2027-02-01"));
        series.books[5].book.release_year = Some(2027);
        // A split part at a whole position, not in the library.
        let mut part = TrackedBook::new(book(31, 100_000), 3.0);
        part.partial = true;
        series.books.push(part);
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-10-01"),
            entry(25, ReadingStatus::WantToRead, &[(10, 2.5)]),
        ];
        let s = summary(&series, &entries);
        assert_eq!(positions(&s), [1.0, 2.0, 2.5, 3.0, 3.5]);
        assert_eq!(s.books[3].book.id, BookId::new(3), "the part is left out");
        assert_eq!(s.books[4].mark(), Mark::Unreleased);
        assert_eq!(
            s.facts(day(TODAY)),
            "1 of 4 read · #3.5 not out yet · Last finished Oct 1"
        );
    }

    #[test]
    fn of_several_books_at_one_position_the_library_wins_then_the_most_read() {
        let mut series = tracked(10, "Lantern", &[(1, 1.0), (2, 2.0)]);
        // A translation at #1 with more readers; a duplicate at #2 with more.
        let mut popular = TrackedBook::new(book(11, 5_000), 1.0);
        popular.release_date = Some(day("2020-01-01"));
        series.books.insert(0, popular);
        let mut more = TrackedBook::new(book(21, 5_000), 2.0);
        more.release_date = Some(day("2020-01-01"));
        series.books.push(more);
        let entries = [finished(
            entry(1, ReadingStatus::Read, &[(10, 1.0)]),
            "2026-10-01",
        )];
        let s = summary(&series, &entries);
        let ids: Vec<i64> = s.books.iter().map(|b| b.book.id.get()).collect();
        assert_eq!(ids, [1, 21]);
    }

    #[test]
    fn an_entry_the_series_doesnt_list_keeps_its_own_position() {
        let series = tracked(10, "Lantern", &[(1, 1.0), (2, 2.0)]);
        // An unmerged duplicate of #1 that the series doesn't list.
        let entries = [finished(
            entry(9, ReadingStatus::Read, &[(10, 1.0)]),
            "2026-10-01",
        )];
        let s = summary(&series, &entries);
        assert_eq!(s.books[0].book.id, BookId::new(9));
        assert_eq!(s.next, Some(1));
    }

    #[test]
    fn a_book_in_two_series_counts_in_both() {
        let saga = tracked(10, "Saga", &[(1, 1.0), (2, 2.0)]);
        let order = tracked(20, "Reading order", &[(5, 1.0), (2, 2.0)]);
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-10-01"),
            finished(
                entry(2, ReadingStatus::Read, &[(10, 2.0), (20, 2.0)]),
                "2026-10-09",
            ),
        ];
        let all = summaries(&[saga, order], &entries, &HashMap::new(), day(TODAY));
        let saga = all.iter().find(|s| s.id == SeriesId::new(10)).unwrap();
        let order = all.iter().find(|s| s.id == SeriesId::new(20)).unwrap();
        assert_eq!(saga.group, Group::Finished);
        assert_eq!(saga.activity, Activity::FinishedOn(day("2026-10-09")));
        assert_eq!(order.next, Some(0));
        assert_eq!(order.read_count(), 1);
    }

    #[test]
    fn an_unreleased_book_makes_a_caught_up_series_wait() {
        let mut series = tracked(10, "Lantern", &[(1, 1.0), (2, 2.0), (3, 3.0)]);
        // #3 has no date in the series; the upcoming books know it.
        series.books[2].release_date = None;
        series.books[2].book.release_year = None;
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-01-10"),
            finished(entry(2, ReadingStatus::Read, &[(10, 2.0)]), "2026-02-02"),
            entry(3, ReadingStatus::WantToRead, &[(10, 3.0)]),
        ];
        let upcoming = HashMap::from([(BookId::new(3), day("2026-11-26"))]);
        let s = summarize(&series, &LibraryIndex::new(&entries), &upcoming, day(TODAY));
        assert_eq!(s.group, Group::Waiting);
        assert_eq!(s.next_kind(), Some(NextKind::Waiting));
        assert_eq!(s.books[2].release_date, Some(day("2026-11-26")));
        assert_eq!(s.books[2].mark(), Mark::Unreleased);
        assert_eq!(s.activity, Activity::CaughtUpSince(day("2026-02-02")));
        assert_eq!(s.facts(day(TODAY)), "2 of 2 read · Caught up since Feb 2");

        // A later year counts as not out, without a date.
        let mut later = tracked(20, "Later", &[(1, 1.0), (2, 2.0)]);
        later.books[1].release_date = None;
        later.books[1].book.release_year = Some(2027);
        let entries = [finished(
            entry(1, ReadingStatus::Read, &[(20, 1.0)]),
            "2026-01-10",
        )];
        assert_eq!(summary(&later, &entries).group, Group::Waiting);
    }

    #[test]
    fn an_unreleased_novella_before_a_released_book_doesnt_hold_it_back() {
        let mut series = tracked(10, "Lantern", &[(1, 1.0), (15, 1.5), (2, 2.0)]);
        series.books[1].release_date = Some(day("2027-03-01"));
        let entries = [finished(
            entry(1, ReadingStatus::Read, &[(10, 1.0)]),
            "2026-10-01",
        )];
        let s = summary(&series, &entries);
        assert_eq!(s.next, Some(2));
        assert_eq!(s.group, Group::InProgress);
    }

    #[test]
    fn a_year_without_activity_is_untouched_and_a_day_less_is_not() {
        let series = tracked(10, "Lantern", &[(1, 1.0), (2, 2.0)]);
        // Finished exactly 365 days before today.
        let year_ago = finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2025-11-20");
        assert_eq!(summary(&series, &[year_ago]).group, Group::InProgress);
        let older = finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2025-11-19");
        let s = summary(&series, std::slice::from_ref(&older));
        assert_eq!(s.group, Group::Untouched);
        assert_eq!(s.activity, Activity::LastFinished(day("2025-11-19")));
        assert_eq!(
            s.facts(day(TODAY)),
            "1 of 2 read · Last finished Nov 19, 2025"
        );
        // A recent update to an entry of the series touches it.
        let mut touched = older;
        touched.updated_at = Some("2026-03-01T10:00:00Z".parse().unwrap());
        assert_eq!(summary(&series, &[touched]).group, Group::InProgress);
    }

    #[test]
    fn a_paused_book_is_the_next_step_and_says_since_when() {
        let series = tracked(10, "Kelidar", &[(1, 1.0), (2, 2.0), (3, 3.0)]);
        let mut paused = reading(
            entry(2, ReadingStatus::Paused, &[(10, 2.0)]),
            "2024-05-01",
            140,
        );
        paused.updated_at = Some("2024-06-02T10:00:00Z".parse().unwrap());
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2024-04-01"),
            paused,
        ];
        let s = summary(&series, &entries);
        assert_eq!(s.next_kind(), Some(NextKind::Paused));
        assert_eq!(s.group, Group::Untouched);
        assert_eq!(
            s.activity.text(day(TODAY)).unwrap(),
            "Paused since Jun 2, 2024"
        );
        assert_eq!(lane_caption(NextKind::Paused, 2.0), "Paused · #2");
    }

    #[test]
    fn ignored_entries_count_for_nothing() {
        let series = tracked(10, "Dune", &[(1, 1.0), (2, 2.0)]);
        let entries = [
            finished(entry(1, ReadingStatus::Read, &[(10, 1.0)]), "2026-10-01"),
            entry(2, ReadingStatus::Ignored, &[(10, 2.0)]),
        ];
        let s = summary(&series, &entries);
        assert_eq!(s.books[1].mark(), Mark::Missing);
        assert_eq!(s.next_kind(), Some(NextKind::Add));
    }

    #[test]
    fn groups_come_in_order_and_sort_by_activity_or_release() {
        let mk = |id: i64, finish: &str, next_date: Option<&str>| {
            let mut s = tracked(
                id,
                &format!("S{id}"),
                &[(id * 10 + 1, 1.0), (id * 10 + 2, 2.0)],
            );
            if let Some(d) = next_date {
                s.books[1].release_date = Some(day(d));
            }
            let e = finished(
                entry(id * 10 + 1, ReadingStatus::Read, &[(id, 1.0)]),
                finish,
            );
            (s, e)
        };
        let rows = [
            mk(1, "2026-03-01", None),
            mk(2, "2026-09-01", None),
            mk(3, "2026-01-01", Some("2027-05-01")),
            mk(4, "2026-08-01", Some("2026-12-01")),
            mk(5, "2023-01-01", None),
        ];
        let series: Vec<TrackedSeries> = rows.iter().map(|(s, _)| s.clone()).collect();
        let entries: Vec<LibraryEntry> = rows.iter().map(|(_, e)| e.clone()).collect();
        let all = summaries(&series, &entries, &HashMap::new(), day(TODAY));
        let order: Vec<(i64, Group)> = all.iter().map(|s| (s.id.get(), s.group)).collect();
        assert_eq!(
            order,
            [
                (2, Group::InProgress),
                (1, Group::InProgress),
                (4, Group::Waiting),
                (3, Group::Waiting),
                (5, Group::Untouched),
            ]
        );
    }

    #[test]
    fn the_filters_count_what_they_show() {
        let finished_series = tracked(1, "The Lantern Cycle", &[(11, 1.0)]);
        let going = tracked(2, "Salt Roads", &[(21, 1.0), (22, 2.0)]);
        let mut other = tracked(3, "Ember Tales", &[(31, 1.0), (32, 2.0)]);
        other.author = Some(AuthorRef::new(AuthorId::new(2), "Lantern Smith"));
        let entries = [
            finished(entry(11, ReadingStatus::Read, &[(1, 1.0)]), "2026-10-01"),
            finished(entry(21, ReadingStatus::Read, &[(2, 1.0)]), "2026-10-01"),
            finished(entry(31, ReadingStatus::Read, &[(3, 1.0)]), "2026-10-01"),
        ];
        let all = summaries(
            &[finished_series, going, other],
            &entries,
            &HashMap::new(),
            day(TODAY),
        );
        let c = counts(&all, "");
        assert_eq!(
            (
                c.of(Filter::NotFinished),
                c.of(Filter::Finished),
                c.of(Filter::All)
            ),
            (2, 1, 3)
        );
        // Every word must start a word of the name or the author.
        let c = counts(&all, "lant");
        assert_eq!((c.not_finished, c.finished), (1, 1));
        assert_eq!(counts(&all, "ant").of(Filter::All), 0);
        assert_eq!(counts(&all, "salt ro").of(Filter::All), 1);
        assert_eq!(counts(&all, "salt quill").of(Filter::All), 1);

        let shown = grouped(&all, Filter::NotFinished, "");
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].1.len(), 2);
        assert_eq!(grouped(&all, Filter::Finished, "salt"), []);
        assert_eq!(grouped(&all, Filter::All, "").len(), 2);
        assert_eq!(hidden_line(1), "1 finished series isn't shown.");
        assert_eq!(hidden_line(3), "3 finished series aren't shown.");
    }

    #[test]
    fn a_long_strip_keeps_the_next_book_in_view() {
        // Everything fits.
        assert_eq!(strip_window(6, Some(2), 6), (0..6, 0));
        // The next is near the start: the first slots and "+N".
        assert_eq!(strip_window(10, Some(2), 6), (0..5, 5));
        // Further on: the window slides to show it and the one after.
        assert_eq!(strip_window(10, Some(6), 6), (3..8, 2));
        // Near the end: no "+N" needed, so every slot shows a book.
        assert_eq!(strip_window(10, Some(8), 6), (4..10, 0));
        assert_eq!(strip_window(10, None, 6), (0..5, 5));
        assert_eq!(strip_window(3, Some(0), 0), (0..1, 2));
    }

    #[test]
    fn the_reading_line_names_the_page_and_the_percentage() {
        let p = ProgressSummary {
            page: Some(418),
            total_pages: Some(1007),
            seconds: None,
            total_seconds: None,
            percent: Some(41.5),
            audio: false,
        };
        assert_eq!(reading_line(&p).unwrap(), "Page 418 of 1,007 · 42%");
        let none = ProgressSummary {
            page: None,
            total_pages: Some(1007),
            seconds: None,
            total_seconds: None,
            percent: None,
            audio: false,
        };
        assert_eq!(reading_line(&none), None);
        let audio = ProgressSummary {
            page: None,
            total_pages: None,
            seconds: Some(11_520),
            total_seconds: Some(34_800),
            percent: Some(33.1),
            audio: true,
        };
        assert_eq!(reading_line(&audio).unwrap(), "3h 12m of 9h 40m · 33%");
        assert_eq!(lane_caption(NextKind::Reading, 1.0), "Reading · #1");
        assert_eq!(lane_caption(NextKind::Waiting, 3.5), "Next · #3.5");
    }
}
