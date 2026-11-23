//! A demo library for screenshots and for trying the app without an account.
//!
//! [`client`] answers from `demo/catalogue.json` through a fake transport, so
//! the rest of the client (queries, parsing, paging) runs exactly as it does
//! against Hardcover. The books are real, popular ones, with facts, editions
//! and cover links from Hardcover and descriptions written for the demo; the
//! reader, their shelves, reads, notes, reviews, reading goal and lists are
//! invented. A book the catalogue lists no editions for has one stand-in
//! edition, with only the book's title and cover and the book's id; a few
//! books have their real editions, audiobooks among them.
//!
//! Every date of the reader's is a number of days from today, resolved when
//! the client is built, so the demo always looks current: the books being
//! read were updated lately (one is idle on purpose), the goal is this
//! year's, dated as Hardcover dates its own (Jan 1 to Jan 1), and a series
//! hasn't been touched for over a year.
//!
//! The demo is writable: a status, rating, progress, read, review, note,
//! privacy, edition, list, Owned list or goal change is applied to the
//! client's own copy of the reader's data ([`state`]) and answered in the
//! shape Hardcover answers it, following the behaviour CLAUDE.md records.
//! Nothing is saved: every client starts from the catalogue.
//!
//! Series and author pages are built from the catalogue alone: a series lists
//! the catalogue's books in it, an author their books in the catalogue. Their
//! ids are the demo's own, not Hardcover's. Authors have no bio, photo or book count.
//! "Similar books" are the catalogue's books sharing the most genres, not
//! Hardcover's picks, and books have no ratings distribution or alternative
//! titles: the catalogue does not record them.
//!
//! Author and series search match the catalogue's names and titles and leave
//! out the counts the catalogue cannot know. "Trending" ranks the catalogue by
//! its own Hardcover numbers, not by recent reads, a different number for
//! each period: newest release for a week, average rating for a month, number
//! of ratings for three months, and number of readers for a year or all time.
//!
//! Some series have books still to come: placeholder titles (like
//! Hardcover's own "Untitled …" books), no cover or description, and release
//! dates a fixed number of days from today. They are on their series pages
//! and answer `UpcomingInSeries` and `BookDetails`.
//!
//! [`cover_cache`] downloads the catalogue's covers (books and editions) and
//! Hardcover's default avatars from Hardcover, the demo's only network
//! access; any other URL is refused. [`Faults`] (`HARDCOVER_DEMO_FAULTS`)
//! makes requests slow, fail or lack a scope. Only built with the `demo`
//! feature (and in tests).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Client, TokenSource};
use crate::covers::CoverCache;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, Method, RequestBody, UreqTransport};
use crate::ratelimit::TokenBucket;
use crate::{Error, Result};

mod faults;
mod reviews;
mod state;
#[cfg(test)]
mod tests;
mod writes;

use faults::Failure;
pub use faults::{ENV as FAULTS_ENV, Faults};
use state::{Clock, Entry, Length, List, Read, State, read_length};

/// A client for the demo library, with no faults. It never touches the network.
#[must_use]
pub fn client() -> Client {
    client_with(&Faults::default())
}

/// A client for the demo library that injects `faults`.
#[must_use]
pub fn client_with(faults: &Faults) -> Client {
    client_at(faults, Clock::System)
}

fn client_at(faults: &Faults, clock: Clock) -> Client {
    let client = Client::with_transport(
        Arc::new(DemoToken),
        Arc::new(ApiTransport {
            state: Mutex::new(State::new(&CATALOGUE, clock)),
            faults: faults.clone(),
        }),
        // No rate limit to respect.
        TokenBucket::new(1_000, 1_000.0),
    );
    // The dates the client sends follow the demo's clock.
    #[cfg(test)]
    if let Clock::Fixed(_) = clock {
        client.pin_today(clock.today());
    }
    client
}

/// The faults in `HARDCOVER_DEMO_FAULTS`, read once per run.
pub fn env_faults() -> &'static Faults {
    static FAULTS: LazyLock<Faults> = LazyLock::new(Faults::from_env);
    &FAULTS
}

/// A cover cache in `dir` that downloads the catalogue's covers from
/// Hardcover, failing as `faults` says a connection does.
///
/// # Errors
///
/// Returns an error if the demo cover cache directory cannot be created.
pub fn cover_cache(dir: impl Into<PathBuf>, faults: &Faults) -> Result<CoverCache> {
    CoverCache::with_transport(
        dir,
        Arc::new(CoverTransport {
            http: Some(UreqTransport::new()),
            faults: faults.clone(),
        }),
    )
}

struct DemoToken;

impl TokenSource for DemoToken {
    fn access_token(&self) -> Result<String> {
        Ok("demo".to_owned())
    }

    fn on_unauthorized(&self, _rejected: &str) -> Result<bool> {
        Ok(false)
    }
}

// ----- catalogue -----

#[derive(Deserialize)]
struct Catalogue {
    user: DemoUser,
    books: Vec<Book>,
    library: Vec<SeedEntry>,
    goals: Vec<SeedGoal>,
    lists: Vec<SeedList>,
    #[serde(default)]
    upcoming: Vec<Upcoming>,
}

/// A book not yet released in one of the catalogue's series. Placeholder
/// titles like Hardcover's own, no cover and no description; the date is a
/// number of days from today, so the demo never goes stale. Ids are far above
/// any Hardcover book id the catalogue uses.
#[derive(Deserialize)]
struct Upcoming {
    id: i64,
    title: String,
    /// A series of the catalogue, by name.
    series: String,
    position: Option<f64>,
    days: i64,
}

