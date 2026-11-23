//! Pure collection logic for the library: shelf counts, the local filter, sort
//! orders and merging a full sync with the writes it may not have seen.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use hardcover_core::{LibraryEntry, ReadingStatus, UserBookId};

/// How a shelf is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SortOrder {
    #[default]
    RecentlyUpdated,
    Title,
    Author,
    DateAdded,
}

impl SortOrder {
    pub const ALL: [SortOrder; 4] = [
        SortOrder::RecentlyUpdated,
        SortOrder::Title,
        SortOrder::Author,
        SortOrder::DateAdded,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::RecentlyUpdated => "Recently updated",
            SortOrder::Title => "Title",
            SortOrder::Author => "Author",
            SortOrder::DateAdded => "Date added",
        }
    }
}

/// Number of entries per status.
pub fn status_counts(entries: &[LibraryEntry]) -> HashMap<ReadingStatus, usize> {
    let mut counts = HashMap::new();
    for e in entries {
        *counts.entry(e.status).or_insert(0) += 1;
    }
    counts
}

/// True if every whitespace-separated word of `query` occurs in the title,
/// subtitle or an author, both folded (`crate::text::fold`: case, accents,
/// Arabic and Persian letter forms, Persian digits). A non-joiner counts for
/// nothing on either side. A blank query matches everything.
pub fn matches_filter(entry: &LibraryEntry, query: &str) -> bool {
    let key = |text: &str| crate::text::fold(text).replace('\u{200c}', "");
    let words: Vec<String> = query.split_whitespace().map(key).collect();
    if words.is_empty() {
        return true;
    }
    let mut haystack = key(&entry.book.title);
    if let Some(sub) = &entry.book.subtitle {
        haystack.push('\n');
        haystack.push_str(&key(sub));
    }
    for author in &entry.book.authors {
        haystack.push('\n');
        haystack.push_str(&key(author));
    }
    words.iter().all(|w| haystack.contains(w.as_str()))
}

/// Title without a leading English article, for sorting.
pub fn sort_title(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            return rest.trim_start().to_owned();
        }
    }
    lower
}

/// Last word of the first author ("Ursula K. Le Guin" sorts under "guin").
fn sort_author(entry: &LibraryEntry) -> String {
    entry
        .book
        .authors
        .first()
        .map_or_default(|a| author_sort_key(a))
}

/// An author's name for sorting: the last word, lowercased.
pub fn author_sort_key(name: &str) -> String {
    name.split_whitespace()
        .last()
        .map_or_default(str::to_lowercase)
}

fn compare(a: &LibraryEntry, b: &LibraryEntry, order: SortOrder) -> Ordering {
    let by_title = || sort_title(&a.book.title).cmp(&sort_title(&b.book.title));
    let primary = match order {
        // Newest first; entries without a timestamp go last.
        SortOrder::RecentlyUpdated => b
            .updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.date_added.cmp(&a.date_added)),
        SortOrder::Title => by_title(),
        SortOrder::Author => match (a.book.authors.is_empty(), b.book.authors.is_empty()) {
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            _ => sort_author(a).cmp(&sort_author(b)).then_with(by_title),
        },
        SortOrder::DateAdded => b.date_added.cmp(&a.date_added),
    };
    // Ids keep the order total and stable across syncs.
    primary.then_with(|| b.user_book_id.cmp(&a.user_book_id))
}

/// Indices into `entries` for one shelf, filtered and sorted.
pub fn shelf_rows(
    entries: &[LibraryEntry],
    status: ReadingStatus,
    query: &str,
    order: SortOrder,
) -> Vec<usize> {
    let mut rows: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.status == status && matches_filter(e, query))
        .map(|(ix, _)| ix)
        .collect();
    rows.sort_by(|&a, &b| compare(&entries[a], &entries[b], order));
    rows
}

/// A library write the app made, as a sync may need to re-apply it.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalChange {
    Upserted(Box<LibraryEntry>),
    Removed(UserBookId),
    /// The entry's review was written (reviews aren't part of the sync).
    Reviewed(UserBookId),
}

/// How long before a sync starts a write still counts as one the sync may
/// not see: Hardcover can answer a read right after a write with the row as
/// it was before.
pub const REAPPLY_WINDOW: Duration = Duration::from_secs(30);

/// The writes of the last moments, for [`merge_sync`]. Wall-clock times, as
/// the sync's own start.
#[derive(Debug, Default)]
pub struct RecentWrites {
    log: Vec<(SystemTime, LocalChange)>,
}

impl RecentWrites {
    pub fn record(&mut self, at: SystemTime, change: LocalChange) {
        self.log.push((at, change));
    }

    /// The writes a sync that started at `started` may not have seen: those
    /// made during it, and in the [`REAPPLY_WINDOW`] before it.
    pub fn since_before(&self, started: SystemTime) -> Vec<LocalChange> {
        let from = started.checked_sub(REAPPLY_WINDOW).unwrap_or(started);
        self.log
            .iter()
            .filter(|(at, _)| *at >= from)
            .map(|(_, change)| change.clone())
            .collect()
    }

