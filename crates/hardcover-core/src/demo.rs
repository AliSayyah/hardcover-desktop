//! A demo library for screenshots and for trying the app without an account.
//!
//! [`client`] answers from `demo/catalogue.json` through a fake transport, so
//! the rest of the client (queries, parsing, paging) runs exactly as it does
//! against Hardcover. The books are real, popular ones, with facts and cover
//! links from Hardcover and descriptions written for the demo; the reader,
//! their shelves, reads, notes, reviews, reading goal and lists are invented.
//! Each book has one stand-in edition for the edition picker, with only the
//! book's title and cover; the Owned list holds that edition.
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
//! [`cover_cache`] downloads the catalogue's covers from Hardcover, the demo's
//! only network access; any other URL is refused. Reads work; every write
//! fails with a "read-only" error. Only built with the `demo` feature (and in
//! tests).

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Client, TokenSource};
use crate::covers::CoverCache;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, Method, RequestBody, UreqTransport};
use crate::ratelimit::TokenBucket;
use crate::{Error, Result};

mod reviews;

/// The reply to every write.
pub const READ_ONLY: &str = "The demo library is read-only.";

/// A client for the demo library. It never touches the network.
pub fn client() -> Client {
    Client::with_transport(
        Arc::new(DemoToken),
        Arc::new(DemoTransport { covers: None }),
        // No rate limit to respect.
        TokenBucket::new(1_000, 1_000.0),
    )
}

