//! A made-up library for screenshots and for trying the app without an account.
//!
//! [`client`] and [`cover_cache`] answer from `demo/catalogue.json` and
//! `demo/covers/` through a fake transport, so the rest of the client (queries,
//! parsing, paging) runs exactly as it does against Hardcover. Everything in
//! the catalogue is invented: the books, their authors and the reader.
//!
//! Reads work; every write fails with a "read-only" error. Only built with the
//! `demo` feature (and in tests).

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Client, TokenSource};
use crate::covers::CoverCache;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, Method, RequestBody};
use crate::ratelimit::TokenBucket;
use crate::{Error, Result};

/// The reply to every write.
pub const READ_ONLY: &str = "The demo library is read-only.";

const COVER_HOST: &str = "https://covers.demo.invalid/";
const COVER_WIDTH: u32 = 320;
const COVER_HEIGHT: u32 = 480;

/// A client for the demo library. It never touches the network.
pub fn client() -> Client {
    Client::with_transport(
        Arc::new(DemoToken),
        Arc::new(DemoTransport),
        // No rate limit to respect.
        TokenBucket::new(1_000, 1_000.0),
    )
}

/// A cover cache in `dir` that "downloads" the demo covers.
pub fn cover_cache(dir: impl Into<PathBuf>) -> Result<CoverCache> {
    CoverCache::with_transport(dir, Arc::new(DemoTransport))
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
    read: Option<Read>,
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

macro_rules! covers {
    ($($id:literal),* $(,)?) => {
        &[$(($id, include_bytes!(concat!("demo/covers/", stringify!($id), ".jpg")))),*]
    };
}

static COVERS: &[(i64, &[u8])] = covers![
    9001, 9002, 9003, 9004, 9005, 9006, 9007, 9008, 9009, 9010, 9011, 9012, 9013, 9014, 9015, 9016,
    9017, 9018, 9019, 9020, 9021, 9022, 9023, 9024,
];

impl Catalogue {
    fn book(&self, id: i64) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    fn entry(&self, id: i64) -> Option<&Entry> {
        self.library.iter().find(|e| e.id == id)
    }
}

// ----- wire shapes -----

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
        "url": format!("{COVER_HOST}{}.jpg", book.id),
        "width": COVER_WIDTH,
        "height": COVER_HEIGHT,
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
        "subtitle": null,
        "slug": slug(&book.title),
        "pages": book.pages,
        "release_year": release_year(book),
        "rating": book.rating,
        "users_count": book.users_count,
        "cached_image": image(book),
        "cached_contributors": [{ "author": { "name": book.author }, "contribution": null }],
    })
}

