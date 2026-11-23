//! Discover's rules, as data (`hardcover-design`, Discover): which sections
//! show, in what order and which one leads; whose similar books "Because
//! you loved" shows; and Trending this month's one book per series. Pure
//! and tested; the view is [`super::discover_view`].

use std::collections::HashSet;

use hardcover_core::{BookId, BookSummary, LibraryEntry, ReadingStatus, TrendingBook};

/// Discover's sections, in the page's order (Browse aside: see [`layout`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    ForYou,
    Browse,
    TopPicks,
    Loved,
    Trending,
}

/// What a shelf has to show, as far as the order and the lead go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shows {
    /// Its covers, or skeleton covers in their place while they load.
    Covers,
    /// A sentence in the shelf's place: a failure, a vibe not built yet.
    Note,
    /// Nothing: the section hides without leaving a gap.
    Hidden,
}

/// The sections shown, in order, and the lead: the first shelf with covers
/// (or loading them), drawn with lg covers and the shadow.
///
/// The shelves keep their order, each in its place even when it shows only
/// a sentence (a vibe not built keeps its title and place); Browse always
/// comes second, after the first section shown, and alone when no shelf
/// shows.
pub fn layout(shelves: &[(Section, Shows)]) -> (Vec<Section>, Option<Section>) {
    let shown: Vec<Section> = shelves
        .iter()
        .filter(|(_, shows)| *shows != Shows::Hidden)
        .map(|(section, _)| *section)
        .collect();
    let lead = shelves
        .iter()
        .find(|(_, shows)| *shows == Shows::Covers)
        .map(|(section, _)| *section);
    let mut order = Vec::with_capacity(shown.len() + 1);
    let mut rest = shown.into_iter();
    order.extend(rest.next());
    order.push(Section::Browse);
    order.extend(rest);
    (order, lead)
}

/// The book "Because you loved" goes from: the reader's latest 5-star read,
/// the Read entry rated 5 finished most recently. An entry with no finish
/// date counts from its last update; ties go to the latest update, then the
/// newest entry. `None` without a 5-star read.
pub fn loved_seed(entries: &[LibraryEntry]) -> Option<&LibraryEntry> {
    entries
        .iter()
        .filter(|e| {
            e.status == ReadingStatus::Read
                && e.rating
                    .is_some_and(|r| r.value() >= hardcover_core::Rating::MAX)
        })
        .max_by_key(|e| {
            let finished = e.reads.iter().filter_map(|r| r.finished_at).max();
            let updated = e.updated_at.map(crate::dates::local_day);
            (finished.or(updated), e.updated_at, e.user_book_id)
        })
}

/// "Because you loved"'s books: the seed's similar books, in Hardcover's
/// order, less those in the library and those `hidden` (Top picks' books,
/// when Top picks shows).
pub fn loved_books<'a>(
    similar: &'a [BookSummary],
    in_library: &HashSet<BookId>,
    hidden: &HashSet<BookId>,
) -> Vec<&'a BookSummary> {
    similar
        .iter()
        .filter(|b| !in_library.contains(&b.id) && !hidden.contains(&b.id))
        .collect()
}

/// One book per series, in the trending order: a book shows unless a book
/// before it shares one of its series. A book's series are those Hardcover
/// features it in, else all it is in (a reading order, say); a book in no
/// series counts alone.
pub fn one_per_series(books: &[TrendingBook]) -> Vec<&BookSummary> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut shown = Vec::new();
    for t in books {
        let featured: Vec<_> = t.series.iter().filter(|s| s.featured).collect();
        let series = if featured.is_empty() {
            t.series.iter().collect()
        } else {
            featured
        };
        let keys: Vec<String> = series
            .iter()
            .map(|s| match s.id {
                Some(id) => format!("id:{}", id.get()),
                None => format!("name:{}", s.name),
            })
            .collect();
        if keys.iter().any(|k| seen.contains(k)) {
            continue;
        }
        seen.extend(keys);
        shown.push(&t.book);
    }
    shown
}

