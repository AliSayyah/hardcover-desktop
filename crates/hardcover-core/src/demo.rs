//! A demo library for screenshots and for trying the app without an account.
//!
//! [`client`] answers from `demo/catalogue.json` through a fake transport, so
//! the rest of the client (queries, parsing, paging) runs exactly as it does
//! against Hardcover. The books are real, popular ones, with facts and cover
//! links from Hardcover and descriptions written for the demo; the reader and
//! their shelves are invented.
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
        "url": book.cover.url,
        "width": book.cover.width,
        "height": book.cover.height,
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
}
