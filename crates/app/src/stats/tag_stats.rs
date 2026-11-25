//! Stats' genres and moods: what the books finished in a period are, from
//! their tags (`library::shelf_tags`, the shelf filter's). Pure and tested.
//!
//! - A book counts once in each of its genres and in each mood of its mood
//!   line (`tags::has_mood`, the line's own test), so the shares are of the
//!   books, and add up to more than 100%.
//! - A genre is named after the spelling most of the books use.
//! - Fiction or not is the book's genres' say (`tags::kind_of`).
//! - A book whose tags aren't known yet counts in none of them, and is
//!   counted apart ([`TagStats::unknown`]).

use std::collections::HashMap;

use hardcover_core::BookId;
use hardcover_core::tags::{Kind, Mood};

use crate::library::tag_filter::{BookFacts, GenrePick};

/// A genre and how many books have it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenreCount {
    pub genre: GenrePick,
    pub books: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagStats {
    pub fiction: u32,
    pub non_fiction: u32,
    /// Known books whose genres don't say either way.
    pub neither: u32,
    /// The most books first; ties by name.
    pub genres: Vec<GenreCount>,
    /// The most books first; ties in [`Mood::ALL`]'s order.
    pub moods: Vec<(Mood, u32)>,
    /// Known books with no genre.
    pub without_genre: u32,
    /// Known books with no mood line.
    pub without_mood: u32,
    /// Books whose tags aren't known.
    pub unknown: u32,
}

impl TagStats {
    /// Books whose tags are known.
    pub fn known(&self) -> u32 {
        self.fiction + self.non_fiction + self.neither
    }
}

pub fn compute<'a>(books: &[BookId], facts: impl Fn(BookId) -> Option<&'a BookFacts>) -> TagStats {
    let mut stats = TagStats::default();
    // key -> (books, spelling -> books)
    let mut genres: HashMap<&str, (u32, HashMap<&str, u32>)> = HashMap::new();
    let mut moods: HashMap<Mood, u32> = HashMap::new();
    for book in books {
        let Some(f) = facts(*book) else {
            stats.unknown += 1;
            continue;
        };
        match f.kind {
            Some(Kind::Fiction) => stats.fiction += 1,
            Some(Kind::NonFiction) => stats.non_fiction += 1,
            None => stats.neither += 1,
        }
        if f.genres.is_empty() {
            stats.without_genre += 1;
        }
        if f.moods.is_empty() {
            stats.without_mood += 1;
        }
        for g in &f.genres {
            let (n, names) = genres.entry(g.key.as_str()).or_default();
            *n += 1;
            *names.entry(g.name.as_str()).or_default() += 1;
        }
        for m in &f.moods {
            *moods.entry(*m).or_default() += 1;
        }
    }
    stats.genres = genres
        .into_iter()
        .map(|(key, (books, names))| {
            let name = names
                .into_iter()
                .max_by(|(a, x), (b, y)| x.cmp(y).then(b.cmp(a)))
                .map_or(key, |(name, _)| name);
            GenreCount {
                genre: GenrePick {
                    key: key.to_owned(),
                    name: name.to_owned(),
                },
                books,
            }
        })
        .collect();
    stats
        .genres
        .sort_by(|a, b| b.books.cmp(&a.books).then(a.genre.name.cmp(&b.genre.name)));
    stats.moods = Mood::ALL
        .into_iter()
        .filter_map(|m| moods.get(&m).map(|n| (m, *n)))
        .collect();
    // Stable: ties keep Mood::ALL's order.
    stats.moods.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tag_filter::GenreName;

    fn facts(genres: &[(&str, &str)], moods: &[Mood], kind: Option<Kind>) -> BookFacts {
        BookFacts {
            genres: genres
                .iter()
                .map(|(key, name)| GenreName {
                    key: (*key).to_owned(),
                    name: (*name).to_owned(),
                })
                .collect(),
            moods: moods.to_vec(),
            kind,
        }
    }

    #[test]
    fn books_count_once_in_each_genre_and_mood() {
        let known: HashMap<BookId, BookFacts> = [
            (
                1,
                facts(
                    &[("fantasy", "Fantasy"), ("adventure", "Adventure")],
                    &[Mood::Adventurous],
                    Some(Kind::Fiction),
                ),
            ),
            (
                2,
                facts(
                    &[("fantasy", "fantasy"), ("young adult", "Young Adult")],
                    &[Mood::Adventurous, Mood::Dark],
                    Some(Kind::Fiction),
                ),
            ),
            (3, facts(&[("fantasy", "Fantasy")], &[], None)),
            (
                4,
                facts(
                    &[("self help", "Self-Help")],
                    &[Mood::Hopeful],
                    Some(Kind::NonFiction),
                ),
            ),
            (5, facts(&[], &[], None)),
        ]
        .into_iter()
        .map(|(id, f)| (BookId::new(id), f))
        .collect();
        let books: Vec<BookId> = (1..=6).map(BookId::new).collect();
        let stats = compute(&books, |id| known.get(&id));

        assert_eq!((stats.fiction, stats.non_fiction, stats.neither), (2, 1, 2));
        assert_eq!(stats.known(), 5);
        assert_eq!(stats.unknown, 1);
        assert_eq!(stats.without_genre, 1);
        assert_eq!(stats.without_mood, 2);
        let genres: Vec<(&str, u32)> = stats
            .genres
            .iter()
            .map(|g| (g.genre.name.as_str(), g.books))
            .collect();
        // Fantasy is named as most of its books spell it; ties by name.
        assert_eq!(
            genres,
            [
                ("Fantasy", 3),
                ("Adventure", 1),
                ("Self-Help", 1),
                ("Young Adult", 1)
            ]
        );
        assert_eq!(
            stats.moods,
            [(Mood::Adventurous, 2), (Mood::Dark, 1), (Mood::Hopeful, 1)]
                .into_iter()
                .collect::<Vec<_>>()
        );
    }
}