#[cfg(test)]
mod tests {
    use hardcover_core::{Rating, ReadId, ReadSession, SeriesId, SeriesRef, UserBookId, UserId};

    use super::*;

    fn book(id: i64, title: &str) -> BookSummary {
        BookSummary::new(BookId::new(id), title)
    }

    fn entry(id: i64, status: ReadingStatus, rating: Option<f32>) -> LibraryEntry {
        let mut e = LibraryEntry::new(
            UserBookId::new(id),
            UserId::new(1),
            book(id * 10, &format!("Book {id}")),
            status,
        );
        e.rating = rating.and_then(Rating::new);
        e
    }

    fn finished(mut e: LibraryEntry, day: &str) -> LibraryEntry {
        let mut read = ReadSession::new(ReadId::new(e.user_book_id.get()));
        read.finished_at = Some(day.parse().unwrap());
        e.reads.push(read);
        e
    }

    fn updated(mut e: LibraryEntry, at: &str) -> LibraryEntry {
        e.updated_at = Some(at.parse().unwrap());
        e
    }

    #[test]
    fn the_seed_is_the_latest_five_star_read() {
        let entries = vec![
            finished(entry(1, ReadingStatus::Read, Some(5.)), "2026-01-10"),
            finished(entry(2, ReadingStatus::Read, Some(5.)), "2026-06-01"),
            // Later, but not 5 stars, or not Read.
            finished(entry(3, ReadingStatus::Read, Some(4.5)), "2026-11-01"),
            finished(
                entry(4, ReadingStatus::DidNotFinish, Some(5.)),
                "2026-11-02",
            ),
            entry(5, ReadingStatus::WantToRead, Some(5.)),
        ];
        assert_eq!(
            loved_seed(&entries).map(|e| e.user_book_id),
            Some(UserBookId::new(2))
        );
    }

    #[test]
    fn without_a_finish_date_the_last_update_counts() {
        let entries = vec![
            updated(
                finished(entry(1, ReadingStatus::Read, Some(5.)), "2026-03-01"),
                "2026-03-01T10:00:00Z",
            ),
            // No read at all: counted from its update, later than entry 1.
            updated(
                entry(2, ReadingStatus::Read, Some(5.)),
                "2026-09-01T10:00:00Z",
            ),
            updated(
                entry(3, ReadingStatus::Read, Some(5.)),
                "2025-09-01T10:00:00Z",
            ),
        ];
        assert_eq!(
            loved_seed(&entries).map(|e| e.user_book_id),
            Some(UserBookId::new(2))
        );
        let only_undated = vec![
            updated(
                entry(7, ReadingStatus::Read, Some(5.)),
                "2025-01-01T10:00:00Z",
            ),
            updated(
                entry(8, ReadingStatus::Read, Some(5.)),
                "2025-02-01T10:00:00Z",
            ),
        ];
        assert_eq!(
            loved_seed(&only_undated).map(|e| e.user_book_id),
            Some(UserBookId::new(8))
        );
    }

    #[test]
    fn no_five_star_read_no_seed() {
        let entries = vec![finished(
            entry(1, ReadingStatus::Read, Some(4.5)),
            "2026-01-10",
        )];
        assert!(loved_seed(&entries).is_none());
        assert!(loved_seed(&[]).is_none());
    }

    #[test]
    fn loved_books_leave_out_the_library_and_top_picks() {
        let similar = vec![book(1, "A"), book(2, "B"), book(3, "C"), book(4, "D")];
        let library: HashSet<BookId> = [BookId::new(2)].into();
        let top: HashSet<BookId> = [BookId::new(4)].into();
        let shown: Vec<&str> = loved_books(&similar, &library, &top)
            .iter()
            .map(|b| b.title.as_str())
            .collect();
        assert_eq!(shown, ["A", "C"], "in Hardcover's order");
        assert!(loved_books(&similar[1..2], &library, &top).is_empty());
    }