impl Upcoming {
    fn release_date(&self) -> chrono::NaiveDate {
        today() + chrono::Duration::days(self.days)
    }
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// One of the reader's lists.
#[derive(Deserialize)]
struct SeedList {
    id: i64,
    name: String,
    slug: String,
    description: Option<String>,
    privacy: i64,
    ranked: bool,
    books: Vec<ListItem>,
}

/// A book on a list, or (on the Owned list) one edition of it. A bare book
/// on the Owned list stands for its first edition.
#[derive(Deserialize)]
#[serde(untagged)]
enum ListItem {
    Book(i64),
    Edition { book: i64, edition: i64 },
}

/// A goal for the current calendar year, whichever year the demo runs in.
#[derive(Deserialize)]
struct SeedGoal {
    id: i64,
    metric: String,
    goal: i64,
}

#[derive(Deserialize)]
struct DemoUser {
    id: i64,
    username: String,
    name: String,
}

#[derive(Deserialize)]
struct Book {
    id: i64,
    title: String,
    subtitle: Option<String>,
    author: String,
    /// Hardcover leaves some books without a page count.
    pages: Option<u32>,
    /// The running time of the book's audiobook, which Hardcover seldom fills.
    audio_seconds: Option<u32>,
    release_date: String,
    rating: f64,
    ratings_count: i64,
    users_count: i64,
    editions_count: i64,
    genres: Vec<String>,
    /// The book's `book_series` rows, as Hardcover has them.
    #[serde(default)]
    series: Vec<SeriesRow>,
    description: String,
    cover: CoverLink,
    /// Real editions, most read first; none means one stand-in edition.
    #[serde(default)]
    editions: Vec<DemoEdition>,
}

impl Book {
    /// The edition the Owned list holds when it names none: the first.
    fn default_edition(&self) -> i64 {
        self.editions.first().map_or(self.id, |e| e.id)
    }

    fn in_series(&self, name: &str) -> Option<&SeriesRow> {
        self.series.iter().find(|s| s.name == name)
    }
}

#[derive(Deserialize)]
struct SeriesRow {
    name: String,
    position: Option<f64>,
    featured: bool,
    #[serde(default)]
    compilation: bool,
}

/// One of a book's editions, from Hardcover.
#[derive(Deserialize)]
struct DemoEdition {
    id: i64,
    /// The book's when absent.
    title: Option<String>,
    /// `edition_format`, such as "Hardcover" or "Audible".
    format: Option<String>,
    /// `reading_format_id`: 1 physical, 2 audiobook, 4 ebook.
    reading_format: Option<i64>,
    pages: Option<u32>,
    audio_seconds: Option<u32>,
    release_date: Option<String>,
    isbn_13: Option<String>,
    publisher: Option<String>,
    language: Option<String>,
    /// The book's when absent.
    cover: Option<CoverLink>,
}

impl DemoEdition {
    fn is_audiobook(&self) -> bool {
        self.reading_format == Some(2)
    }
}

#[derive(Deserialize)]
struct CoverLink {
    url: String,
    width: u32,
    height: u32,
}

/// One of the reader's entries. Every date is a number of days from today:
/// 0 today, -1 yesterday.
#[derive(Deserialize)]
struct SeedEntry {
    id: i64,
    book: i64,
    status: i64,
    rating: Option<f64>,
    added: i64,
    updated: i64,
    /// An edition id from the book's `editions`.
    edition: Option<i64>,
    #[serde(default)]
    reads: Vec<SeedRead>,
    private_notes: Option<String>,
    /// `privacy_setting_id`; Public when absent.
    privacy: Option<i64>,
    review: Option<SeedReview>,
}

/// The demo reader's own review, written for the demo.
#[derive(Deserialize)]
struct SeedReview {
    markdown: String,
    #[serde(default)]
    spoilers: bool,
    reviewed: i64,
}

#[derive(Deserialize)]
struct SeedRead {
    id: i64,
    started: Option<i64>,
    finished: Option<i64>,
    progress_pages: Option<u32>,
    progress_seconds: Option<u32>,
    edition: Option<i64>,
}

static CATALOGUE: LazyLock<Catalogue> = LazyLock::new(|| {
    serde_json::from_str(include_str!("demo/catalogue.json")).expect("demo/catalogue.json parses")
});

/// Demo ids for authors and series, which the catalogue names but does not number.
const AUTHOR_IDS: i64 = 70_000;
const SERIES_IDS: i64 = 80_000;

impl Catalogue {
    fn book(&self, id: i64) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    /// An edition and its book; `None` for the edition is the book's
    /// stand-in, whose id is the book's.
    fn edition(&self, id: i64) -> Option<(&Book, Option<&DemoEdition>)> {
        self.books.iter().find_map(|b| {
            if let Some(e) = b.editions.iter().find(|e| e.id == id) {
                Some((b, Some(e)))
            } else {
                (b.id == id && b.editions.is_empty()).then_some((b, None))
            }
        })
    }

    /// Distinct names in catalogue order, numbered from `base + 1`.
    fn numbered<'a>(names: impl Iterator<Item = &'a str>, base: i64) -> Vec<(i64, &'a str)> {
        let mut seen: Vec<&str> = Vec::new();
        for name in names {
            if !seen.contains(&name) {
                seen.push(name);
            }
        }
        ((base + 1)..).zip(seen).collect()
    }

    fn authors(&self) -> Vec<(i64, &str)> {
        Self::numbered(self.books.iter().map(|b| b.author.as_str()), AUTHOR_IDS)
    }

    fn series(&self) -> Vec<(i64, &str)> {
        Self::numbered(
            self.books
                .iter()
                .flat_map(|b| b.series.iter().map(|s| s.name.as_str())),
            SERIES_IDS,
        )
    }

    fn author_id(&self, name: &str) -> i64 {
        self.authors()
            .into_iter()
            .find(|(_, n)| *n == name)
            .map_or(0, |(id, _)| id)
    }

    fn series_id(&self, name: &str) -> i64 {
        self.series()
            .into_iter()
            .find(|(_, n)| *n == name)
            .map_or(0, |(id, _)| id)
    }

    /// Other books sharing at least one genre, most shared genres first, then
    /// the most read.
    fn similar(&self, book: &Book) -> Vec<i64> {
        let mut others: Vec<(usize, &Book)> = self
            .books
            .iter()
            .filter(|b| b.id != book.id)
            .map(|b| {
                (
                    b.genres.iter().filter(|g| book.genres.contains(g)).count(),
                    b,
                )
            })
            .filter(|(shared, _)| *shared > 0)
            .collect();
        others.sort_by_key(|(shared, b)| {
            (std::cmp::Reverse(*shared), std::cmp::Reverse(b.users_count))
        });
        others.into_iter().take(16).map(|(_, b)| b.id).collect()
    }

