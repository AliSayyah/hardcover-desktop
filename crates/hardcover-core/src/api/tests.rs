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
    json!({ "data": v }).to_string()
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
    let first: Vec<Value> = (1..=100).map(|i| user_book(i, 1 + i % 5)).collect();
    // Second page holds one shelf entry and one with a status id we don't know.
    let second = vec![user_book(150, 3), user_book(151, 42)];
    http.push(200, &data(json!({ "user_books": first })));
    http.push(200, &data(json!({ "user_books": second })));
    let mut pages = Vec::new();
    let lib = c
        .library_with_progress(UserId::new(77), |p| pages.push(p.len()))
        .unwrap();
    assert_eq!(lib.len(), 101);
    assert_eq!(pages, vec![100, 1]);
    let reqs = http.requests();
    assert_eq!(reqs.len(), 2);
    let v0 = &reqs[0].json.as_ref().unwrap()["variables"];
    let v1 = &reqs[1].json.as_ref().unwrap()["variables"];
    assert_eq!(
        (
            v0["userId"].as_i64(),
            v0["afterId"].as_i64(),
            v0["limit"].as_i64()
        ),
        (Some(77), Some(0), Some(100))
    );
    assert_eq!(v1["afterId"].as_i64(), Some(100));

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
}

#[test]
fn library_stops_on_empty_first_page() {
    let (c, http) = client(TestTokens::new(false));
    http.push(200, &data(json!({ "user_books": [] })));
    assert!(c.library(UserId::new(1)).unwrap().is_empty());
    assert_eq!(http.requests().len(), 1);
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
    assert!(c.search_books("   ", 1).unwrap().books.is_empty());
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
    assert!(p.books.is_empty());
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

    assert!(c.search_series("  ", 1).unwrap().hits.is_empty());
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
    assert!(c.trending_books(TrendingPeriod::Week).unwrap().is_empty());
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
    assert!(d.tags.is_empty());
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
    assert!(d.ratings_distribution.is_empty());
    assert!(d.alternative_titles.is_empty());
    assert!(d.similar_book_ids.is_empty());

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
    assert!(c.books_by_ids(&[]).unwrap().is_empty());
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
        run(
            429,
            &[("Retry-After", "86400")],
            r#"{"error":"Too Many Requests","message":"slow down"}"#
        ),
        Error::RateLimited {
            retry_after: Some(Duration::from_secs(86400))
        }
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

    // Daily limit (long wait) or no Retry-After: no retry.
    http.push_with_headers(429, &[("retry-after", "3600")], "{}");
    assert_eq!(
        c.me(),
        Err(Error::RateLimited {
            retry_after: Some(Duration::from_secs(3600))
        })
    );
    assert_eq!(http.remaining(), 0);
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
    assert_eq!(
        reqs[1].json.as_ref().unwrap()["variables"],
        json!({"id": 3, "object": {"status_id": 2}})
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
    http.push(200, &data(json!({"user_books": [{"id": 3, "status_id": 2, "book": {"pages": null, "audio_seconds": 3600}, "edition": null, "user_book_reads": []}]})));
    http.push(200, &data(json!({"insert_user_book_read": {"error": null, "id": 95, "user_book_read": {"id": 95, "user_book_id": 3}}})));
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 2)})));
    c.update_progress(UserBookId::new(3), None, ProgressUpdate::Percent(10.0))
        .unwrap();
    let reqs = http.requests();
    let vars = &reqs[1].json.as_ref().unwrap()["variables"];
    assert_eq!(vars["userBookId"], 3);
    assert_eq!(vars["read"]["progress_seconds"], 360);
    assert!(vars["read"]["started_at"].as_str().unwrap().len() == 10);
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
    let r = e.current_read().unwrap().clone();
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
    let lib = c.library(UserId::new(77)).unwrap();
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
    let editions = c.book_editions(BookId::new(1001), 50).unwrap();
    assert_eq!(
        vars(&http, 0),
        json!({"bookId": 1001, "limit": EDITIONS_PER_PAGE, "offset": 50})
    );
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
fn mark_owned_sends_the_edition() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"edition_owned": {"id": 77, "list_book": {"id": 77, "edition_id": 5003}}})),
    );
    c.mark_owned(EditionId::new(5003)).unwrap();
    assert_eq!(vars(&http, 0), json!({"id": 5003}));
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
fn an_unfinished_read_never_scales_to_the_last_unit() {
    assert_eq!(super::scale(0.5, 300), 150);
    assert_eq!(super::scale(0.997, 150), 149);
    assert_eq!(super::scale(1.0, 150), 150);
    assert_eq!(super::scale(0.5, 0), 0);
}