    fn trending(id: i64, series: &[(i64, bool)]) -> TrendingBook {
        let mut t = TrendingBook::new(book(id, &format!("T{id}")));
        t.series = series
            .iter()
            .map(|(s, featured)| {
                let mut r = SeriesRef::new(format!("S{s}"));
                r.id = Some(SeriesId::new(*s));
                r.featured = *featured;
                r
            })
            .collect();
        t
    }

    #[test]
    fn trending_keeps_the_most_trending_book_of_each_series() {
        let books = vec![
            trending(1, &[(10, true)]),
            // The same series, less trending: left out.
            trending(2, &[(10, true)]),
            // No series: each counts alone.
            trending(3, &[]),
            trending(4, &[]),
            // Featured in its own series; a reading order shared with 1
            // doesn't count while it is featured elsewhere.
            trending(5, &[(11, true), (10, false)]),
            // Featured nowhere: all its series count, and 11 was shown.
            trending(6, &[(12, false), (11, false)]),
            trending(7, &[(12, false)]),
        ];
        let shown: Vec<i64> = one_per_series(&books).iter().map(|b| b.id.get()).collect();
        assert_eq!(shown, [1, 3, 4, 5, 7]);
    }

    #[test]
    fn trending_tells_series_without_ids_by_name() {
        let mut a = TrendingBook::new(book(1, "A"));
        a.series = vec![SeriesRef::new("Saga")];
        let mut b = TrendingBook::new(book(2, "B"));
        b.series = vec![SeriesRef::new("Saga")];
        assert_eq!(one_per_series(&[a, b]).len(), 1);
    }

    fn order(shelves: &[(Section, Shows)]) -> (Vec<Section>, Option<Section>) {
        layout(shelves)
    }

    use Section::*;
    use Shows::*;

    #[test]
    fn for_you_leads_and_browse_comes_second() {
        let (sections, lead) = order(&[
            (ForYou, Covers),
            (TopPicks, Covers),
            (Loved, Covers),
            (Trending, Covers),
        ]);
        assert_eq!(sections, [ForYou, Browse, TopPicks, Loved, Trending]);
        assert_eq!(lead, Some(ForYou));
    }

    #[test]
    fn without_the_vibes_because_you_loved_leads() {
        let (sections, lead) = order(&[
            (ForYou, Hidden),
            (TopPicks, Hidden),
            (Loved, Covers),
            (Trending, Covers),
        ]);
        assert_eq!(sections, [Loved, Browse, Trending]);
        assert_eq!(lead, Some(Loved));
    }

    #[test]
    fn with_nothing_loved_trending_leads() {
        let (sections, lead) = order(&[
            (ForYou, Hidden),
            (TopPicks, Hidden),
            (Loved, Hidden),
            (Trending, Covers),
        ]);
        assert_eq!(sections, [Trending, Browse]);
        assert_eq!(lead, Some(Trending));
    }

    #[test]
    fn a_vibe_not_built_keeps_its_place_and_the_next_shelf_leads() {
        let (sections, lead) = order(&[
            (ForYou, Note),
            (TopPicks, Note),
            (Loved, Covers),
            (Trending, Covers),
        ]);
        assert_eq!(sections, [ForYou, Browse, TopPicks, Loved, Trending]);
        assert_eq!(lead, Some(Loved));
    }

    #[test]
    fn with_no_shelf_browse_stands_alone() {
        let (sections, lead) = order(&[
            (ForYou, Hidden),
            (TopPicks, Hidden),
            (Loved, Hidden),
            (Trending, Note),
        ]);
        assert_eq!(sections, [Trending, Browse]);
        assert_eq!(lead, None, "a failure doesn't lead");
        let (sections, lead) = order(&[(Loved, Hidden), (Trending, Hidden)]);
        assert_eq!(sections, [Browse]);
        assert_eq!(lead, None);
    }
}
