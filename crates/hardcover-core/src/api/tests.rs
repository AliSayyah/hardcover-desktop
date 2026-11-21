//! Client tests against a scripted transport. All fixtures are synthetic: made-up
//! books and users that mirror the shapes the live API returns.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use super::*;
use crate::http::fake::FakeTransport;

/// Hands out "tok-0", then "tok-1" after each refresh.
struct TestTokens {
    current: Mutex<usize>,
    refreshes: AtomicUsize,
    can_refresh: bool,
}

impl TestTokens {
    fn new(can_refresh: bool) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(0),
            refreshes: AtomicUsize::new(0),
            can_refresh,
        })
    }
}

impl TokenSource for TestTokens {
    fn access_token(&self) -> Result<String> {
        Ok(format!("tok-{}", self.current.lock().unwrap()))
    }
    fn on_unauthorized(&self, rejected: &str) -> Result<bool> {
        let mut cur = self.current.lock().unwrap();
        if rejected != format!("tok-{cur}") {
            return Ok(true);
        }
        if !self.can_refresh {
            return Ok(false);
        }
        *cur += 1;
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

fn client(tokens: Arc<TestTokens>) -> (Client, Arc<FakeTransport>) {
    let http = Arc::new(FakeTransport::new());
    // A large bucket so tests never sleep.
    let c = Client::with_transport(tokens, http.clone(), TokenBucket::new(1000, 1000.0));
    (c, http)
}

fn book(id: i64, title: &str) -> Value {
    json!({
        "id": id,
        "title": title,
        "subtitle": null,
        "slug": format!("book-{id}"),
        "pages": 320,
        "release_year": 1999,
        "rating": 3.91234,
        "users_count": 1234,
        "cached_image": {"id": 1, "url": format!("https://assets.hardcover.app/edition/{id}/cover.jpg"), "color": "#000000", "width": 400, "height": 600, "color_name": "Black"},
        "cached_contributors": [
            {"author": {"id": 9, "slug": "jane-quill", "name": "Jane Quill", "image": {}}, "contribution": null, "primary": true},
            {"author": {"id": 10, "slug": "lou-lingua", "name": "Lou Lingua", "image": {}}, "contribution": "Translator"}
        ]
    })
}

fn user_book(id: i64, status_id: i64) -> Value {
    json!({
        "id": id,
        "user_id": 77,
        "status_id": status_id,
        "rating": 4.5,
        "date_added": "2024-01-02",
        "updated_at": "2024-01-03T04:05:06.123456+00:00",
        "book": book(1000 + id, &format!("Synthetic Book {id}")),
        "edition": {"id": 5000 + id, "title": "Synthetic Edition", "pages": 400, "audio_seconds": null, "edition_format": "Paperback", "cached_image": {}},
        "user_book_reads": [
            {"id": 9000 + id, "started_at": "2024-02-01", "finished_at": null, "progress": 25.0, "progress_pages": 100, "progress_seconds": null, "edition_id": 5000 + id}
        ]
    })
}

fn data(v: Value) -> String {
    Value::Object(serde_json::Map::from_iter([("data".into(), v)])).to_string()
}

#[test]
fn me_parses_and_sends_bearer_token() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({"me": [{"id": 77, "username": "reader", "name": "A. Reader", "image": {"url": "https://assets.hardcover.app/u.png", "width": 50, "height": 50}}]})));
    let me = c.me().unwrap();
    assert_eq!(me.id, UserId::new(77));
    assert_eq!(me.username, "reader");
    assert_eq!(me.name.as_deref(), Some("A. Reader"));
    assert_eq!(me.avatar.unwrap().width, Some(50));
    let req = &http.requests()[0];
    assert_eq!(req.url, GRAPHQL_ENDPOINT);
    assert_eq!(req.header("authorization"), Some("Bearer tok-0"));
    assert_eq!(req.json.as_ref().unwrap()["operationName"], "Me");
}

#[test]
fn library_paginates_by_keyset() {
    let (c, http) = client(TestTokens::new(false));
    let full = i64::from(LIBRARY_PAGE_SIZE);
    let mut first: Vec<Value> = (1..=full).map(|i| user_book(i, 1 + i % 5)).collect();
    // One book is in two series, one of them listed twice; the others list none.
    first[1]["book"]["book_series"] = json!([
        {"position": 2.0, "featured": true, "series": {"id": 31, "name": "Synthetic Saga", "slug": "synthetic-saga"}},
        {"position": null, "featured": false, "compilation": true, "series": {"id": 32, "name": "Omnibus", "slug": null}},
        {"position": 7.0, "featured": false, "series": {"id": 31, "name": "Synthetic Saga", "slug": "synthetic-saga"}}
    ]);
    // Second page holds one shelf entry, one with a status id we don't know
    // and one whose book didn't resolve: those two are dropped, not the page.
    let mut no_book = user_book(full + 52, 1);
    no_book["book"] = Value::Null;
    let second = vec![user_book(full + 50, 3), user_book(full + 51, 42), no_book];
    http.push(200, &data(json!({ "user_books": first })));
    http.push(200, &data(json!({ "user_books": second })));
    http.push(200, &data(json!({ "user_books": [] })));
    let mut pages = Vec::new();
    let sync = c
        .library_with_progress(UserId::new(77), |p| pages.push(p.len()))
        .unwrap();
    assert_eq!(
        sync.coverage,
        Coverage::Complete {
            unreadable: vec![UserBookId::new(full + 51), UserBookId::new(full + 52)]
        }
    );
    let lib = sync.entries;
    assert_eq!(lib.len(), LIBRARY_PAGE_SIZE as usize + 1);
    assert_eq!(pages, vec![LIBRARY_PAGE_SIZE as usize, 1]);
    let reqs = http.requests();
    assert_eq!(reqs.len(), 3, "only an empty page ends it");
    let v0 = &reqs[0].json.as_ref().unwrap()["variables"];
    let v1 = &reqs[1].json.as_ref().unwrap()["variables"];
    assert_eq!(
        (
            v0["userId"].as_i64(),
            v0["afterId"].as_i64(),
            v0["limit"].as_i64()
        ),
        (Some(77), Some(0), Some(full))
    );
    assert_eq!(v1["afterId"].as_i64(), Some(full));

    let e = &lib[0];
    assert_eq!(e.user_book_id, UserBookId::new(1));
    assert_eq!(e.user_id, UserId::new(77));
    assert_eq!(e.status, ReadingStatus::CurrentlyReading);
    assert_eq!(e.rating.unwrap().value(), 4.5);
    assert_eq!(e.book.authors, vec!["Jane Quill"]);
    assert_eq!(
        e.book.average_rating.map(|r| (r * 100.0).round()),
        Some(391.0)
    );
    assert!(e.edition.as_ref().unwrap().cover.is_none());
    assert_eq!(
        e.cover().unwrap().url,
        "https://assets.hardcover.app/edition/1001/cover.jpg"
    );
    assert_eq!(e.total_pages(), Some(400));
    let read = e.current_read().unwrap();
    assert_eq!(read.progress_pages, Some(100));
    assert_eq!(read.progress_percent, Some(25.0));
    assert_eq!(read.started_at.unwrap().to_string(), "2024-02-01");
    assert_eq!(e.date_added.unwrap().to_string(), "2024-01-02");
    assert!(e.updated_at.is_some());
    assert!(e.series.is_empty(), "a book without book_series has none");

    let series = &lib[1].series;
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].name, "Synthetic Saga");
    assert_eq!(series[0].id, Some(SeriesId::new(31)));
    assert_eq!(series[0].position, Some(2.0));
    assert!(series[0].featured);
    assert_eq!(
        (series[1].name.as_str(), series[1].position),
        ("Omnibus", None)
    );
    assert!(!series[0].compilation);
    assert!(series[1].compilation);
}

#[test]
fn library_stops_on_empty_first_page() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({ "user_books": [] })));
    let sync = c.library(UserId::new(1)).unwrap();
    assert_eq!(sync.entries, []);
    assert_eq!(sync.coverage, Coverage::Complete { unreadable: vec![] });
    assert_eq!(http.requests().len(), 1);
}

/// D9: a page shorter than asked for is not the end (Hardcover caps some
/// tables at the page sizes used); only an empty page is.
#[test]
fn a_short_library_page_is_not_taken_as_the_last() {
    let (c, http) = client(TestTokens::new(false));
    // A server cap of 2 rows a page, 5 entries in all.
    http.push(
        200,
        &data(json!({ "user_books": [user_book(1, 1), user_book(2, 1)] })),
    );
    http.push(
        200,
        &data(json!({ "user_books": [user_book(3, 1), user_book(4, 1)] })),
    );
    http.push(200, &data(json!({ "user_books": [user_book(9, 1)] })));
    http.push(200, &data(json!({ "user_books": [] })));
    let sync = c.library(UserId::new(1)).unwrap();
    let ids: Vec<i64> = sync.entries.iter().map(|e| e.user_book_id.get()).collect();
    assert_eq!(ids, [1, 2, 3, 4, 9]);
    assert!(matches!(sync.coverage, Coverage::Complete { .. }));
    let afters: Vec<i64> = http
        .requests()
        .iter()
        .map(|r| {
            r.json.as_ref().unwrap()["variables"]["afterId"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(afters, [0, 2, 4, 9]);
}

/// D9: paging that can't move forward stops, and the sync says it's
/// partial, so nothing may be deleted on its strength.
#[test]
fn a_library_that_stops_paging_forward_is_partial() {
    let (c, http) = client(TestTokens::new(false));
    // The server ignores the cursor and sends the same rows again.
    http.push(200, &data(json!({ "user_books": [user_book(1, 1)] })));
    http.push(200, &data(json!({ "user_books": [user_book(1, 1)] })));
    let sync = c.library(UserId::new(1)).unwrap();
    assert_eq!(sync.coverage, Coverage::Partial);
    assert_eq!(http.remaining(), 0);
}

#[test]
fn library_entry_is_scoped_to_me() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"me": [{"id": 77, "username": "reader", "name": null, "image": null}]})),
    );
    http.push(200, &data(json!({ "user_books": [user_book(5, 6)] })));
    http.push(200, &data(json!({ "user_books": [] })));
    let e = c.library_entry(UserBookId::new(5)).unwrap().unwrap();
    assert_eq!(e.status, ReadingStatus::Ignored);
    assert!(c.library_entry(UserBookId::new(6)).unwrap().is_none());
    let reqs = http.requests();
    // `me` is fetched once and remembered.
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[1].json.as_ref().unwrap()["variables"]["userId"], 77);
}

#[test]
fn search_parses_typesense_hits() {
    let (c, http) = client(TestTokens::new(false));
    let results = json!({
        "facet_counts": [], "found": 61, "out_of": 1000, "page": 2, "search_time_ms": 3,
        "hits": [
            {"document": {"id": "42", "title": "Sand and Stars", "subtitle": "A Novel", "author_names": ["Jane Quill", "Jane Quill"],
                          "image": {"url": "https://assets.hardcover.app/x.jpg", "width": 10, "height": 15},
                          "pages": 500, "release_year": 1970, "slug": "sand-and-stars", "rating": 4.2, "users_count": 99},
             "highlight": {}, "text_match": 1},
            {"document": {"id": 43, "title": "Second Hit", "contributions": [{"author": {"name": "Bo Writer"}, "contribution": null}], "image": {}}},
            {"document": {"title": "No id, skipped"}},
            {"document": {"id": "44"}},
            {"nodocument": true}
        ]
    });
    http.push(
        200,
        &data(json!({"search": {"error": null, "page": 2, "per_page": 25, "results": results}})),
    );
    let page = c.search_books("  sand  ", 2).unwrap();
    assert_eq!(page.page, 2);
    assert_eq!(page.per_page, 25);
    assert_eq!(page.total, Some(61));
    assert!(page.has_more());
    assert_eq!(page.books.len(), 2);
    let b = &page.books[0];
    assert_eq!(b.id, BookId::new(42));
    assert_eq!(b.authors, vec!["Jane Quill"]);
    assert_eq!(b.cover.as_ref().unwrap().height, Some(15));
    assert_eq!(b.release_year, Some(1970));
    assert_eq!(page.books[1].authors, vec!["Bo Writer"]);
    assert!(page.books[1].cover.is_none());
    let reqs0 = http.requests();
    let vars = &reqs0[0].json.as_ref().unwrap()["variables"];
    assert_eq!(vars["query"], "sand");
    assert_eq!(vars["page"], 2);
}

#[test]
fn search_blank_query_sends_nothing_and_error_field_maps() {
    let (c, http) = client(TestTokens::new(false));
    assert_eq!(c.search_books("   ", 1).unwrap().books, []);
    assert_eq!(http.requests().len(), 0);
    http.push(200, &data(json!({"search": {"error": "Search timed out", "page": 1, "per_page": 25, "results": null}})));
    assert_eq!(
        c.search_books("x", 1),
        Err(Error::Api("Search timed out".into()))
    );
    http.push(
        200,
        &data(json!({"search": {"error": null, "results": {"hits": "weird"}}})),
    );
    let p = c.search_books("x", 3).unwrap();
    assert_eq!(p.books, []);
    assert_eq!((p.page, p.total), (3, None));
}

