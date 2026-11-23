//! Raw response shapes and their lenient conversion into the public models.
//!
//! Hardcover's `cached_*` columns are JSON blobs with no schema, and search results
//! are a raw Typesense response, so those are read field by field from
//! `serde_json::Value` and anything unexpected is dropped rather than failing the
//! whole response.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::model::{
    AuthorBook, AuthorDetails, AuthorHit, AuthorId, AuthorRef, AuthorSeries, BookDetails, BookId,
    BookSummary, CommunityReview, Contributor, Cover, Date, EditionId, EditionParts,
    EditionSummary, Goal, GoalConditions, GoalId, GoalMetric, Length, LibraryEntry, ListBook,
    ListBookId, ListId, ListMembership, Measure, Privacy, Rating, RatingBucket, ReadId,
    ReadSession, ReadingStatus, Review, Reviewer, SearchHits, SearchPage, SeriesBook,
    SeriesDetails, SeriesHit, SeriesId, SeriesRef, Timestamp, TrackedBook, TrackedSeries,
    UpcomingBook, User, UserBookId, UserId, UserList, measure_of, read_order,
};

// ---------- scalar helpers ----------

pub(crate) fn as_i64(v: &Value) -> Option<i64> {
    match v {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the lenient wire decoder also accepts decimal numbers and discards their fractional part"
        )]
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub(crate) fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "GraphQL Float is f64 and the display model uses f32"
)]
fn model_float(value: f64) -> f32 {
    value as f32
}

fn as_u32(v: &Value) -> Option<u32> {
    as_i64(v).and_then(|n| u32::try_from(n).ok())
}