    /// Forgets writes no sync starting at `now` or later could need.
    pub fn prune(&mut self, now: SystemTime) {
        let from = now.checked_sub(REAPPLY_WINDOW).unwrap_or(now);
        self.log.retain(|(at, _)| *at >= from);
    }

    pub fn clear(&mut self) {
        self.log.clear();
    }
}

/// The synced library with recent writes applied on top, so a sync that
/// read rows from before a write cannot undo it. A synced row changed after
/// the write (a later `updated_at`) is newer than it, and stays.
pub fn merge_sync(mut synced: Vec<LibraryEntry>, changes: &[LocalChange]) -> Vec<LibraryEntry> {
    for change in changes {
        match change {
            LocalChange::Upserted(entry) => {
                match synced
                    .iter_mut()
                    .find(|e| e.user_book_id == entry.user_book_id)
                {
                    Some(slot) if changed_after(slot, entry) => {}
                    Some(slot) => *slot = (**entry).clone(),
                    None => synced.push((**entry).clone()),
                }
            }
            LocalChange::Removed(id) => synced.retain(|e| e.user_book_id != *id),
            LocalChange::Reviewed(_) => {}
        }
    }
    synced
}

/// True if `synced` was changed after `written` was: on Hardcover, by
/// another device or the website.
fn changed_after(synced: &LibraryEntry, written: &LibraryEntry) -> bool {
    matches!((synced.updated_at, written.updated_at), (Some(s), Some(w)) if s > w)
}