#[test]
fn author_search_parses_documented_fields() {
    let (c, http) = client(TestTokens::new(false));
    // Fields as the search guide lists them for authors; the envelope and `id`
    // are as for books.
    let results = json!({
        "found": 3, "page": 1,
        "hits": [
            {"document": {"id": "901", "name": "Jane Quill", "name_personal": "Quill, Jane",
                          "alternate_names": ["J. Quill"], "slug": "jane-quill", "books_count": 12,
                          "books": ["Sand and Stars", "Sand and Stars", "The Long Tide"],
                          "series_names": ["Sand"],
                          "image": {"url": "https://assets.hardcover.app/a.jpg", "width": 400, "height": 600}}},
            {"document": {"name": "No Id Here", "books": [{"title": "Object Title"}], "image": {}}},
            {"document": {"id": 903}}
        ]
    });
    http.push(
        200,
        &data(json!({"search": {"error": null, "page": 1, "per_page": 25,
                                 "ids": [901, 902, 903], "results": results}})),
    );
    let page = c.search_authors(" quill ", 1).unwrap();
    assert_eq!(page.total, Some(3));
    assert!(!page.has_more());
    assert_eq!(page.hits.len(), 2, "a hit without a name is skipped");
    let a = &page.hits[0];
    assert_eq!(a.id, AuthorId::new(901));
    assert_eq!(a.name, "Jane Quill");
    assert_eq!(a.slug.as_deref(), Some("jane-quill"));
    assert_eq!(a.books_count, Some(12));
    assert_eq!(a.top_books, vec!["Sand and Stars", "The Long Tide"]);
    assert_eq!(a.photo.as_ref().unwrap().width, Some(400));
    // No `id` in the document: `ids` lines up with the hits, so it stands in.
    assert_eq!(page.hits[1].id, AuthorId::new(902));
    assert_eq!(page.hits[1].top_books, vec!["Object Title"]);
    assert!(page.hits[1].photo.is_none());

    let req = &http.requests()[0];
    let body = req.json.as_ref().unwrap();
    assert_eq!(body["operationName"], "SearchAuthors");
    assert!(
        body["query"]
            .as_str()
            .unwrap()
            .contains("query_type: \"author\"")
    );
    assert_eq!(body["variables"]["query"], "quill");
}

#[test]
fn search_ids_stand_in_only_when_they_line_up() {
    let (c, http) = client(TestTokens::new(false));
    let results = json!({"hits": [
        {"document": {"name": "First"}},
        {"document": {"id": 5, "name": "Second"}}
    ]});
    http.push(
        200,
        &data(json!({"search": {"ids": [1], "results": results}})),
    );
    let page = c.search_authors("x", 1).unwrap();
    let ids: Vec<i64> = page.hits.iter().map(|a| a.id.get()).collect();
    assert_eq!(ids, vec![5], "ids shorter than the hits are not trusted");
}

#[test]
fn series_search_parses_documented_fields() {
    let (c, http) = client(TestTokens::new(false));
    let results = json!({
        "found": 60, "page": 2,
        "hits": [
            {"document": {"id": "77", "name": "The Sand Cycle", "slug": "the-sand-cycle",
                          "author_name": "Jane Quill", "books_count": 9, "primary_books_count": 6,
                          "readers_count": 1234, "books": ["Sand and Stars", "Sand and Sea"]}},
            {"document": {"id": 78, "name": "Tides", "author": {"id": 5, "name": "Bo Writer"},
                          "books": [{"title": "Low Tide", "position": 1}, {"title": "High Tide"}]}},
            {"document": {"id": 79, "name": "  "}}
        ]
    });
    http.push(
        200,
        &data(json!({"search": {"error": null, "page": 2, "per_page": 25, "results": results}})),
    );
    let page = c.search_series("sand", 2).unwrap();
    assert!(page.has_more());
    assert_eq!(page.hits.len(), 2);
    let s = &page.hits[0];
    assert_eq!(s.id, SeriesId::new(77));
    assert_eq!(s.author.as_deref(), Some("Jane Quill"));
    assert_eq!((s.books_count, s.primary_books_count), (Some(9), Some(6)));
    assert_eq!(s.books, vec!["Sand and Stars", "Sand and Sea"]);
    let t = &page.hits[1];
    assert_eq!(t.author.as_deref(), Some("Bo Writer"));
    assert_eq!(t.books, vec!["Low Tide", "High Tide"]);
    assert_eq!(t.books_count, None);
    let body = http.requests()[0].json.clone().unwrap();
    assert!(
        body["query"]
            .as_str()
            .unwrap()
            .contains("query_type: \"series\"")
    );

    assert_eq!(c.search_series("  ", 1).unwrap().hits, []);
    assert_eq!(http.requests().len(), 1, "a blank query sends nothing");
    http.push(200, &data(json!({"search": {"error": "Search timed out"}})));
    assert_eq!(
        c.search_series("x", 1),
        Err(Error::Api("Search timed out".into()))
    );
}

#[test]
fn trending_fetches_ids_then_books_in_rank_order() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"books_trending": {"error": null, "ids": [3, null, 1, 3, 2]}})),
    );
    // Hardcover returns the books in any order; one id is unknown.
    http.push(
        200,
        &data(json!({ "books": [book(1, "One"), book(3, "Three")] })),
    );
    let books = c.trending_books(TrendingPeriod::Year).unwrap();
    let titles: Vec<&str> = books.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(titles, vec!["Three", "One"]);
    let reqs = http.requests();
    assert_eq!(reqs.len(), 2);
    let first = reqs[0].json.as_ref().unwrap();
    assert_eq!(first["operationName"], "BooksTrending");
    assert_eq!(first["variables"]["duration"], "one_year");
    assert_eq!(first["variables"]["limit"], TRENDING_LIMIT);
    assert_eq!(
        reqs[1].json.as_ref().unwrap()["variables"]["ids"],
        json!([3, 1, 2])
    );
}

#[test]
fn trending_errors_and_empty_results() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"books_trending": {"error": "Something broke", "ids": null}})),
    );
    assert_eq!(
        c.trending_books(TrendingPeriod::Week),
        Err(Error::Api("Something broke".into()))
    );
    http.push(
        200,
        &data(json!({"books_trending": {"error": null, "ids": null}})),
    );
    assert_eq!(c.trending_books(TrendingPeriod::Week).unwrap(), []);
    assert_eq!(http.requests().len(), 2, "no ids, no books request");
    http.push(200, &data(json!({"books_trending": null})));
    assert!(matches!(
        c.trending_books(TrendingPeriod::Week),
        Err(Error::Decode(_))
    ));
}

#[test]
fn book_details_parse() {
    let (c, http) = client(TestTokens::new(false));
    let mut b = book(7, "Detailed");
    let extra = json!({
        "description": "  A synthetic description.  ",
        "release_date": "1999-05-06",
        "editions_count": 12,
        "ratings_count": 34,
        "cached_tags": {
            "Genre": [{"tag": "Science Fiction", "count": 5}, {"tag": "Adventure", "count": 2}],
            "Mood": [{"tag": "tense", "count": 3}],
            "Tag": [],
            "Content Warning": [{"tag": "Violence", "count": 1}]
        },
        "book_series": [
            {"position": 1.0, "featured": true, "series": {"id": 31, "name": "Synthetic Saga", "slug": "synthetic-saga"}},
            {"position": null, "featured": false, "series": {"name": "Synthetic Saga", "slug": "dup"}},
            {"position": 2.5, "featured": false, "series": {"name": "Omnibus Order", "slug": null}},
            {"position": 1.0, "featured": false, "series": null}
        ]
    });
    b.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    http.push(200, &data(json!({ "books_by_pk": b })));
    let d = c.book_details(BookId::new(7)).unwrap();
    assert_eq!(d.summary.title, "Detailed");
    assert_eq!(d.description.as_deref(), Some("A synthetic description."));
    assert_eq!(d.release_date.unwrap().to_string(), "1999-05-06");
    assert_eq!(d.genres, vec!["Science Fiction", "Adventure"]);
    assert_eq!(d.moods, vec!["tense"]);
    assert_eq!(d.tags, [] as [String; 0]);
    assert_eq!(d.content_warnings, vec!["Violence"]);
    assert_eq!(d.series.len(), 2);
    assert!(d.series[0].featured);
    assert_eq!(d.series[1].position, Some(2.5));
    assert_eq!((d.editions_count, d.ratings_count), (Some(12), Some(34)));
    assert_eq!(d.series[0].id, Some(SeriesId::new(31)));
    assert_eq!(d.series[1].id, None);
    // The fixture's cached_contributors: an author and a translator, with ids.
    assert_eq!(d.contributors.len(), 2);
    assert_eq!(d.contributors[0].author_id, Some(AuthorId::new(9)));
    assert!(d.contributors[0].is_author());
    assert_eq!(d.contributors[1].role.as_deref(), Some("Translator"));
    // Not in the fixture: empty, not an error.
    assert_eq!(d.ratings_distribution, []);
    assert_eq!(d.alternative_titles, [] as [String; 0]);
    assert_eq!(d.similar_book_ids, []);

    http.push(200, &data(json!({ "books_by_pk": null })));
    assert!(matches!(c.book_details(BookId::new(8)), Err(Error::Api(_))));
}

#[test]
fn book_details_parse_catalogue_blobs() {
    let (c, http) = client(TestTokens::new(false));
    let mut b = book(7, "Detailed");
    let extra = json!({
        "ratings_distribution": [{"rating": 5, "count": 9}, {"rating": 4.5, "count": 3}],
        "alternative_titles": ["Detailliert", "Detailed"],
        "cached_similar_book_ids": [11, 12, 11],
        "cached_tags": {},
        "book_series": []
    });
    b.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    http.push(200, &data(json!({ "books_by_pk": b })));
    let d = c.book_details(BookId::new(7)).unwrap();
    assert_eq!(d.ratings_distribution.len(), 2);
    assert_eq!(d.ratings_distribution[0].rating, 5.0);
    assert_eq!(d.alternative_titles, vec!["Detailliert"]);
    assert_eq!(d.similar_book_ids, vec![BookId::new(11), BookId::new(12)]);
}

#[test]
fn series_page_orders_and_filters_books() {
    let (c, http) = client(TestTokens::new(false));
    let with = |mut b: Value, users: i64, partial: Option<bool>| {
        b["users_count"] = json!(users);
        b["is_partial_book"] = json!(partial);
        b
    };
    http.push(
        200,
        &data(json!({ "series_by_pk": {
            "id": 31,
            "name": "Synthetic Saga",
            "slug": "synthetic-saga",
            "description": null,
            "primary_books_count": 3,
            "is_completed": true,
            "author": {"id": 9, "name": "Jane Quill"},
            "book_series": [
                {"position": 1.0, "compilation": false, "book": with(book(101, "First"), 500, Some(false))},
                {"position": 1.0, "compilation": false, "book": with(book(102, "First, translated"), 20, None)},
                {"position": 1.5, "compilation": false, "book": with(book(103, "Interlude"), 40, Some(false))},
                {"position": 2.0, "compilation": false, "book": with(book(104, "Second"), 300, Some(false))},
                {"position": 1.0, "compilation": true, "book": with(book(105, "Omnibus"), 900, Some(false))},
                {"position": 3.0, "compilation": false, "book": with(book(106, "Part of Third"), 100, Some(true))},
                {"position": 3.0, "compilation": false, "book": with(book(107, "Third"), 200, Some(false))},
                {"position": null, "compilation": false, "book": with(book(108, "Companion"), 10, None)},
                {"position": 4.0, "compilation": false, "book": null}
            ]
        }})),
    );
    let s = c.series(SeriesId::new(31)).unwrap();
    let req = &http.requests()[0];
    assert_eq!(req.json.as_ref().unwrap()["operationName"], "SeriesPage");
    assert_eq!(req.json.as_ref().unwrap()["variables"]["id"], 31);
    assert_eq!(s.name, "Synthetic Saga");
    assert_eq!(s.author.as_ref().unwrap().id, AuthorId::new(9));
    assert_eq!(s.is_completed, Some(true));
    assert_eq!(s.primary_books_count, Some(3));
    assert_eq!(
        s.web_url().as_deref(),
        Some("https://hardcover.app/series/synthetic-saga")
    );
    let ids: Vec<i64> = s.books.iter().map(|b| b.book.id.get()).collect();
    assert_eq!(ids, vec![101, 103, 104, 107, 108]);
    assert_eq!(s.books.iter().filter(|b| b.is_main()).count(), 3);
    assert_eq!(s.books[1].position, Some(1.5));
    assert_eq!(s.books[4].position, None);

    http.push(200, &data(json!({ "series_by_pk": null })));
    assert!(matches!(c.series(SeriesId::new(32)), Err(Error::Api(_))));
}

#[test]
fn author_page_merges_roles_and_drops_partial_books() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({ "authors_by_pk": {
            "id": 9,
            "name": "Jane Quill",
            "slug": "jane-quill",
            "bio": "  A made-up writer.  ",
            "cached_image": {"url": "https://assets.hardcover.app/authors/9.jpg", "width": 200, "height": 300},
            "born_year": 1950,
            "death_year": null,
            "books_count": 4,
            "contributions": [
                {"contribution": null, "book": book(201, "Own Novel")},
                {"contribution": "Translator", "book": book(202, "Translated Novel")},
                {"contribution": "Illustrator", "book": book(203, "Picture Book")},
                {"contribution": "Author", "book": book(203, "Picture Book")},
                {"contribution": null, "book": {"id": 204, "title": "Excerpt", "is_partial_book": true}},
                {"contribution": null, "book": null}
            ]
        }})),
    );
    let a = c.author(AuthorId::new(9)).unwrap();
    let req = &http.requests()[0];
    assert_eq!(req.json.as_ref().unwrap()["operationName"], "AuthorPage");
    assert_eq!(a.name, "Jane Quill");
    assert_eq!(a.bio.as_deref(), Some("A made-up writer."));
    assert_eq!(a.photo.as_ref().unwrap().width, Some(200));
    assert_eq!((a.born_year, a.death_year), (Some(1950), None));
    assert_eq!(a.books_count, Some(4));
    let books: Vec<(i64, Option<&str>)> = a
        .books
        .iter()
        .map(|b| (b.book.id.get(), b.role.as_deref()))
        .collect();
    assert_eq!(
        books,
        vec![(201, None), (202, Some("Translator")), (203, None)]
    );

    http.push(200, &data(json!({ "authors_by_pk": null })));
    assert!(matches!(c.author(AuthorId::new(10)), Err(Error::Api(_))));
}

