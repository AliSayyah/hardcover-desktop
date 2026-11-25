//! Genres, merged across their spellings.
//!
//! Hardcover's genres are free text that readers add, so one genre comes in
//! many spellings: "Science Fiction", "Sci-fi", "science-fiction";
//! "Non-Fiction" and "Nonfiction"; "Self-Help" and "Self help". A book's
//! `cached_tags` lists each spelling with its own votes. [`genre_key`]
//! gives every spelling of a genre one key, and [`BookGenres`] adds their
//! votes up under it. A genre that names two ("Science Fiction & Fantasy")
//! stays one genre of its own: it isn't split into both.

use serde::{Deserialize, Serialize};

use super::TagId;

/// A book's genre needs this many votes, its spellings together, to count
/// (a shelf's filter chip, say), so one reader's stray tag doesn't.
pub const MIN_GENRE_VOTES: u32 = 3;

/// A genre's name as lowercase words one space apart: `&` reads "and",
/// apostrophes go, and any other punctuation (hyphens, slashes, quotes)
/// separates words. Letters of any script are kept.
#[must_use]
pub fn genre_words(name: &str) -> String {
    let mut spaced = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        match c {
            '&' => spaced.push_str(" and "),
            '\'' | '\u{2018}' | '\u{2019}' => {}
            c if c.is_alphanumeric() => spaced.push(c),
            _ => spaced.push(' '),
        }
    }
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The key every spelling of a genre shares: its [`genre_words`], with
/// "sci fi" and "scifi" read as "science fiction" and "non fiction" as
/// "nonfiction". Empty for a name with no letters or digits.
#[must_use]
pub fn genre_key(name: &str) -> String {
    let words = genre_words(name);
    let words: Vec<&str> = words.split(' ').filter(|w| !w.is_empty()).collect();
    let mut key: Vec<&str> = Vec::with_capacity(words.len() + 1);
    let mut i = 0;
    while i < words.len() {
        match (words[i], words.get(i + 1).copied()) {
            ("sci", Some("fi")) => {
                key.extend(["science", "fiction"]);
                i += 2;
            }
            ("scifi", _) => {
                key.extend(["science", "fiction"]);
                i += 1;
            }
            ("non", Some("fiction")) => {
                key.push("nonfiction");
                i += 2;
            }
            (word, _) => {
                key.push(word);
                i += 1;
            }
        }
    }
    key.join(" ")
}

/// One of a book's genres, its spellings merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Genre {
    /// [`genre_key`]: what a filter matches.
    pub key: String,
    /// The most voted spelling, as written.
    pub name: String,
    /// Every spelling's votes together.
    pub votes: u32,
}

/// A book's genres, merged by [`genre_key`], the most voted first. Genres
/// with fewer than [`MIN_GENRE_VOTES`] are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookGenres {
    genres: Vec<Genre>,
}

impl BookGenres {
    /// The genres from `(name, votes)` pairs, as `cached_tags` lists them.
    /// Spellings with the same key add up; the name is the most voted
    /// spelling's (the first listed on a tie). Ties between genres keep the
    /// order of their first spelling.
    pub fn from_tags<S: AsRef<str>>(tags: impl IntoIterator<Item = (S, u32)>) -> Self {
        // (genre, the votes of the spelling it is named after)
        let mut merged: Vec<(Genre, u32)> = Vec::new();
        for (name, votes) in tags {
            let name = tidy(name.as_ref());
            let key = genre_key(name);
            if key.is_empty() {
                continue;
            }
            if let Some((genre, named)) = merged.iter_mut().find(|(g, _)| g.key == key) {
                genre.votes = genre.votes.saturating_add(votes);
                if votes > *named {
                    genre.name = name.to_owned();
                    *named = votes;
                }
            } else {
                merged.push((
                    Genre {
                        key,
                        name: name.to_owned(),
                        votes,
                    },
                    votes,
                ));
            }
        }
        let mut genres: Vec<Genre> = merged
            .into_iter()
            .map(|(g, _)| g)
            .filter(|g| g.votes >= MIN_GENRE_VOTES)
            .collect();
        // Stable: equal votes keep their order.
        genres.sort_by_key(|g| std::cmp::Reverse(g.votes));
        Self { genres }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Genre] {
        &self.genres
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Genre> {
        self.genres.iter()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.genres.is_empty()
    }

    /// Whether the book has the genre with this [`genre_key`].
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.genres.iter().any(|g| g.key == key)
    }
}

impl<'a> IntoIterator for &'a BookGenres {
    type Item = &'a Genre;
    type IntoIter = std::slice::Iter<'a, Genre>;

    fn into_iter(self) -> Self::IntoIter {
        self.genres.iter()
    }
}

/// A name with surrounding spaces, quotes and brackets taken off: a few
/// genres come with them (`"Fiction"`, `"Audiobooks"]`).
fn tidy(name: &str) -> &str {
    name.trim_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '[' | ']'))
}

/// A genre with a tag page on Discover's Browse list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BrowseGenre {
    pub id: TagId,
    pub name: &'static str,
}