fn goal_row(id: i64, metric: &str, goal: i64, progress: Value) -> Value {
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
            goal_row(5, "book", 30, json!("12")),
            goal_row(6, "hour", 100, json!(36000)),
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
        &data(json!({"insert_goal": {"errors": null, "id": 5, "goal": goal_row(5, "book", 30, json!(0))}})),
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

#[test]
fn update_goal_resends_everything_and_refetches_when_needed() {
    let (c, http) = client(TestTokens::new(false));
    let mut row = goal_row(5, "page", 9000, json!(1200));
    row["conditions"] = json!({"readingFormatId": 1, "authorBipoc": null});
    // A payload without the goal: re-read by id.
    http.push(
        200,
        &data(json!({"update_goal": {"errors": [], "id": 5, "goal": null}})),
    );
    http.push(200, &data(json!({"goals_by_pk": row})));
    let mut goal = RawGoal::deserialize(goal_row(5, "page", 9000, json!(1200)))
        .unwrap()
        .into_goal()
        .unwrap();
    goal.conditions = GoalConditions(json!({"readingFormatId": 1, "authorBipoc": null}));
    goal.target = 10000;
    let saved = c.update_goal(&goal).unwrap();
    assert_eq!(saved.id, GoalId::new(5));
    let reqs = http.requests();
    assert_eq!(
        reqs[0].json.as_ref().unwrap()["variables"],
        json!({"id": 5, "object": {
            "metric": "page",
            "goal": 10000,
            "description": "9000 pages in 2030",
            "start_date": "2030-01-01",
            "end_date": "2030-12-31",
            "conditions": {"readingFormatId": 1},
            "archived": false,
            "privacy_setting_id": 2,
        }})
    );
    assert_eq!(reqs[1].json.as_ref().unwrap()["operationName"], "GoalById");

    // Conditions it can't send back block the update before any request.
    let conditions = std::mem::replace(
        &mut goal.conditions,
        GoalConditions(json!({"genre": "poetry"})),
    );
    assert!(matches!(c.update_goal(&goal), Err(Error::InvalidInput(_))));
    assert_eq!(http.requests().len(), 2);
    goal.conditions = conditions;

    // So does a missing description, which Hardcover requires.
    for description in [None, Some("  ".to_owned())] {
        goal.description = description;
        assert!(matches!(c.update_goal(&goal), Err(Error::InvalidInput(_))));
    }
    assert_eq!(http.requests().len(), 2);
}

#[test]
fn update_goal_keeps_a_privacy_setting_it_does_not_know() {
    let (c, http) = client(TestTokens::new(false));
    let mut row = goal_row(5, "book", 30, json!(3));
    row["privacy_setting_id"] = json!(7);
    let mut goal = RawGoal::deserialize(row.clone())
        .unwrap()
        .into_goal()
        .unwrap();
    assert_eq!((goal.privacy_id, goal.privacy()), (Some(7), None));
    http.push(
        200,
        &data(json!({"update_goal": {"errors": [], "id": 5, "goal": row}})),
    );
    goal.target = 40;
    c.update_goal(&goal).unwrap();
    assert_eq!(
        http.requests()[0].json.as_ref().unwrap()["variables"]["object"]["privacy_setting_id"],
        7
    );
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
    entry
        .as_object_mut()
        .unwrap()
        .extend(review.as_object().unwrap().clone());
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
    assert_eq!(written.entry.user_book_id, UserBookId::new(3));
    assert_eq!(written.entry.status, ReadingStatus::CurrentlyReading);
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
fn delete_review_sends_nulls_and_reports_what_hardcover_kept() {
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
    assert_eq!(
        vars(&http, 0),
        json!({
            "id": 3,
            "statusId": 3,
            "markdown": null,
            "spoilers": false,
            "reviewedAt": null,
        })
    );

    // A review Hardcover did not clear is reported as still there.
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
    assert_eq!(written.review.text(), Some("Still here."));

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
fn review_write_without_an_entry_reads_both_back() {
    let (c, http) = client(TestTokens::new(false));
    http.push(
        200,
        &data(json!({"update_user_book": {"error": null, "id": 3, "user_book": null}})),
    );
    http.push(200, &data(json!({"user_books_by_pk": user_book(3, 3)})));
    http.push(
        200,
        &data(json!({"user_books_by_pk": {
            "id": 3, "review_markdown": "Short and sharp.", "review_raw": null,
            "review_has_spoilers": true, "reviewed_at": "2024-06-07T00:00:00",
        }})),
    );
    let written = c
        .set_review(
            UserBookId::new(3),
            "Short and sharp.",
            true,
            None,
            ReadingStatus::Read,
        )
        .unwrap();
    assert_eq!(written.entry.status, ReadingStatus::Read);
    assert!(written.review.has_spoilers);
    let ops: Vec<Value> = http
        .requests()
        .iter()
        .map(|r| r.json.as_ref().unwrap()["operationName"].clone())
        .collect();
    assert_eq!(
        ops,
        vec![
            json!("UpdateUserBookReview"),
            json!("UserBookById"),
            json!("UserBookReview")
        ]
    );
}