#[test]
fn books_by_ids_keeps_the_given_order() {
    let (c, http) = client(TestTokens::new(false));
    assert_eq!(c.books_by_ids(&[]).unwrap(), []);
    assert_eq!(http.requests().len(), 0, "no ids, no request");

    http.push(
        200,
        &data(json!({ "books": [book(3, "Three"), book(1, "One")] })),
    );
    let ids: Vec<BookId> = (1..=40).map(BookId::new).collect();
    let books = c.books_by_ids(&ids).unwrap();
    let titles: Vec<&str> = books.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(titles, vec!["One", "Three"]);
    let vars = http.requests()[0].json.clone().unwrap()["variables"].clone();
    assert_eq!(vars["ids"].as_array().unwrap().len(), BOOKS_BY_IDS_LIMIT);
    assert_eq!(vars["limit"], BOOKS_BY_IDS_LIMIT);
}

#[test]
fn unauthorized_refreshes_once_and_retries() {
    let tokens = TestTokens::new(true);
    let (c, http) = client(tokens.clone());
    http.push(
        401,
        r#"{"error":"invalid_token","error_description":"expired"}"#,
    );
    http.push(200, &data(json!({"me": [{"id": 1, "username": "u"}]})));
    assert_eq!(c.me().unwrap().id, UserId::new(1));
    let reqs = http.requests();
    assert_eq!(reqs[0].header("authorization"), Some("Bearer tok-0"));
    assert_eq!(reqs[1].header("authorization"), Some("Bearer tok-1"));
    assert_eq!(tokens.refreshes.load(Ordering::SeqCst), 1);

    // A second 401 after the retry is final.
    http.push(401, r#"{"error":"invalid_token"}"#);
    http.push(401, r#"{"error":"invalid_token"}"#);
    assert_eq!(c.me(), Err(Error::Unauthorized));
    assert_eq!(http.remaining(), 0);
}

#[test]
fn unauthorized_without_refresh_fails_immediately() {
    let (c, http) = client(TestTokens::new(false));
    http.push(401, r#"{"error":"invalid_token"}"#);
    assert_eq!(c.me(), Err(Error::Unauthorized));
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn http_errors_map_to_typed_errors() {
    let (c, http) = client(TestTokens::new(false));
    let run = |status: u16, headers: &[(&str, &str)], body: &str| {
        http.push_with_headers(status, headers, body);
        c.me().unwrap_err()
    };
    assert_eq!(
        run(
            403,
            &[],
            r#"{"error":"insufficient_scope","error_description":"missing","scope":"read:library"}"#
        ),
        Error::InsufficientScope {
            scope: Some("read:library".into())
        }
    );
    assert_eq!(
        run(
            403,
            &[],
            r#"{"errors":["request_exceeds_capacity"],"message":"11 fields > burst 10"}"#
        ),
        Error::Api("request_exceeds_capacity: 11 fields > burst 10".into())
    );
    assert_eq!(
        run(
            403,
            &[],
            r#"{"error":"ilike and related operations are not permitted on this server."}"#
        ),
        Error::Api("ilike and related operations are not permitted on this server.".into())
    );
    assert_eq!(
        run(408, &[], r#"{"error":"Request timeout"}"#),
        Error::Timeout
    );
    assert!(
        matches!(run(503, &[], r#"{"error":"Service temporarily unavailable"}"#), Error::Network(m) if m.contains("503"))
    );
    assert_eq!(
        run(
            400,
            &[],
            r#"{"error":"invalid_query","error_description":"bad"}"#
        ),
        Error::Api("HTTP 400: invalid_query: bad".into())
    );
    assert_eq!(
        run(
            200,
            &[],
            r#"{"errors":[{"message":"field 'x' not found in type: 'users'","extensions":{"code":"validation-failed"}}]}"#
        ),
        Error::Api("field 'x' not found in type: 'users'".into())
    );
    assert!(matches!(run(200, &[], "<html>"), Error::Decode(_)));
    assert!(matches!(
        run(200, &[], r#"{"data":{"me":"nope"}}"#),
        Error::Decode(_)
    ));
    http.push_err(Error::Timeout);
    assert_eq!(c.me(), Err(Error::Timeout));
    // Last: the daily limit's block is remembered, so it refuses what follows.
    assert_eq!(
        run(
            429,
            &[("Retry-After", "86400")],
            r#"{"error":"Too Many Requests","message":"slow down"}"#
        ),
        Error::RateLimited {
            retry_after: Some(Duration::from_hours(24))
        }
    );
}

#[test]
fn rate_limited_response_waits_and_retries_once() {
    let tokens = TestTokens::new(false);
    let http = Arc::new(FakeTransport::new());
    let c = Client::with_transport(tokens, http.clone(), TokenBucket::new(5, 50.0));
    // Short Retry-After: wait it out (bucket emptied, ~20ms for a token) and retry.
    http.push_with_headers(429, &[("retry-after", "0")], "{}");
    http.push(200, &data(json!({"me": [{"id": 1, "username": "u"}]})));
    let start = std::time::Instant::now();
    c.me().unwrap();
    assert!(start.elapsed() >= Duration::from_millis(15));
    assert_eq!(http.requests().len(), 2);

    // Throttled twice in a row: give up with the typed error.
    http.push_with_headers(429, &[("retry-after", "0")], "{}");
    http.push_with_headers(429, &[("retry-after", "0")], "{}");
    assert!(matches!(c.me(), Err(Error::RateLimited { .. })));

    // The daily limit (a long wait): no retry, and the block is remembered,
    // so the calls that follow fail at once without a request.
    http.push_with_headers(429, &[("retry-after", "3600")], "{}");
    assert_eq!(
        c.me(),
        Err(Error::RateLimited {
            retry_after: Some(Duration::from_secs(3600))
        })
    );
    let sent = http.requests().len();
    assert!(matches!(
        c.me(),
        Err(Error::RateLimited { retry_after: Some(left) }) if left > Duration::from_secs(3500)
    ));
    assert_eq!(http.requests().len(), sent);
    assert_eq!(http.remaining(), 0);
}

#[test]
fn a_retry_that_cant_go_out_soon_enough_fails() {
    let tokens = TestTokens::new(false);
    let http = Arc::new(FakeTransport::new());
    let c = Client::with_transport(tokens, http.clone(), TokenBucket::new(5, 50.0));
    // 30 s is waited out by the queue, but past the retry's 15 s cap.
    http.push_with_headers(429, &[("retry-after", "30")], "{}");
    let start = std::time::Instant::now();
    assert_eq!(
        c.me(),
        Err(Error::RateLimited {
            retry_after: Some(Duration::from_secs(30))
        })
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn writes_searches_and_their_reads_are_interactive() {
    let (c, _) = client(TestTokens::new(false));
    for (name, doc) in queries::ALL {
        let op = doc
            .split_whitespace()
            .nth(1)
            .and_then(|w| w.split(['(', '{']).next())
            .unwrap();
        let interactive =
            is_mutation(doc) || op.starts_with("Search") || WRITE_STEP_READS.contains(&op);
        let expected = if interactive {
            Priority::Interactive
        } else {
            Priority::Background
        };
        assert_eq!(c.priority_of(op, doc), expected, "{name}");
    }
    assert_eq!(
        c.priority_of("LibraryPage", queries::LIBRARY_PAGE),
        Priority::Background
    );
    assert_eq!(
        c.priority_of("UpdateUserBook", queries::UPDATE_USER_BOOK),
        Priority::Interactive
    );
    assert_eq!(
        c.priority_of("SearchBooks", queries::SEARCH_BOOKS),
        Priority::Interactive
    );
    // A client can raise its own reads.
    let interactive = c.with_priority(Priority::Interactive);
    assert_eq!(
        interactive.priority_of("LibraryPage", queries::LIBRARY_PAGE),
        Priority::Interactive
    );
    for (op, _) in WRITE_STEP_READS.iter().map(|op| (op, ())) {
        assert!(
            queries::ALL
                .iter()
                .any(|(_, doc)| doc.contains(&format!("query {op}("))),
            "{op} names no query"
        );
    }
}

#[test]
fn a_write_without_an_answer_is_unconfirmed_and_not_retried() {
    let (c, http) = client(TestTokens::new(false));
    http.push_err(Error::Timeout);
    assert_eq!(
        c.set_privacy(UserBookId::new(3), Privacy::Private),
        Err(Error::OutcomeUnknown)
    );
    http.push(504, r#"{"error":"Gateway Timeout"}"#);
    assert_eq!(
        c.set_privacy(UserBookId::new(3), Privacy::Private),
        Err(Error::OutcomeUnknown)
    );
    // A failure to connect never reached Hardcover: a definite failure.
    http.push_err(Error::Network("could not connect (connect)".into()));
    assert!(matches!(
        c.set_privacy(UserBookId::new(3), Privacy::Private),
        Err(Error::Network(_))
    ));
    // A read that times out is just a timeout.
    http.push_err(Error::Timeout);
    assert_eq!(c.me(), Err(Error::Timeout));
    assert_eq!(http.requests().len(), 4);
}

#[test]
fn concurrent_first_callers_share_one_me_request() {
    let (c, http) = client(TestTokens::new(false));
    // One `me`, then each caller's own goals request.
    http.push(200, &data(json!({"me": [{"id": 7, "username": "u"}]})));
    for _ in 0..4 {
        http.push(200, &data(json!({"goals": []})));
    }
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let c = c.clone();
            std::thread::spawn(move || c.goals().unwrap())
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let me_requests = http
        .requests()
        .iter()
        .filter(|r| r.json.as_ref().unwrap()["operationName"] == "Me")
        .count();
    assert_eq!(me_requests, 1);
}

#[test]
fn add_and_update_use_payload_entry_or_refetch() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"insert_user_book": {"error": null, "id": 3, "user_book": user_book(3, 1)}})),
    );
    let e = c
        .add_to_library(BookId::new(1003), ReadingStatus::WantToRead)
        .unwrap();
    assert_eq!(e.user_book_id, UserBookId::new(3));
    let reqs0 = http.requests();
    let vars = &reqs0[0].json.as_ref().unwrap()["variables"];
    assert_eq!(vars["object"], json!({"book_id": 1003, "status_id": 1}));

    // Payload without the nested entry: re-read by id.
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": null}})),
    );
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    let e = c
        .set_status(UserBookId::new(3), ReadingStatus::CurrentlyReading)
        .unwrap();
    assert_eq!(e.status, ReadingStatus::CurrentlyReading);
    let reqs = http.requests();
    let today = chrono::Local::now().date_naive().to_string();
    assert_eq!(
        reqs[1].json.as_ref().unwrap()["variables"],
        json!({"id": 3, "object": {"status_id": 2, "user_date": today}})
    );
    assert_eq!(
        reqs[2].json.as_ref().unwrap()["operationName"],
        "UserBookById"
    );

    // Mutation `error` field.
    http.push(
        200,
        &data(json!({"update_user_book": {"error": "Not allowed", "id": null, "user_book": null}})),
    );
    assert_eq!(
        c.set_status(UserBookId::new(3), ReadingStatus::Read),
        Err(Error::Api("Not allowed".into()))
    );
}

#[test]
fn set_rating_sends_number_or_null() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": user_book(3, 3)}})),
    );
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": user_book(3, 3)}})),
    );
    c.set_rating(UserBookId::new(3), Rating::new(3.5), ReadingStatus::Paused)
        .unwrap();
    c.set_rating(UserBookId::new(3), None, ReadingStatus::Read)
        .unwrap();
    let reqs = http.requests();
    assert_eq!(
        reqs[0].json.as_ref().unwrap()["variables"]["object"],
        json!({"rating": 3.5, "status_id": 4})
    );
    assert_eq!(
        reqs[1].json.as_ref().unwrap()["variables"]["object"],
        json!({"rating": null, "status_id": 3})
    );
}

#[test]
fn remove_from_library() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"delete_user_book": {"id": 3, "book_id": 1003, "user_id": 77}})),
    );
    c.remove_from_library(UserBookId::new(3)).unwrap();
    http.push(200, &data(json!({"delete_user_book": null})));
    assert!(matches!(
        c.remove_from_library(UserBookId::new(3)),
        Err(Error::Api(_))
    ));
}

#[test]
fn progress_percent_converts_with_read_edition() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"user_books": [{
            "id": 3,
            "book": {"pages": 100, "audio_seconds": null},
            "edition": {"pages": 200, "audio_seconds": null},
            "user_book_reads": [{"id": 91, "edition": {"pages": 300, "audio_seconds": null}}]
        }]})),
    );
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 91, "user_book_read": {"id": 91, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(UserBookId::new(3), None, ProgressUpdate::Percent(50.0))
        .unwrap();
    let reqs = http.requests();
    let ctx_vars = &reqs[0].json.as_ref().unwrap()["variables"];
    assert_eq!(
        ctx_vars["reads"],
        json!({"finished_at": {"_is_null": true}})
    );
    assert_eq!(
        reqs[1].json.as_ref().unwrap()["variables"],
        json!({"id": 91, "read": {"progress_pages": 150}})
    );
}

