//! A tag's page, as data (`hardcover-design`, A tag's page): which tag it
//! is (a mood or a genre), what the one select can narrow it by, what to
//! ask Hardcover for, and the page's words. Pure and tested; the view is
//! [`super::tag_view`].
//!
//! The order always goes by the mood's votes when a mood is involved: on a
//! mood's page narrowed by a genre, and on a genre's page narrowed by a
//! mood. A genre's page with no mood chosen goes by the genre's votes. So
//! both pages of one mood and genre ask for the same books
//! ([`request_key`]).

use hardcover_core::BookSummary;
use hardcover_core::tags::{BROWSE_GENRES, BrowseGenre, Mood};

use crate::format::count_text;
use crate::resource::catalog::TagBooksKey;

/// A tag with a page: one of the 14 moods, or a genre on Discover's Browse
/// list (`tags::browse_genre` finds one from a genre's name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Mood(Mood),
    Genre(BrowseGenre),
}

impl Tag {
    /// The tag's name, as the page's title: "Relaxing", "Science Fiction".
    pub fn name(self) -> &'static str {
        match self {
            Tag::Mood(mood) => mood.name(),
            Tag::Genre(genre) => genre.name,
        }
    }

    /// The tags of the other kind, which the page's select narrows by, in
    /// their order: Browse's genres on a mood's page, the 14 moods on a
    /// genre's.
    pub fn others(self) -> Vec<Tag> {
        match self {
            Tag::Mood(_) => BROWSE_GENRES.into_iter().map(Tag::Genre).collect(),
            Tag::Genre(_) => Mood::ALL.into_iter().map(Tag::Mood).collect(),
        }
    }

    /// The select's first choice, which narrows nothing.
    pub fn any_other(self) -> &'static str {
        match self {
            Tag::Mood(_) => "Any genre",
            Tag::Genre(_) => "Any mood",
        }
    }
}

/// The mood and the genre of a page and its choice, either of them absent.
fn mood_and_genre(tag: Tag, within: Option<Tag>) -> (Option<Mood>, Option<BrowseGenre>) {
    let mut mood = None;
    let mut genre = None;
    for t in std::iter::once(tag).chain(within) {
        match t {
            Tag::Mood(m) => mood = Some(m),
            Tag::Genre(g) => genre = Some(g),
        }
    }
    (mood, genre)
}

/// What the page asks Hardcover for: the mood ranks whenever there is one,
/// narrowed by the genre; else the genre ranks alone.
pub fn request_key(tag: Tag, within: Option<Tag>) -> TagBooksKey {
    match mood_and_genre(tag, within) {
        (Some(mood), genre) => TagBooksKey {
            tag: mood.id(),
            within: genre.map(|g| g.id),
        },
        (None, Some(genre)) => TagBooksKey {
            tag: genre.id,
            within: None,
        },
        (None, None) => unreachable!("a page has a tag"),
    }
}

/// The muted line under the title, at the prose measure.
pub fn intro(tag: Tag, within: Option<Tag>) -> String {
    match mood_and_genre(tag, within) {
        (Some(mood), Some(genre)) => format!(
            "{} books Hardcover readers call {}, the most votes first.",
            genre.name,
            mood.name().to_lowercase()
        ),
        (Some(mood), None) => format!(
            "Books Hardcover readers call {}, the most votes first.",
            mood.name().to_lowercase()
        ),
        (None, Some(genre)) => format!(
            "Books Hardcover readers file under {}, the most votes first.",
            genre.name
        ),
        (None, None) => String::new(),
    }
}

/// The empty state's sentence, for a choice with no books.
pub fn empty_text(tag: Tag, within: Option<Tag>) -> String {
    match mood_and_genre(tag, within) {
        (Some(mood), Some(genre)) => format!(
            "Hardcover has no {} books readers call {}.",
            genre.name,
            mood.name().to_lowercase()
        ),
        (Some(mood), None) => format!(
            "Hardcover has no books readers call {}.",
            mood.name().to_lowercase()
        ),
        (None, Some(genre)) => format!("Hardcover has no books readers file under {}.", genre.name),
        (None, None) => String::new(),
    }
}