    fn upcoming(&self, id: i64) -> Option<&Upcoming> {
        self.upcoming.iter().find(|u| u.id == id)
    }

    /// The author of a series: that of its first book in the catalogue.
    fn series_author(&self, series: &str) -> Option<&str> {
        self.books
            .iter()
            .find(|b| b.in_series(series).is_some())
            .map(|b| b.author.as_str())
    }

    /// Every URL the demo may download: the catalogue's covers and edition
    /// covers, and the reviewers' avatars, Hardcover's default ones.
    fn downloads(&'static self) -> HashSet<&'static str> {
        self.books
            .iter()
            .flat_map(|b| {
                std::iter::once(b.cover.url.as_str()).chain(
                    b.editions
                        .iter()
                        .filter_map(|e| e.cover.as_ref().map(|c| c.url.as_str())),
                )
            })
            .chain(reviews::avatars())
            .collect()
    }
}

// ----- wire shapes -----

/// `ListFields`.
fn list(list: &List) -> Value {
    json!({
        "id": list.id,
        "name": list.name,
        "slug": list.slug,
        "description": list.description,
        "privacy_setting_id": list.privacy,
        "ranked": list.ranked,
        "books_count": list.rows.len(),
        "default_view": list.default_view,
        "featured_profile": list.featured_profile,
        "url": list.url,
    })
}

/// Every row on the reader's lists as `(book, {id list_id book_id edition_id})`.
fn memberships(state: &State) -> Vec<(i64, Value)> {
    state
        .lists
        .iter()
        .flat_map(|l| {
            l.rows.iter().map(move |r| {
                (
                    r.book,
                    json!({
                        "id": r.id,
                        "list_id": l.id,
                        "book_id": r.book,
                        "edition_id": r.edition,
                    }),
                )
            })
        })
        .collect()
}

fn slug(title: &str) -> String {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn image(cover: &CoverLink) -> Value {
    json!({
        "url": cover.url,
        "width": cover.width,
        "height": cover.height,
    })
}

/// `reading_format { format }`, by Hardcover's names.
fn reading_format_name(id: i64) -> Option<&'static str> {
    match id {
        1 => Some("Read"),
        2 => Some("Listened"),
        4 => Some("Ebook"),
        _ => None,
    }
}

/// `EditionFields`, and a read's shorter edition (which ignores the rest).
/// The stand-in edition has only the book's id, title and cover: the
/// catalogue's page count and date are the book's, which another edition may
/// not share.
fn edition(book: &Book, edition: Option<&DemoEdition>) -> Value {
    let Some(e) = edition else {
        return json!({
            "id": book.id,
            "title": book.title,
            "pages": null,
            "audio_seconds": null,
            "edition_format": null,
            "release_date": null,
            "isbn_13": null,
            "cached_image": image(&book.cover),
            "reading_format_id": null,
            "reading_format": null,
            "publisher": null,
            "language": null,
        });
    };
    json!({
        "id": e.id,
        "title": e.title.as_deref().unwrap_or(&book.title),
        "pages": e.pages,
        "audio_seconds": e.audio_seconds,
        "edition_format": e.format,
        "release_date": e.release_date,
        "isbn_13": e.isbn_13,
        "cached_image": image(e.cover.as_ref().unwrap_or(&book.cover)),
        "reading_format_id": e.reading_format,
        "reading_format": e.reading_format
            .and_then(reading_format_name)
            .map(|f| json!({ "format": f })),
        "publisher": e.publisher.as_ref().map(|n| json!({ "name": n })),
        "language": e.language.as_ref().map(|l| json!({ "language": l })),
    })
}

fn edition_by_id(id: i64) -> Option<Value> {
    let (book, e) = CATALOGUE.edition(id)?;
    Some(edition(book, e))
}

fn release_year(book: &Book) -> Option<i64> {
    book.release_date.get(..4)?.parse().ok()
}

/// `BookSummaryFields`.
fn book_summary(book: &Book) -> Value {
    json!({
        "id": book.id,
        "title": book.title,
        "subtitle": book.subtitle,
        "slug": slug(&book.title),
        "pages": book.pages,
        "release_year": release_year(book),
        "rating": book.rating,
        "users_count": book.users_count,
        "cached_image": image(&book.cover),
        "cached_contributors": [{
            "author": { "id": CATALOGUE.author_id(&book.author), "name": book.author },
            "contribution": null,
        }],
    })
}

/// `BookSummaryFields` for an upcoming book, plus its `release_date`.
fn upcoming_summary(catalogue: &Catalogue, book: &Upcoming) -> Value {
    let date = book.release_date();
    let contributors: Vec<Value> = catalogue
        .series_author(&book.series)
        .map(|name| {
            json!({
                "author": { "id": catalogue.author_id(name), "name": name },
                "contribution": null,
            })
        })
        .into_iter()
        .collect();
    json!({
        "id": book.id,
        "title": book.title,
        "subtitle": null,
        "slug": slug(&book.title),
        "pages": null,
        "release_year": chrono::Datelike::year(&date),
        "release_date": date.format("%Y-%m-%d").to_string(),
        "rating": null,
        "users_count": null,
        "cached_image": {},
        "cached_contributors": contributors,
    })
}

/// An upcoming book's `book_series` row, in `series` shape.
fn upcoming_series(catalogue: &Catalogue, book: &Upcoming) -> Value {
    json!({
        "id": catalogue.series_id(&book.series),
        "name": book.series,
        "slug": slug(&book.series),
    })
}

/// `book_series` for `UpcomingInSeries`: the upcoming books in the series
/// asked for, released after `after`, the soonest first.
fn upcoming_rows(catalogue: &Catalogue, vars: &Value) -> Value {
    let series: Vec<i64> = vars["series"]
        .as_array()
        .map_or_default(|ids| ids.iter().filter_map(Value::as_i64).collect());
    let after = vars["after"]
        .as_str()
        .and_then(|d| d.parse::<chrono::NaiveDate>().ok())
        .unwrap_or_else(today);
    let limit = usize::try_from(vars["limit"].as_i64().unwrap_or(200)).unwrap_or_default();
    let mut books: Vec<&Upcoming> = catalogue
        .upcoming
        .iter()
        .filter(|u| series.contains(&catalogue.series_id(&u.series)))
        .filter(|u| u.release_date() > after)
        .collect();
    books.sort_by_key(|u| (u.release_date(), u.id));
    let rows: Vec<Value> = books
        .into_iter()
        .take(limit)
        .map(|u| {
            let mut book = upcoming_summary(catalogue, u);
            book["is_partial_book"] = json!(false);
            json!({
                "position": u.position,
                "compilation": false,
                "series": upcoming_series(catalogue, u),
                "book": book,
            })
        })
        .collect();
    json!({ "book_series": rows })
}

/// `books_by_pk` for an upcoming book: no description (none is made up for
/// a real series' unreleased book), no tags, no similar books.
fn upcoming_details(catalogue: &Catalogue, book: &Upcoming) -> Value {
    let mut details = upcoming_summary(catalogue, book);
    let extra = json!({
        "description": null,
        "editions_count": null,
        "ratings_count": null,
        "cached_tags": {},
        "ratings_distribution": [],
        "alternative_titles": [],
        "cached_similar_book_ids": [],
        "book_series": [{
            "position": book.position,
            "featured": true,
            "series": upcoming_series(catalogue, book),
        }],
    });
    if let (Some(map), Value::Object(extra)) = (details.as_object_mut(), extra) {
        map.extend(extra);
    }
    details
}

/// An entry's book: `BookSummaryFields`, its audiobook edition and its
/// series. A book still to come can be on the Want to Read shelf too.
fn entry_book(id: i64) -> Option<Value> {
    let catalogue = &*CATALOGUE;
    if let Some(book) = catalogue.book(id) {
        let mut value = book_summary(book);
        // Hardcover names an audiobook edition whenever it lists one.
        value["default_audio_edition_id"] = json!(
            book.editions
                .iter()
                .find(|e| e.reading_format == Some(2))
                .map(|e| e.id)
        );
        value["book_series"] = Value::Array(book_series(book));
        return Some(value);
    }
    let upcoming = catalogue.upcoming(id)?;
    let mut value = upcoming_summary(catalogue, upcoming);
    value["default_audio_edition_id"] = Value::Null;
    value["book_series"] = json!([{
        "position": upcoming.position,
        "featured": true,
        "series": upcoming_series(catalogue, upcoming),
    }]);
    Some(value)
}

/// `UserBookFields`.
fn user_book(entry: &Entry) -> Option<Value> {
    let book = entry_book(entry.book)?;
    let mut reads: Vec<&Read> = entry.reads.iter().collect();
    reads.sort_by_key(|r| std::cmp::Reverse(r.id));
    let reads: Vec<Value> = reads.into_iter().map(|r| read(entry, r)).collect();
    Some(json!({
        "id": entry.id,
        "user_id": CATALOGUE.user.id,
        "status_id": entry.status,
        "rating": entry.rating,
        "date_added": entry.date_added.to_string(),
        "updated_at": entry.updated_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        "privacy_setting_id": entry.privacy,
        "private_notes": entry.private_notes,
        "book": book,
        "edition": entry.edition.and_then(edition_by_id),
        "user_book_reads": reads,
    }))
}

/// A `user_book_reads` row, with the percentage Hardcover works out from the
/// length the read counts against.
fn read(entry: &Entry, read: &Read) -> Value {
    let percent = |done: u32, of: u32| f64::from(done) * 100.0 / f64::from(of);
    let progress = match read_length(entry, read) {
        Some(Length::Pages(of)) if of > 0 => read.progress_pages.map(|p| percent(p, of)),
        Some(Length::Seconds(of)) if of > 0 => read.progress_seconds.map(|s| percent(s, of)),
        _ => None,
    };
    json!({
        "id": read.id,
        "started_at": read.started.map(|d| d.to_string()),
        "finished_at": read.finished.map(|d| d.to_string()),
        "progress": progress,
        "progress_pages": read.progress_pages,
        "progress_seconds": read.progress_seconds,
        "edition_id": read.edition,
        "edition": read.edition.and_then(edition_by_id),
    })
}

/// A book's `book_series` rows, featured first, then by position.
fn book_series(book: &Book) -> Vec<Value> {
    let mut rows: Vec<&SeriesRow> = book.series.iter().collect();
    rows.sort_by(|a, b| {
        b.featured
            .cmp(&a.featured)
            .then_with(|| by_position(a.position, b.position))
    });
    rows.into_iter()
        .map(|s| {
            json!({
                "position": s.position,
                "featured": s.featured,
                "series": {
                    "id": CATALOGUE.series_id(&s.name),
                    "name": s.name,
                    "slug": slug(&s.name),
                },
            })
        })
        .collect()
}

/// `asc_nulls_last` on positions.
fn by_position(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn book_details(book: &Book) -> Value {
    let mut details = book_summary(book);
    let genres: Vec<Value> = book
        .genres
        .iter()
        .map(|g| json!({ "tag": g, "count": 1 }))
        .collect();
    let series = book_series(book);
    let extra = json!({
        "description": book.description,
        "release_date": book.release_date,
        "editions_count": book.editions_count,
        "ratings_count": book.ratings_count,
        "cached_tags": { "Genre": genres },
        "ratings_distribution": [],
        "alternative_titles": [],
        "cached_similar_book_ids": CATALOGUE.similar(book),
        "book_series": series,
    });
    if let (Some(map), Value::Object(extra)) = (details.as_object_mut(), extra) {
        map.extend(extra);
    }
    details
}

/// `series_by_pk` for `SeriesPage`: every book in the series, compilations
/// included, by position (none last), then the most read.
fn series_page(catalogue: &Catalogue, id: i64) -> Option<Value> {
    let (id, name) = catalogue.series().into_iter().find(|(i, _)| *i == id)?;
    let author = catalogue
        .series_author(name)
        .map(|a| json!({ "id": catalogue.author_id(a), "name": a }));
    let mut rows: Vec<(Option<f64>, i64, Value)> = catalogue
        .books
        .iter()
        .filter_map(|b| {
            let s = b.in_series(name)?;
            let mut book = book_summary(b);
            book["is_partial_book"] = json!(false);
            Some((
                s.position,
                b.users_count,
                json!({ "position": s.position, "compilation": s.compilation, "book": book }),
            ))
        })
        // Books still to come are on the series page, as on Hardcover.
        .chain(
            catalogue
                .upcoming
                .iter()
                .filter(|u| u.series == name)
                .map(|u| {
                    let mut book = upcoming_summary(catalogue, u);
                    book["is_partial_book"] = json!(false);
                    (
                        u.position,
                        0,
                        json!({ "position": u.position, "compilation": false, "book": book }),
                    )
                }),
        )
        .collect();
    rows.sort_by(|a, b| by_position(a.0, b.0).then_with(|| b.1.cmp(&a.1)));
    Some(json!({
        "id": id,
        "name": name,
        "slug": slug(name),
        "description": null,
        "primary_books_count": null,
        "is_completed": null,
        "author": author,
        "book_series": rows.into_iter().map(|(_, _, row)| row).collect::<Vec<_>>(),
    }))
}

/// `series` for `SeriesOverview`: each series asked for, with the books the
/// query keeps (a position, not merged, not a compilation), the catalogue's
/// and those still to come, by position.
fn series_overview(catalogue: &Catalogue, vars: &Value) -> Value {
    let ids: Vec<i64> = vars["ids"]
        .as_array()
        .map_or_default(|ids| ids.iter().filter_map(Value::as_i64).collect());
    let series: Vec<Value> = catalogue
        .series()
        .into_iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, name)| {
            let mut rows: Vec<(f64, Value)> = catalogue
                .books
                .iter()
                .filter_map(|b| {
                    let s = b.in_series(name).filter(|s| !s.compilation)?;
                    let mut book = book_summary(b);
                    book["canonical_id"] = Value::Null;
                    book["release_date"] = json!(b.release_date);
                    book["is_partial_book"] = json!(false);
                    Some((s.position?, book))
                })
                .chain(
                    catalogue
                        .upcoming
                        .iter()
                        .filter(|u| u.series == name)
                        .filter_map(|u| {
                            let mut book = upcoming_summary(catalogue, u);
                            book["canonical_id"] = Value::Null;
                            book["is_partial_book"] = json!(false);
                            Some((u.position?, book))
                        }),
                )
                .collect();
            rows.sort_by(|a, b| a.0.total_cmp(&b.0));
            let author = catalogue
                .series_author(name)
                .map(|a| json!({ "id": catalogue.author_id(a), "name": a }));
            let rows: Vec<Value> = rows
                .into_iter()
                .map(|(position, book)| {
                    json!({ "position": position, "compilation": false, "book": book })
                })
                .collect();
            json!({
                "id": id,
                "name": name,
                "slug": slug(name),
                "primary_books_count": null,
                "is_completed": null,
                "author": author,
                "book_series": rows,
            })
        })
        .collect();
    json!({ "series": series })
}

