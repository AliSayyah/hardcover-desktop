//! Books in the library that match a search query, found on this computer so
//! they show while Hardcover is still being asked.
//!
//! Every word of the query must start a word of the book's title, authors or
//! series ("hail mar" finds Project Hail Mary; "art" doesn't find The
//! Martian). Ignored books don't count as being in the library here: they
//! stay with the Hardcover results, tagged Ignored.

use hardcover_core::{BookSummary, LibraryEntry, ReadingStatus};

/// Library matches shown above the Hardcover results. More would push those
/// down; the rest still appear among them, tagged with their shelf.
pub const LIMIT: usize = 5;

/// A query shorter than this (in letters) matches too much to be useful.
const MIN_QUERY_CHARS: usize = 2;

/// The library's matches for `query`, best first, at most [`LIMIT`]. Each
/// book carries the cover the library shows for it.
pub fn library_matches(entries: &[LibraryEntry], query: &str) -> Vec<BookSummary> {
    let query = words(query);
    if query.iter().map(|w| w.chars().count()).sum::<usize>() < MIN_QUERY_CHARS {
        return Vec::new();
    }
    let mut found: Vec<(u8, &LibraryEntry)> = entries
        .iter()
        .filter(|e| counts_as_shelved(e.status))
        .filter_map(|e| rank(e, &query).map(|r| (r, e)))
        .collect();
    found.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then_with(|| {
                a.book
                    .title
                    .to_lowercase()
                    .cmp(&b.book.title.to_lowercase())
            })
            .then(a.book.id.cmp(&b.book.id))
    });
    found
        .into_iter()
        .take(LIMIT)
        .map(|(_, e)| {
            let mut book = e.book.clone();
            book.cover = e.cover().cloned();
            book
        })
        .collect()
}

/// Whether a book on this shelf is "in your library" for search.
pub fn counts_as_shelved(status: ReadingStatus) -> bool {
    status != ReadingStatus::Ignored
}

/// 0: the title starts with the query; 1: every word is in the title;
/// 2: some are only in the authors or series. `None`: no match.
fn rank(entry: &LibraryEntry, query: &[String]) -> Option<u8> {
    let title = words(&entry.book.title);
    let mut rest = Vec::new();
    for author in &entry.book.authors {
        rest.extend(words(author));
    }
    for series in &entry.series {
        rest.extend(words(&series.name));
    }
    let in_title = |q: &String| title.iter().any(|w| w.starts_with(q.as_str()));
    let in_rest = |q: &String| rest.iter().any(|w| w.starts_with(q.as_str()));
    if !query.iter().all(|q| in_title(q) || in_rest(q)) {
        return None;
    }
    let starts = title.len() >= query.len()
        && query.iter().zip(&title).enumerate().all(|(ix, (q, w))| {
            if ix + 1 == query.len() {
                w.starts_with(q.as_str())
            } else {
                w == q
            }
        });
    Some(if starts {
        0
    } else if query.iter().all(in_title) {
        1
    } else {
        2
    })
}

/// Lowercase words: runs of letters and digits. An apostrophe inside a word
/// is dropped, so "ender's" and "enders" match.
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace(['\'', '’'], "")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{BookId, SeriesRef, UserBookId, UserId};

    fn entry(id: i64, title: &str, authors: &[&str], status: ReadingStatus) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(id), title);
        book.authors = authors.iter().map(|a| a.to_string()).collect();
        LibraryEntry::new(UserBookId::new(id), UserId::new(1), book, status)
    }

    fn titles(found: &[BookSummary]) -> Vec<&str> {
        found.iter().map(|b| b.title.as_str()).collect()
    }

    fn library() -> Vec<LibraryEntry> {
        let mut mistborn = entry(
            4,
            "The Final Empire",
            &["Brandon Sanderson"],
            ReadingStatus::Read,
        );
        mistborn.series.push(SeriesRef::new("Mistborn"));
        vec![
            entry(1, "Dune", &["Frank Herbert"], ReadingStatus::Read),
            entry(
                2,
                "Dune Messiah",
                &["Frank Herbert"],
                ReadingStatus::WantToRead,
            ),
            entry(3, "The Martian", &["Andy Weir"], ReadingStatus::Read),
            mistborn,
            entry(
                5,
                "Project Hail Mary",
                &["Andy Weir"],
                ReadingStatus::CurrentlyReading,
            ),
            entry(
                6,
                "Ender's Game",
                &["Orson Scott Card"],
                ReadingStatus::Read,
            ),
            entry(
                7,
                "Children of Dune",
                &["Frank Herbert"],
                ReadingStatus::Ignored,
            ),
            entry(
                8,
                "سمفونی مردگان",
                &["عباس معروفی"],
                ReadingStatus::WantToRead,
            ),
        ]
    }

    #[test]
    fn words_must_start_words() {
        let lib = library();
        assert_eq!(
            titles(&library_matches(&lib, "hail mar")),
            ["Project Hail Mary"]
        );
        assert!(
            library_matches(&lib, "art").is_empty(),
            "not inside Martian"
        );
        assert_eq!(titles(&library_matches(&lib, "MART")), ["The Martian"]);
        assert_eq!(titles(&library_matches(&lib, "enders")), ["Ender's Game"]);
        assert_eq!(titles(&library_matches(&lib, "سمفونی")), ["سمفونی مردگان"]);
    }

    #[test]
    fn authors_and_series_match_after_titles() {
        let lib = library();
        assert_eq!(
            titles(&library_matches(&lib, "weir")),
            ["Project Hail Mary", "The Martian"]
        );
        assert_eq!(
            titles(&library_matches(&lib, "mistborn")),
            ["The Final Empire"]
        );
        // A title that starts with the query comes first, then the title
        // that only contains it; Ignored books are left to Hardcover's list.
        assert_eq!(
            titles(&library_matches(&lib, "dune")),
            ["Dune", "Dune Messiah"]
        );
        assert_eq!(
            titles(&library_matches(&lib, "herbert messiah")),
            ["Dune Messiah"]
        );
    }

    #[test]
    fn short_queries_and_long_lists_are_cut() {
        let lib = library();
        assert!(library_matches(&lib, "d").is_empty());
        assert!(library_matches(&lib, "  ").is_empty());
        let many: Vec<LibraryEntry> = (1..=9)
            .map(|i| entry(i, &format!("Book {i}"), &[], ReadingStatus::Read))
            .collect();
        assert_eq!(library_matches(&many, "book").len(), LIMIT);
    }

    #[test]
    fn the_library_cover_is_used() {
        let mut e = entry(1, "Dune", &[], ReadingStatus::Read);
        let mut edition = hardcover_core::EditionSummary::new(hardcover_core::EditionId::new(9));
        edition.cover = Some(hardcover_core::Cover::new("https://example.com/e.jpg"));
        e.edition = Some(edition);
        let found = library_matches(&[e], "dune");
        assert_eq!(
            found[0].cover.as_ref().map(|c| c.url.as_str()),
            Some("https://example.com/e.jpg")
        );
    }
}