/// `UserBookFields`.
fn user_book(catalogue: &Catalogue, entry: &Entry) -> Option<Value> {
    let book = catalogue.book(entry.book)?;
    let reads: Vec<Value> = entry
        .read
        .iter()
        .map(|read| {
            json!({
                "id": read.id,
                "started_at": read.started_at,
                "finished_at": read.finished_at,
                "progress": read.progress_pages.map(|p| p as f64 * 100.0 / book.pages as f64),
                "progress_pages": read.progress_pages,
                "progress_seconds": null,
                "edition_id": null,
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
        "book": book_summary(book),
        "edition": null,
        "user_book_reads": reads,
    }))
}

fn book_details(book: &Book) -> Value {
    let mut details = book_summary(book);
    let genres: Vec<Value> = book
        .genres
        .iter()
        .map(|g| json!({ "tag": g, "count": 1 }))
        .collect();
    let series: Vec<Value> = book
        .series
        .iter()
        .map(|s| {
            json!({
                "position": s.position,
                "featured": true,
                "series": { "name": s.name, "slug": slug(&s.name) },
            })
        })
        .collect();
    let extra = json!({
        "description": book.description,
        "release_date": book.release_date,
        "editions_count": book.editions_count,
        "ratings_count": book.ratings_count,
        "cached_tags": { "Genre": genres },
        "book_series": series,
    });
    if let (Some(map), Value::Object(extra)) = (details.as_object_mut(), extra) {
        map.extend(extra);
    }
    details
}

fn search_document(book: &Book) -> Value {
    json!({
        "id": book.id,
        "title": book.title,
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

struct DemoTransport;

impl HttpTransport for DemoTransport {
    fn send(&self, req: HttpRequest) -> Result<HttpResponse> {
        match req.method {
            Method::Get => Ok(cover(&req.url)),
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

fn cover(url: &str) -> HttpResponse {
    let bytes = url
        .strip_prefix(COVER_HOST)
        .and_then(|name| name.strip_suffix(".jpg"))
        .and_then(|id| id.parse::<i64>().ok())
        .and_then(|id| COVERS.iter().find(|(cover, _)| *cover == id))
        .map(|(_, bytes)| *bytes);
    match bytes {
        Some(bytes) => HttpResponse {
            status: 200,
            headers: vec![("content-type".into(), "image/jpeg".into())],
            body: bytes.to_vec(),
        },
        None => HttpResponse {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        },
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
        "BookDetails" => json!({
            "books_by_pk": catalogue.book(int("id")).map(book_details),
        }),
        "ProgressContext" => {
            let found = catalogue.entry(int("id")).and_then(|e| {
                let book = catalogue.book(e.book)?;
                Some(json!({
                    "id": e.id,
                    "status_id": e.status,
                    "book": { "pages": book.pages, "audio_seconds": null },
                    "edition": null,
                    "user_book_reads": e.read.iter().map(|r| json!({ "id": r.id, "edition": null }))
                        .collect::<Vec<_>>(),
                }))
            });
            json!({ "user_books": found.into_iter().collect::<Vec<_>>() })
        }
        "InsertUserBook" | "UpdateUserBook" | "DeleteUserBook" | "InsertUserBookRead"
        | "UpdateUserBookRead" => return error(READ_ONLY),
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
    let matches: Vec<&Book> = catalogue
        .books
        .iter()
        .filter(|b| {
            let haystack = format!("{} {}", b.title, b.author).to_lowercase();
            words.iter().all(|w| haystack.contains(w))
        })
        .collect();
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
        for status in [
            ReadingStatus::CurrentlyReading,
            ReadingStatus::WantToRead,
            ReadingStatus::Read,
            ReadingStatus::Paused,
            ReadingStatus::DidNotFinish,
        ] {
            assert!(
                library.iter().any(|e| e.status == status),
                "nothing on {status:?}"
            );
        }
        let reading = library
            .iter()
            .find(|e| e.status == ReadingStatus::CurrentlyReading)
            .unwrap();
        assert!(
            reading
                .current_read
                .as_ref()
                .unwrap()
                .progress_pages
                .is_some()
        );
        assert!(reading.book.cover.is_some());
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
    fn every_book_has_details_and_a_cover() {
        let client = client();
        let dir = tempfile::tempdir().unwrap();
        let covers = cover_cache(dir.path()).unwrap();
        for book in &CATALOGUE.books {
            let details = client.book_details(BookId::new(book.id)).unwrap();
            assert_eq!(details.summary.title, book.title);
            assert!(details.description.is_some());
            let cover = details.summary.cover.expect("a cover");
            assert!(covers.fetch(&cover).is_ok(), "no cover for {}", book.id);
        }
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
                .library_entry_for_book(me.id, BookId::new(9023))
                .unwrap()
                .is_none(),
            "9023 is only in search"
        );
    }

    #[test]
    fn search_matches_titles_and_authors() {
        let client = client();
        let wren = client.search_books("tobias wren", 1).unwrap();
        assert_eq!(wren.books.len(), 3);
        let salt = client.search_books("Salt", 1).unwrap();
        assert_eq!(salt.books.len(), 2);
        assert!(client.search_books("zzz", 1).unwrap().books.is_empty());
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

    #[test]
    fn every_book_has_a_cover_file() {
        for book in &CATALOGUE.books {
            assert!(
                COVERS.iter().any(|(id, _)| *id == book.id),
                "no cover for {}",
                book.id
            );
        }
    }
}