/// `authors_by_pk` for `AuthorPage`, most read first.
fn author_page(catalogue: &Catalogue, id: i64) -> Option<Value> {
    let (id, name) = catalogue.authors().into_iter().find(|(i, _)| *i == id)?;
    let mut books: Vec<&Book> = catalogue
        .books
        .iter()
        .filter(|b| b.author == name)
        .collect();
    books.sort_by_key(|b| std::cmp::Reverse(b.users_count));
    let contributions: Vec<Value> = books
        .iter()
        .map(|b| {
            let mut book = book_summary(b);
            book["is_partial_book"] = json!(false);
            book["book_series"] = Value::Array(author_book_series(catalogue, b));
            json!({ "contribution": null, "book": book })
        })
        .collect();
    Some(json!({
        "id": id,
        "name": name,
        "slug": slug(name),
        "bio": null,
        "cached_image": {},
        "born_year": null,
        "death_year": null,
        // The catalogue holds a few of their books, not their count.
        "books_count": null,
        "contributions": contributions,
    }))
}

/// A book's `book_series` rows for `AuthorPage`, with the facts it asks for
/// of each series. The catalogue records no merged series, and no main-book
/// count, so that is its own books in the series with a position, not
/// compilations.
fn author_book_series(catalogue: &Catalogue, book: &Book) -> Vec<Value> {
    book.series
        .iter()
        .map(|s| {
            let main_books = catalogue
                .books
                .iter()
                .filter_map(|b| b.in_series(&s.name))
                .filter(|row| row.position.is_some() && !row.compilation)
                .count();
            json!({
                "series": {
                    "id": catalogue.series_id(&s.name),
                    "name": s.name,
                    "author_id": catalogue.series_author(&s.name).map(|a| catalogue.author_id(a)),
                    "canonical_id": null,
                    "primary_books_count": main_books,
                    "is_completed": null,
                },
            })
        })
        .collect()
}