#[test]
fn progress_pages_on_known_read_skips_context() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 91, "user_book_read": {"id": 91, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(
        UserBookId::new(3),
        Some(ReadId::new(91)),
        ProgressUpdate::Pages(42),
    )
    .unwrap();
    let reqs = http.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(
        reqs[0].json.as_ref().unwrap()["variables"],
        json!({"id": 91, "read": {"progress_pages": 42}})
    );
}

#[test]
fn progress_creates_a_read_when_none_is_open() {
    let (c, http) = client(TestTokens::new(false));
    // The book's own audiobook length is no unit for a read without an
    // edition: its pages are.
    http.push(200, &data(json!({"user_books": [{"id": 3, "status_id": 2, "book": {"pages": 300, "audio_seconds": 3600}, "edition": null, "user_book_reads": []}]})));
    http.push(200, &data(json!({"insert_user_book_read": {"error": null, "id": 95, "user_book_read": {"id": 95, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(UserBookId::new(3), None, ProgressUpdate::Percent(10.0))
        .unwrap();
    let reqs = http.requests();
    let vars = &reqs[1].json.as_ref().unwrap()["variables"];
    assert_eq!(vars["userBookId"], 3);
    assert_eq!(vars["read"]["progress_pages"], 30);
    assert_eq!(vars["read"]["started_at"].as_str().unwrap().len(), 10);
}

#[test]
fn progress_percent_without_length_is_rejected_before_writing() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({"user_books": [{"id": 3, "book": {"pages": null, "audio_seconds": null}, "edition": null, "user_book_reads": [{"id": 9, "edition": null}]}]})));
    assert!(matches!(
        c.update_progress(UserBookId::new(3), None, ProgressUpdate::Percent(10.0)),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        c.update_progress(
            UserBookId::new(3),
            Some(ReadId::new(9)),
            ProgressUpdate::Percent(140.0)
        ),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(http.remaining(), 0);
}

#[test]
fn set_read_dates_sends_both_fields() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 91, "user_book_read": {"id": 91, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 3)})));
    let start = Date::from_ymd_opt(2024, 3, 1).unwrap();
    c.set_read_dates(ReadId::new(91), Some(start), None)
        .unwrap();
    assert_eq!(
        http.requests()[0].json.as_ref().unwrap()["variables"],
        json!({"id": 91, "read": {"started_at": "2024-03-01", "finished_at": null}})
    );
    let end = Date::from_ymd_opt(2024, 2, 1).unwrap();
    assert!(matches!(
        c.set_read_dates(ReadId::new(91), Some(start), Some(end)),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn read_write_prefers_payload_entry_over_refetch() {
    let (c, http) = client(TestTokens::new(false));
    // The nested entry still lists an older read; the written read wins.
    let mut ub = user_book(3, 2);
    ub["user_book_reads"] = json!([{"id": 90, "started_at": null, "finished_at": null, "progress": null,
                                    "progress_pages": null, "progress_seconds": null, "edition_id": null}]);
    http.push(
        200,
        &data(json!({"update_user_book_read": {"error": null, "id": 91, "user_book_read": {
            "id": 91, "user_book_id": 3, "started_at": "2024-05-01", "finished_at": null,
            "progress": 50.0, "progress_pages": 200, "progress_seconds": null, "edition_id": 5003,
            "user_book": ub
        }}})),
    );
    let e = c
        .update_progress(
            UserBookId::new(3),
            Some(ReadId::new(91)),
            ProgressUpdate::Pages(200),
        )
        .unwrap();
    assert_eq!(
        http.requests().len(),
        1,
        "no refetch when the payload has the entry"
    );
    let r = e.entry.current_read().unwrap().clone();
    assert_eq!(
        (r.id, r.progress_pages, r.progress_percent),
        (ReadId::new(91), Some(200), Some(50.0))
    );
}

#[test]
fn progress_without_open_read_moves_to_currently_reading_first() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({"user_books": [{"id": 3, "status_id": 1, "book": {"pages": 300, "audio_seconds": null}, "edition": null, "user_book_reads": []}]})));
    // set_status(2): the server starts read 9003 (see `user_book`).
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": user_book(3, 2)}})),
    );
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 9003, "user_book_read": {"id": 9003, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(UserBookId::new(3), None, ProgressUpdate::Pages(12))
        .unwrap();
    let reqs = http.requests();
    let ops: Vec<_> = reqs
        .iter()
        .map(|r| {
            r.json.as_ref().unwrap()["operationName"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        ops,
        [
            "ProgressContext",
            "UpdateUserBook",
            "UpdateUserBookRead",
            "UserBookById"
        ]
    );
    assert_eq!(
        reqs[2].json.as_ref().unwrap()["variables"],
        json!({"id": 9003, "read": {"progress_pages": 12}})
    );
}

/// D21: the move to Currently Reading went through, so the entry it left is
/// what to show, with the progress's failure beside it.
#[test]
fn progress_that_fails_after_the_move_returns_the_moved_entry() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({"user_books": [{"id": 3, "status_id": 1, "book": {"pages": 300, "audio_seconds": null}, "edition": null, "user_book_reads": []}]})));
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": user_book(3, 2)}})),
    );
    http.push_err(Error::Timeout);
    let change = c
        .update_progress(UserBookId::new(3), None, ProgressUpdate::Pages(12))
        .unwrap();
    assert_eq!(change.entry.status, ReadingStatus::CurrentlyReading);
    assert_eq!(change.progress_left_out, Some(Error::OutcomeUnknown));
    // Without a move there's nothing to show: the failure is the result.
    http.push_err(Error::Network("reset".into()));
    assert!(matches!(
        c.update_progress(
            UserBookId::new(3),
            Some(ReadId::new(91)),
            ProgressUpdate::Pages(5)
        ),
        Err(Error::Network(_))
    ));
}

/// The progress context picks the read `LibraryEntry::open_read` picks: the
/// newest by date, not by id (a past read added later has the higher id).
#[test]
fn progress_goes_on_the_read_the_entry_calls_open() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"user_books": [{"id": 3, "status_id": 2,
        "book": {"pages": 300, "audio_seconds": null}, "edition": null,
        "user_book_reads": [
            {"id": 95, "started_at": "2019-03-01", "finished_at": null, "edition": null},
            {"id": 91, "started_at": "2026-11-01", "finished_at": null, "edition": null}
        ]}]})),
    );
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 91, "user_book_read": {"id": 91, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(UserBookId::new(3), None, ProgressUpdate::Pages(12))
        .unwrap();
    assert_eq!(
        vars(&http, 1),
        json!({"id": 91, "read": {"progress_pages": 12}})
    );
}

/// D20: the read a status change starts or finishes is dated today here,
/// not in the account's time zone.
#[test]
fn status_changes_that_date_a_read_send_todays_date() {
    let (c, http) = client(TestTokens::new(false));
    let today = chrono::Local::now().date_naive().to_string();
    for _ in 0..4 {
        http.push(
            200,
            &data(
                json!({"update_user_book": {"error": null, "id": 3, "user_book": user_book(3, 2)}}),
            ),
        );
    }
    http.push(
        200,
        &data(json!({"insert_user_book": {"error": null, "id": 3, "user_book": user_book(3, 3)}})),
    );
    let id = UserBookId::new(3);
    c.set_status(id, ReadingStatus::CurrentlyReading).unwrap();
    c.set_status(id, ReadingStatus::Read).unwrap();
    c.set_status(id, ReadingStatus::Paused).unwrap();
    c.set_status(id, ReadingStatus::WantToRead).unwrap();
    c.add_to_library(BookId::new(1003), ReadingStatus::Read)
        .unwrap();
    assert_eq!(
        vars(&http, 0)["object"],
        json!({"status_id": 2, "user_date": today})
    );
    assert_eq!(
        vars(&http, 1)["object"],
        json!({"status_id": 3, "user_date": today})
    );
    assert_eq!(vars(&http, 2)["object"], json!({"status_id": 4}));
    assert_eq!(vars(&http, 3)["object"], json!({"status_id": 1}));
    assert_eq!(
        vars(&http, 4)["object"],
        json!({"book_id": 1003, "status_id": 3, "user_date": today})
    );
}

fn ops(http: &FakeTransport) -> Vec<String> {
    http.requests()
        .iter()
        .map(|r| {
            r.json.as_ref().unwrap()["operationName"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn vars(http: &FakeTransport, ix: usize) -> Value {
    http.requests()[ix].json.as_ref().unwrap()["variables"].clone()
}

fn raw_read(id: i64, started: Option<&str>, finished: Option<&str>) -> Value {
    json!({"id": id, "started_at": started, "finished_at": finished, "progress": null,
           "progress_pages": null, "progress_seconds": null, "edition_id": null, "edition": null})
}

#[test]
fn entry_keeps_every_read_and_the_new_fields() {
    let (c, http) = client(TestTokens::new(false));
    let mut ub = user_book(3, 3);
    ub["privacy_setting_id"] = json!(3);
    ub["private_notes"] = json!("  Signed copy  ");
    // Newest id first, as the query orders them: a past read added after the latest one.
    ub["user_book_reads"] = json!([
        raw_read(92, Some("2015-01-01"), Some("2015-02-01")),
        raw_read(91, Some("2024-01-01"), Some("2024-02-01")),
        raw_read(90, Some("2020-01-01"), Some("2020-03-01")),
    ]);
    ub["edition"] = json!({"id": 5003, "title": "Synthetic Edition", "pages": null, "audio_seconds": 36000,
        "edition_format": "Audible Audio", "release_date": "2020-05-06", "isbn_13": "9780000000002",
        "cached_image": {}, "reading_format": {"format": "Listened"}, "publisher": {"name": "Synthetic Audio"},
        "language": {"language": "English"}});
    http.push(200, &data(json!({"user_books_by_pk": ub})));
    let e = c.entry_by_id(3).unwrap().unwrap();
    let ids: Vec<i64> = e.reads.iter().map(|r| r.id.get()).collect();
    assert_eq!(ids, [91, 90, 92], "most recent read first, by date");
    assert_eq!(e.current_read().unwrap().id, ReadId::new(91));
    assert_eq!(e.privacy, Some(Privacy::Private));
    assert_eq!(e.private_notes.as_deref(), Some("  Signed copy  "));
    let ed = e.edition.as_ref().unwrap();
    assert_eq!(ed.length(), Some(Length::Seconds(36000)));
    assert_eq!(ed.publisher.as_deref(), Some("Synthetic Audio"));
    assert_eq!(ed.reading_format.as_deref(), Some("Listened"));
    assert_eq!(ed.language.as_deref(), Some("English"));
    assert_eq!(ed.release_date, Date::from_ymd_opt(2020, 5, 6));
    assert_eq!(e.length(), Some(Length::Seconds(36000)));
}

#[test]
fn library_page_asks_for_ignored_too() {
    assert!(queries::LIBRARY_PAGE.contains("[1, 2, 3, 4, 5, 6]"));
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"user_books": [user_book(1, 6), user_book(2, 1)]})),
    );
    http.push(200, &data(json!({"user_books": []})));
    let lib = c.library(UserId::new(77)).unwrap().entries;
    assert_eq!(lib[0].status, ReadingStatus::Ignored);
}

#[test]
fn book_editions_pages_by_offset() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"editions": [
            {"id": 1, "title": "Synthetic", "pages": 320, "audio_seconds": null, "edition_format": "Paperback",
             "release_date": "2001-02-03", "isbn_13": "9780000000001", "cached_image": {},
             "reading_format": {"format": "Read"}, "publisher": {"name": "Synthetic House"}, "language": null},
            {"id": 2, "title": null, "pages": 0, "audio_seconds": null, "edition_format": " ",
             "release_date": null, "isbn_13": null, "cached_image": {}, "reading_format": null,
             "publisher": null, "language": null}
        ]})),
    );
    let page = c.book_editions(BookId::new(1001), 50).unwrap();
    // One row more than a page, so a full last page isn't taken for more.
    assert_eq!(
        vars(&http, 0),
        json!({"bookId": 1001, "limit": EDITIONS_PER_PAGE + 1, "offset": 50})
    );
    assert!(!page.has_more);
    assert_eq!(page.next_offset, 52);
    let editions = page.items;
    assert_eq!(editions.len(), 2);
    assert_eq!(editions[0].format.as_deref(), Some("Paperback"));
    assert_eq!(editions[0].publisher.as_deref(), Some("Synthetic House"));
    assert_eq!(editions[0].length(), Some(Length::Pages(320)));
    assert_eq!(
        (editions[1].pages, editions[1].format.as_deref()),
        (None, None)
    );
    assert_eq!(editions[1].length(), None);
}

/// `user_book(3, 2)` with its open read 9003 at 25% (page 100 of 400) on
/// edition 5003, moved to `edition`.
fn moved_entry(edition: Value) -> Value {
    let mut ub = user_book(3, 2);
    ub["edition"] = edition;
    ub
}