const fn browse(id: i64, name: &'static str) -> BrowseGenre {
    BrowseGenre {
        id: TagId::new(id),
        name,
    }
}

/// Discover's Browse list, in its order, as the design system has it, with
/// Hardcover's genre tag ids.
pub const BROWSE_GENRES: [BrowseGenre; 12] = [
    browse(2, "Fantasy"),
    browse(6, "Science Fiction"),
    browse(3, "Young Adult"),
    browse(11, "Romance"),
    browse(7, "Mystery"),
    browse(18, "Thriller"),
    browse(52, "Horror"),
    browse(26, "History"),
    browse(1, "Classics"),
    browse(16, "Biography"),
    browse(4, "Adventure"),
    browse(21, "War"),
];

/// The Browse genre a genre name is a spelling of, if any: how a genre on a
/// book (which `cached_tags` names without an id) finds its tag page.
#[must_use]
pub fn browse_genre(name: &str) -> Option<BrowseGenre> {
    let key = genre_key(name);
    BROWSE_GENRES.into_iter().find(|g| genre_key(g.name) == key)
}

/// Whether a book is fiction or not, as its genres say (Stats' split).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Fiction,
    NonFiction,
}

/// Genres that are only ever non-fiction, by [`genre_key`].
const NON_FICTION_KEYS: [&str; 26] = [
    "nonfiction",
    "biography",
    "autobiography",
    "memoir",
    "memoirs",
    "history",
    "self help",
    "psychology",
    "business",
    "philosophy",
    "science",
    "popular science",
    "true crime",
    "essays",
    "religion",
    "politics",
    "economics",
    "travel",
    "health",
    "education",
    "cooking",
    "reference",
    "parenting",
    "spirituality",
    "sociology",
    "personal development",
];

/// Genres that are only ever fiction, by [`genre_key`]. Any key ending in
/// "fiction" (science fiction, literary fiction) or naming fantasy counts
/// too.
const FICTION_KEYS: [&str; 16] = [
    "fiction",
    "romance",
    "romantasy",
    "mystery",
    "thriller",
    "horror",
    "dystopian",
    "dystopia",
    "litrpg",
    "space opera",
    "cyberpunk",
    "magical realism",
    "crime",
    "suspense",
    "short stories",
    "novels",
];

fn is_fiction_key(key: &str) -> bool {
    FICTION_KEYS.contains(&key)
        || key.ends_with(" fiction")
        || key.split(' ').any(|w| w == "fantasy")
}

/// Fiction or non-fiction, from the votes of the book's genres that say
/// which: the side with more votes wins. `None` when no genre says, or the
/// two sides tie. Genres such as Classics, Adventure or Young Adult say
/// nothing either way.
#[must_use]
pub fn kind_of(genres: &BookGenres) -> Option<Kind> {
    let (mut fiction, mut non_fiction) = (0u32, 0u32);
    for genre in genres {
        if NON_FICTION_KEYS.contains(&genre.key.as_str()) {
            non_fiction = non_fiction.saturating_add(genre.votes);
        } else if is_fiction_key(&genre.key) {
            fiction = fiction.saturating_add(genre.votes);
        }
    }
    match fiction.cmp(&non_fiction) {
        std::cmp::Ordering::Greater => Some(Kind::Fiction),
        std::cmp::Ordering::Less => Some(Kind::NonFiction),
        std::cmp::Ordering::Equal => None,
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    fn kind(tags: &[(&str, u32)]) -> Option<Kind> {
        kind_of(&BookGenres::from_tags(tags.iter().copied()))
    }

    #[test]
    fn the_side_with_more_votes_decides() {
        assert_eq!(
            kind(&[("Fantasy", 29), ("Adventure", 4)]),
            Some(Kind::Fiction)
        );
        assert_eq!(
            kind(&[("Science Fiction", 67), ("Space", 15)]),
            Some(Kind::Fiction)
        );
        assert_eq!(kind(&[("Sci-fi", 5)]), Some(Kind::Fiction));
        assert_eq!(
            kind(&[("Self-Help", 40), ("Psychology", 12)]),
            Some(Kind::NonFiction)
        );
        assert_eq!(
            kind(&[("Non-Fiction", 9), ("Memoir", 8)]),
            Some(Kind::NonFiction)
        );
        // A novel about history is still a novel.
        assert_eq!(
            kind(&[("Historical Fiction", 30), ("History", 6)]),
            Some(Kind::Fiction)
        );
        assert_eq!(
            kind(&[("Fiction", 10), ("Nonfiction", 20)]),
            Some(Kind::NonFiction)
        );
    }

    #[test]
    fn genres_that_say_neither_leave_it_unknown() {
        assert_eq!(kind(&[("Classics", 40), ("Adventure", 12)]), None);
        assert_eq!(kind(&[]), None);
        assert_eq!(kind(&[("Fiction", 10), ("Nonfiction", 10)]), None);
        // "Science" alone is non-fiction; "Science Fiction" is not.
        assert_eq!(kind(&[("Science", 10)]), Some(Kind::NonFiction));
    }
}