fn search_document(book: &Book) -> Value {
    json!({
        "id": book.id,
        "title": book.title,
        "subtitle": book.subtitle,
        "author_names": [book.author],
        "image": image(&book.cover),
        "pages": book.pages,
        "release_year": release_year(book),
        "slug": slug(&book.title),
        "rating": book.rating,
        "users_count": book.users_count,
    })
}

// ----- transports -----

/// The GraphQL endpoint: answers from the catalogue and the client's own
/// state, after the faults have had their say.
struct ApiTransport {
    state: Mutex<State>,
    faults: Faults,
}

impl HttpTransport for ApiTransport {
    fn send(&self, req: HttpRequest) -> Result<HttpResponse> {
        let RequestBody::Json(body) = &req.body else {
            return Err(Error::Network("the demo only answers GraphQL".into()));
        };
        let body: Value = serde_json::from_slice(body)
            .map_err(|e| Error::Network(format!("bad demo request: {e}")))?;
        let op = body["operationName"].as_str().unwrap_or_default();
        // Slept before the state is locked, so slow requests don't queue.
        let latency = self.faults.latency();
        if !latency.is_zero() {
            std::thread::sleep(latency);
        }
        if let Some(scope) = self.faults.refused_scope(op) {
            return Ok(insufficient_scope(scope));
        }
        if let Some(failure) = self.faults.failure(writes::is_write(op), random_unit) {
            return fail(failure);
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(json_response(&answer(&mut state, op, &body["variables"])))
    }
}

/// Covers: the catalogue's own, downloaded from Hardcover.
struct CoverTransport {
    /// Downloads; `None` refuses everything (the tests never download).
    http: Option<UreqTransport>,
    faults: Faults,
}

static DOWNLOADS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| CATALOGUE.downloads());