#[test]
fn set_edition_moves_the_open_read_at_the_same_percentage() {
    let (c, http) = client(TestTokens::new(false));
    let audio = json!({"id": 6000, "title": "Synthetic Audio", "pages": null, "audio_seconds": 36000,
        "edition_format": "Audiobook", "cached_image": {}});
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": moved_entry(audio.clone())}})),
    );
    let mut after = moved_entry(audio);
    after["user_book_reads"][0]["edition_id"] = json!(6000);
    http.push(
        200,
        &data(json!({"update_user_book_read": {"error": null, "id": 9003, "user_book_read": {
            "id": 9003, "user_book_id": 3, "started_at": "2024-02-01", "finished_at": null,
            "progress": 25.0, "progress_pages": null, "progress_seconds": 9000, "edition_id": 6000,
            "edition": {"id": 6000, "title": null, "pages": null, "audio_seconds": 36000, "edition_format": "Audiobook"},
            "user_book": after
        }}})),
    );
    let change = c
        .set_edition(UserBookId::new(3), EditionId::new(6000))
        .unwrap();
    assert!(change.read_left_behind.is_none());
    let e = change.entry;
    assert_eq!(ops(&http), ["UpdateUserBook", "UpdateUserBookRead"]);
    assert_eq!(
        vars(&http, 0),
        json!({"id": 3, "object": {"edition_id": 6000}})
    );
    assert_eq!(
        vars(&http, 1),
        json!({"id": 9003, "read": {"edition_id": 6000, "progress_pages": null, "progress_seconds": 9000}})
    );
    let read = e.open_read().unwrap();
    assert_eq!(read.edition_id, Some(EditionId::new(6000)));
    assert_eq!(e.length(), Some(Length::Seconds(36000)));
}

/// An open read at 100% moves one unit short of the new edition's end:
/// progress at the end would finish the read.
#[test]
fn set_edition_never_moves_a_read_to_the_end() {
    let (c, http) = client(TestTokens::new(false));
    let audio = json!({"id": 6000, "title": "Synthetic Audio", "pages": null, "audio_seconds": 36000,
        "edition_format": "Audiobook", "reading_format_id": 2, "cached_image": {}});
    let mut entry = moved_entry(audio);
    entry["user_book_reads"][0]["progress"] = json!(100.0);
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": entry}})),
    );
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 9003, "user_book_read": {"id": 9003, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.set_edition(UserBookId::new(3), EditionId::new(6000))
        .unwrap();
    assert_eq!(
        vars(&http, 1)["read"],
        json!({"edition_id": 6000, "progress_pages": null, "progress_seconds": 35999})
    );
}

#[test]
fn set_edition_without_an_open_read_is_one_request() {
    let (c, http) = client(TestTokens::new(false));
    let mut ub = user_book(3, 3);
    ub["user_book_reads"] = json!([raw_read(91, Some("2024-01-01"), Some("2024-02-01"))]);
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": ub}})),
    );
    c.set_edition(UserBookId::new(3), EditionId::new(6000))
        .unwrap();
    assert_eq!(ops(&http), ["UpdateUserBook"]);
}

#[test]
fn set_edition_keeps_the_change_when_moving_the_read_fails() {
    let (c, http) = client(TestTokens::new(false));
    let paper = json!({"id": 6001, "title": null, "pages": 200, "audio_seconds": null,
        "edition_format": "Paperback", "cached_image": {}});
    http.push(
        200,
        &data(
            json!({"update_user_book": {"error": null, "id": 3, "user_book": moved_entry(paper)}}),
        ),
    );
    http.push(500, "{}");
    let change = c
        .set_edition(UserBookId::new(3), EditionId::new(6001))
        .unwrap();
    assert!(change.read_left_behind.is_some());
    let e = change.entry;
    assert_eq!(
        vars(&http, 1)["read"],
        json!({"edition_id": 6001, "progress_pages": 50, "progress_seconds": null})
    );
    assert_eq!(e.edition.as_ref().unwrap().id, EditionId::new(6001));
    assert_eq!(
        e.open_read().unwrap().edition_id,
        Some(EditionId::new(5003))
    );
}

#[test]
fn notes_and_privacy_update_the_entry() {
    let (c, http) = client(TestTokens::new(false));
    for _ in 0..3 {
        http.push(
            200,
            &data(
                json!({"update_user_book": {"error": null, "id": 3, "user_book": user_book(3, 3)}}),
            ),
        );
    }
    c.set_private_notes(UserBookId::new(3), "  Lent to Sam\n")
        .unwrap();
    c.set_private_notes(UserBookId::new(3), " \n ").unwrap();
    c.set_privacy(UserBookId::new(3), Privacy::FollowersOnly)
        .unwrap();
    assert_eq!(
        vars(&http, 0)["object"],
        json!({"private_notes": "Lent to Sam"})
    );
    assert_eq!(vars(&http, 1)["object"], json!({"private_notes": null}));
    assert_eq!(vars(&http, 2)["object"], json!({"privacy_setting_id": 2}));
}

#[test]
fn add_read_needs_a_finish_after_the_start() {
    let (c, http) = client(TestTokens::new(false));
    let d = |y, m, d| Date::from_ymd_opt(y, m, d).unwrap();
    assert!(matches!(
        c.add_read(UserBookId::new(3), Some(d(2020, 2, 1)), d(2020, 1, 1), None),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(http.requests().len(), 0);

    // The nested entry does not list the new read yet; it is added.
    let mut ub = user_book(3, 3);
    ub["user_book_reads"] = json!([raw_read(91, Some("2024-01-01"), Some("2024-02-01"))]);
    http.push(
        200,
        &data(json!({"insert_user_book_read": {"error": null, "id": 95, "user_book_read": {
            "id": 95, "user_book_id": 3, "started_at": "2010-01-01", "finished_at": "2010-02-01",
            "progress": null, "progress_pages": null, "progress_seconds": null, "edition_id": 5003,
            "edition": null, "user_book": ub
        }}})),
    );
    let e = c
        .add_read(
            UserBookId::new(3),
            Some(d(2010, 1, 1)),
            d(2010, 2, 1),
            Some(EditionId::new(5003)),
        )
        .unwrap();
    assert_eq!(
        vars(&http, 0),
        json!({"userBookId": 3, "read": {"started_at": "2010-01-01", "finished_at": "2010-02-01", "edition_id": 5003}})
    );
    let ids: Vec<i64> = e.reads.iter().map(|r| r.id.get()).collect();
    assert_eq!(ids, [91, 95], "the past read goes after the latest one");
    assert_eq!(e.current_read().unwrap().id, ReadId::new(91));
}

#[test]
fn delete_read_reads_the_entry_back_without_it() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"delete_user_book_read": {"error": null, "id": 91}})),
    );
    // A stale read-back that still lists the deleted read.
    let mut ub = user_book(3, 3);
    ub["user_book_reads"] = json!([
        raw_read(91, Some("2024-01-01"), Some("2024-02-01")),
        raw_read(90, Some("2020-01-01"), Some("2020-02-01")),
    ]);
    http.push(200, &data(json!({"user_books_by_pk": ub})));
    let e = c.delete_read(UserBookId::new(3), ReadId::new(91)).unwrap();
    assert_eq!(ops(&http), ["DeleteUserBookRead", "UserBookById"]);
    assert_eq!(vars(&http, 0), json!({"id": 91}));
    assert_eq!(e.reads.len(), 1);
    assert_eq!(e.current_read().unwrap().id, ReadId::new(90));

    http.push(
        200,
        &data(json!({"delete_user_book_read": {"error": "Not yours", "id": null}})),
    );
    assert_eq!(
        c.delete_read(UserBookId::new(3), ReadId::new(91)),
        Err(Error::Api("Not yours".into()))
    );
    http.push(200, &data(json!({"delete_user_book_read": null})));
    assert!(matches!(
        c.delete_read(UserBookId::new(3), ReadId::new(91)),
        Err(Error::Api(_))
    ));
    assert_eq!(http.remaining(), 0, "no read-back after a failed delete");
}

#[test]
fn mark_owned_sends_the_edition_and_returns_the_row() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"edition_owned": {"id": 77, "list_book": {"id": 77, "list_id": 40, "book_id": 1003, "edition_id": 5003}}})),
    );
    let row = c.mark_owned(EditionId::new(5003)).unwrap().unwrap();
    assert_eq!(vars(&http, 0), json!({"id": 5003}));
    assert_eq!(row.id, ListBookId::new(77));
    assert_eq!(row.list_id, ListId::new(40));
    assert_eq!(row.book_id, BookId::new(1003));
    assert_eq!(row.edition_id, Some(EditionId::new(5003)));
    // Only an id: added, but the row is unknown.
    http.push(
        200,
        &data(json!({"edition_owned": {"id": 78, "list_book": null}})),
    );
    assert_eq!(c.mark_owned(EditionId::new(5003)).unwrap(), None);
    http.push(
        200,
        &data(json!({"edition_owned": {"id": null, "list_book": null}})),
    );
    assert!(matches!(
        c.mark_owned(EditionId::new(5003)),
        Err(Error::Api(_))
    ));
}

#[test]
fn progress_percent_on_an_audiobook_counts_time() {
    let (c, http) = client(TestTokens::new(false));
    // The book has a page count, but the entry is an audiobook.
    http.push(
        200,
        &data(json!({"user_books": [{
            "id": 3, "status_id": 2,
            "book": {"pages": 300, "audio_seconds": null},
            "edition": {"pages": null, "audio_seconds": 7200},
            "user_book_reads": [{"id": 91, "edition": null}]
        }]})),
    );
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 91, "user_book_read": {"id": 91, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(UserBookId::new(3), None, ProgressUpdate::Percent(25.0))
        .unwrap();
    assert_eq!(
        vars(&http, 1),
        json!({"id": 91, "read": {"progress_seconds": 1800}})
    );
}

#[test]
fn progress_percent_follows_the_reading_format_not_the_first_length() {
    // An audiobook edition that also lists pages counts time, as Hardcover
    // does by its reading format.
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"user_books": [{
            "id": 3, "status_id": 2,
            "book": {"pages": 300, "audio_seconds": null},
            "edition": null,
            "user_book_reads": [{"id": 91, "edition": {"pages": 280, "audio_seconds": 7200, "reading_format_id": 2}}]
        }]})),
    );
    http.push(200, &data(json!({"update_user_book_read": {"error": null, "id": 91, "user_book_read": {"id": 91, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(
        UserBookId::new(3),
        Some(ReadId::new(91)),
        ProgressUpdate::Percent(50.0),
    )
    .unwrap();
    assert_eq!(
        vars(&http, 1),
        json!({"id": 91, "read": {"progress_seconds": 3600}})
    );

    // An audiobook without a running time can't take a percentage, rather
    // than counting it in the book's pages.
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"user_books": [{
            "id": 3, "status_id": 2,
            "book": {"pages": 300, "audio_seconds": null},
            "edition": {"pages": null, "audio_seconds": null, "reading_format_id": 2},
            "user_book_reads": [{"id": 91, "edition": null}]
        }]})),
    );
    assert!(matches!(
        c.update_progress(UserBookId::new(3), None, ProgressUpdate::Percent(10.0)),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(http.remaining(), 0);
}

#[test]
fn a_synced_entry_knows_its_books_audiobook_and_its_editions_format() {
    let mut ub = user_book(3, 2);
    ub["book"]["default_audio_edition_id"] = json!(6000);
    ub["user_book_reads"] = json!([{
        "id": 91, "started_at": "2026-11-01", "finished_at": null, "progress": null,
        "progress_pages": null, "progress_seconds": 600, "edition_id": 6000,
        "edition": {"id": 6000, "title": null, "pages": null, "audio_seconds": null,
            "edition_format": "Audible", "reading_format_id": 2}
    }]);
    let entry: wire::RawUserBook = serde_json::from_value(ub).unwrap();
    let entry = entry.into_entry().unwrap();
    assert_eq!(entry.book_audio_edition, Some(EditionId::new(6000)));
    let read_edition = entry.open_read().unwrap().edition.as_ref().unwrap();
    assert!(read_edition.is_audiobook());
    // The read's audiobook decides, though its running time is unknown.
    assert_eq!(
        entry.measure(),
        Some(Measure {
            unit: ProgressUnit::Time,
            total: None
        })
    );
    assert_eq!(entry.length(), None);
    assert!(entry.is_audio());
}

#[test]
fn an_unfinished_read_never_scales_to_the_last_unit() {
    assert_eq!(
        Length::Pages(476).position_at(42.0),
        ProgressUpdate::Pages(200)
    );
    assert_eq!(Length::Pages(476).units_at(99.99), 475);
    assert_eq!(Length::Pages(476).units_at(100.0), 476);
    assert_eq!(
        Length::Seconds(36_000).position_at(50.0),
        ProgressUpdate::Seconds(18_000)
    );
    assert_eq!(Length::Seconds(36_000).units_at(f64::NAN), 0);
    assert_eq!(super::model::scale(0.5, 300), 150);
    assert_eq!(super::model::scale(0.997, 150), 149);
    assert_eq!(super::model::scale(1.0, 150), 150);
    assert_eq!(super::model::scale(0.5, 0), 0);
}

fn goal_row(id: i64, metric: &str, goal: i64, progress: &Value) -> Value {
    json!({
        "id": id,
        "metric": metric,
        "goal": goal,
        "progress": progress,
        "description": format!("{goal} {metric}s in 2030"),
        "start_date": "2030-01-01",
        "end_date": "2030-12-31",
        "conditions": {},
        "archived": false,
        "completed_at": null,
        "privacy_setting_id": 2,
    })
}

#[test]
fn goals_are_scoped_to_me_and_convert_hours() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"me": [{"id": 77, "username": "reader", "name": null, "image": null}]})),
    );
    http.push(
        200,
        &data(json!({"goals": [
            goal_row(5, "book", 30, &json!("12")),
            goal_row(6, "hour", 100, &json!(36000)),
            // Unparseable dates are skipped, not fatal.
            {"id": 7, "metric": "book", "goal": 1, "progress": 0, "start_date": "soon", "end_date": "later"},
        ]})),
    );
    let goals = c.goals().unwrap();
    assert_eq!(goals.len(), 2);
    let books = &goals[0];
    assert_eq!(books.id, GoalId::new(5));
    assert_eq!(books.metric, GoalMetric::Books);
    assert_eq!((books.target, books.progress), (30, 12.0));
    assert_eq!(books.start, Date::from_ymd_opt(2030, 1, 1).unwrap());
    assert_eq!(books.privacy(), Some(Privacy::FollowersOnly));
    assert!(books.conditions.is_empty());
    // Hour goals: target in hours, the API's progress in seconds.
    assert_eq!((goals[1].target, goals[1].progress), (100, 10.0));
    let reqs = http.requests();
    assert_eq!(reqs[1].json.as_ref().unwrap()["operationName"], "Goals");
    assert_eq!(reqs[1].json.as_ref().unwrap()["variables"]["userId"], 77);
}