/// A cover cache in `dir` that downloads the catalogue's covers from Hardcover.
pub fn cover_cache(dir: impl Into<PathBuf>) -> Result<CoverCache> {
    CoverCache::with_transport(
        dir,
        Arc::new(DemoTransport {
            covers: Some(UreqTransport::new()),
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
    library: Vec<Entry>,
    goals: Vec<DemoGoal>,
    lists: Vec<DemoList>,
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

#[derive(Deserialize)]
/// One of the reader's lists. The Owned list's books are on it as their one
/// demo edition, whose id is the book's.
struct DemoList {
    id: i64,
    name: String,
    slug: String,
    description: Option<String>,
    privacy: i64,
    ranked: bool,
    books: Vec<i64>,
}

impl DemoList {
    fn is_owned(&self) -> bool {
        self.slug == crate::api::OWNED_LIST_SLUG
    }

    /// `(row id, book id, position)` for each book, in the list's order.
    /// Row ids are the list's id times 1,000 plus the place.
    fn rows(&self) -> impl Iterator<Item = (i64, i64, Option<i64>)> + '_ {
        self.books.iter().enumerate().map(|(i, book)| {
            let place = i as i64 + 1;
            (self.id * 1_000 + place, *book, self.ranked.then_some(place))
        })
    }
}

#[derive(Deserialize)]
/// A goal for the current calendar year, whichever year the demo runs in.
struct DemoGoal {
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
    pages: i64,
    release_date: String,
    rating: f64,
    ratings_count: i64,
    users_count: i64,
    editions_count: i64,
    genres: Vec<String>,
    series: Option<Series>,
    description: String,
    cover: CoverLink,
}

#[derive(Deserialize)]
struct CoverLink {
    url: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct Series {
    name: String,
    position: f64,
}

#[derive(Deserialize)]
struct Entry {
    id: i64,
    book: i64,
    status: i64,
    rating: Option<f64>,
    date_added: String,
    updated_at: String,
    #[serde(default)]
    reads: Vec<Read>,
    private_notes: Option<String>,
    /// `privacy_setting_id`; Public when absent.
    privacy: Option<i64>,
    review: Option<DemoReview>,
}

/// The demo reader's own review, written for the demo.
#[derive(Deserialize)]
struct DemoReview {
    markdown: String,
    #[serde(default)]
    spoilers: bool,
    reviewed_at: String,
}

#[derive(Deserialize)]
struct Read {
    id: i64,
    started_at: Option<String>,
    finished_at: Option<String>,
    progress_pages: Option<i64>,
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

    /// Distinct names in catalogue order, numbered from `base + 1`.
    fn numbered<'a>(
        &'a self,
        names: impl Iterator<Item = &'a str>,
        base: i64,
    ) -> Vec<(i64, &'a str)> {
        let mut seen: Vec<&str> = Vec::new();
        for name in names {
            if !seen.contains(&name) {
                seen.push(name);
            }
        }
        seen.into_iter()
            .enumerate()
            .map(|(i, name)| (base + 1 + i as i64, name))
            .collect()
    }

    fn authors(&self) -> Vec<(i64, &str)> {
        self.numbered(self.books.iter().map(|b| b.author.as_str()), AUTHOR_IDS)
    }

    fn series(&self) -> Vec<(i64, &str)> {
        self.numbered(
            self.books
                .iter()
                .filter_map(|b| b.series.as_ref().map(|s| s.name.as_str())),
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
        others.into_iter().take(10).map(|(_, b)| b.id).collect()
    }

    fn entry(&self, id: i64) -> Option<&Entry> {
        self.library.iter().find(|e| e.id == id)
    }

    fn upcoming(&self, id: i64) -> Option<&Upcoming> {
        self.upcoming.iter().find(|u| u.id == id)
    }

    /// The author of a series: that of its first book in the catalogue.
    fn series_author(&self, series: &str) -> Option<&str> {
        self.books
            .iter()
            .find(|b| b.series.as_ref().is_some_and(|s| s.name == series))
            .map(|b| b.author.as_str())
    }
}

// ----- wire shapes -----

/// `GoalFields`, for the calendar year `year`. Progress is worked out from the
/// library like Hardcover does: the reads (or their books' pages) finished
/// within the goal's dates.
fn goal(catalogue: &Catalogue, goal: &DemoGoal, year: i32) -> Value {
    let (start, end) = (format!("{year}-01-01"), format!("{year}-12-31"));
    let finished = catalogue.library.iter().flat_map(|e| {
        e.reads
            .iter()
            .filter_map(|r| r.finished_at.as_deref())
            .filter(|f| (start.as_str()..=end.as_str()).contains(f))
            .map(move |_| e)
    });
    let progress: i64 = match goal.metric.as_str() {
        "page" => finished
            .filter_map(|e| catalogue.book(e.book))
            .map(|b| b.pages)
            .sum(),
        _ => finished.count() as i64,
    };
    json!({
        "id": goal.id,
        "metric": goal.metric,
        "goal": goal.goal,
        "progress": progress,
        "description": format!("{} {}s in {year}", goal.goal, goal.metric),
        "start_date": start,
        "end_date": end,
        "conditions": {},
        "archived": false,
        "completed_at": null,
        "privacy_setting_id": 1,
    })
}

/// `ListFields`.
fn list(list: &DemoList) -> Value {
    json!({
        "id": list.id,
        "name": list.name,
        "slug": list.slug,
        "description": list.description,
        "privacy_setting_id": list.privacy,
        "ranked": list.ranked,
        "books_count": list.books.len(),
        "default_view": "list",
        "featured_profile": false,
        "url": null,
    })
}

/// Every row on the reader's lists as `{id list_id book_id edition_id}`.
fn memberships(catalogue: &Catalogue) -> Vec<(i64, Value)> {
    catalogue
        .lists
        .iter()
        .flat_map(|l| {
            l.rows().map(move |(id, book, _)| {
                (
                    book,
                    json!({
                        "id": id,
                        "list_id": l.id,
                        "book_id": book,
                        "edition_id": l.is_owned().then_some(book),
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

fn image(book: &Book) -> Value {
    json!({
        "url": book.cover.url,
        "width": book.cover.width,
        "height": book.cover.height,
    })
}

/// `EditionFields` for the book's one demo edition. Its id is the book's,
/// not a Hardcover edition id. Only the title and cover are filled in: the
/// catalogue's page count and date are the book's, which another edition
/// may not share.
fn edition(book: &Book) -> Value {
    json!({
        "id": book.id,
        "title": book.title,
        "pages": null,
        "audio_seconds": null,
        "edition_format": null,
        "release_date": null,
        "isbn_13": null,
        "cached_image": image(book),
        "reading_format": null,
        "publisher": null,
        "language": null,
    })
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
        "cached_image": image(book),
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
        .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
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

/// `UserBookFields`.
fn user_book(catalogue: &Catalogue, entry: &Entry) -> Option<Value> {
    let book = catalogue.book(entry.book)?;
    let mut reads: Vec<&Read> = entry.reads.iter().collect();
    reads.sort_by_key(|r| std::cmp::Reverse(r.id));
    let reads: Vec<Value> = reads
        .into_iter()
        .map(|read| {
            json!({
                "id": read.id,
                "started_at": read.started_at,
                "finished_at": read.finished_at,
                "progress": read.progress_pages.map(|p| p as f64 * 100.0 / book.pages as f64),
                "progress_pages": read.progress_pages,
                "progress_seconds": null,
                "edition_id": null,
                "edition": null,
            })
        })
        .collect();
    Some(json!({
        "id": entry.id,
        "user_id": catalogue.user.id,
        "status_id": entry.status,
        "rating": entry.rating,
        "date_added": entry.date_added,
        "updated_at": entry.updated_at,
        "privacy_setting_id": entry.privacy.unwrap_or(1),
        "private_notes": entry.private_notes,
        "book": book_with_series(book),
        "edition": null,
        "user_book_reads": reads,
    }))
}

/// `BookSummaryFields` plus the book's series, as `UserBookFields` asks.
fn book_with_series(book: &Book) -> Value {
    let mut value = book_summary(book);
    if let Some(map) = value.as_object_mut() {
        map.insert("book_series".into(), Value::Array(book_series(book)));
    }
    value
}

/// A book's `book_series` rows.
fn book_series(book: &Book) -> Vec<Value> {
    book.series
        .iter()
        .map(|s| {
            json!({
                "position": s.position,
                "featured": true,
                "series": {
                    "id": CATALOGUE.series_id(&s.name),
                    "name": s.name,
                    "slug": slug(&s.name),
                },
            })
        })
        .collect()
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

/// `series_by_pk` for `SeriesPage`.
fn series_page(catalogue: &Catalogue, id: i64) -> Option<Value> {
    let (id, name) = catalogue.series().into_iter().find(|(i, _)| *i == id)?;
    let books: Vec<&Book> = catalogue
        .books
        .iter()
        .filter(|b| b.series.as_ref().is_some_and(|s| s.name == name))
        .collect();
    let author = books
        .first()
        .map(|b| json!({ "id": catalogue.author_id(&b.author), "name": b.author }));
    let mut rows: Vec<Value> = books
        .iter()
        .map(|b| {
            let mut book = book_summary(b);
            book["is_partial_book"] = json!(false);
            json!({
                "position": b.series.as_ref().map(|s| s.position),
                "compilation": false,
                "book": book,
            })
        })
        .collect();
    // Books still to come are on the series page, as on Hardcover.
    rows.extend(
        catalogue
            .upcoming
            .iter()
            .filter(|u| u.series == name)
            .map(|u| {
                let mut book = upcoming_summary(catalogue, u);
                book["is_partial_book"] = json!(false);
                json!({ "position": u.position, "compilation": false, "book": book })
            }),
    );
    Some(json!({
        "id": id,
        "name": name,
        "slug": slug(name),
        "description": null,
        "primary_books_count": null,
        "is_completed": null,
        "author": author,
        "book_series": rows,
    }))
}

/// `series` for `SeriesOverview`: each series asked for, with the books the
/// query keeps (a position, not merged, not a compilation), the catalogue's
/// and those still to come, by position.
fn series_overview(catalogue: &Catalogue, vars: &Value) -> Value {
    let ids: Vec<i64> = vars["ids"]
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    let series: Vec<Value> = catalogue
        .series()
        .into_iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, name)| {
            let mut rows: Vec<(f64, Value)> = catalogue
                .books
                .iter()
                .filter_map(|b| {
                    let s = b.series.as_ref().filter(|s| s.name == name)?;
                    let mut book = book_summary(b);
                    book["canonical_id"] = Value::Null;
                    book["release_date"] = json!(b.release_date);
                    book["is_partial_book"] = json!(false);
                    Some((s.position, book))
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

fn search_document(book: &Book) -> Value {
    json!({
        "id": book.id,
        "title": book.title,
        "subtitle": book.subtitle,
        "author_names": [book.author],
        "image": image(book),
        "pages": book.pages,
        "release_year": release_year(book),
        "slug": slug(&book.title),
        "rating": book.rating,
        "users_count": book.users_count,
    })
}

// ----- transport -----

struct DemoTransport {
    /// Downloads covers; `None` refuses them (the API client never asks).
    covers: Option<UreqTransport>,
}

impl HttpTransport for DemoTransport {
    fn send(&self, req: HttpRequest) -> Result<HttpResponse> {
        match req.method {
            Method::Get => {
                let known = CATALOGUE.books.iter().any(|b| b.cover.url == req.url);
                match &self.covers {
                    Some(http) if known => http.send(req),
                    _ => Ok(HttpResponse {
                        status: 404,
                        headers: Vec::new(),
                        body: Vec::new(),
                    }),
                }
            }
            Method::Post => {
                let RequestBody::Json(body) = &req.body else {
                    return Err(Error::Network("the demo only answers GraphQL".into()));
                };
                let body: Value = serde_json::from_slice(body)
                    .map_err(|e| Error::Network(format!("bad demo request: {e}")))?;
                let op = body["operationName"].as_str().unwrap_or_default();
                Ok(json_response(answer(op, &body["variables"])))
            }
        }
    }
}

fn json_response(body: Value) -> HttpResponse {
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
fn answer(op: &str, vars: &Value) -> Value {
    let catalogue = &*CATALOGUE;
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    let mine = || int("userId") == catalogue.user.id;
    let year = chrono::Datelike::year(&chrono::Local::now());
    let data = match op {
        "Me" => json!({ "me": [{
            "id": catalogue.user.id,
            "username": catalogue.user.username,
            "name": catalogue.user.name,
            "image": {},
        }]}),
        "LibraryPage" => {
            let after = int("afterId");
            let limit = usize::try_from(int("limit")).unwrap_or_default();
            let mut entries: Vec<&Entry> = catalogue
                .library
                .iter()
                .filter(|e| mine() && e.id > after)
                .collect();
            entries.sort_by_key(|e| e.id);
            let page: Vec<Value> = entries
                .into_iter()
                .take(limit)
                .filter_map(|e| user_book(catalogue, e))
                .collect();
            json!({ "user_books": page })
        }
        "UserBookById" => json!({
            "user_books_by_pk": catalogue.entry(int("id")).and_then(|e| user_book(catalogue, e)),
        }),
        "UserBookForUser" => {
            let found = catalogue
                .entry(int("id"))
                .filter(|_| mine())
                .and_then(|e| user_book(catalogue, e));
            json!({ "user_books": found.into_iter().collect::<Vec<_>>() })
        }
        "UserBookForBook" => {
            let found = catalogue
                .library
                .iter()
                .find(|e| mine() && e.book == int("bookId"))
                .and_then(|e| user_book(catalogue, e));
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
        "BooksByIds" => {
            let ids: Vec<i64> = vars["ids"]
                .as_array()
                .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            let books: Vec<Value> = catalogue
                .books
                .iter()
                .filter(|b| ids.contains(&b.id))
                .map(book_summary)
                .collect();
            json!({ "books": books })
        }
        "ProgressContext" => {
            // `$reads` is either `{id: {_eq: n}}` or "unfinished".
            let wanted = vars["reads"]["id"]["_eq"].as_i64();
            let found = catalogue.entry(int("id")).and_then(|e| {
                let book = catalogue.book(e.book)?;
                let read = e
                    .reads
                    .iter()
                    .filter(|r| match wanted {
                        Some(id) => r.id == id,
                        None => r.finished_at.is_none(),
                    })
                    .max_by_key(|r| r.id);
                Some(json!({
                    "id": e.id,
                    "status_id": e.status,
                    "book": { "pages": book.pages, "audio_seconds": null },
                    "edition": null,
                    "user_book_reads": read.map(|r| json!({ "id": r.id, "edition": null }))
                        .into_iter()
                        .collect::<Vec<_>>(),
                }))
            });
            json!({ "user_books": found.into_iter().collect::<Vec<_>>() })
        }
        "BookEditions" => {
            let editions: Vec<Value> = catalogue
                .book(int("bookId"))
                .filter(|_| int("offset") == 0)
                .map(edition)
                .into_iter()
                .collect();
            json!({ "editions": editions })
        }
        "UserBookReview" => json!({
            "user_books_by_pk": catalogue.entry(int("id")).map(|e| match &e.review {
                Some(r) => json!({
                    "id": e.id,
                    "review_markdown": r.markdown,
                    "review_raw": null,
                    "review_has_spoilers": r.spoilers,
                    "reviewed_at": format!("{}T00:00:00", r.reviewed_at),
                    "has_review": true,
                }),
                None => json!({
                    "id": e.id,
                    "review_markdown": null,
                    "review_raw": null,
                    "review_has_spoilers": false,
                    "reviewed_at": null,
                    "has_review": false,
                }),
            }),
        }),
        "Goals" => json!({
            "goals": catalogue.goals.iter()
                .filter(|_| mine())
                .map(|g| goal(catalogue, g, year))
                .collect::<Vec<_>>(),
        }),
        "GoalById" => json!({
            "goals_by_pk": catalogue.goals.iter()
                .find(|g| g.id == int("id"))
                .map(|g| goal(catalogue, g, year)),
        }),
        "Lists" => {
            let after = int("afterId");
            let limit = usize::try_from(int("limit")).unwrap_or_default();
            let lists: Vec<Value> = catalogue
                .lists
                .iter()
                .filter(|l| mine() && l.id > after)
                .take(limit)
                .map(list)
                .collect();
            json!({ "lists": lists })
        }
        "ListById" => json!({
            "lists_by_pk": catalogue.lists.iter().find(|l| l.id == int("id")).map(list),
        }),
        "ListMemberships" => {
            let after = int("afterId");
            let limit = usize::try_from(int("limit")).unwrap_or_default();
            let mut rows = memberships(catalogue);
            rows.sort_by_key(|(_, row)| row["id"].as_i64());
            let rows: Vec<Value> = rows
                .into_iter()
                .map(|(_, row)| row)
                .filter(|row| mine() && row["id"].as_i64() > Some(after))
                .take(limit)
                .collect();
            json!({ "list_books": rows })
        }
        "BookListMemberships" => {
            let rows: Vec<Value> = memberships(catalogue)
                .into_iter()
                .filter(|(book, _)| mine() && *book == int("bookId"))
                .map(|(_, row)| row)
                .collect();
            json!({ "list_books": rows })
        }
        "ListBooks" => {
            let offset = usize::try_from(int("offset")).unwrap_or_default();
            let limit = usize::try_from(int("limit")).unwrap_or_default();
            let rows: Vec<Value> = catalogue
                .lists
                .iter()
                .find(|l| l.id == int("listId"))
                .into_iter()
                .flat_map(|l| {
                    l.rows().filter_map(|(id, book, position)| {
                        let book = catalogue.book(book)?;
                        Some(json!({
                            "id": id,
                            "position": position,
                            "edition_id": l.is_owned().then_some(book.id),
                            "book": book_summary(book),
                        }))
                    })
                })
                .skip(offset)
                .take(limit)
                .collect();
            json!({ "list_books": rows })
        }
        "BookReviews" => reviews::page(catalogue, vars),
        "BookReviewsCount" => reviews::count(catalogue, vars),
        "InsertUserBook"
        | "UpdateUserBook"
        | "DeleteUserBook"
        | "InsertUserBookRead"
        | "UpdateUserBookRead"
        | "DeleteUserBookRead"
        | "EditionOwned"
        | "UpdateUserBookReview"
        | "DeleteUserBookReview"
        | "InsertGoal"
        | "UpdateGoal"
        | "DeleteGoal"
        | "InsertList"
        | "UpdateList"
        | "DeleteList"
        | "InsertListBook"
        | "DeleteListBook" => {
            return error(READ_ONLY);
        }
        other => return error(format!("The demo has no answer for {other}.")),
    };
    json!({ "data": data })
}

/// Every word of the query must appear in the title or the author.
fn search(catalogue: &Catalogue, vars: &Value) -> Value {
    let query = vars["query"].as_str().unwrap_or_default().to_lowercase();
    let page = vars["page"].as_u64().unwrap_or(1).max(1) as usize;
    let per_page = vars["perPage"].as_u64().unwrap_or(25).max(1) as usize;
    let words: Vec<&str> = query.split_whitespace().collect();
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
        .skip((page - 1) * per_page)
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
    let page = vars["page"].as_u64().unwrap_or(1).max(1) as usize;
    let per_page = vars["perPage"].as_u64().unwrap_or(25).max(1) as usize;
    (words, page, per_page)
}

/// A `search` response for one page of `(id, document)` matches, with `ids`.
fn search_response(matches: Vec<(i64, Value)>, page: usize, per_page: usize) -> Value {
    let found = matches.len();
    let shown: Vec<(i64, Value)> = matches
        .into_iter()
        .skip((page - 1) * per_page)
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
            let mut books: Vec<&Book> = catalogue
                .books
                .iter()
                .filter(|b| b.series.as_ref().is_some_and(|s| s.name == name))
                .collect();
            books.sort_by(|a, b| {
                let position = |b: &Book| b.series.as_ref().map_or(0.0, |s| s.position);
                position(a).total_cmp(&position(b))
            });
            let author = books.first().map(|b| b.author.as_str());
            let haystack = std::iter::once(name)
                .chain(books.iter().map(|b| b.title.as_str()))
                .chain(author)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !words.iter().all(|w| haystack.contains(w.as_str())) {
                return None;
            }
            let readers = books.iter().map(|b| b.users_count).sum();
            let titles: Vec<&str> = books.iter().map(|b| b.title.as_str()).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::DOCUMENTS;
    use crate::{BookId, ReadingStatus, UserBookId};

    #[test]
    fn signs_in_as_the_demo_reader() {
        let me = client().me().unwrap();
        assert_eq!(me.username, "demo_reader");
        assert!(me.avatar.is_none());
    }

    #[test]
    fn the_library_fills_every_shelf() {
        let client = client();
        let me = client.me().unwrap();
        let library = client.library(me.id).unwrap();
        assert_eq!(library.len(), CATALOGUE.library.len());
        for status in ReadingStatus::ALL {
            assert!(
                library.iter().any(|e| e.status == status),
                "nothing on {status:?}"
            );
        }
        let reading = library
            .iter()
            .find(|e| e.status == ReadingStatus::CurrentlyReading)
            .unwrap();
        assert!(reading.open_read().unwrap().progress_pages.is_some());
        assert!(reading.book.cover.is_some());
        assert!(
            library.iter().any(|e| e.reads.len() > 1),
            "no entry shows a read history"
        );
        assert!(library.iter().any(|e| e.private_notes.is_some()));
    }

    #[test]
    fn every_book_has_one_edition() {
        let client = client();
        for book in &CATALOGUE.books {
            let editions = client.book_editions(BookId::new(book.id), 0).unwrap();
            assert_eq!(editions.len(), 1, "{}", book.title);
            let edition = &editions[0];
            assert_eq!(edition.cover.as_ref().unwrap().url, book.cover.url);
            assert!(
                client
                    .book_editions(BookId::new(book.id), 1)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn library_pages_honour_the_cursor_and_limit() {
        let page = |after: i64| {
            let body = answer(
                "LibraryPage",
                &json!({ "userId": 1, "afterId": after, "limit": 5 }),
            );
            body["data"]["user_books"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].as_i64().unwrap())
                .collect::<Vec<_>>()
        };
        let mut seen: Vec<i64> = Vec::new();
        let mut after = 0;
        loop {
            let ids = page(after);
            assert!(ids.len() <= 5);
            seen.extend(&ids);
            match ids.last() {
                Some(last) => after = *last,
                None => break,
            }
        }
        assert_eq!(seen.len(), CATALOGUE.library.len());
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn every_book_has_details_and_a_hardcover_cover() {
        let client = client();
        for book in &CATALOGUE.books {
            let details = client.book_details(BookId::new(book.id)).unwrap();
            assert_eq!(details.summary.title, book.title);
            assert!(details.description.is_some());
            let cover = details.summary.cover.expect("a cover");
            assert!(
                cover.url.starts_with("https://assets.hardcover.app/"),
                "{}",
                cover.url
            );
        }
    }

    #[test]
    fn only_catalogue_covers_are_fetched() {
        // No transport for covers: nothing reaches the network in tests.
        let transport = DemoTransport { covers: None };
        let response = transport
            .send(HttpRequest::new(Method::Get, "https://example.com/x.jpg"))
            .unwrap();
        assert_eq!(response.status, 404);
    }

    #[test]
    fn entries_can_be_read_back() {
        let client = client();
        let me = client.me().unwrap();
        let first = &CATALOGUE.library[0];
        let entry = client.library_entry(UserBookId::new(first.id)).unwrap();
        assert_eq!(entry.unwrap().user_book_id, UserBookId::new(first.id));
        let by_book = client
            .library_entry_for_book(me.id, BookId::new(first.book))
            .unwrap();
        assert!(by_book.is_some());
        assert!(
            client
                .library_entry_for_book(me.id, BookId::new(427314))
                .unwrap()
                .is_none(),
            "Children of Dune is only in search"
        );
    }

    #[test]
    fn search_matches_titles_and_authors() {
        let client = client();
        let herbert = client.search_books("frank herbert", 1).unwrap();
        assert_eq!(herbert.books.len(), 4);
        let dune = client.search_books("Dune", 1).unwrap();
        assert_eq!(dune.books.len(), 4);
        assert_eq!(dune.books[0].title, "Dune", "the most read first");
        assert!(client.search_books("zzz", 1).unwrap().books.is_empty());
    }

    #[test]
    fn the_reader_has_a_goal_counted_from_their_library() {
        let client = client();
        let goals = client.goals().unwrap();
        assert_eq!(goals.len(), 1);
        let goal = &goals[0];
        assert_eq!(goal.metric, crate::GoalMetric::Books);
        assert_eq!(goal.target, 12);
        // Always this year's, so the demo shows a current goal.
        assert!(goal.is_current(chrono::Local::now().date_naive()));
        let err = client.delete_goal(goal.id).unwrap_err();
        assert!(err.to_string().contains(READ_ONLY), "{err}");
    }

    #[test]
    fn the_demo_goal_counts_the_reads_finished_in_its_year() {
        let finished_in = |year: i32| {
            CATALOGUE
                .library
                .iter()
                .flat_map(|e| &e.reads)
                .filter_map(|r| r.finished_at.as_deref())
                .filter(|f| f.starts_with(&format!("{year}-")))
                .count() as i64
        };
        assert!(finished_in(2026) > 0);
        for year in [2026, 2027] {
            let g = goal(&CATALOGUE, &CATALOGUE.goals[0], year);
            assert_eq!(g["start_date"], format!("{year}-01-01"));
            assert_eq!(g["end_date"], format!("{year}-12-31"));
            assert_eq!(g["description"], format!("12 books in {year}"));
            assert_eq!(g["progress"], finished_in(year));
        }
    }

    #[test]
    fn the_reader_has_reviewed_a_few_books() {
        let client = client();
        let dune = client.review(UserBookId::new(521)).unwrap();
        assert!(dune.text().unwrap().contains("Dune Messiah"));
        assert_eq!(
            dune.reviewed_at,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 13)
        );
        assert!(!dune.has_spoilers);
        let reviewed = CATALOGUE.library.iter().filter(|e| e.review.is_some());
        assert_eq!(reviewed.count(), 2);
        assert!(!client.review(UserBookId::new(501)).unwrap().exists());
        let err = client
            .delete_review(UserBookId::new(521), ReadingStatus::Read)
            .unwrap_err();
        assert!(err.to_string().contains(READ_ONLY), "{err}");
    }

    #[test]
    fn writes_are_refused() {
        let client = client();
        let err = client
            .set_status(UserBookId::new(501), ReadingStatus::Read)
            .unwrap_err();
        assert!(err.to_string().contains(READ_ONLY), "{err}");
    }

    #[test]
    fn series_and_author_pages_link_up() {
        let client = client();
        let dune = client.book_details(BookId::new(312460)).unwrap();
        let series_id = dune.series[0].id.expect("a series id");
        let series = client.series(series_id).unwrap();
        assert_eq!(series.name, "Dune");
        let titles: Vec<&str> = series.books.iter().map(|b| b.book.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "Dune",
                "Dune Messiah",
                "Children of Dune",
                "God Emperor of Dune"
            ]
        );
        let author = series.author.expect("an author");
        assert_eq!(author.name, "Frank Herbert");
        assert_eq!(dune.contributors[0].author_id, Some(author.id));

        let page = client.author(author.id).unwrap();
        assert_eq!(page.books.len(), 4);
        assert_eq!(page.books[0].book.title, "Dune", "the most read first");
        assert!(page.bio.is_none() && page.photo.is_none());
        assert!(client.author(crate::AuthorId::new(1)).is_err());
    }

    #[test]
    fn library_entries_carry_their_series() {
        let client = client();
        let me = client.me().unwrap();
        let library = client.library(me.id).unwrap();
        let messiah = library
            .iter()
            .find(|e| e.book.title == "Dune Messiah")
            .expect("Dune Messiah is shelved");
        let details = client.book_details(messiah.book.id).unwrap();
        assert_eq!(messiah.series, details.series, "the same as the book page");
        assert_eq!(messiah.series[0].name, "Dune");
        assert_eq!(messiah.series[0].position, Some(2.0));
        assert!(messiah.series[0].id.is_some());
        let habits = library.iter().find(|e| e.book.title == "Atomic Habits");
        assert!(habits.expect("shelved").series.is_empty());
    }

    #[test]
    fn similar_books_come_from_the_catalogue() {
        let client = client();
        let dune = client.book_details(BookId::new(312460)).unwrap();
        assert!(!dune.similar_book_ids.is_empty());
        assert!(!dune.similar_book_ids.contains(&BookId::new(312460)));
        let books = client.books_by_ids(&dune.similar_book_ids).unwrap();
        assert_eq!(books.len(), dune.similar_book_ids.len());
        assert_eq!(books[0].id, dune.similar_book_ids[0]);
        assert!(dune.ratings_distribution.is_empty());
    }

    #[test]
    fn author_and_series_hits_open_their_pages() {
        let client = client();
        let authors = client.search_authors("herbert", 1).unwrap();
        assert_eq!(authors.hits.len(), 1);
        let herbert = &authors.hits[0];
        assert_eq!(herbert.name, "Frank Herbert");
        assert_eq!(herbert.top_books[0], "Dune");
        assert_eq!(herbert.books_count, None);
        assert_eq!(client.author(herbert.id).unwrap().name, "Frank Herbert");

        let series = client.search_series("dune", 1).unwrap();
        assert_eq!(series.hits.len(), 1);
        let dune = &series.hits[0];
        assert_eq!(dune.author.as_deref(), Some("Frank Herbert"));
        assert_eq!(dune.books.first().map(String::as_str), Some("Dune"));
        assert_eq!(client.series(dune.id).unwrap().name, "Dune");

        // Every author and series in the catalogue can be found and opened.
        for (id, name) in CATALOGUE.authors() {
            let hits = client.search_authors(name, 1).unwrap().hits;
            let hit = hits.iter().find(|a| a.id.get() == id).expect(name);
            assert_eq!(client.author(hit.id).unwrap().name, name);
        }
        for (id, name) in CATALOGUE.series() {
            let hits = client.search_series(name, 1).unwrap().hits;
            let hit = hits.iter().find(|s| s.id.get() == id).expect(name);
            assert_eq!(client.series(hit.id).unwrap().name, name);
        }
        assert!(
            client
                .search_series("no such series", 1)
                .unwrap()
                .hits
                .is_empty()
        );
    }

    #[test]
    fn trending_resolves_every_id_for_every_period() {
        use crate::TrendingPeriod::*;
        let client = client();
        let mut firsts = Vec::new();
        for period in [Week, Month, ThreeMonths, Year, AllTime] {
            let books = client.trending_books(period).unwrap();
            assert_eq!(
                books.len(),
                crate::api::TRENDING_LIMIT.min(CATALOGUE.books.len())
            );
            firsts.push(books[0].id);
        }
        // Periods rank differently, so switching shows a change.
        assert_ne!(firsts[0], firsts[1]);
        assert_ne!(firsts[1], firsts[3]);
    }

    #[test]
    fn the_reader_has_lists_and_an_owned_list() {
        let client = client();
        let lists = client.lists().unwrap();
        assert!(lists.len() >= 3);
        let owned: Vec<_> = lists.iter().filter(|l| l.is_owned()).collect();
        assert_eq!(owned.len(), 1);
        let ranked = lists.iter().find(|l| l.ranked).expect("a ranked list");
        let books = client.list_books(ranked.id, 0).unwrap();
        assert_eq!(books.len() as u32, ranked.books_count);
        let positions: Vec<Option<i32>> = books.iter().map(|b| b.position).collect();
        assert_eq!(positions[..2], [Some(1), Some(2)]);
        assert!(client.list_books(ranked.id, 100).unwrap().is_empty());

        let all = client.list_memberships().unwrap();
        assert!(all.complete);
        let total: u32 = lists.iter().map(|l| l.books_count).sum();
        assert_eq!(all.rows.len() as u32, total);
        // Owned rows name the book's demo edition; other lists name none.
        for row in &all.rows {
            let on_owned = row.list_id == owned[0].id;
            assert_eq!(row.edition_id.is_some(), on_owned, "{row:?}");
        }
        let dune = client.book_list_memberships(BookId::new(312460)).unwrap();
        assert!(dune.iter().any(|r| r.list_id == owned[0].id));

        let err = client
            .add_to_list(ranked.id, BookId::new(312460), None)
            .unwrap_err();
        assert!(err.to_string().contains(READ_ONLY), "{err}");
        let err = client.remove_from_list(dune[0].id).unwrap_err();
        assert!(err.to_string().contains(READ_ONLY), "{err}");
    }

    #[test]
    fn upcoming_books_follow_the_library_series() {
        let catalogue = &*CATALOGUE;
        for u in &catalogue.upcoming {
            assert!(catalogue.book(u.id).is_none(), "{} is a catalogue id", u.id);
            assert_ne!(
                catalogue.series_id(&u.series),
                0,
                "{} has no series",
                u.title
            );
            assert!(u.days > 0, "{} is already out", u.title);
        }
        assert!(
            catalogue.upcoming.iter().any(|u| u.days <= 14),
            "Home shows one soon"
        );

        let client = client();
        let library = client.library(client.me().unwrap().id).unwrap();
        let series: Vec<crate::SeriesId> = library
            .iter()
            .flat_map(|e| e.series.iter().filter_map(|s| s.id))
            .collect();
        let today = today();
        let books = client.upcoming_in_series(&series, today).unwrap();
        assert_eq!(books.len(), catalogue.upcoming.len());
        assert!(
            books
                .windows(2)
                .all(|w| w[0].release_date <= w[1].release_date)
        );
        assert!(
            books
                .iter()
                .all(|b| b.release_date > today && b.book.cover.is_none())
        );
        assert!(books.iter().all(|b| !b.series.is_empty()));
        // Only the series asked for; none, no books.
        let first = books[0].series[0].id.unwrap();
        let one = client.upcoming_in_series(&[first], today).unwrap();
        assert!(one.iter().all(|b| b.series[0].id == Some(first)));
        assert!(one.len() < books.len());

        // Its page opens, with no description, and its series lists it.
        let details = client.book_details(books[0].book.id).unwrap();
        assert_eq!(details.release_date, Some(books[0].release_date));
        assert!(details.description.is_none());
        let page = client.series(first).unwrap();
        assert!(page.books.iter().any(|b| b.book.id == books[0].book.id));
    }

    #[test]
    fn the_series_overview_lists_catalogue_and_upcoming_books() {
        let catalogue = &*CATALOGUE;
        let client = client();
        let ids: Vec<crate::SeriesId> = catalogue
            .series()
            .into_iter()
            .map(|(id, _)| crate::SeriesId::new(id))
            .collect();
        let series = client.tracked_series(&ids).unwrap();
        assert_eq!(series.len(), ids.len());
        let named = |name: &str| series.iter().find(|s| s.name == name).unwrap();

        let dune: Vec<f32> = named("Dune").books.iter().map(|b| b.position).collect();
        assert_eq!(dune, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(named("Dune").author.as_ref().unwrap().name, "Frank Herbert");
        // A novella still to come, with its date.
        let mistborn = named("The Mistborn Saga: The Original Trilogy");
        let novella = mistborn.books.iter().find(|b| b.position == 3.5).unwrap();
        assert!(novella.release_date.unwrap() > today());
        // An upcoming book without a position is filtered, as the query does.
        assert!(
            named("Reidverse")
                .books
                .iter()
                .all(|b| b.book.title != "Untitled Reidverse Novel")
        );
        assert!(
            series
                .iter()
                .all(|s| { s.books.windows(2).all(|w| w[0].position <= w[1].position) })
        );
        // Only the series asked for.
        let one = client.tracked_series(&ids[..1]).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, ids[0]);
    }

    #[test]
    fn every_document_has_an_answer() {
        for (name, doc) in DOCUMENTS {
            let op = doc
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.split(['(', '{']).next())
                .unwrap();
            let body = answer(op, &json!({}));
            let unanswered = body["errors"][0]["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("The demo has no answer"));
            assert!(!unanswered, "{name} ({op}) has no demo answer");
        }
    }
}