impl HttpTransport for CoverTransport {
    fn send(&self, req: HttpRequest) -> Result<HttpResponse> {
        if req.method != Method::Get {
            return Err(Error::Network(
                "the demo's covers are only downloaded".into(),
            ));
        }
        if let Some(failure) = self.faults.cover_failure(random_unit) {
            return fail(failure);
        }
        match &self.http {
            Some(http) if DOWNLOADS.contains(req.url.as_str()) => http.send(req),
            _ => Ok(HttpResponse {
                status: 404,
                headers: Vec::new(),
                body: Vec::new(),
            }),
        }
    }
}

/// A number in `0..1` for the error rate.
fn random_unit() -> f64 {
    // 53 random bits, a float's mantissa.
    #[expect(clippy::cast_precision_loss)]
    getrandom::u64().map_or(0.0, |n| (n >> 11) as f64 / (1_u64 << 53) as f64)
}

/// What an injected failure looks like on the wire.
fn fail(failure: Failure) -> Result<HttpResponse> {
    let response = |status: u16, headers: Vec<(String, String)>, body: Value| HttpResponse {
        status,
        headers,
        body: serde_json::to_vec(&body).expect("serializing a Value"),
    };
    match failure {
        Failure::Offline => Err(Error::Network(
            "demo fault: no connection (HARDCOVER_DEMO_FAULTS=offline)".into(),
        )),
        Failure::Timeout => Err(Error::Timeout),
        // Longer than the client retries by itself, so it reports it at once.
        Failure::RateLimited => Ok(response(
            429,
            vec![("retry-after".into(), "60".into())],
            json!({ "error": "Throttled" }),
        )),
        Failure::Unauthorized => Ok(response(
            401,
            Vec::new(),
            json!({ "error": "invalid_token" }),
        )),
        Failure::ServerError => Ok(response(
            500,
            Vec::new(),
            json!({ "errors": [{ "message": "demo fault: server error" }] }),
        )),
    }
}

/// Hardcover's answer to a token without the scope a request needs.
fn insufficient_scope(scope: &str) -> HttpResponse {
    HttpResponse {
        status: 403,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_vec(&json!({
            "error": "insufficient_scope",
            "scope": scope,
            "error_description": format!("This request needs the {scope} scope."),
        }))
        .expect("serializing a Value"),
    }
}

fn json_response(body: &Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_vec(&body).expect("serializing a Value"),
    }
}

fn error(message: impl Into<String>) -> Value {
    json!({ "errors": [{ "message": message.into() }] })
}

/// The response body for one GraphQL operation.
fn answer(state: &mut State, op: &str, vars: &Value) -> Value {
    if writes::is_write(op) {
        return match writes::apply(state, op, vars) {
            Ok(data) => json!({ "data": data }),
            Err(message) => error(message),
        };
    }
    let state = &*state;
    let catalogue = &*CATALOGUE;
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    let mine = || int("userId") == catalogue.user.id;
    let page_size = |name| usize::try_from(int(name)).unwrap_or_default();
    let data = match op {
        "Me" => json!({ "me": [{
            "id": catalogue.user.id,
            "username": catalogue.user.username,
            "name": catalogue.user.name,
            "image": {},
        }]}),
        "LibraryPage" => library_page(state, mine(), int("afterId"), page_size("limit")),
        "UserBookById" => json!({
            "user_books_by_pk": state.entry(int("id")).and_then(user_book),
        }),
        "UserBookForUser" => {
            let found = state
                .entry(int("id"))
                .filter(|_| mine())
                .and_then(user_book);
            json!({ "user_books": found.into_iter().collect::<Vec<_>>() })
        }
        "UserBookForBook" => {
            let found = state
                .library
                .iter()
                .filter(|e| mine() && e.book == int("bookId"))
                .min_by_key(|e| e.id)
                .and_then(user_book);
            json!({ "user_books": found.into_iter().collect::<Vec<_>>() })
        }
        "SearchBooks" => search(catalogue, vars),
        "SearchAuthors" => search_authors(catalogue, vars),
        "SearchSeries" => search_series(catalogue, vars),
        "BooksTrending" => trending(catalogue, vars),
        "BookDetails" => json!({
            "books_by_pk": catalogue.book(int("id")).map(book_details).or_else(|| {
                catalogue.upcoming(int("id")).map(|u| upcoming_details(catalogue, u))
            }),
        }),
        "SeriesPage" => json!({ "series_by_pk": series_page(catalogue, int("id")) }),
        "UpcomingInSeries" => upcoming_rows(catalogue, vars),
        "SeriesOverview" => series_overview(catalogue, vars),
        "AuthorPage" => json!({ "authors_by_pk": author_page(catalogue, int("id")) }),
        "BooksByIds" => books_by_ids(catalogue, vars),
        "ProgressContext" => progress_context(state, int("id"), vars),
        "BookEditions" => {
            let editions: Vec<Value> = catalogue
                .book(int("bookId"))
                .map_or_default(|book| {
                    if book.editions.is_empty() {
                        vec![edition(book, None)]
                    } else {
                        book.editions
                            .iter()
                            .map(|e| edition(book, Some(e)))
                            .collect()
                    }
                })
                .into_iter()
                .skip(page_size("offset"))
                .take(page_size("limit").max(1))
                .collect();
            json!({ "editions": editions })
        }
        "UserBookReview" => {
            json!({ "user_books_by_pk": state.entry(int("id")).map(review_fields) })
        }
        "Goals" => {
            let mut goals: Vec<&state::Goal> = state
                .goals
                .iter()
                .filter(|g| mine() && !g.archived)
                .collect();
            goals.sort_by(|a, b| b.end.cmp(&a.end).then_with(|| b.id.cmp(&a.id)));
            json!({ "goals": goals.into_iter().map(|g| state.goal_json(g)).collect::<Vec<_>>() })
        }
        "GoalById" => json!({
            "goals_by_pk": state.goals.iter()
                .find(|g| g.id == int("id"))
                .map(|g| state.goal_json(g)),
        }),
        "Lists" => lists_page(state, mine(), int("afterId"), page_size("limit")),
        "ListById" => json!({ "lists_by_pk": state.list(int("id")).map(list) }),
        "ListMemberships" => {
            list_memberships_page(state, mine(), int("afterId"), page_size("limit"))
        }
        "BookListMemberships" => {
            let mut rows: Vec<Value> = memberships(state)
                .into_iter()
                .filter(|(book, _)| mine() && *book == int("bookId"))
                .map(|(_, row)| row)
                .collect();
            rows.sort_by_key(|row| row["id"].as_i64());
            json!({ "list_books": rows })
        }
        "ListBooks" => list_books_page(
            state,
            int("listId"),
            page_size("offset"),
            page_size("limit"),
        ),
        "BookReviews" => reviews::page(state, vars),
        "BookReviewsCount" => reviews::count(state, vars),
        other => return error(format!("The demo has no answer for {other}.")),
    };
    json!({ "data": data })
}