#[test]
fn create_goal_sends_a_whole_goal_input() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"insert_goal": {"errors": null, "id": 5, "goal": goal_row(5, "book", 30, &json!(0))}})),
    );
    let d = |m, d| Date::from_ymd_opt(2030, m, d).unwrap();
    let new = NewGoal {
        metric: GoalMetric::Books,
        target: 30,
        start: d(1, 1),
        end: d(12, 31),
        description: "30 books in 2030".into(),
    };
    let goal = c.create_goal(&new).unwrap();
    assert_eq!(goal.id, GoalId::new(5));
    assert_eq!(
        http.requests()[0].json.as_ref().unwrap()["variables"]["object"],
        json!({
            "metric": "book",
            "goal": 30,
            "description": "30 books in 2030",
            "start_date": "2030-01-01",
            "end_date": "2030-12-31",
            "conditions": {},
        })
    );
    // Checked before sending.
    let bad = NewGoal {
        target: 0,
        ..new.clone()
    };
    assert!(matches!(c.create_goal(&bad), Err(Error::InvalidInput(_))));
    let blank = NewGoal {
        description: " ".into(),
        ..new.clone()
    };
    assert!(matches!(c.create_goal(&blank), Err(Error::InvalidInput(_))));
    assert_eq!(http.requests().len(), 1);
    // `errors` is a list.
    http.push(
        200,
        &data(json!({"insert_goal": {"errors": ["Goal already exists", null], "id": null, "goal": null}})),
    );
    assert_eq!(
        c.create_goal(&new),
        Err(Error::Api("Goal already exists".into()))
    );
}

/// D19: the goal is read just before writing, so the edit changes only
/// what the user edited and everything else goes back as Hardcover has it
/// now, not as the app loaded it.
#[test]
fn update_goal_reads_the_goal_and_changes_only_the_edit() {
    let (c, http) = client(TestTokens::new(false));
    // Changed on the website since the app loaded it: description,
    // conditions and privacy.
    let mut fresh = goal_row(5, "page", 9000, &json!(1200));
    fresh["description"] = json!("Renamed on the website");
    fresh["conditions"] = json!({"readingFormatId": 1, "authorBipoc": null});
    fresh["privacy_setting_id"] = json!(3);
    http.push(200, &data(json!({"goals_by_pk": fresh})));
    // A payload without the goal: re-read by id.
    http.push(
        200,
        &data(json!({"update_goal": {"errors": [], "id": 5, "goal": null}})),
    );
    http.push(200, &data(json!({"goals_by_pk": fresh})));
    let saved = c.update_goal(GoalId::new(5), |g| g.target = 10000).unwrap();
    assert_eq!(saved.id, GoalId::new(5));
    assert_eq!(ops(&http), ["GoalById", "UpdateGoal", "GoalById"]);
    assert_eq!(
        vars(&http, 1),
        json!({"id": 5, "object": {
            "metric": "page",
            "goal": 10000,
            "description": "Renamed on the website",
            "start_date": "2030-01-01",
            "end_date": "2030-12-31",
            "conditions": {"readingFormatId": 1},
            "archived": false,
            "privacy_setting_id": 3,
        }})
    );
}

#[test]
fn update_goal_checks_the_goal_as_read_before_writing() {
    let (c, http) = client(TestTokens::new(false));
    // Conditions it can't send back block the update after the read.
    let mut row = goal_row(5, "page", 9000, &json!(1200));
    row["conditions"] = json!({"genre": "poetry"});
    http.push(200, &data(json!({"goals_by_pk": row})));
    assert!(matches!(
        c.update_goal(GoalId::new(5), |g| g.target = 10),
        Err(Error::InvalidInput(_))
    ));
    // So does a missing description, which Hardcover requires.
    let mut row = goal_row(5, "page", 9000, &json!(1200));
    row["description"] = json!("  ");
    http.push(200, &data(json!({"goals_by_pk": row})));
    assert!(matches!(
        c.update_goal(GoalId::new(5), |g| g.target = 10),
        Err(Error::InvalidInput(_))
    ));
    // A goal deleted meanwhile.
    http.push(200, &data(json!({"goals_by_pk": null})));
    assert!(matches!(
        c.update_goal(GoalId::new(5), |g| g.target = 10),
        Err(Error::Api(_))
    ));
    assert_eq!(ops(&http), ["GoalById", "GoalById", "GoalById"]);
}

#[test]
fn update_goal_keeps_a_privacy_setting_it_does_not_know() {
    let (c, http) = client(TestTokens::new(false));
    let mut row = goal_row(5, "book", 30, &json!(3));
    row["privacy_setting_id"] = json!(7);
    let goal = RawGoal::deserialize(row.clone())
        .unwrap()
        .into_goal()
        .unwrap();
    assert_eq!((goal.privacy_id, goal.privacy()), (Some(7), None));
    http.push(200, &data(json!({"goals_by_pk": row})));
    http.push(
        200,
        &data(json!({"update_goal": {"errors": [], "id": 5, "goal": row}})),
    );
    c.update_goal(GoalId::new(5), |g| g.target = 40).unwrap();
    assert_eq!(vars(&http, 1)["object"]["privacy_setting_id"], 7);
}

#[test]
fn delete_goal_checks_the_payload() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"delete_goal": {"errors": null, "id": 5}})),
    );
    c.delete_goal(GoalId::new(5)).unwrap();
    assert_eq!(
        http.requests()[0].json.as_ref().unwrap()["variables"],
        json!({"id": 5})
    );
    // What Hardcover actually answers after deleting a goal.
    http.push(
        200,
        &data(json!({"delete_goal": {"errors": null, "id": null}})),
    );
    c.delete_goal(GoalId::new(5)).unwrap();
    http.push(
        200,
        &data(json!({"delete_goal": {"errors": ["Not found"], "id": null}})),
    );
    assert_eq!(
        c.delete_goal(GoalId::new(5)),
        Err(Error::Api("Not found".into()))
    );
}

// ----- reviews -----

fn reviewed_user_book(id: i64, status_id: i64, review: Value) -> Value {
    let mut entry = user_book(id, status_id);
    let Value::Object(fields) = review else {
        panic!("expected review fields")
    };
    entry.as_object_mut().unwrap().extend(fields);
    entry
}

#[test]
fn review_is_fetched_on_its_own() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"user_books_by_pk": {
            "id": 3,
            "review_markdown": "  A *quiet* book.\n\nSecond thought.  ",
            "review_raw": "A quiet book. Second thought.",
            "review_has_spoilers": true,
            "reviewed_at": "2024-03-05T00:00:00",
        }})),
    );
    let review = c.review(UserBookId::new(3)).unwrap();
    assert_eq!(
        review.markdown.as_deref(),
        Some("A *quiet* book.\n\nSecond thought.")
    );
    assert_eq!(review.text(), review.markdown.as_deref());
    assert!(review.has_spoilers);
    assert_eq!(review.reviewed_at, Date::from_ymd_opt(2024, 3, 5));
    let req = &http.requests()[0];
    assert_eq!(
        req.json.as_ref().unwrap()["operationName"],
        "UserBookReview"
    );
    assert_eq!(vars(&http, 0), json!({"id": 3}));

    // Plain text only: shown as it is.
    http.push(
        200,
        &data(json!({"user_books_by_pk": {
            "id": 3, "review_markdown": null, "review_raw": "Plain words.",
            "review_has_spoilers": false, "reviewed_at": null,
        }})),
    );
    let review = c.review(UserBookId::new(3)).unwrap();
    assert!(review.exists());
    assert_eq!(review.text(), Some("Plain words."));

    // No review, a leftover flag, or no entry at all: nothing to show.
    http.push(
        200,
        &data(json!({"user_books_by_pk": {
            "id": 3, "review_markdown": " ", "review_raw": null,
            "review_has_spoilers": true, "reviewed_at": "2024-03-05T00:00:00",
        }})),
    );
    assert_eq!(c.review(UserBookId::new(3)).unwrap(), Review::default());
    http.push(200, &data(json!({"user_books_by_pk": null})));
    assert_eq!(c.review(UserBookId::new(3)).unwrap(), Review::default());

    // A review Hardcover has but in no form this app reads.
    http.push(
        200,
        &data(json!({"user_books_by_pk": {
            "id": 3, "review_markdown": null, "review_raw": null,
            "review_has_spoilers": true, "reviewed_at": "2024-03-05T00:00:00",
            "has_review": true,
        }})),
    );
    let review = c.review(UserBookId::new(3)).unwrap();
    assert!(review.exists());
    assert_eq!(review.text(), None);
    assert!(review.has_spoilers);
}

#[test]
fn set_review_keeps_the_status_and_uses_the_written_entry() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": reviewed_user_book(3, 2, json!({
            "review_markdown": "Loved the ending.",
            "review_raw": "Loved the ending.",
            "review_has_spoilers": false,
            "reviewed_at": "2024-06-07T00:00:00",
        }))}})),
    );
    let written = c
        .set_review(
            UserBookId::new(3),
            "  Loved the ending.\n",
            false,
            Date::from_ymd_opt(2024, 6, 7),
            ReadingStatus::CurrentlyReading,
        )
        .unwrap();
    let entry = written.entry.as_ref().unwrap();
    assert_eq!(entry.user_book_id, UserBookId::new(3));
    assert_eq!(entry.status, ReadingStatus::CurrentlyReading);
    assert_eq!(
        written.review.markdown.as_deref(),
        Some("Loved the ending.")
    );
    assert_eq!(written.review.reviewed_at, Date::from_ymd_opt(2024, 6, 7));
    let reqs = http.requests();
    assert_eq!(reqs.len(), 1, "no read-back");
    assert_eq!(
        reqs[0].json.as_ref().unwrap()["operationName"],
        "UpdateUserBookReview"
    );
    assert_eq!(
        vars(&http, 0),
        json!({
            "id": 3,
            "statusId": 2,
            "markdown": "Loved the ending.",
            "spoilers": false,
            "reviewedAt": "2024-06-07",
        })
    );

    // Blank text is refused before any request.
    assert!(matches!(
        c.set_review(UserBookId::new(3), " \n", true, None, ReadingStatus::Read),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn delete_review_clears_every_writable_form_and_reports_what_hardcover_kept() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": reviewed_user_book(3, 3, json!({
            "review_markdown": null, "review_raw": null,
            "review_has_spoilers": false, "reviewed_at": null,
        }))}})),
    );
    let written = c
        .delete_review(UserBookId::new(3), ReadingStatus::Read)
        .unwrap();
    assert!(!written.review.exists());
    let request = http.requests()[0].json.clone().unwrap();
    assert_eq!(request["operationName"], "DeleteUserBookReview");
    let query = request["query"].as_str().unwrap();
    for cleared in ["review_slate: $slate", "review_has_spoilers: false"] {
        assert!(query.contains(cleared), "{cleared}");
    }
    let sent = vars(&http, 0);
    assert_eq!((&sent["id"], &sent["statusId"]), (&json!(3), &json!(3)));
    assert_eq!(
        sent["slate"]["document"]["children"][0]["children"][0]["text"],
        json!("")
    );

    // Markdown Hardcover did not clear is reported as still there.
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": reviewed_user_book(3, 3, json!({
            "review_markdown": "Still here.", "review_raw": null,
            "review_has_spoilers": false, "reviewed_at": null,
        }))}})),
    );
    let written = c
        .delete_review(UserBookId::new(3), ReadingStatus::Read)
        .unwrap();
    assert_eq!(written.review.markdown.as_deref(), Some("Still here."));

    // Plain text left behind is Hardcover's own copy, not Markdown.
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": reviewed_user_book(3, 3, json!({
            "review_markdown": null, "review_raw": "Old words.",
            "review_has_spoilers": false, "reviewed_at": null, "has_review": true,
        }))}})),
    );
    let written = c
        .delete_review(UserBookId::new(3), ReadingStatus::Read)
        .unwrap();
    assert!(written.review.exists());
    assert_eq!(written.review.markdown, None);

    http.push(
        200,
        &data(json!({"update_user_book": {"error": "Not allowed", "id": null, "user_book": null}})),
    );
    assert_eq!(
        c.delete_review(UserBookId::new(3), ReadingStatus::Read),
        Err(Error::Api("Not allowed".into()))
    );
}

