//! The demo's community tags: each catalogue book's real votes for the
//! moods, pace and character tags (`votes`, from Hardcover's public
//! `taggable_counts`, read 2026-11-23) and its genres with their real votes
//! (`cached_tags`, the names the catalogue already had).
//!
//! `cached_tags` names a genre without an id, but a tag page asks by id. A
//! genre the Browse list has ([`crate::tags::BROWSE_GENRES`], by exact name)
//! takes its Hardcover id; every other genre name gets a demo id of its own,
//! numbered from [`GENRE_IDS`] in catalogue order, as authors and series
//! are. Two spellings of one genre ("Science Fiction", "Sci-fi") are two
//! tags, as on Hardcover.
//!
//! The upcoming books have no tags.

use serde_json::{Value, json};

use super::{Book, CATALOGUE, Catalogue, book_summary};
use crate::tags::BROWSE_GENRES;

/// Demo ids for the genres the Browse list doesn't have.
const GENRE_IDS: i64 = 90_000;

/// The ids in the variable `name`, an `[Int!]!`; none when it is missing.
fn ids(vars: &Value, name: &str) -> Vec<i64> {
    vars[name]
        .as_array()
        .map_or_default(|ids| ids.iter().filter_map(Value::as_i64).collect())
}

impl Catalogue {
    /// The tag id of a genre name: the Browse list's, else a demo one.
    pub(super) fn genre_id(&self, name: &str) -> i64 {
        if let Some(g) = BROWSE_GENRES.iter().find(|g| g.name == name) {
            return g.id.get();
        }
        let others = self.books.iter().flat_map(|b| {
            b.genres
                .iter()
                .map(|g| g.name.as_str())
                .filter(|n| BROWSE_GENRES.iter().all(|g| g.name != *n))
        });
        Self::numbered(others, GENRE_IDS)
            .into_iter()
            .find(|(_, n)| *n == name)
            .map_or(0, |(id, _)| id)
    }

    /// A book's votes for a tag, a genre or a vote tag; 0 for none.
    fn tag_votes(&self, book: &Book, tag: i64) -> u32 {
        if let Some(n) = book.votes.get(&tag) {
            return *n;
        }
        book.genres
            .iter()
            .find(|g| self.genre_id(&g.name) == tag)
            .map_or(0, |g| g.votes)
    }
}

/// `cached_tags`' Genre list for a book.
pub(super) fn genre_list(book: &Book) -> Value {
    book.genres
        .iter()
        .map(|g| {
            json!({
                "tag": g.name,
                "tagSlug": super::slug(&g.name),
                "category": "Genre",
                "categorySlug": "genre",
                "spoilerRatio": 0,
                "count": g.votes,
            })
        })
        .collect()
}

/// `taggable_counts(where: {tag_id: {_in: $tags}})` on a book.
pub(super) fn taggable_counts(book: &Book, vars: &Value) -> Value {
    ids(vars, "tags")
        .into_iter()
        .filter_map(|tag| {
            let n = CATALOGUE.tag_votes(book, tag);
            (n > 0).then(|| json!({ "tag_id": tag, "count": n }))
        })
        .collect()
}

/// `BooksByTag` and, with `narrowed`, `BooksByTagWith`: the catalogue's
/// books with votes for `$tag` (and at least `$min` for `$other`), the most
/// voted first, then by book id, a page by `$offset` and `$limit`.
pub(super) fn books_by_tag(catalogue: &Catalogue, vars: &Value, narrowed: bool) -> Value {
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    let tag = int("tag");
    let (other, min) = (int("other"), int("min"));
    let mut rows: Vec<(u32, &Book)> = catalogue
        .books
        .iter()
        .map(|b| (catalogue.tag_votes(b, tag), b))
        .filter(|(n, _)| *n > 0)
        .filter(|(_, b)| !narrowed || i64::from(catalogue.tag_votes(b, other)) >= min)
        .collect();
    rows.sort_by_key(|(n, b)| (std::cmp::Reverse(*n), b.id));
    let offset = usize::try_from(int("offset")).unwrap_or_default();
    let limit = usize::try_from(int("limit")).unwrap_or_default();
    let rows: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(n, b)| json!({ "count": n, "book": book_summary(b) }))
        .collect();
    json!({ "taggable_counts": rows })
}

/// `BookTags`: the books in `$ids`, each with its genres and its votes for
/// the tags in `$tags`. An upcoming book has neither.
pub(super) fn book_tags(catalogue: &Catalogue, vars: &Value) -> Value {
    let wanted = ids(vars, "ids");
    let limit = vars["limit"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or_default();
    let books = catalogue
        .books
        .iter()
        .filter(|b| wanted.contains(&b.id))
        .map(|b| {
            json!({
                "id": b.id,
                "cached_tags": genre_list(b),
                "taggable_counts": taggable_counts(b, vars),
            })
        })
        .chain(
            catalogue
                .upcoming
                .iter()
                .filter(|u| wanted.contains(&u.id))
                .map(|u| json!({ "id": u.id, "cached_tags": [], "taggable_counts": [] })),
        )
        .take(limit)
        .collect::<Vec<_>>();
    json!({ "books": books })
}