/// `ReviewFields`, with the entry's id.
fn review_fields(entry: &Entry) -> Value {
    let review = &entry.review;
    json!({
        "id": entry.id,
        "review_markdown": review.markdown,
        "review_raw": review.raw,
        "review_has_spoilers": review.spoilers,
        "reviewed_at": review.reviewed_at.map(|d| format!("{d}T00:00:00")),
        "has_review": review.exists(),
    })
}

fn library_page(state: &State, mine: bool, after: i64, limit: usize) -> Value {
    let mut entries: Vec<&Entry> = state
        .library
        .iter()
        .filter(|e| mine && e.id > after)
        .collect();
    entries.sort_by_key(|e| e.id);
    let page: Vec<Value> = entries
        .into_iter()
        .take(limit)
        .filter_map(user_book)
        .collect();
    json!({ "user_books": page })
}

/// `{pages audio_seconds reading_format_id}` of an edition.
fn lengths(id: Option<i64>) -> Value {
    match id.and_then(|id| CATALOGUE.edition(id)) {
        Some((_, Some(e))) => json!({
            "pages": e.pages,
            "audio_seconds": e.audio_seconds,
            "reading_format_id": e.reading_format,
        }),
        Some((_, None)) => {
            json!({ "pages": null, "audio_seconds": null, "reading_format_id": null })
        }
        None => Value::Null,
    }
}

fn progress_context(state: &State, id: i64, vars: &Value) -> Value {
    // `$reads` is either `{id: {_eq: n}}` or `{finished_at: {_is_null: true}}`.
    let wanted = vars["reads"]["id"]["_eq"].as_i64();
    let found = state.entry(id).map(|e| {
        let book = CATALOGUE.book(e.book);
        // Every matching read: the client picks the open one by date.
        let reads: Vec<Value> = e
            .reads
            .iter()
            .filter(|r| match wanted {
                Some(id) => r.id == id,
                None => r.finished.is_none(),
            })
            .map(|r| {
                json!({
                    "id": r.id,
                    "started_at": r.started.map(|d| d.to_string()),
                    "finished_at": r.finished.map(|d| d.to_string()),
                    "edition": lengths(r.edition),
                })
            })
            .collect();
        json!({
            "id": e.id,
            "status_id": e.status,
            "book": {
                "pages": book.and_then(|b| b.pages),
                "audio_seconds": book.and_then(|b| b.audio_seconds),
            },
            "edition": lengths(e.edition),
            "user_book_reads": reads,
        })
    });
    json!({ "user_books": found.into_iter().collect::<Vec<_>>() })
}

fn lists_page(state: &State, mine: bool, after: i64, limit: usize) -> Value {
    let mut lists: Vec<&List> = state
        .lists
        .iter()
        .filter(|l| mine && l.id > after)
        .collect();
    lists.sort_by_key(|l| l.id);
    let lists: Vec<Value> = lists.into_iter().take(limit).map(list).collect();
    json!({ "lists": lists })
}

/// A page of one list's rows, by position (none last), then id.
fn list_books_page(state: &State, id: i64, offset: usize, limit: usize) -> Value {
    let Some(l) = state.list(id) else {
        return json!({ "list_books": [] });
    };
    let mut rows: Vec<&state::ListRow> = l.rows.iter().collect();
    rows.sort_by(|a, b| {
        by_position(a.position.map(|p| p as f64), b.position.map(|p| p as f64))
            .then_with(|| a.id.cmp(&b.id))
    });
    let rows: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| {
            let book = CATALOGUE.book(r.book)?;
            Some(json!({
                "id": r.id,
                "position": r.position,
                "edition_id": r.edition,
                "book": book_summary(book),
            }))
        })
        .skip(offset)
        .take(limit)
        .collect();
    json!({ "list_books": rows })
}

fn list_memberships_page(state: &State, mine: bool, after: i64, limit: usize) -> Value {
    let mut rows = memberships(state);
    rows.sort_by_key(|(_, row)| row["id"].as_i64());
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|(_, row)| row)
        .filter(|row| mine && row["id"].as_i64() > Some(after))
        .take(limit)
        .collect();
    json!({ "list_books": rows })
}

fn books_by_ids(catalogue: &Catalogue, vars: &Value) -> Value {
    let ids: Vec<i64> = vars["ids"]
        .as_array()
        .map_or_default(|ids| ids.iter().filter_map(Value::as_i64).collect());
    let books: Vec<Value> = catalogue
        .books
        .iter()
        .filter(|b| ids.contains(&b.id))
        .map(book_summary)
        .collect();
    json!({ "books": books })
}