#[test]
fn review_write_without_an_entry_reads_the_entry_back_and_keeps_the_sent_review() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": null}})),
    );
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 3)})));
    let written = c
        .set_review(
            UserBookId::new(3),
            " Short and sharp.\n",
            true,
            Date::from_ymd_opt(2024, 6, 7),
            ReadingStatus::Read,
        )
        .unwrap();
    assert_eq!(written.entry.unwrap().status, ReadingStatus::Read);
    // The review is the one sent: reading it back so soon can be stale.
    assert_eq!(
        written.review,
        Review::new("Short and sharp.", true, Date::from_ymd_opt(2024, 6, 7))
    );
    let ops: Vec<Value> = http
        .requests()
        .iter()
        .map(|r| r.json.as_ref().unwrap()["operationName"].clone())
        .collect();
    assert_eq!(
        ops,
        vec![json!("UpdateUserBookReview"), json!("UserBookById")]
    );

    // A failed read-back doesn't turn the written review into an error.
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": null}})),
    );
    http.push(500, "oops");
    let written = c
        .delete_review(UserBookId::new(3), ReadingStatus::Read)
        .unwrap();
    assert_eq!(written.entry, None);
    assert_eq!(written.review, Review::default());
    assert_eq!(http.remaining(), 0);
}

// ----- lists -----

fn list_row(id: i64, name: &str, slug: &str) -> Value {
    json!({
        "id": id, "name": name, "slug": slug, "description": "  ", "privacy_setting_id": 3,
        "ranked": true, "books_count": 4, "default_view": "list", "featured_profile": false,
        "url": null,
    })
}

fn membership(id: i64, list: i64, book: i64, edition: Option<i64>) -> Value {
    json!({"id": id, "list_id": list, "book_id": book, "edition_id": edition})
}

fn me_reply(http: &FakeTransport) {
    http.push(
        200,
        &data(json!({"me": [{"id": 77, "username": "reader", "name": null, "image": null}]})),
    );
}

#[test]
fn lists_page_by_keyset_and_find_owned() {
    let (c, http) = client(TestTokens::new(false));
    me_reply(&http);
    let first: Vec<Value> = (1..=100)
        .map(|i| list_row(i, &format!("Synthetic list {i}"), &format!("list-{i}")))
        .collect();
    http.push(200, &data(json!({ "lists": first })));
    http.push(
        200,
        &data(json!({ "lists": [list_row(300, "Owned", "owned"), {"id": 301, "name": null}] })),
    );
    http.push(200, &data(json!({ "lists": [] })));
    let lists = c.lists().unwrap();
    assert_eq!(lists.len(), 102);
    assert_eq!(ops(&http), ["Me", "Lists", "Lists", "Lists"]);
    assert_eq!(
        vars(&http, 1),
        json!({"userId": 77, "afterId": 0, "limit": 100})
    );
    assert_eq!(vars(&http, 2)["afterId"], 100);
    let first = &lists[0];
    assert_eq!(first.id, ListId::new(1));
    assert_eq!(first.description, None, "blank is no description");
    assert_eq!(first.privacy(), Some(Privacy::Private));
    assert!(first.ranked && !first.is_owned());
    assert_eq!(first.books_count, 4);
    assert!(lists[100].is_owned());
    assert_eq!(
        lists[100].web_url("reader").as_deref(),
        Some("https://hardcover.app/@reader/lists/owned")
    );
    // Missing fields fall back instead of failing the page.
    assert_eq!(lists[101].name, "Untitled list");
    assert_eq!(lists[101].books_count, 0);

    // Paging that can't move forward is an error, not a short list.
    http.push(200, &data(json!({ "lists": [list_row(1, "One", "one")] })));
    http.push(200, &data(json!({ "lists": [list_row(1, "One", "one")] })));
    assert!(matches!(c.lists(), Err(Error::Decode(_))));
}

#[test]
fn memberships_stop_after_six_requests_and_say_so() {
    let (c, http) = client(TestTokens::new(false));
    me_reply(&http);
    let page_of = |page: i64| -> Vec<Value> {
        (1..=1000)
            .map(|i| membership(page * 1000 + i, 1, i, None))
            .collect()
    };
    // Five full pages and the empty one that finds the end: complete.
    for page in 0..5 {
        http.push(200, &data(json!({ "list_books": page_of(page) })));
    }
    http.push(200, &data(json!({ "list_books": [] })));
    let all = c.list_memberships().unwrap();
    assert_eq!(all.rows.len(), 5000);
    assert!(all.complete);
    assert_eq!(vars(&http, 5)["afterId"], 4000);
    // Rows still coming on the sixth request: incomplete, and no seventh.
    for page in 0..6 {
        http.push(200, &data(json!({ "list_books": page_of(page) })));
    }
    let all = c.list_memberships().unwrap();
    assert_eq!(all.rows.len(), 6000);
    assert!(!all.complete);
    assert_eq!(http.remaining(), 0);

    // An empty page ends it; a short one before it doesn't.
    http.push(
        200,
        &data(json!({ "list_books": [membership(9, 40, 1003, Some(5003))] })),
    );
    http.push(200, &data(json!({ "list_books": [] })));
    let all = c.list_memberships().unwrap();
    assert!(all.complete);
    assert_eq!(
        all.rows,
        [ListMembership {
            id: ListBookId::new(9),
            list_id: ListId::new(40),
            book_id: BookId::new(1003),
            edition_id: Some(EditionId::new(5003)),
        }]
    );
}

#[test]
fn one_books_memberships_are_scoped_to_me() {
    let (c, http) = client(TestTokens::new(false));
    me_reply(&http);
    http.push(
        200,
        &data(json!({ "list_books": [membership(9, 40, 1003, None), membership(10, 41, 1003, None)] })),
    );
    let rows = c.book_list_memberships(BookId::new(1003)).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(ops(&http), ["Me", "BookListMemberships"]);
    assert_eq!(vars(&http, 1)["bookId"], 1003);
    assert_eq!(vars(&http, 1)["userId"], 77);
}

#[test]
fn list_books_page_by_offset_and_skip_missing_books() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({ "list_books": [
            {"id": 1, "position": 1, "edition_id": null, "book": book(1001, "First")},
            {"id": 2, "position": null, "edition_id": 5002, "book": null},
            {"id": 3, "position": null, "edition_id": null, "book": book(1003, "Third")},
        ]})),
    );
    let page = c.list_books(ListId::new(40), 100).unwrap();
    assert_eq!(
        vars(&http, 0),
        json!({"listId": 40, "limit": 101, "offset": 100})
    );
    assert!(!page.has_more);
    // The unresolved row still counts towards the offset.
    assert_eq!(page.next_offset, 103);
    let books = page.items;
    let titles: Vec<&str> = books.iter().map(|b| b.book.title.as_str()).collect();
    assert_eq!(titles, ["First", "Third"]);
    assert_eq!(books[0].position, Some(1));
    assert_eq!(books[1].id, ListBookId::new(3));
    assert_eq!(books[0].book.authors, ["Jane Quill"]);
}

/// D7: an unresolved row on a full page neither ends the list nor shifts
/// the next page's offset.
#[test]
fn an_unresolved_list_row_does_not_cut_the_list_short() {
    let (c, http) = client(TestTokens::new(false));
    let mut rows: Vec<Value> = (1..=101)
        .map(|i| json!({"id": i, "position": i, "edition_id": null, "book": book(1000 + i, "Synthetic")}))
        .collect();
    rows[4]["book"] = Value::Null;
    http.push(200, &data(json!({ "list_books": rows })));
    let page = c.list_books(ListId::new(40), 0).unwrap();
    assert_eq!(page.items.len(), 99);
    assert!(page.has_more);
    assert_eq!(page.next_offset, 100);
    assert_eq!(page.items.last().unwrap().id, ListBookId::new(100));
}

#[test]
fn create_list_sends_the_draft_and_reports_errors() {
    let (c, http) = client(TestTokens::new(false));
    let draft = ListDraft {
        name: "  Synthetic picks ".into(),
        description: " For testing. ".into(),
        privacy: Some(Privacy::FollowersOnly),
        ranked: false,
    };
    http.push(
        200,
        &data(json!({"insert_list": {"errors": [], "id": 50, "list": list_row(50, "Synthetic picks", "synthetic-picks")}})),
    );
    let list = c.create_list(&draft).unwrap();
    assert_eq!(list.id, ListId::new(50));
    assert_eq!(
        vars(&http, 0),
        json!({"object": {"name": "Synthetic picks", "description": "For testing.",
                          "privacy_setting_id": 2, "ranked": false}})
    );
    // `errors` is a list; any entry fails the write.
    http.push(
        200,
        &data(json!({"insert_list": {"errors": [null, "Name has already been taken"], "id": null, "list": null}})),
    );
    let err = c.create_list(&draft).unwrap_err();
    assert!(err.to_string().contains("already been taken"), "{err}");
    // No list in the payload: read it back by id.
    http.push(
        200,
        &data(json!({"insert_list": {"errors": null, "id": 51, "list": null}})),
    );
    http.push(
        200,
        &data(json!({"lists_by_pk": list_row(51, "Synthetic picks", "synthetic-picks-2")})),
    );
    assert_eq!(c.create_list(&draft).unwrap().id, ListId::new(51));
    assert_eq!(ops(&http)[3], "ListById");
    // A blank name is refused before any request.
    let before = http.requests().len();
    let blank = ListDraft {
        name: "   ".into(),
        ..draft
    };
    assert!(matches!(c.create_list(&blank), Err(Error::InvalidInput(_))));
    assert_eq!(http.requests().len(), before);
}

/// D19: the list is read just before writing; only the fields the user
/// edited change, and the rest goes back as Hardcover has it now.
#[test]
fn update_list_reads_the_list_and_changes_only_the_edit() {
    let (c, http) = client(TestTokens::new(false));
    let shown = RawList {
        id: 50,
        name: Some("Old name".into()),
        slug: Some("old-name".into()),
        description: Some("Old".into()),
        privacy_setting_id: Some(1),
        ranked: Some(false),
        books_count: Some(2),
        default_view: Some("grid".into()),
        featured_profile: Some(true),
        url: Some("https://example.com/synthetic".into()),
    }
    .into_list();
    // The user renamed it; nothing else in the dialog changed.
    let draft = ListDraft {
        name: "New name".into(),
        description: "Old".into(),
        privacy: shown.privacy(),
        ranked: false,
    };
    let edit = ListEdit::between(&shown, &draft);
    assert_eq!(
        edit,
        ListEdit {
            name: Some("New name".into()),
            ..ListEdit::default()
        }
    );
    // Meanwhile, on the website: a description, ranking, view and privacy
    // (one this version doesn't know).
    let mut fresh = list_row(50, "Old name", "old-name");
    fresh["description"] = json!("Written on the website");
    fresh["privacy_setting_id"] = json!(9);
    fresh["ranked"] = json!(true);
    fresh["default_view"] = json!("list");
    http.push(200, &data(json!({"lists_by_pk": fresh})));
    http.push(
        200,
        &data(json!({"update_list": {"errors": [], "id": 50, "list": list_row(50, "New name", "new-name")}})),
    );
    let saved = c.update_list(ListId::new(50), &edit).unwrap();
    assert_eq!(saved.name, "New name");
    assert_eq!(ops(&http), ["ListById", "UpdateList"]);
    assert_eq!(
        vars(&http, 1),
        json!({"id": 50, "object": {
            "name": "New name", "description": "Written on the website", "ranked": true,
            "privacy_setting_id": 9, "default_view": "list", "featured_profile": false,
        }})
    );
    // An edit that clears the description and picks a privacy.
    let clear = ListEdit::between(
        &shown,
        &ListDraft {
            description: " ".into(),
            privacy: Some(Privacy::Private),
            ..draft.clone()
        },
    );
    assert_eq!(clear.description.as_deref(), Some(""));
    assert_eq!(clear.privacy, Some(Privacy::Private));
    http.push(200, &data(json!({"lists_by_pk": fresh})));
    http.push(
        200,
        &data(json!({"update_list": {"errors": [], "id": 50, "list": list_row(50, "New name", "new-name")}})),
    );
    c.update_list(ListId::new(50), &clear).unwrap();
    assert_eq!(vars(&http, 3)["object"]["description"], "");
    assert_eq!(vars(&http, 3)["object"]["privacy_setting_id"], 3);
    // The Owned list, as read, is the server's: no write.
    http.push(
        200,
        &data(json!({"lists_by_pk": list_row(51, "Owned", OWNED_LIST_SLUG)})),
    );
    assert!(matches!(
        c.update_list(ListId::new(51), &edit),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(http.requests().len(), 5);
}

#[test]
fn delete_list_checks_success() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({"delete_list": {"success": true}})));
    c.delete_list(ListId::new(50)).unwrap();
    assert_eq!(vars(&http, 0), json!({"id": 50}));
    http.push(200, &data(json!({"delete_list": {"success": false}})));
    assert!(matches!(c.delete_list(ListId::new(50)), Err(Error::Api(_))));
    http.push(200, &data(json!({"delete_list": null})));
    assert!(matches!(c.delete_list(ListId::new(50)), Err(Error::Api(_))));
}