fn as_string(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn opt_u32(n: Option<i64>) -> Option<u32> {
    n.and_then(|n| u32::try_from(n).ok())
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// A description or bio, as plain text: Hardcover sends some as HTML.
fn catalogue_text(s: Option<String>) -> Option<String> {
    non_empty(s.map(|s| super::markup::plain_text(&s)))
}

pub(crate) fn parse_date(s: &str) -> Option<Date> {
    let s = s.trim();
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

pub(crate) fn parse_timestamp(s: &str) -> Option<Timestamp> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    // Hasura sometimes renders "+00:00" as "+00", and `timestamp` columns have no offset.
    let trimmed = s.strip_suffix("+00").unwrap_or(s);
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc())
}

// ---------- shared blobs ----------

/// An `images` row or a `cached_image` blob. An image-less book has `{}`.
pub(crate) fn cover_from_value(v: &Value) -> Option<Cover> {
    let url = v.get("url").and_then(as_string)?;
    let mut c = Cover::new(url);
    c.width = v.get("width").and_then(as_u32);
    c.height = v.get("height").and_then(as_u32);
    Some(c)
}

/// `cached_contributors` (and search `contributions`): primary authors are the
/// entries with no role or the "Author" role. Falls back to everyone credited.
pub(crate) fn authors_from_contributors(v: &Value) -> Vec<String> {
    let Some(list) = v.as_array() else {
        return Vec::new();
    };
    let named: Vec<(Option<String>, String)> = list
        .iter()
        .filter_map(|c| {
            let name = c
                .get("author")
                .and_then(|a| a.get("name"))
                .and_then(as_string)?;
            Some((c.get("contribution").and_then(as_string), name))
        })
        .collect();
    let mut authors: Vec<String> = named
        .iter()
        .filter(|(role, _)| {
            role.as_deref()
                .is_none_or(|r| r.eq_ignore_ascii_case("author"))
        })
        .map(|(_, n)| n.clone())
        .collect();
    if authors.is_empty() {
        authors = named.into_iter().map(|(_, n)| n).collect();
    }
    dedup(authors)
}

/// Everyone in `cached_contributors`, in credit order, with their author id
/// (`author.id`) and role. The same person in the same role once.
pub(crate) fn contributors_from_cached(v: &Value) -> Vec<Contributor> {
    let Some(list) = v.as_array() else {
        return Vec::new();
    };
    let mut out: Vec<Contributor> = Vec::new();
    for c in list {
        let author = c.get("author");
        let Some(name) = author.and_then(|a| a.get("name")).and_then(as_string) else {
            continue;
        };
        let mut contributor = Contributor::new(name);
        contributor.author_id = author
            .and_then(|a| a.get("id"))
            .and_then(as_i64)
            .map(AuthorId::new);
        contributor.role = c.get("contribution").and_then(as_string);
        let duplicate = out.iter().any(|o| {
            o.name == contributor.name
                && o.is_author() == contributor.is_author()
                && (contributor.is_author() || o.role == contributor.role)
        });
        if !duplicate {
            out.push(contributor);
        }
    }
    out
}

/// `ratings_distribution`, highest rating first. Accepts a list of
/// `{rating, count}` objects or an object keyed by rating (`{"4.5": 12}`);
/// zero counts and ratings outside 0.5..=5 are dropped.
pub(crate) fn ratings_distribution(v: &Value) -> Vec<RatingBucket> {
    let pairs: Vec<(Option<f64>, Option<i64>)> = match v {
        Value::Array(items) => items
            .iter()
            .map(|i| {
                (
                    i.get("rating").and_then(as_f64),
                    i.get("count").and_then(as_i64),
                )
            })
            .collect(),
        Value::Object(map) => map
            .iter()
            .map(|(k, n)| (k.trim().parse::<f64>().ok(), as_i64(n)))
            .collect(),
        _ => Vec::new(),
    };
    let mut buckets: Vec<RatingBucket> = Vec::new();
    for (rating, count) in pairs {
        let (Some(rating), Some(count)) = (rating, count.and_then(|c| u32::try_from(c).ok()))
        else {
            continue;
        };
        if !(0.5..=5.0).contains(&rating) || count == 0 {
            continue;
        }
        let Some(rating) = Rating::from_api(rating).map(Rating::value) else {
            continue;
        };
        match buckets
            .iter_mut()
            .find(|b| b.rating.total_cmp(&rating).is_eq())
        {
            Some(b) => b.count = b.count.saturating_add(count),
            None => buckets.push(RatingBucket::new(rating, count)),
        }
    }
    buckets.sort_by(|a, b| b.rating.total_cmp(&a.rating));
    buckets
}

/// `alternative_titles`: strings, or objects with a `title`. Leaves out the
/// book's own title and repeats.
fn alternative_titles(v: &Value, own: &str) -> Vec<String> {
    let Some(list) = v.as_array() else {
        return Vec::new();
    };
    let mut titles: Vec<String> = Vec::new();
    for title in list
        .iter()
        .filter_map(|t| as_string(t).or_else(|| t.get("title").and_then(as_string)))
    {
        let repeat = title.eq_ignore_ascii_case(own)
            || titles.iter().any(|t| t.eq_ignore_ascii_case(&title));
        if !repeat {
            titles.push(title);
        }
    }
    titles
}

/// `cached_similar_book_ids`: ids (numbers or strings), or objects with an `id`.
fn book_ids(v: &Value) -> Vec<BookId> {
    let Some(list) = v.as_array() else {
        return Vec::new();
    };
    let mut ids: Vec<BookId> = Vec::new();
    for item in list {
        let id = as_i64(item).or_else(|| item.get("id").and_then(as_i64));
        if let Some(id) = id.filter(|id| *id > 0).map(BookId::new)
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    ids
}

fn dedup(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map_or_default(|a| a.iter().filter_map(as_string).collect())
}

/// `cached_tags` is `{"Genre": [{"tag": "...", "count": n, ...}], "Mood": [...], ...}`.
fn tags_in(cached_tags: &Value, category: &str) -> Vec<String> {
    let Some(list) = cached_tags.get(category).and_then(Value::as_array) else {
        return Vec::new();
    };
    dedup(
        list.iter()
            .filter_map(|t| t.get("tag").and_then(as_string))
            .collect(),
    )
}

// ---------- books ----------

#[derive(Debug, Deserialize)]
pub(crate) struct RawBook {
    pub id: i64,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub slug: Option<String>,
    pub pages: Option<i64>,
    pub release_year: Option<i64>,
    pub rating: Option<Value>,
    pub users_count: Option<i64>,
    #[serde(default)]
    pub cached_image: Value,
    #[serde(default)]
    pub cached_contributors: Value,
}

impl RawBook {
    pub fn into_summary(self) -> BookSummary {
        let mut b = BookSummary::new(
            BookId::new(self.id),
            non_empty(self.title).unwrap_or_else(|| "Untitled".to_owned()),
        );
        b.subtitle = non_empty(self.subtitle);
        b.authors = authors_from_contributors(&self.cached_contributors);
        b.cover = cover_from_value(&self.cached_image);
        b.pages = opt_u32(self.pages).filter(|p| *p > 0);
        b.release_year = self.release_year.and_then(|y| i32::try_from(y).ok());
        b.slug = non_empty(self.slug);
        b.average_rating = self.rating.as_ref().and_then(as_f64).map(model_float);
        b.users_count = opt_u32(self.users_count);
        b
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawBookDetails {
    #[serde(flatten)]
    pub book: RawBook,
    pub description: Option<String>,
    pub release_date: Option<String>,
    pub editions_count: Option<i64>,
    pub ratings_count: Option<i64>,
    #[serde(default)]
    pub cached_tags: Value,
    #[serde(default)]
    pub ratings_distribution: Value,
    #[serde(default)]
    pub alternative_titles: Value,
    #[serde(default)]
    pub cached_similar_book_ids: Value,
    #[serde(default)]
    pub book_series: Vec<RawBookSeries>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawBookSeries {
    pub position: Option<f64>,
    #[serde(default)]
    pub featured: bool,
    /// Asked for by `UserBookFields` only.
    #[serde(default)]
    pub compilation: bool,
    pub series: Option<RawSeries>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawSeries {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub slug: Option<String>,
}

impl RawBookDetails {
    pub fn into_details(self) -> BookDetails {
        let tags = &self.cached_tags;
        let contributors = contributors_from_cached(&self.book.cached_contributors);
        let mut d = BookDetails::new(self.book.into_summary());
        d.contributors = contributors;
        d.ratings_distribution = ratings_distribution(&self.ratings_distribution);
        d.alternative_titles = alternative_titles(&self.alternative_titles, &d.summary.title);
        d.similar_book_ids = book_ids(&self.cached_similar_book_ids);
        d.description = catalogue_text(self.description);
        d.release_date = self.release_date.as_deref().and_then(parse_date);
        d.genres = tags_in(tags, "Genre");
        d.moods = tags_in(tags, "Mood");
        d.tags = tags_in(tags, "Tag");
        d.content_warnings = tags_in(tags, "Content Warning");
        d.editions_count = opt_u32(self.editions_count);
        d.ratings_count = opt_u32(self.ratings_count);
        d.series = series_refs(self.book_series);
        d
    }
}

/// A book's `book_series` rows, in the order asked for (featured first). A
/// series listed twice, or without a name, is left out.
fn series_refs(rows: Vec<RawBookSeries>) -> Vec<SeriesRef> {
    let mut refs: Vec<SeriesRef> = Vec::new();
    for bs in rows {
        let Some(series) = bs.series else { continue };
        let Some(name) = non_empty(series.name) else {
            continue;
        };
        if refs.iter().any(|s| s.name == name) {
            continue;
        }
        let mut s = SeriesRef::new(name);
        s.id = series.id.map(SeriesId::new);
        s.slug = non_empty(series.slug);
        s.position = bs.position.filter(|p| p.is_finite()).map(model_float);
        s.featured = bs.featured;
        s.compilation = bs.compilation;
        refs.push(s);
    }
    refs
}

// ---------- series and authors ----------

#[derive(Debug, Deserialize)]
pub(crate) struct RawAuthorRef {
    pub id: i64,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawSeriesPage {
    pub id: i64,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub primary_books_count: Option<i64>,
    pub is_completed: Option<bool>,
    pub author: Option<RawAuthorRef>,
    #[serde(default)]
    pub book_series: Vec<RawSeriesEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawSeriesEntry {
    pub position: Option<f64>,
    #[serde(default)]
    pub compilation: bool,
    pub book: Option<RawPartialBook>,
}

/// A book with the `is_partial_book` flag the catalogue pages filter on.
#[derive(Debug, Deserialize)]
pub(crate) struct RawPartialBook {
    #[serde(flatten)]
    pub book: RawBook,
    pub is_partial_book: Option<bool>,
}

impl RawSeriesPage {
    pub fn into_details(self) -> SeriesDetails {
        let mut d = SeriesDetails::new(
            SeriesId::new(self.id),
            non_empty(self.name).unwrap_or_else(|| "Untitled series".to_owned()),
        );
        d.slug = non_empty(self.slug);
        d.description = catalogue_text(self.description);
        d.primary_books_count = opt_u32(self.primary_books_count);
        d.is_completed = self.is_completed;
        d.author = self
            .author
            .and_then(|a| Some(AuthorRef::new(AuthorId::new(a.id), non_empty(a.name)?)));
        let entries = self.book_series.into_iter().filter_map(|e| {
            let book = e.book?;
            if e.compilation || book.is_partial_book == Some(true) {
                return None;
            }
            let mut b = SeriesBook::new(book.book.into_summary());
            b.position = e
                .position
                .filter(|p| p.is_finite() && *p >= 0.0)
                .map(model_float);
            Some(b)
        });
        d.books = series_reading_order(entries.collect());
        d
    }
}

/// Numbered books by position, keeping the most-read book for each position
/// (unmerged duplicates and translations share it), then unnumbered books by
/// popularity with repeated titles dropped. A book appears once.
pub(crate) fn series_reading_order(books: Vec<SeriesBook>) -> Vec<SeriesBook> {
    let users = |b: &SeriesBook| b.book.users_count.unwrap_or(0);
    let mut numbered: Vec<SeriesBook> = Vec::new();
    let mut unnumbered: Vec<SeriesBook> = Vec::new();
    for b in books {
        let seen = numbered
            .iter()
            .chain(unnumbered.iter())
            .any(|o| o.book.id == b.book.id);
        if seen {
            continue;
        }
        match b.position {
            Some(p) => match numbered.iter_mut().find(|o| o.position == Some(p)) {
                Some(o) if users(&b) > users(o) => *o = b,
                Some(_) => {}
                None => numbered.push(b),
            },
            None => unnumbered.push(b),
        }
    }
    numbered.sort_by(|a, b| {
        a.position
            .unwrap_or_default()
            .total_cmp(&b.position.unwrap_or_default())
    });
    unnumbered.sort_by_key(|b| std::cmp::Reverse(users(b)));
    let mut out = numbered;
    for b in unnumbered {
        let repeat = out
            .iter()
            .any(|o| o.book.title.eq_ignore_ascii_case(&b.book.title));
        if !repeat {
            out.push(b);
        }
    }
    out
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawAuthorPage {
    pub id: i64,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub bio: Option<String>,
    #[serde(default)]
    pub cached_image: Value,
    pub born_year: Option<i64>,
    pub death_year: Option<i64>,
    pub books_count: Option<i64>,
    #[serde(default)]
    pub contributions: Vec<RawAuthorContribution>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawAuthorContribution {
    pub contribution: Option<String>,
    pub book: Option<RawAuthorBook>,
}

/// A book on an author page, with its series.
#[derive(Debug, Deserialize)]
pub(crate) struct RawAuthorBook {
    #[serde(flatten)]
    pub book: RawPartialBook,
    #[serde(default)]
    pub book_series: Vec<RawAuthorBookSeries>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawAuthorBookSeries {
    pub series: Option<RawAuthorSeries>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawAuthorSeries {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub author_id: Option<i64>,
    pub canonical_id: Option<i64>,
    pub primary_books_count: Option<i64>,
    pub is_completed: Option<bool>,
}

impl RawAuthorPage {
    pub fn into_details(self) -> AuthorDetails {
        let year = |y: Option<i64>| y.and_then(|y| i32::try_from(y).ok()).filter(|y| *y != 0);
        let mut d = AuthorDetails::new(
            AuthorId::new(self.id),
            non_empty(self.name).unwrap_or_else(|| "Unknown author".to_owned()),
        );
        d.slug = non_empty(self.slug);
        d.bio = catalogue_text(self.bio);
        d.photo = cover_from_value(&self.cached_image);
        d.born_year = year(self.born_year);
        d.death_year = year(self.death_year);
        d.books_count = opt_u32(self.books_count);
        // Each book's series, in the order of the books.
        let mut series_of: Vec<(BookId, Vec<RawAuthorSeries>)> = Vec::new();
        for c in self.contributions {
            let Some(book) = c.book else { continue };
            if book.book.is_partial_book == Some(true) {
                continue;
            }
            let role = non_empty(c.contribution).filter(|r| !r.eq_ignore_ascii_case("author"));
            let summary = book.book.book.into_summary();
            if let Some(existing) = d.books.iter_mut().find(|b| b.book.id == summary.id) {
                if role.is_none() {
                    existing.role = None;
                }
            } else {
                series_of.push((
                    summary.id,
                    book.book_series
                        .into_iter()
                        .filter_map(|s| s.series)
                        .collect(),
                ));
                let mut b = AuthorBook::new(summary);
                b.role = role;
                d.books.push(b);
            }
        }
        d.series = author_series(d.id, &d.books, series_of);
        d
    }
}

/// An author's series, from the books on their page (most read first) and
/// each book's series. Only books they wrote count, not those they edited,
/// introduced or translated. A series counts when Hardcover names them its
/// author, or when at least two of their books are in it: one book alone in
/// someone else's series is mostly a crossover, a shared world or a mistaken
/// credit (checked live, read-only, on six prolific authors: Batman, Magic:
/// The Gathering, Pern), while a co-written series keeps two or more (The
/// Wheel of Time). The most read first, by the readers of their books in it.
fn author_series(
    author: AuthorId,
    books: &[AuthorBook],
    series_of: Vec<(BookId, Vec<RawAuthorSeries>)>,
) -> Vec<AuthorSeries> {
    struct Tally {
        series: AuthorSeries,
        theirs: bool,
        readers: u64,
    }
    let mut tallies: Vec<Tally> = Vec::new();
    for (book_id, series) in series_of {
        let Some(book) = books
            .iter()
            .find(|b| b.book.id == book_id && b.role.is_none())
        else {
            continue;
        };
        let mut seen: Vec<i64> = Vec::new();
        for s in series {
            let Some(id) = s.id else { continue };
            if s.canonical_id.is_some() || seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let readers = u64::from(book.book.users_count.unwrap_or(0));
            if let Some(t) = tallies.iter_mut().find(|t| t.series.id.get() == id) {
                t.series.books += 1;
                t.readers += readers;
                continue;
            }
            let Some(name) = non_empty(s.name) else {
                continue;
            };
            let mut series = AuthorSeries::new(SeriesId::new(id), name, book.book.clone());
            series.primary_books_count = opt_u32(s.primary_books_count).filter(|n| *n > 0);
            series.is_completed = s.is_completed;
            tallies.push(Tally {
                series,
                theirs: s.author_id == Some(author.get()),
                readers,
            });
        }
    }
    tallies.retain(|t| t.theirs || t.series.books >= 2);
    // Stable: equal readers keep the order of their most-read book.
    tallies.sort_by_key(|t| std::cmp::Reverse(t.readers));
    tallies.into_iter().map(|t| t.series).collect()
}

// ---------- library ----------

/// `EditionFields`, or the shorter selection on a read's edition (the rest absent).
#[derive(Debug, Deserialize)]
pub(crate) struct RawEdition {
    pub id: i64,
    pub title: Option<String>,
    pub pages: Option<i64>,
    pub audio_seconds: Option<i64>,
    pub edition_format: Option<String>,
    #[serde(default)]
    pub cached_image: Value,
    pub release_date: Option<String>,
    pub isbn_13: Option<String>,
    pub reading_format: Option<RawReadingFormat>,
    #[serde(default)]
    pub reading_format_id: Option<i64>,
    pub publisher: Option<RawName>,
    pub language: Option<RawLanguage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawReadingFormat {
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawName {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawLanguage {
    pub language: Option<String>,
}

impl RawEdition {
    pub(crate) fn into_summary(self) -> EditionSummary {
        let mut e = EditionSummary::new(EditionId::new(self.id));
        e.title = non_empty(self.title);
        e.pages = opt_u32(self.pages).filter(|p| *p > 0);
        e.audio_seconds = opt_u32(self.audio_seconds).filter(|s| *s > 0);
        e.cover = cover_from_value(&self.cached_image);
        e.format = non_empty(self.edition_format);
        e.reading_format = self.reading_format.and_then(|f| non_empty(f.format));
        e.reading_format_id = self.reading_format_id;
        e.release_date = self.release_date.as_deref().and_then(parse_date);
        e.isbn_13 = non_empty(self.isbn_13);
        e.publisher = self.publisher.and_then(|p| non_empty(p.name));
        e.language = self.language.and_then(|l| non_empty(l.language));
        e
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawRead {
    pub id: i64,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub progress: Option<f64>,
    pub progress_pages: Option<i64>,
    pub progress_seconds: Option<i64>,
    pub edition_id: Option<i64>,
    pub edition: Option<RawEdition>,
}

impl RawRead {
    pub(crate) fn into_session(self) -> ReadSession {
        let mut r = ReadSession::new(ReadId::new(self.id));
        r.started_at = self.started_at.as_deref().and_then(parse_date);
        r.finished_at = self.finished_at.as_deref().and_then(parse_date);
        r.progress_pages = opt_u32(self.progress_pages);
        r.progress_seconds = opt_u32(self.progress_seconds);
        r.progress_percent = self
            .progress
            .filter(|p| p.is_finite())
            .map(|p| model_float(p.clamp(0.0, 100.0)));
        r.edition_id = self.edition_id.map(EditionId::new);
        r.edition = self.edition.map(RawEdition::into_summary);
        r
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawUserBook {
    pub id: i64,
    pub user_id: i64,
    pub status_id: i64,
    pub rating: Option<Value>,
    pub date_added: Option<String>,
    pub updated_at: Option<String>,
    pub privacy_setting_id: Option<i64>,
    pub private_notes: Option<String>,
    /// Null when the book doesn't resolve (removed or hidden from the
    /// catalogue): the row is dropped, not the page.
    pub book: Option<RawUserBookBook>,
    pub edition: Option<RawEdition>,
    #[serde(default)]
    pub user_book_reads: Vec<RawRead>,
}

/// An entry's book: its summary and its series.
#[derive(Debug, Deserialize)]
pub(crate) struct RawUserBookBook {
    #[serde(flatten)]
    pub book: RawBook,
    #[serde(default)]
    pub book_series: Vec<RawBookSeries>,
    /// `books.default_audio_edition_id`: the book's audiobook edition, set
    /// whenever Hardcover lists one.
    #[serde(default)]
    pub default_audio_edition_id: Option<i64>,
}

impl RawUserBook {
    /// `None` when the status id is unknown to [`ReadingStatus`] or the book
    /// didn't resolve.
    pub fn into_entry(self) -> Option<LibraryEntry> {
        let status = ReadingStatus::from_id(self.status_id)?;
        let book = self.book?;
        let mut e = LibraryEntry::new(
            UserBookId::new(self.id),
            UserId::new(self.user_id),
            book.book.into_summary(),
            status,
        );
        e.series = series_refs(book.book_series);
        e.book_audio_edition = book.default_audio_edition_id.map(EditionId::new);
        e.rating = self
            .rating
            .as_ref()
            .and_then(as_f64)
            .and_then(Rating::from_api);
        e.edition = self.edition.map(RawEdition::into_summary);
        e.reads = self
            .user_book_reads
            .into_iter()
            .map(RawRead::into_session)
            .collect();
        e.reads.sort_by(read_order);
        e.date_added = self.date_added.as_deref().and_then(parse_date);
        e.updated_at = self.updated_at.as_deref().and_then(parse_timestamp);
        e.private_notes = self.private_notes.filter(|n| !n.trim().is_empty());
        e.privacy = self.privacy_setting_id.and_then(Privacy::from_id);
        Some(e)
    }
}

/// `<table>_aggregate { aggregate { count } }`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawAggregate {
    pub aggregate: Option<RawCount>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawCount {
    pub count: Option<u64>,
}

impl RawAggregate {
    pub fn count(&self) -> Option<u64> {
        self.aggregate.as_ref().and_then(|a| a.count)
    }
}

// ---------- me ----------

#[derive(Debug, Deserialize)]
pub(crate) struct RawUser {
    pub id: i64,
    pub username: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub image: Value,
}

impl RawUser {
    pub fn into_user(self) -> User {
        let mut u = User::new(UserId::new(self.id), self.username.unwrap_or_default());
        u.name = non_empty(self.name);
        u.avatar = cover_from_value(&self.image);
        u
    }
}

// ---------- search ----------

#[derive(Debug, Deserialize)]
pub(crate) struct RawSearch {
    pub error: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    /// The hits' ids in order; only author and series searches ask for them.
    #[serde(default)]
    pub ids: Value,
    #[serde(default)]
    pub results: Value,
}

impl RawSearch {
    fn total(&self) -> Option<u32> {
        self.results.get("found").and_then(as_u32)
    }
}

/// `results` is the Typesense response: `{found, page, hits: [{document: {...}}]}`.
/// Hits without a usable id are skipped.
pub(crate) fn search_page(raw: &RawSearch, page: u32, per_page: u32) -> SearchPage {
    let results = &raw.results;
    let books = results
        .get("hits")
        .and_then(Value::as_array)
        .map_or_default(|hits| {
            hits.iter()
                .filter_map(|h| h.get("document"))
                .filter_map(book_from_search_document)
                .collect()
        });
    let mut p = SearchPage::new(
        books,
        raw.page.and_then(|n| u32::try_from(n).ok()).unwrap_or(page),
        raw.per_page
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(per_page),
    );
    p.total = raw.total();
    p
}

fn book_from_search_document(doc: &Value) -> Option<BookSummary> {
    let id = doc.get("id").and_then(as_i64)?;
    let title = doc.get("title").and_then(as_string)?;
    let mut b = BookSummary::new(BookId::new(id), title);
    b.subtitle = doc.get("subtitle").and_then(as_string);
    b.authors = dedup(string_list(doc.get("author_names")));
    if b.authors.is_empty() {
        b.authors = doc
            .get("contributions")
            .map_or_default(authors_from_contributors);
    }
    b.cover = doc.get("image").and_then(cover_from_value);
    b.pages = doc.get("pages").and_then(as_u32).filter(|p| *p > 0);
    b.release_year = doc
        .get("release_year")
        .and_then(as_i64)
        .and_then(|y| i32::try_from(y).ok());
    b.slug = doc.get("slug").and_then(as_string);
    b.average_rating = doc.get("rating").and_then(as_f64).map(model_float);
    b.users_count = doc.get("users_count").and_then(as_u32);
    Some(b)
}

// ---------- goals ----------

#[derive(Debug, Deserialize)]
pub(crate) struct RawGoal {
    pub id: i64,
    pub metric: String,
    pub goal: i64,
    pub progress: Option<Value>,
    pub description: Option<String>,
    pub start_date: String,
    pub end_date: String,
    #[serde(default)]
    pub conditions: Value,
    pub archived: Option<bool>,
    pub completed_at: Option<String>,
    pub privacy_setting_id: Option<i64>,
}

impl RawGoal {
    /// `None` if the dates don't parse.
    pub fn into_goal(self) -> Option<Goal> {
        let metric = GoalMetric::from_api(&self.metric);
        let mut progress = self.progress.as_ref().and_then(as_f64).unwrap_or(0.0);
        if metric == GoalMetric::Hours {
            // The target is in hours, the progress in seconds.
            progress /= 3600.0;
        }
        let mut g = Goal::new(
            GoalId::new(self.id),
            metric,
            u32::try_from(self.goal).unwrap_or(0),
            parse_date(&self.start_date)?,
            parse_date(&self.end_date)?,
        );
        g.progress = progress;
        g.description = non_empty(self.description);
        g.archived = self.archived.unwrap_or(false);
        g.completed_at = self.completed_at.as_deref().and_then(parse_timestamp);
        g.privacy_id = self.privacy_setting_id;
        g.conditions = GoalConditions(self.conditions);
        Some(g)
    }
}

/// `GoalIdType`: the goal mutations' payload.
#[derive(Debug, Deserialize)]
pub(crate) struct GoalPayload {
    pub errors: Option<Vec<Option<String>>>,
    pub id: Option<i64>,
    pub goal: Option<RawGoal>,
}

impl GoalPayload {
    /// The payload's errors, joined, if there are any.
    pub fn error(&self) -> Option<String> {
        joined_errors(self.errors.as_deref())
    }
}

/// `results` as for books, with a `parse` for one hit's document. The
/// document's own `id` is preferred; `ids[i]` stands in only when `ids` lines up
/// with the hits. Hits without a usable id or name are skipped.
pub(crate) fn search_hits<T>(
    raw: &RawSearch,
    page: u32,
    per_page: u32,
    parse: impl Fn(&Value, Option<i64>) -> Option<T>,
) -> SearchHits<T> {
    let results = &raw.results;
    let hits: Vec<&Value> = results
        .get("hits")
        .and_then(Value::as_array)
        .map_or_default(|hits| hits.iter().filter_map(|h| h.get("document")).collect());
    let ids: Vec<Option<i64>> = raw
        .ids
        .as_array()
        .map_or_default(|ids| ids.iter().map(as_i64).collect());
    let fallback = |i: usize| {
        if ids.len() == hits.len() {
            ids[i]
        } else {
            None
        }
    };
    let items = hits
        .iter()
        .enumerate()
        .filter_map(|(i, doc)| parse(doc, doc.get("id").and_then(as_i64).or(fallback(i))))
        .collect();
    let mut p = SearchHits::new(
        items,
        raw.page.and_then(|n| u32::try_from(n).ok()).unwrap_or(page),
        raw.per_page
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(per_page),
    );
    p.total = raw.total();
    p
}

/// Titles from a list of strings or of objects with a `title`.
fn titles(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map_or_default(|a| {
        a.iter()
            .filter_map(|t| as_string(t).or_else(|| t.get("title").and_then(as_string)))
            .collect()
    })
}

/// An author search document: `name`, `slug`, `image`, `books_count` and
/// `books` (titles of the top five books).
pub(crate) fn author_from_search_document(doc: &Value, id: Option<i64>) -> Option<AuthorHit> {
    let name = doc.get("name").and_then(as_string)?;
    let mut a = AuthorHit::new(AuthorId::new(id?), name);
    a.slug = doc.get("slug").and_then(as_string);
    a.photo = doc.get("image").and_then(cover_from_value);
    a.books_count = doc.get("books_count").and_then(as_u32);
    a.top_books = dedup(titles(doc.get("books")));
    Some(a)
}

/// A series search document: `name`, `slug`, `author_name` (or an `author`
/// object), `books_count`, `primary_books_count` and `books`.
pub(crate) fn series_from_search_document(doc: &Value, id: Option<i64>) -> Option<SeriesHit> {
    let name = doc.get("name").and_then(as_string)?;
    let mut s = SeriesHit::new(SeriesId::new(id?), name);
    s.slug = doc.get("slug").and_then(as_string);
    s.author = doc.get("author_name").and_then(as_string).or_else(|| {
        doc.get("author")
            .and_then(|a| a.get("name"))
            .and_then(as_string)
    });
    s.books_count = doc.get("books_count").and_then(as_u32);
    s.primary_books_count = doc.get("primary_books_count").and_then(as_u32);
    s.books = dedup(titles(doc.get("books")));
    Some(s)
}

// ---------- trending ----------

#[derive(Debug, Deserialize)]
pub(crate) struct RawTrending {
    pub error: Option<String>,
    #[serde(default)]
    pub ids: Value,
}

impl RawTrending {
    /// The ranked ids, without nulls or repeats.
    pub fn into_ids(self) -> Vec<BookId> {
        let mut out: Vec<BookId> = Vec::new();
        for id in self.ids.as_array().into_iter().flatten().filter_map(as_i64) {
            let id = BookId::new(id);
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

// ---------- lists ----------

#[derive(Debug, Deserialize)]
pub(crate) struct RawList {
    pub id: i64,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub privacy_setting_id: Option<i64>,
    pub ranked: Option<bool>,
    pub books_count: Option<i64>,
    pub default_view: Option<String>,
    pub featured_profile: Option<bool>,
    pub url: Option<String>,
}

impl RawList {
    pub fn into_list(self) -> UserList {
        let mut l = UserList::new(
            ListId::new(self.id),
            non_empty(self.name).unwrap_or_else(|| "Untitled list".to_owned()),
        );
        l.slug = non_empty(self.slug);
        l.description = non_empty(self.description);
        l.privacy_id = self.privacy_setting_id;
        l.ranked = self.ranked.unwrap_or(false);
        l.books_count = opt_u32(self.books_count).unwrap_or(0);
        l.default_view = self.default_view;
        l.featured_profile = self.featured_profile;
        l.url = self.url;
        l
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawListMembership {
    pub id: i64,
    pub list_id: i64,
    pub book_id: i64,
    pub edition_id: Option<i64>,
}

impl RawListMembership {
    pub fn into_membership(self) -> ListMembership {
        ListMembership {
            id: ListBookId::new(self.id),
            list_id: ListId::new(self.list_id),
            book_id: BookId::new(self.book_id),
            edition_id: self.edition_id.map(EditionId::new),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawListBook {
    pub id: i64,
    pub position: Option<i64>,
    pub edition_id: Option<i64>,
    pub book: Option<RawBook>,
}

impl RawListBook {
    /// `None` for a row whose book did not resolve.
    pub fn into_list_book(self) -> Option<ListBook> {
        let mut b = ListBook::new(ListBookId::new(self.id), self.book?.into_summary());
        b.edition_id = self.edition_id.map(EditionId::new);
        b.position = self.position.and_then(|p| i32::try_from(p).ok());
        Some(b)
    }
}

/// `ListIdType`: `insert_list` and `update_list`'s payload.
#[derive(Debug, Deserialize)]
pub(crate) struct ListPayload {
    pub errors: Option<Vec<Option<String>>>,
    pub id: Option<i64>,
    pub list: Option<RawList>,
}

impl ListPayload {
    pub fn error(&self) -> Option<String> {
        joined_errors(self.errors.as_deref())
    }
}

/// `ListBookIdType`: `insert_list_book` and `edition_owned`'s payload.
#[derive(Debug, Deserialize)]
pub(crate) struct ListBookPayload {
    pub id: Option<i64>,
    pub list_book: Option<RawListMembership>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListDeletePayload {
    pub success: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListBookDeletePayload {
    pub id: Option<i64>,
}

/// An `errors: [String]` field, joined, if it holds any.
fn joined_errors(errors: Option<&[Option<String>]>) -> Option<String> {
    let errors: Vec<&str> = errors?
        .iter()
        .flatten()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .collect();
    (!errors.is_empty()).then(|| errors.join("; "))
}

// ---------- mutation payloads ----------

#[derive(Debug, Deserialize)]
pub(crate) struct UserBookIdPayload {
    pub error: Option<String>,
    pub id: Option<i64>,
    pub user_book: Option<RawUserBook>,
}

// ---------- reviews ----------

/// `ReviewFields`.
#[derive(Debug, Deserialize)]
pub(crate) struct RawReview {
    pub review_markdown: Option<String>,
    pub review_raw: Option<String>,
    pub review_has_spoilers: Option<bool>,
    /// A `timestamp`, though it is written as a `date`.
    pub reviewed_at: Option<String>,
    pub has_review: Option<bool>,
}

impl RawReview {
    pub fn into_review(self) -> Review {
        let markdown = non_empty(self.review_markdown);
        let plain = non_empty(self.review_raw);
        let readable = markdown.is_some() || plain.is_some();
        let exists = readable || self.has_review.unwrap_or(false);
        Review {
            markdown,
            plain,
            has_review: exists,
            // Without text, a leftover flag or date describes nothing.
            has_spoilers: exists && self.review_has_spoilers.unwrap_or(false),
            reviewed_at: self
                .reviewed_at
                .as_deref()
                .and_then(parse_date)
                .filter(|_| exists),
        }
    }
}

/// `user_book { ...UserBookFields ...ReviewFields }`.
#[derive(Debug, Deserialize)]
pub(crate) struct RawReviewedUserBook {
    #[serde(flatten)]
    pub entry: RawUserBook,
    #[serde(flatten)]
    pub review: RawReview,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReviewedUserBookPayload {
    pub error: Option<String>,
    pub id: Option<i64>,
    pub user_book: Option<RawReviewedUserBook>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadIdPayload {
    pub error: Option<String>,
    pub id: Option<i64>,
    pub user_book_read: Option<ReadRef>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadRef {
    #[serde(flatten)]
    pub read: RawRead,
    pub user_book_id: i64,
    pub user_book: Option<RawUserBook>,
}

impl ReadRef {
    /// The nested entry with the written read in it, as written: the nested
    /// list may or may not include the write yet.
    pub fn into_entry(self) -> Option<LibraryEntry> {
        let mut entry = self.user_book?.into_entry()?;
        entry.upsert_read(self.read.into_session());
        Some(entry)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeletePayload {
    pub id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteReadPayload {
    pub error: Option<String>,
    pub id: Option<i64>,
}

// ---------- progress context ----------

#[derive(Debug, Deserialize)]
pub(crate) struct Lengths {
    pub pages: Option<i64>,
    pub audio_seconds: Option<i64>,
    /// An edition's; a book has none.
    #[serde(default)]
    pub reading_format_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ContextRead {
    pub id: i64,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub edition: Option<Lengths>,
}

impl ContextRead {
    fn session(&self) -> ReadSession {
        let mut r = ReadSession::new(ReadId::new(self.id));
        r.started_at = self.started_at.as_deref().and_then(parse_date);
        r.finished_at = self.finished_at.as_deref().and_then(parse_date);
        r
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProgressContext {
    pub status_id: Option<i64>,
    pub book: Option<Lengths>,
    pub edition: Option<Lengths>,
    #[serde(default)]
    pub user_book_reads: Vec<ContextRead>,
}

impl Lengths {
    fn parts(&self) -> EditionParts {
        EditionParts {
            pages: opt_u32(self.pages),
            audio_seconds: opt_u32(self.audio_seconds),
            reading_format_id: self.reading_format_id,
        }
    }
}

impl ProgressContext {
    /// Puts the reads in [`read_order`], the order [`LibraryEntry::reads`]
    /// keeps, so the first is the read [`LibraryEntry::open_read`] picks:
    /// the newest by date, not by id (a past read added later has the
    /// highest id).
    pub fn sort_reads(&mut self) {
        self.user_book_reads
            .sort_by(|a, b| read_order(&a.session(), &b.session()));
    }

    /// The read progress goes on when no read is named: the first unfinished
    /// one in [`read_order`] (call [`Self::sort_reads`] first).
    pub fn open_read(&self) -> Option<ReadId> {
        self.user_book_reads
            .first()
            .filter(|r| r.finished_at.is_none())
            .map(|r| ReadId::new(r.id))
    }

    /// The length of the read's edition, else the entry's edition, else the
    /// book's page count, decided as [`LibraryEntry::measure`] decides it: an
    /// audiobook counts time even though the book has a page count, and an
    /// audiobook without a running time has no length rather than the book's
    /// pages.
    pub fn length(&self) -> Option<Length> {
        let read = self
            .user_book_reads
            .first()
            .and_then(|r| r.edition.as_ref());
        measure_of(
            [read, self.edition.as_ref()]
                .into_iter()
                .flatten()
                .map(Lengths::parts),
            self.book.as_ref().and_then(|b| opt_u32(b.pages)),
        )
        .and_then(Measure::length)
    }
}

// ---------- other readers' reviews ----------

/// A row of `BookReviews`.
#[derive(Debug, Deserialize)]
pub(crate) struct RawCommunityReview {
    pub id: i64,
    pub rating: Option<f64>,
    pub likes_count: Option<i64>,
    /// A `timestamp`, though it is written as a `date`.
    pub reviewed_at: Option<String>,
    pub review_has_spoilers: Option<bool>,
    pub review_raw: Option<String>,
    pub user: Option<RawReviewer>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawReviewer {
    pub id: i64,
    pub username: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub cached_image: Value,
}

impl RawCommunityReview {
    /// `None` for a row with no text or no user.
    pub fn into_review(self) -> Option<CommunityReview> {
        let text = non_empty(self.review_raw)?;
        let user = self.user?;
        let mut reviewer = Reviewer::new(UserId::new(user.id));
        reviewer.username = non_empty(user.username);
        reviewer.name = non_empty(user.name);
        reviewer.avatar = cover_from_value(&user.cached_image);
        reviewer.color = user
            .cached_image
            .get("color")
            .and_then(Value::as_str)
            .and_then(hex_color);
        let mut review = CommunityReview::new(UserBookId::new(self.id), reviewer, text);
        review.rating = self.rating.and_then(Rating::from_api);
        review.likes = opt_u32(self.likes_count).unwrap_or(0);
        review.reviewed_at = self.reviewed_at.as_deref().and_then(parse_date);
        review.has_spoilers = self.review_has_spoilers.unwrap_or(false);
        Some(review)
    }
}

/// `#rrggbb` or `#rgb`, with or without the `#`.
fn hex_color(s: &str) -> Option<[u8; 3]> {
    let hex = s.trim().trim_start_matches('#');
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
    match hex.len() {
        6 => Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ]),
        3 => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17]),
        _ => None,
    }
}

#[cfg(test)]
mod community_review_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(hex_color("#1A2b3C"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(hex_color("fff"), Some([255, 255, 255]));
        assert_eq!(hex_color("#12345"), None);
        assert_eq!(hex_color("rgb(1,2,3)"), None);
        assert_eq!(hex_color("#ééé"), None);
    }

    #[test]
    fn rows_without_text_or_user_are_dropped() {
        let row = |raw: Value, user: Value| -> Option<CommunityReview> {
            serde_json::from_value::<RawCommunityReview>(json!({
                "id": 1, "rating": 3.5, "likes_count": 2, "reviewed_at": null,
                "review_has_spoilers": null, "review_raw": raw, "user": user,
            }))
            .unwrap()
            .into_review()
        };
        let user = json!({"id": 9, "username": "u9", "name": "", "cached_image": {}});
        assert!(row(json!("  \n "), user.clone()).is_none());
        assert!(row(json!(null), user.clone()).is_none());
        assert!(row(json!("Text"), json!(null)).is_none());
        let review = row(json!("Text"), user).unwrap();
        assert_eq!(review.reviewer.name, None);
        assert_eq!(review.reviewer.avatar, None);
        assert_eq!(review.rating, Rating::new(3.5));
        assert!(!review.has_spoilers);
    }
}

// ---------- upcoming books ----------

/// One `book_series` row of `UpcomingInSeries`.
#[derive(Debug, Deserialize)]
pub(crate) struct RawUpcomingRow {
    pub position: Option<f64>,
    #[serde(default)]
    pub compilation: bool,
    pub series: Option<RawSeries>,
    pub book: Option<RawUpcomingBook>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawUpcomingBook {
    #[serde(flatten)]
    pub book: RawBook,
    pub release_date: Option<String>,
    pub is_partial_book: Option<bool>,
}

/// One book per id, in the order of its first row, with every series it was
/// listed under. Compilations, partial books and books without a release
/// date are left out.
pub(crate) fn upcoming_books(rows: Vec<RawUpcomingRow>) -> Vec<UpcomingBook> {
    let mut books: Vec<UpcomingBook> = Vec::new();
    for row in rows {
        let Some(raw) = row.book else { continue };
        if row.compilation || raw.is_partial_book == Some(true) {
            continue;
        }
        let Some(release_date) = raw.release_date.as_deref().and_then(parse_date) else {
            continue;
        };
        let id = BookId::new(raw.book.id);
        let ix = if let Some(ix) = books.iter().position(|b| b.book.id == id) {
            ix
        } else {
            books.push(UpcomingBook::new(raw.book.into_summary(), release_date));
            books.len() - 1
        };
        let series = series_refs(vec![RawBookSeries {
            position: row.position,
            featured: false,
            compilation: false,
            series: row.series,
        }]);
        for s in series {
            let known = books[ix]
                .series
                .iter()
                .any(|o| o.id == s.id && o.name == s.name);
            if !known {
                books[ix].series.push(s);
            }
        }
    }
    books
}

// ---------- tracked series ----------

/// One `series` row of `SeriesOverview`.
#[derive(Debug, Deserialize)]
pub(crate) struct RawTrackedSeries {
    pub id: i64,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub primary_books_count: Option<i64>,
    pub is_completed: Option<bool>,
    pub author: Option<RawAuthorRef>,
    #[serde(default)]
    pub book_series: Vec<RawTrackedRow>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawTrackedRow {
    pub position: Option<f64>,
    #[serde(default)]
    pub compilation: bool,
    pub book: Option<RawTrackedBook>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawTrackedBook {
    #[serde(flatten)]
    pub book: RawBook,
    pub canonical_id: Option<i64>,
    pub release_date: Option<String>,
    pub is_partial_book: Option<bool>,
}

impl RawTrackedSeries {
    /// The series and its books by position, then the most-read first.
    /// Compilations, merged duplicates (a `canonical_id`), books without a
    /// usable position and repeats of one book are left out, whatever the
    /// query already filtered.
    pub fn into_tracked(self) -> TrackedSeries {
        let mut s = TrackedSeries::new(
            SeriesId::new(self.id),
            non_empty(self.name).unwrap_or_else(|| "Untitled series".to_owned()),
        );
        s.slug = non_empty(self.slug);
        s.primary_books_count = opt_u32(self.primary_books_count);
        s.is_completed = self.is_completed;
        s.author = self
            .author
            .and_then(|a| Some(AuthorRef::new(AuthorId::new(a.id), non_empty(a.name)?)));
        for row in self.book_series {
            let Some(raw) = row.book else { continue };
            if row.compilation || raw.canonical_id.is_some() {
                continue;
            }
            let Some(position) = row.position.filter(|p| p.is_finite() && *p >= 0.0) else {
                continue;
            };
            let release_date = raw.release_date.as_deref().and_then(parse_date);
            let partial = raw.is_partial_book == Some(true);
            let book = raw.book.into_summary();
            if s.books.iter().any(|b| b.book.id == book.id) {
                continue;
            }
            let mut b = TrackedBook::new(book, model_float(position));
            b.release_date = release_date;
            b.partial = partial;
            s.books.push(b);
        }
        s.books.sort_by(|a, b| {
            a.position.total_cmp(&b.position).then_with(|| {
                b.book
                    .users_count
                    .unwrap_or(0)
                    .cmp(&a.book.users_count.unwrap_or(0))
            })
        });
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timestamps_and_dates() {
        let t = parse_timestamp("2025-03-04T05:06:07.123456+00:00").unwrap();
        assert_eq!(t.to_rfc3339(), "2025-03-04T05:06:07.123456+00:00");
        assert!(parse_timestamp("2025-03-04T05:06:07.5").is_some());
        assert!(parse_timestamp("2025-03-04T07:06:07+02:00").is_some());
        assert!(parse_timestamp("garbage").is_none());
        assert_eq!(parse_date("2024-02-29").unwrap().to_string(), "2024-02-29");
        assert!(parse_date("2024").is_none());
    }

    #[test]
    fn empty_cached_image_is_no_cover() {
        assert!(cover_from_value(&json!({})).is_none());
        assert!(cover_from_value(&Value::Null).is_none());
        assert!(cover_from_value(&json!({"url": ""})).is_none());
        let c = cover_from_value(
            &json!({"url": "https://assets.example/x.jpg", "width": 10, "height": "20"}),
        )
        .unwrap();
        assert_eq!((c.width, c.height), (Some(10), Some(20)));
    }

    #[test]
    fn authors_prefer_author_role() {
        let v = json!([
            {"author": {"name": "Ada Writer"}, "contribution": null},
            {"author": {"name": "Tom Translator"}, "contribution": "Translator"},
            {"author": {"name": "Bea Coauthor"}, "contribution": "Author"},
            {"author": {"name": "Ada Writer"}, "contribution": "Author"},
            {"contribution": "Author"}
        ]);
        assert_eq!(
            authors_from_contributors(&v),
            vec!["Ada Writer", "Bea Coauthor"]
        );
        let only_editor = json!([{"author": {"name": "Ed Itor"}, "contribution": "Editor"}]);
        assert_eq!(
            contributors_from_cached(&only_editor)[0].role.as_deref(),
            Some("Editor")
        );
        assert_eq!(authors_from_contributors(&only_editor), vec!["Ed Itor"]);
        assert_eq!(authors_from_contributors(&json!({})), [] as [String; 0]);
    }

    #[test]
    fn contributors_keep_ids_roles_and_order() {
        let v = json!([
            {"author": {"id": 9, "name": "Ada Writer"}, "contribution": null},
            {"author": {"id": "10", "name": "Tom Translator"}, "contribution": "Translator"},
            {"author": {"name": "No Id"}, "contribution": "Illustrator"},
            {"author": {"id": 9, "name": "Ada Writer"}, "contribution": "Author"},
            {"author": {"id": 11}, "contribution": "Narrator"}
        ]);
        let c = contributors_from_cached(&v);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].author_id, Some(AuthorId::new(9)));
        assert!(c[0].is_author());
        assert_eq!(c[1].author_id, Some(AuthorId::new(10)));
        assert_eq!(c[1].role.as_deref(), Some("Translator"));
        assert_eq!(c[2].author_id, None);
        assert!(!c[2].is_author());
        assert_eq!(contributors_from_cached(&json!({})), []);
    }

    #[test]
    fn ratings_distribution_accepts_both_shapes() {
        let list = json!([
            {"rating": 0.5, "count": 3},
            {"rating": 5, "count": 40},
            {"rating": "4.5", "count": 12},
            {"rating": 4.5, "count": 1},
            {"rating": 3, "count": 0},
            {"rating": 7, "count": 9},
            {"count": 2}
        ]);
        let b = ratings_distribution(&list);
        assert_eq!(
            b.iter().map(|b| (b.rating, b.count)).collect::<Vec<_>>(),
            vec![(5.0, 40), (4.5, 13), (0.5, 3)]
        );
        let map = json!({"1": 2, "2.5": "4", "5": 8, "bad": 1});
        let b = ratings_distribution(&map);
        assert_eq!(
            b.iter().map(|b| (b.rating, b.count)).collect::<Vec<_>>(),
            vec![(5.0, 8), (2.5, 4), (1.0, 2)]
        );
        assert_eq!(ratings_distribution(&Value::Null), []);
        assert_eq!(ratings_distribution(&json!({})), []);
    }

    #[test]
    fn alternative_titles_and_similar_ids_are_lenient() {
        let titles = json!(["Le Livre", {"title": "Das Buch"}, "le livre", "The Book", "", 3]);
        assert_eq!(
            alternative_titles(&titles, "The Book"),
            vec!["Le Livre", "Das Buch"]
        );
        assert_eq!(alternative_titles(&json!({}), "x"), [] as [String; 0]);
        let ids = json!([3, "4", {"id": 5}, 3, -1, null]);
        assert_eq!(
            book_ids(&ids),
            vec![BookId::new(3), BookId::new(4), BookId::new(5)]
        );
        assert_eq!(book_ids(&json!({})), []);
    }

    fn series_book(id: i64, title: &str, position: Option<f32>, users: u32) -> SeriesBook {
        let mut book = BookSummary::new(BookId::new(id), title);
        book.users_count = Some(users);
        let mut b = SeriesBook::new(book);
        b.position = position;
        b
    }

    #[test]
    fn series_reading_order_dedupes_positions_and_titles() {
        let books = vec![
            series_book(1, "Two", Some(2.0), 50),
            series_book(2, "One", Some(1.0), 90),
            series_book(3, "One (translated)", Some(1.0), 10),
            series_book(4, "Novella", Some(1.5), 5),
            series_book(5, "One more", Some(1.0), 200),
            series_book(6, "Extra", None, 3),
            series_book(7, "Guide", None, 30),
            series_book(8, "two", None, 1),
            series_book(1, "Two", Some(2.0), 50),
        ];
        let ordered = series_reading_order(books);
        let ids: Vec<i64> = ordered.iter().map(|b| b.book.id.get()).collect();
        assert_eq!(ids, vec![5, 4, 1, 7, 6]);
        assert!(ordered[0].is_main());
        assert!(!ordered[1].is_main());
        assert!(!ordered[3].is_main());
    }
}