/// Every word of the query must appear in the title or the author.
fn search(catalogue: &Catalogue, vars: &Value) -> Value {
    let (words, page, per_page) = search_terms(vars);
    let mut matches: Vec<&Book> = catalogue
        .books
        .iter()
        .filter(|b| {
            let haystack = format!("{} {}", b.title, b.author).to_lowercase();
            words.iter().all(|w| haystack.contains(w))
        })
        .collect();
    // Most readers first, roughly how Hardcover ranks.
    matches.sort_by_key(|b| std::cmp::Reverse(b.users_count));
    let hits: Vec<Value> = matches
        .iter()
        .skip((page - 1).saturating_mul(per_page))
        .take(per_page)
        .map(|b| json!({ "document": search_document(b) }))
        .collect();
    json!({ "search": {
        "error": null,
        "page": page,
        "per_page": per_page,
        "results": { "found": matches.len(), "page": page, "hits": hits },
    }})
}

/// `query` split into lowercase words, and the page asked for.
fn search_terms(vars: &Value) -> (Vec<String>, usize, usize) {
    let words = vars["query"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let page = usize::try_from(vars["page"].as_u64().unwrap_or(1).max(1)).unwrap_or(usize::MAX);
    let per_page =
        usize::try_from(vars["perPage"].as_u64().unwrap_or(25).max(1)).unwrap_or(usize::MAX);
    (words, page, per_page)
}

/// A `search` response for one page of `(id, document)` matches, with `ids`.
fn search_response(matches: Vec<(i64, Value)>, page: usize, per_page: usize) -> Value {
    let found = matches.len();
    let shown: Vec<(i64, Value)> = matches
        .into_iter()
        .skip((page - 1).saturating_mul(per_page))
        .take(per_page)
        .collect();
    let ids: Vec<i64> = shown.iter().map(|(id, _)| *id).collect();
    let hits: Vec<Value> = shown
        .into_iter()
        .map(|(_, doc)| json!({ "document": doc }))
        .collect();
    json!({ "search": {
        "error": null,
        "page": page,
        "per_page": per_page,
        "ids": ids,
        "results": { "found": found, "page": page, "hits": hits },
    }})
}

/// Catalogue authors whose name or books contain every word, most read first.
fn search_authors(catalogue: &Catalogue, vars: &Value) -> Value {
    let (words, page, per_page) = search_terms(vars);
    let mut matches: Vec<(i64, i64, Value)> = catalogue
        .authors()
        .into_iter()
        .filter_map(|(id, name)| {
            let mut books: Vec<&Book> = catalogue
                .books
                .iter()
                .filter(|b| b.author == name)
                .collect();
            books.sort_by_key(|b| std::cmp::Reverse(b.users_count));
            let haystack = std::iter::once(name)
                .chain(books.iter().map(|b| b.title.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !words.iter().all(|w| haystack.contains(w.as_str())) {
                return None;
            }
            let readers = books.iter().map(|b| b.users_count).sum();
            let titles: Vec<&str> = books.iter().take(5).map(|b| b.title.as_str()).collect();
            Some((
                id,
                readers,
                json!({
                    "id": id.to_string(),
                    "name": name,
                    "slug": slug(name),
                    "image": null,
                    // The catalogue holds a few of their books, not their count.
                    "books_count": null,
                    "books": titles,
                }),
            ))
        })
        .collect();
    matches.sort_by_key(|(_, readers, _)| std::cmp::Reverse(*readers));
    search_response(
        matches.into_iter().map(|(id, _, doc)| (id, doc)).collect(),
        page,
        per_page,
    )
}

/// Catalogue series whose name, books or author contain every word, most read
/// first.
fn search_series(catalogue: &Catalogue, vars: &Value) -> Value {
    let (words, page, per_page) = search_terms(vars);
    let mut matches: Vec<(i64, i64, Value)> = catalogue
        .series()
        .into_iter()
        .filter_map(|(id, name)| {
            let mut books: Vec<(Option<f64>, &Book)> = catalogue
                .books
                .iter()
                .filter_map(|b| Some((b.in_series(name)?.position, b)))
                .collect();
            books.sort_by(|a, b| by_position(a.0, b.0));
            let author = catalogue.series_author(name);
            let haystack = std::iter::once(name)
                .chain(books.iter().map(|(_, b)| b.title.as_str()))
                .chain(author)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !words.iter().all(|w| haystack.contains(w.as_str())) {
                return None;
            }
            let readers = books.iter().map(|(_, b)| b.users_count).sum();
            let titles: Vec<&str> = books.iter().map(|(_, b)| b.title.as_str()).collect();
            Some((
                id,
                readers,
                json!({
                    "id": id.to_string(),
                    "name": name,
                    "slug": slug(name),
                    "author_name": author,
                    // The catalogue holds some of the series, not its size.
                    "books_count": null,
                    "primary_books_count": null,
                    "readers_count": null,
                    "books": titles,
                }),
            ))
        })
        .collect();
    matches.sort_by_key(|(_, readers, _)| std::cmp::Reverse(*readers));
    search_response(
        matches.into_iter().map(|(id, _, doc)| (id, doc)).collect(),
        page,
        per_page,
    )
}

/// `books_trending`: the catalogue ranked by one of its numbers per period.
fn trending(catalogue: &Catalogue, vars: &Value) -> Value {
    let limit = usize::try_from(vars["limit"].as_i64().unwrap_or(25)).unwrap_or_default();
    let mut books: Vec<&Book> = catalogue.books.iter().collect();
    match vars["duration"].as_str().unwrap_or("month") {
        "week" => books.sort_by(|a, b| b.release_date.cmp(&a.release_date)),
        "three_month" => books.sort_by_key(|b| std::cmp::Reverse(b.ratings_count)),
        "one_year" | "all" => books.sort_by_key(|b| std::cmp::Reverse(b.users_count)),
        _ => books.sort_by(|a, b| b.rating.total_cmp(&a.rating)),
    }
    let ids: Vec<i64> = books.into_iter().take(limit).map(|b| b.id).collect();
    json!({ "books_trending": { "error": null, "ids": ids } })
}