#[test]
fn add_to_list_returns_the_row() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"insert_list_book": {"id": 9, "list_book": membership(9, 40, 1003, None)}})),
    );
    let row = c
        .add_to_list(ListId::new(40), BookId::new(1003), None)
        .unwrap();
    assert_eq!(row.id, ListBookId::new(9));
    assert_eq!(
        vars(&http, 0),
        json!({"object": {"list_id": 40, "book_id": 1003}})
    );
    // Only the id: the rest is what was sent.
    http.push(
        200,
        &data(json!({"insert_list_book": {"id": 10, "list_book": null}})),
    );
    let row = c
        .add_to_list(
            ListId::new(40),
            BookId::new(1003),
            Some(EditionId::new(5003)),
        )
        .unwrap();
    assert_eq!(row.id, ListBookId::new(10));
    assert_eq!(row.edition_id, Some(EditionId::new(5003)));
    assert_eq!(vars(&http, 1)["object"]["edition_id"], 5003);
    http.push(
        200,
        &data(json!({"insert_list_book": {"id": null, "list_book": null}})),
    );
    assert!(matches!(
        c.add_to_list(ListId::new(40), BookId::new(1003), None),
        Err(Error::Api(_))
    ));
}

#[test]
fn remove_from_list_takes_the_row_id() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"delete_list_book": {"id": 9, "list_id": 40}})),
    );
    c.remove_from_list(ListBookId::new(9)).unwrap();
    assert_eq!(vars(&http, 0), json!({"id": 9}));
    http.push(
        200,
        &data(json!({"delete_list_book": {"id": null, "list_id": null}})),
    );
    assert!(matches!(
        c.remove_from_list(ListBookId::new(9)),
        Err(Error::Api(_))
    ));
}

fn review_row(id: i64, user: i64, text: Option<&str>) -> Value {
    json!({
        "id": id,
        "rating": 4.0,
        "likes_count": 3,
        "reviewed_at": "2026-02-22T00:00:00",
        "review_has_spoilers": false,
        "review_raw": text,
        "user": {
            "id": user,
            "username": format!("reader{user}"),
            "name": "",
            "cached_image": {"url": format!("https://assets.example/users/{user}.png"),
                             "color": "#336699", "width": 64, "height": 64},
        },
    })
}

#[test]
fn book_reviews_page_by_offset_and_ask_one_row_ahead() {
    let (c, http) = client(TestTokens::new(false));
    // Eleven rows: a full page and one more. The second row has no text.
    let rows: Vec<Value> = (1..=11)
        .map(|i| {
            review_row(
                i,
                100 + i,
                (i != 2).then_some("A synthetic review.\n\nTwo paragraphs."),
            )
        })
        .collect();
    http.push(200, &data(json!({ "user_books": rows })));
    let page = c
        .book_reviews(BookId::new(1001), UserId::new(77), 20)
        .unwrap();
    assert_eq!(
        vars(&http, 0),
        json!({"bookId": 1001, "me": 77, "limit": 11, "offset": 20})
    );
    assert!(page.has_more);
    // The textless row is left out but still counts towards the offset; the
    // eleventh row is the next page's.
    assert_eq!(page.reviews.len(), 9);
    assert_eq!(page.next_offset, 30);
    let first = &page.reviews[0];
    assert_eq!(first.id, UserBookId::new(1));
    assert_eq!(first.reviewer.display_name(), "@reader101");
    assert_eq!(first.reviewer.color, Some([0x33, 0x66, 0x99]));
    assert_eq!(first.rating, Rating::new(4.0));
    assert_eq!(first.likes, 3);
    assert_eq!(
        first.reviewed_at,
        chrono::NaiveDate::from_ymd_opt(2026, 2, 22)
    );
    assert_eq!(first.paragraphs().count(), 2);

    http.push(
        200,
        &data(json!({ "user_books": [review_row(31, 131, Some("Last."))] })),
    );
    let last = c
        .book_reviews(BookId::new(1001), UserId::new(77), 30)
        .unwrap();
    assert!(!last.has_more);
    assert_eq!(last.next_offset, 31);
    assert_eq!(last.reviews.len(), 1);
}

#[test]
fn book_reviews_count_reads_the_aggregate() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({ "user_books_aggregate": { "aggregate": { "count": 42 } } })),
    );
    assert_eq!(
        c.book_reviews_count(BookId::new(1001), UserId::new(77))
            .unwrap(),
        42
    );
    assert_eq!(vars(&http, 0), json!({"bookId": 1001, "me": 77}));
    let body = http.requests()[0].json.clone().unwrap();
    assert_eq!(body["operationName"], "BookReviewsCount");
}

#[test]
fn upcoming_in_series_merges_books_and_names_every_series() {
    let (c, http) = client(TestTokens::new(false));
    let today: Date = "2026-11-20".parse().unwrap();
    assert_eq!(c.upcoming_in_series(&[], today).unwrap(), []);
    assert_eq!(http.requests().len(), 0, "no series, no request");

    let upcoming = |id: i64, title: &str, date: Option<&str>, partial: Option<bool>| {
        let mut b = book(id, title);
        b["release_date"] = json!(date);
        b["is_partial_book"] = json!(partial);
        b
    };
    let series = |id: i64, name: &str| json!({ "id": id, "name": name, "slug": null });
    http.push(
        200,
        &data(json!({ "book_series": [
            {"position": 2.5, "compilation": false, "series": series(41, "Lantern Cycle"),
             "book": upcoming(201, "The Ember Interlude", Some("2026-12-08"), Some(false))},
            {"position": null, "compilation": false, "series": series(42, "Tidewright"),
             "book": upcoming(202, "Salt Roads", Some("2026-12-20"), None)},
            {"position": 3.0, "compilation": false, "series": series(43, "Tidewright Reading Order"),
             "book": upcoming(202, "Salt Roads", Some("2026-12-20"), None)},
            {"position": 1.0, "compilation": true, "series": series(41, "Lantern Cycle"),
             "book": upcoming(203, "Lantern Cycle Omnibus", Some("2027-01-05"), Some(false))},
            {"position": 4.0, "compilation": false, "series": series(41, "Lantern Cycle"),
             "book": upcoming(204, "Lantern, Part One", Some("2027-02-01"), Some(true))},
            {"position": 5.0, "compilation": false, "series": series(41, "Lantern Cycle"),
             "book": upcoming(205, "Untitled Lantern Cycle #5", None, None)},
            {"position": 6.0, "compilation": false, "series": series(41, "Lantern Cycle"),
             "book": null},
            {"position": 6.0, "compilation": false, "series": series(41, "Lantern Cycle"),
             "book": upcoming(206, "Untitled Lantern Cycle #6", Some("2030-01-01"), None)}
        ]})),
    );
    let ids = [
        SeriesId::new(43),
        SeriesId::new(41),
        SeriesId::new(42),
        SeriesId::new(41),
    ];
    let books = c.upcoming_in_series(&ids, today).unwrap();

    let req = &http.requests()[0];
    let body = req.json.as_ref().unwrap();
    assert_eq!(body["operationName"], "UpcomingInSeries");
    assert_eq!(
        body["variables"]["series"],
        json!([41, 42, 43]),
        "sorted, once each"
    );
    assert_eq!(body["variables"]["after"], "2026-11-20");
    assert_eq!(body["variables"]["limit"], 200);

    let titles: Vec<&str> = books.iter().map(|b| b.book.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "The Ember Interlude",
            "Salt Roads",
            "Untitled Lantern Cycle #6"
        ]
    );
    let ember = &books[0];
    assert_eq!(ember.release_date, "2026-12-08".parse::<Date>().unwrap());
    assert_eq!(ember.book.authors, ["Jane Quill"]);
    assert_eq!(ember.series.len(), 1);
    assert_eq!(ember.series[0].id, Some(SeriesId::new(41)));
    assert_eq!(ember.series[0].position, Some(2.5));

    // Listed under two series: one book, naming both.
    let salt = &books[1];
    let names: Vec<(&str, Option<f32>)> = salt
        .series
        .iter()
        .map(|s| (s.name.as_str(), s.position))
        .collect();
    assert_eq!(
        names,
        [
            ("Tidewright", None),
            ("Tidewright Reading Order", Some(3.0))
        ]
    );
}

/// One failed batch doesn't throw away the series the others loaded.
#[test]
fn a_failed_series_batch_keeps_the_others() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, r#"{"errors":[{"message":"series 42: bad row"}]}"#);
    http.push(
        200,
        &data(
            json!({ "series": [{ "id": 78, "name": "Synthetic", "slug": null,
            "primary_books_count": null, "is_completed": null, "author": null,
            "book_series": [] }]}),
        ),
    );
    let ids: Vec<SeriesId> = (1..=26).map(|i| SeriesId::new(i * 3)).collect();
    let load = c.tracked_series(&ids);
    assert_eq!(load.series.len(), 1);
    assert_eq!(load.series[0].id, SeriesId::new(78));
    assert_eq!(load.missing, ids[..25]);
    assert!(matches!(load.error, Some(Error::Api(_))));
}

/// A failure that would repeat for every batch (the network, the session,
/// the rate limit) stops the load: the rest aren't sent, and are missing.
#[test]
fn a_lasting_series_failure_stops_the_batches() {
    let (c, http) = client(TestTokens::new(false));
    http.push_err(Error::Timeout);
    let ids: Vec<SeriesId> = (1..=60).map(SeriesId::new).collect();
    let load = c.tracked_series(&ids);
    assert_eq!(http.requests().len(), 1);
    assert!(load.series.is_empty());
    assert_eq!(load.missing, ids);
    assert_eq!(load.error, Some(Error::Timeout));
}

#[test]
fn tracked_series_batches_ids_and_keeps_every_candidate() {
    let (c, http) = client(TestTokens::new(false));
    let none = c.tracked_series(&[]);
    assert!(none.series.is_empty() && none.error.is_none());
    assert_eq!(http.requests().len(), 0, "no ids, no request");

    let row = |position: Option<f64>, compilation: bool, book: Value| json!({ "position": position, "compilation": compilation, "book": book });
    let tracked = |id: i64, users: i64, date: Option<&str>, canonical: Option<i64>, partial| {
        let mut b = book(id, &format!("Book {id}"));
        b["users_count"] = json!(users);
        b["release_date"] = json!(date);
        b["canonical_id"] = json!(canonical);
        b["is_partial_book"] = json!(partial);
        b
    };
    // 26 ids: two requests, 25 and 1.
    http.push(
        200,
        &data(json!({ "series": [{
            "id": 41, "name": " Lantern Cycle ", "slug": "lantern-cycle",
            "primary_books_count": 3, "is_completed": false,
            "author": { "id": 7, "name": "Ada Quill" },
            "book_series": [
                row(Some(2.0), false, tracked(102, 40, Some("2024-03-01"), None, false)),
                row(Some(1.0), false, tracked(101, 90, Some("2022-01-10"), None, false)),
                // Another book at the same position: kept, most-read first.
                row(Some(1.0), false, tracked(111, 10, None, None, false)),
                row(Some(1.0), false, tracked(112, 500, None, Some(101), false)),
                row(Some(1.5), false, tracked(115, 5, Some("2027-02-01"), None, true)),
                row(Some(1.0), true, tracked(190, 900, None, None, false)),
                row(None, false, tracked(191, 900, None, None, false)),
                row(Some(-1.0), false, tracked(192, 900, None, None, false)),
                row(Some(3.0), false, Value::Null),
                row(Some(2.0), false, tracked(102, 40, Some("2024-03-01"), None, false)),
            ],
        }]})),
    );
    http.push(
        200,
        &data(json!({ "series": [{ "id": 77, "name": null, "slug": null,
            "primary_books_count": null, "is_completed": null, "author": null,
            "book_series": [] }]})),
    );
    let ids: Vec<SeriesId> = (1..=26).rev().map(|i| SeriesId::new(i * 3)).collect();
    let load = c.tracked_series(&ids);
    assert!(load.error.is_none() && load.missing.is_empty());
    let series = load.series;

    let reqs = http.requests();
    assert_eq!(reqs.len(), 2);
    let first = &reqs[0].json.as_ref().unwrap()["variables"];
    assert_eq!(
        reqs[0].json.as_ref().unwrap()["operationName"],
        "SeriesOverview"
    );
    assert_eq!(first["ids"].as_array().unwrap().len(), 25);
    assert_eq!(first["ids"][0], 3, "sorted");
    assert_eq!(first["limit"], 25);
    assert_eq!(first["rows"], 200);
    let second = &reqs[1].json.as_ref().unwrap()["variables"];
    assert_eq!(second["ids"], json!([78]));
    assert_eq!(second["limit"], 1);

    assert_eq!(series.len(), 2);
    let s = &series[0];
    assert_eq!(s.id, SeriesId::new(41));
    assert_eq!(s.name, "Lantern Cycle");
    assert_eq!(s.slug.as_deref(), Some("lantern-cycle"));
    assert_eq!(
        s.author.as_ref().map(|a| a.name.as_str()),
        Some("Ada Quill")
    );
    assert_eq!(s.primary_books_count, Some(3));
    assert_eq!(s.is_completed, Some(false));
    let books: Vec<(i64, f32)> = s
        .books
        .iter()
        .map(|b| (b.book.id.get(), b.position))
        .collect();
    assert_eq!(books, [(101, 1.0), (111, 1.0), (115, 1.5), (102, 2.0)]);
    assert_eq!(s.books[0].release_date, Some("2022-01-10".parse().unwrap()));
    assert_eq!(s.books[1].release_date, None);
    assert!(s.books[2].partial);
    assert!(!s.books[0].partial);
    assert_eq!(series[1].name, "Untitled series");
    assert_eq!(series[1].books, []);
}