/// True if `a` and `b` hold the same entries, in any order: a sync that
/// brings nothing new then leaves the library, and everything worked out from
/// it, as it is.
pub fn same_entries(a: &[LibraryEntry], b: &[LibraryEntry]) -> bool {
    fn by_id(entries: &[LibraryEntry]) -> Vec<&LibraryEntry> {
        let mut sorted: Vec<&LibraryEntry> = entries.iter().collect();
        sorted.sort_by_key(|e| e.user_book_id);
        sorted
    }
    a.len() == b.len() && by_id(a) == by_id(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use hardcover_core::{BookId, BookSummary, UserId};

    fn entry(id: i64, title: &str, author: &str, status: ReadingStatus) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(1000 + id), title);
        if !author.is_empty() {
            book.authors = vec![author.to_owned()];
        }
        LibraryEntry::new(UserBookId::new(id), UserId::new(1), book, status)
    }

    fn titles(entries: &[LibraryEntry], rows: &[usize]) -> Vec<String> {
        rows.iter()
            .map(|&ix| entries[ix].book.title.clone())
            .collect()
    }

    #[test]
    fn the_same_entries_in_another_order_are_the_same() {
        let a = vec![
            entry(1, "A", "", ReadingStatus::Read),
            entry(2, "B", "", ReadingStatus::Paused),
        ];
        let reversed: Vec<_> = a.iter().rev().cloned().collect();
        assert!(same_entries(&a, &reversed));
        assert!(same_entries(&[], &[]));

        let mut changed = a.clone();
        changed[1].status = ReadingStatus::Read;
        assert!(!same_entries(&a, &changed));
        assert!(!same_entries(&a, &a[..1]));
        let duplicated = vec![a[0].clone(), a[0].clone()];
        assert!(!same_entries(&a, &duplicated));
    }

    #[test]
    fn counts_per_status() {
        let entries = vec![
            entry(1, "A", "", ReadingStatus::Read),
            entry(2, "B", "", ReadingStatus::Read),
            entry(3, "C", "", ReadingStatus::Paused),
        ];
        let counts = status_counts(&entries);
        assert_eq!(counts[&ReadingStatus::Read], 2);
        assert_eq!(counts[&ReadingStatus::Paused], 1);
        assert_eq!(counts.get(&ReadingStatus::WantToRead), None);
    }

    #[test]
    fn filter_matches_title_subtitle_and_author_words() {
        let mut e = entry(
            1,
            "The Left Hand of Darkness",
            "Ursula K. Le Guin",
            ReadingStatus::Read,
        );
        e.book.subtitle = Some("A novel".into());
        assert!(matches_filter(&e, ""));
        assert!(matches_filter(&e, "  "));
        assert!(matches_filter(&e, "darkness"));
        assert!(matches_filter(&e, "LEFT guin"));
        assert!(matches_filter(&e, "novel"));
        assert!(!matches_filter(&e, "darkness tolkien"));
        let persian = entry(2, "كليدر", "Gabriel García Márquez", ReadingStatus::Read);
        assert!(matches_filter(&persian, "کلیدر"), "Persian letter forms");
        assert!(matches_filter(&persian, "garcia marquez"), "accents");
    }

    #[test]
    fn shelf_rows_filter_status_and_sort_by_title_ignoring_articles() {
        let entries = vec![
            entry(1, "The Hobbit", "J. R. R. Tolkien", ReadingStatus::Read),
            entry(2, "Anathem", "Neal Stephenson", ReadingStatus::Read),
            entry(
                3,
                "A Wizard of Earthsea",
                "Ursula K. Le Guin",
                ReadingStatus::Read,
            ),
            entry(4, "Dune", "Frank Herbert", ReadingStatus::WantToRead),
        ];
        let rows = shelf_rows(&entries, ReadingStatus::Read, "", SortOrder::Title);
        assert_eq!(
            titles(&entries, &rows),
            ["Anathem", "The Hobbit", "A Wizard of Earthsea"]
        );
        let rows = shelf_rows(&entries, ReadingStatus::Read, "", SortOrder::Author);
        assert_eq!(
            titles(&entries, &rows),
            ["A Wizard of Earthsea", "Anathem", "The Hobbit"]
        );
        let rows = shelf_rows(&entries, ReadingStatus::Read, "hob", SortOrder::Title);
        assert_eq!(titles(&entries, &rows), ["The Hobbit"]);
    }

    #[test]
    fn author_sort_puts_unknown_authors_last() {
        let entries = vec![
            entry(1, "Anonymous", "", ReadingStatus::Read),
            entry(2, "Named", "Zed Zulu", ReadingStatus::Read),
        ];
        let rows = shelf_rows(&entries, ReadingStatus::Read, "", SortOrder::Author);
        assert_eq!(titles(&entries, &rows), ["Named", "Anonymous"]);
    }

    #[test]
    fn recent_and_date_added_sort_newest_first() {
        let mut a = entry(1, "Old", "", ReadingStatus::Read);
        a.updated_at = Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        a.date_added = NaiveDate::from_ymd_opt(2025, 1, 1);
        let mut b = entry(2, "New", "", ReadingStatus::Read);
        b.updated_at = Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        b.date_added = NaiveDate::from_ymd_opt(2020, 1, 1);
        let c = entry(3, "Never", "", ReadingStatus::Read);
        let entries = vec![a, b, c];
        let rows = shelf_rows(
            &entries,
            ReadingStatus::Read,
            "",
            SortOrder::RecentlyUpdated,
        );
        assert_eq!(titles(&entries, &rows), ["New", "Old", "Never"]);
        let rows = shelf_rows(&entries, ReadingStatus::Read, "", SortOrder::DateAdded);
        assert_eq!(titles(&entries, &rows), ["Old", "New", "Never"]);
    }

    #[test]
    fn merge_reapplies_writes_made_during_a_sync() {
        let synced = vec![
            entry(1, "A", "", ReadingStatus::WantToRead),
            entry(2, "B", "", ReadingStatus::Read),
        ];
        let changes = vec![
            LocalChange::Upserted(Box::new(entry(1, "A", "", ReadingStatus::CurrentlyReading))),
            LocalChange::Upserted(Box::new(entry(3, "C", "", ReadingStatus::WantToRead))),
            LocalChange::Removed(UserBookId::new(2)),
        ];
        let merged = merge_sync(synced, &changes);
        let ids: Vec<i64> = merged.iter().map(|e| e.user_book_id.get()).collect();
        assert_eq!(ids, [1, 3]);
        assert_eq!(merged[0].status, ReadingStatus::CurrentlyReading);
    }

    #[test]
    fn a_synced_row_changed_after_the_write_stays() {
        let at = |h| Utc.with_ymd_and_hms(2026, 5, 1, h, 0, 0).unwrap();
        let mut written = entry(1, "A", "", ReadingStatus::Read);
        written.updated_at = Some(at(10));
        // Stale: read from before the write.
        let mut stale = entry(1, "A", "", ReadingStatus::WantToRead);
        stale.updated_at = Some(at(9));
        let changes = [LocalChange::Upserted(Box::new(written.clone()))];
        assert_eq!(
            merge_sync(vec![stale], &changes)[0].status,
            ReadingStatus::Read
        );
        // The same version: the write's own copy.
        let mut same = written.clone();
        same.status = ReadingStatus::Paused;
        assert_eq!(
            merge_sync(vec![same], &changes)[0].status,
            ReadingStatus::Read
        );
        // Changed on the website after the write.
        let mut later = entry(1, "A", "", ReadingStatus::Paused);
        later.updated_at = Some(at(11));
        assert_eq!(
            merge_sync(vec![later], &changes)[0].status,
            ReadingStatus::Paused
        );
    }

    #[test]
    fn writes_shortly_before_a_sync_are_reapplied_and_older_ones_are_not() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut recent = RecentWrites::default();
        let change = |id| LocalChange::Removed(UserBookId::new(id));
        recent.record(start - REAPPLY_WINDOW - Duration::from_secs(1), change(1));
        recent.record(start - Duration::from_secs(5), change(2));
        recent.record(start + Duration::from_secs(1), change(3));
        assert_eq!(recent.since_before(start), [change(2), change(3)]);
        recent.prune(start + REAPPLY_WINDOW);
        assert_eq!(recent.since_before(start), [change(3)]);
    }
}