/// "1 vote", "1,204 votes".
pub fn votes_text(votes: u32) -> String {
    if votes == 1 {
        "1 vote".to_owned()
    } else {
        format!("{} votes", count_text(votes))
    }
}

/// A row's muted line: the author, the year and the votes the order goes
/// by ("Travis Baldree · 2022 · 1,204 votes").
pub fn row_line(book: &BookSummary, votes: u32) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !book.authors.is_empty() {
        parts.push(book.authors.join(", "));
    }
    if let Some(year) = book.release_year {
        parts.push(year.to_string());
    }
    parts.push(votes_text(votes));
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use hardcover_core::BookId;
    use hardcover_core::tags::browse_genre;

    use super::*;

    fn mystery() -> Tag {
        Tag::Genre(browse_genre("Mystery").unwrap())
    }

    fn mystery_id() -> hardcover_core::TagId {
        browse_genre("Mystery").unwrap().id
    }

    #[test]
    fn the_mood_ranks_whenever_there_is_one() {
        let relaxing = Tag::Mood(Mood::Relaxing);
        let mood_page = request_key(relaxing, Some(mystery()));
        let genre_page = request_key(mystery(), Some(relaxing));
        assert_eq!(mood_page, genre_page, "both pages ask for the same books");
        assert_eq!(mood_page.tag, Mood::Relaxing.id());
        assert_eq!(mood_page.within, Some(mystery_id()));

        assert_eq!(
            request_key(relaxing, None),
            TagBooksKey {
                tag: Mood::Relaxing.id(),
                within: None
            }
        );
        assert_eq!(
            request_key(mystery(), None),
            TagBooksKey {
                tag: mystery_id(),
                within: None
            }
        );
    }

    #[test]
    fn the_intro_and_the_empty_state_name_the_choice() {
        let relaxing = Tag::Mood(Mood::Relaxing);
        let science = Tag::Genre(browse_genre("Sci-fi").unwrap());
        assert_eq!(
            intro(relaxing, None),
            "Books Hardcover readers call relaxing, the most votes first."
        );
        assert_eq!(
            intro(relaxing, Some(mystery())),
            "Mystery books Hardcover readers call relaxing, the most votes first."
        );
        assert_eq!(
            intro(mystery(), Some(relaxing)),
            intro(relaxing, Some(mystery())),
            "a genre's page narrowed by a mood reads the same"
        );
        assert_eq!(
            intro(science, None),
            "Books Hardcover readers file under Science Fiction, the most votes first."
        );
        assert_eq!(
            empty_text(relaxing, Some(mystery())),
            "Hardcover has no Mystery books readers call relaxing."
        );
    }

    #[test]
    fn the_select_offers_the_other_kind() {
        let mood = Tag::Mood(Mood::Dark);
        assert_eq!(mood.any_other(), "Any genre");
        assert_eq!(mood.others().len(), BROWSE_GENRES.len());
        assert!(mood.others().iter().all(|t| matches!(t, Tag::Genre(_))));
        assert_eq!(mystery().any_other(), "Any mood");
        assert_eq!(mystery().others().len(), 14);
        assert_eq!(mystery().others()[0], Tag::Mood(Mood::Adventurous));
    }

    #[test]
    fn a_row_reads_author_year_and_votes() {
        let mut book = BookSummary::new(BookId::new(1), "Legends & Lattes");
        assert_eq!(row_line(&book, 1), "1 vote");
        book.authors = vec!["Travis Baldree".into()];
        book.release_year = Some(2022);
        book.average_rating = Some(4.2);
        assert_eq!(
            row_line(&book, 1204),
            "Travis Baldree · 2022 · 1,204 votes",
            "no average: the votes are what the order goes by"
        );
    }
}
