//! Books in the library that match a search query, found on this computer so
//! they show while Hardcover is still being asked.
//!
//! Every word of the query must start a word of the book's title, authors or
//! series ("hail mar" finds Project Hail Mary; "art" doesn't find The
//! Martian). Words are compared folded (`crate::text`): Arabic and Persian
//! letter forms, Persian digits, accents and the zero-width non-joiner don't
//! keep a book from being found. Ignored books don't count as being in the
//! library here: they stay with the Hardcover results, tagged Ignored.

use hardcover_core::{BookSummary, LibraryEntry, ReadingStatus};

use crate::text::{indexed_words, search_words};

/// Library matches shown above the Hardcover results. More would push those
/// down; the rest still appear among them, tagged with their shelf.
pub const LIMIT: usize = 5;

/// A query shorter than this (in letters) matches too much to be useful.
const MIN_QUERY_CHARS: usize = 2;

/// The library's matches for `query`, best first, at most [`LIMIT`]. Each
/// book carries the cover the library shows for it.
pub fn library_matches(entries: &[LibraryEntry], query: &str) -> Vec<BookSummary> {
    let query = search_words(query);
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
    // The title's words for "starts with", and with their parts for matching.
    let title = search_words(&entry.book.title);
    let title_parts = indexed_words(&entry.book.title);
    let mut rest = Vec::new();
    for author in &entry.book.authors {
        rest.extend(indexed_words(author));
    }
    for series in &entry.series {
        rest.extend(indexed_words(&series.name));
    }
    let in_title = |q: &String| title_parts.iter().any(|w| w.starts_with(q.as_str()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{BookId, SeriesRef, UserBookId, UserId};

    fn entry(id: i64, title: &str, authors: &[&str], status: ReadingStatus) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(id), title);
        book.authors = authors
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
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
    fn letter_forms_digits_accents_and_non_joiners_match() {
        let lib = vec![
            entry(1, "كليدر", &["محمود دولت‌آبادی"], ReadingStatus::Read),
            entry(2, "Les Misérables", &["Victor Hugo"], ReadingStatus::Read),
            entry(3, "۱۹۸۴", &["جورج اورول"], ReadingStatus::Read),
            entry(4, "1984", &["George Orwell"], ReadingStatus::WantToRead),
            entry(5, "کلیدر جلد دوم", &[], ReadingStatus::WantToRead),
            entry(6, "نمی\u{200c}خوریم", &[], ReadingStatus::Read),
        ];
        // A title in Arabic letter forms, typed on a Persian keyboard, and
        // the other way round.
        let kelidar = ["كليدر", "کلیدر جلد دوم"];
        assert_eq!(titles(&library_matches(&lib, "کلیدر")), kelidar);
        assert_eq!(titles(&library_matches(&lib, "كليدر")), kelidar);
        // Accents, either way.
        assert_eq!(titles(&library_matches(&lib, "miserables")), ["Les Misérables"]);
        assert_eq!(titles(&library_matches(&lib, "MISÉR")), ["Les Misérables"]);
        // Persian digits and ASCII ones find each other.
        assert_eq!(titles(&library_matches(&lib, "1984")), ["1984", "۱۹۸۴"]);
        assert_eq!(titles(&library_matches(&lib, "۱۹۸۴")), ["1984", "۱۹۸۴"]);
        // A word with a non-joiner, typed without it, with it or with a space.
        for query in ["نمیخوریم", "نمی\u{200c}خوریم", "نمی خوریم", "خوریم"] {
            let found = library_matches(&lib, query);
            assert_eq!(titles(&found), ["نمی\u{200c}خوریم"], "{query}");
        }
        assert_eq!(titles(&library_matches(&lib, "دولت آبادی")), ["كليدر"]);
        assert_eq!(titles(&library_matches(&lib, "دولتآبادی")), ["كليدر"]);
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
        assert_eq!(library_matches(&lib, "d"), []);
        assert_eq!(library_matches(&lib, "  "), []);
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
