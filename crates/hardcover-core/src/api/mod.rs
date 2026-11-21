//! Blocking GraphQL client for the Hardcover API.

mod model;
mod queries;
#[cfg(test)]
pub(crate) mod sdl;
mod wire;

use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

pub use model::*;

/// Every GraphQL document the client sends, as `(name, document)`. For tooling
/// that checks them against the live API (the `api_check` example).
#[doc(hidden)]
pub const DOCUMENTS: &[(&str, &str)] = queries::ALL;

use crate::http::{HttpRequest, HttpResponse, HttpTransport, Method, UreqTransport};
pub use crate::ratelimit::Priority;
use crate::ratelimit::TokenBucket;
use crate::{Error, Result};
pub(crate) use wire::as_i64 as wire_i64;
use wire::{
    DeletePayload, DeleteReadPayload, GoalPayload, ListBookDeletePayload, ListBookPayload,
    ListDeletePayload, ListPayload, ProgressContext, RawAggregate, RawAuthorPage, RawBook,
    RawBookDetails, RawCommunityReview, RawEdition, RawGoal, RawList, RawListBook,
    RawListMembership, RawReview, RawSearch, RawSeriesPage, RawTrackedSeries, RawTrending,
    RawUpcomingRow, RawUser, RawUserBook, ReadIdPayload, ReadRef, ReviewedUserBookPayload,
    UserBookIdPayload, author_from_search_document, search_hits, search_page,
    series_from_search_document, upcoming_books,
};

/// GraphQL endpoint.
pub const GRAPHQL_ENDPOINT: &str = "https://api.hardcover.app/v1/graphql";

/// Library page size. One request per page; each costs one rate-limit token.
/// Measured live on 834 entries (2026-11-21): 100 a page took 9 requests and
/// 3.6 s, 500 took 2 requests and 1.3 s, 1,000 one request of 3.2 s. Every
/// size returned all 834, so Hardcover doesn't cap the rows below 1,000.
const LIBRARY_PAGE_SIZE: u32 = 500;
/// Search results per page.
pub const SEARCH_PER_PAGE: u32 = 25;
/// Editions per page in [`Client::book_editions`].
pub const EDITIONS_PER_PAGE: u32 = 50;
/// Rows a series page asks for, duplicates included. Long-running series with
/// many unmerged translations can exceed it; the most-read come first.
const SERIES_ROWS_LIMIT: u32 = 200;
/// Books an author page lists: Hardcover caps nested contributions at 100.
pub const AUTHOR_BOOKS_LIMIT: u32 = 100;
/// Most books [`Client::books_by_ids`] asks for in one request.
pub const BOOKS_BY_IDS_LIMIT: usize = 25;
/// Books [`Client::trending_books`] asks for: all of them fit one
/// [`Client::books_by_ids`] request.
pub const TRENDING_LIMIT: usize = BOOKS_BY_IDS_LIMIT;
/// Lists per request: Hardcover returns at most 100 `lists` rows.
const LISTS_PAGE_SIZE: u32 = 100;
/// List memberships per request: Hardcover returns at most 1,000 `list_books` rows.
const MEMBERSHIPS_PAGE_SIZE: u32 = 1_000;
/// Most requests [`Client::list_memberships`] makes, so a huge collection
/// can't spend a minute of the rate limit at once.
const MEMBERSHIPS_MAX_PAGES: usize = 5;
/// Books per page in [`Client::list_books`].
pub const LIST_BOOKS_PER_PAGE: u32 = 100;
/// Reviews per page in [`Client::book_reviews`].
pub const REVIEWS_PER_PAGE: u32 = 10;
/// Rows [`Client::upcoming_in_series`] asks for. A library of 210 series had
/// 27 in 2026.
const UPCOMING_ROWS_LIMIT: u32 = 200;
/// Series per [`Client::tracked_series`] request: a reader with 200 started
/// series costs 8 requests, within the rate limit's burst.
pub const TRACKED_SERIES_BATCH: usize = 25;
/// Books each series in [`Client::tracked_series`] lists at most, duplicates
/// included. Ordered by position, so a longer series loses only its tail.
const TRACKED_SERIES_ROWS: u32 = 200;

/// The server cuts queries off at 30s and searches at 2s; leave room for the network.
const QUERY_TIMEOUT: Duration = Duration::from_secs(40);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
/// A 429 is retried once if the retry can go out within this long: the
/// whole wait, the server's block and the queue for the rate limit both.
const MAX_AUTO_RETRY_WAIT: Duration = Duration::from_secs(15);

/// Reads that are a step of a write (the context it needs, a read-back of
/// what it wrote, the row it edits): the user waits on them as on the write.
const WRITE_STEP_READS: &[&str] = &["ProgressContext", "UserBookById", "GoalById", "ListById"];

/// Supplies bearer tokens to the [`Client`].
pub trait TokenSource: Send + Sync {
    /// A token to send now. May refresh first if the current one is about to expire.
    ///
    /// # Errors
    ///
    /// Returns an error if the access token cannot be obtained or refreshed.
    fn access_token(&self) -> Result<String>;

    /// The API rejected `rejected` with HTTP 401. Return `Ok(true)` if a different
    /// token is now available (the client retries once), `Ok(false)` to give up
    /// with [`Error::Unauthorized`].
    ///
    /// `rejected` lets an implementation see that another thread already refreshed,
    /// so concurrent 401s do not spend a rotating refresh token twice.
    ///
    /// # Errors
    ///
    /// Returns an error if the token source cannot recover from the rejection.
    fn on_unauthorized(&self, rejected: &str) -> Result<bool>;
}

/// Hardcover API client. Cheap to clone; clones share the rate limiter, the
/// token source and the cached `me` id.
///
/// Every method blocks (including while waiting for the client-side rate limiter)
/// and must run off the UI thread.
///
/// Requests that wait for the rate limit go in two lanes (see [`Priority`]).
/// Every write, every search and every read that is a step of a write is
/// [`Priority::Interactive`] whatever the client's priority, so no call
/// site can forget it; the rest goes at the client's own priority,
/// [`Priority::Background`] unless [`Client::with_priority`] says otherwise.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
    priority: Priority,
}

struct Inner {
    tokens: Arc<dyn TokenSource>,
    http: Arc<dyn HttpTransport>,
    bucket: TokenBucket,
    endpoint: String,
    me_id: OnceLock<UserId>,
    /// Held while `me` is fetched for [`Client::my_id`], so concurrent
    /// callers wait for one request instead of each sending their own.
    me_fetch: Mutex<()>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("endpoint", &self.inner.endpoint)
            .finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(tokens: Arc<dyn TokenSource>) -> Self {
        Self::with_transport(
            tokens,
            Arc::new(UreqTransport::new()),
            TokenBucket::default(),
        )
    }

    pub(crate) fn with_transport(
        tokens: Arc<dyn TokenSource>,
        http: Arc<dyn HttpTransport>,
        bucket: TokenBucket,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                tokens,
                http,
                bucket,
                endpoint: GRAPHQL_ENDPOINT.to_owned(),
                me_id: OnceLock::new(),
                me_fetch: Mutex::new(()),
            }),
            priority: Priority::Background,
        }
    }

    /// A clone whose reads wait for the rate limit at `priority`. Writes,
    /// searches and the reads that are steps of a write are always
    /// [`Priority::Interactive`].
    #[must_use]
    pub fn with_priority(&self, priority: Priority) -> Self {
        Self {
            inner: self.inner.clone(),
            priority,
        }
    }

    /// The priority this client's reads wait at.
    #[must_use]
    pub fn priority(&self) -> Priority {
        self.priority
    }

    // ----- reads -----

    /// The signed-in user. Also remembers their id for [`Client::library_entry`].
    ///
    /// # Errors
    ///
    /// Returns an error if the account request fails or the response contains no
    /// user.
    pub fn me(&self) -> Result<User> {
        #[derive(Deserialize)]
        struct Data {
            me: Vec<RawUser>,
        }
        let data: Data = self.query("Me", queries::ME, json!({}), QUERY_TIMEOUT)?;
        let user = data
            .me
            .into_iter()
            .next()
            .ok_or_else(|| Error::Decode("`me` returned no user".into()))?
            .into_user();
        let _ = self.inner.me_id.set(user.id);
        Ok(user)
    }

    /// The signed-in user's id, fetched once. Concurrent first callers share
    /// one request; a failure isn't remembered, so the next call tries again.
    fn my_id(&self) -> Result<UserId> {
        if let Some(id) = self.inner.me_id.get() {
            return Ok(*id);
        }
        let _one_at_a_time = self
            .inner
            .me_fetch
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match self.inner.me_id.get() {
            Some(id) => Ok(*id),
            None => Ok(self.me()?.id),
        }
    }

    /// The user's whole library, Ignored entries included, ordered by
    /// `user_book_id`. Pages of [`LIBRARY_PAGE_SIZE`], one request each,
    /// with every read nested. See [`LibrarySync::coverage`] for what the
    /// result vouches for.
    ///
    /// # Errors
    ///
    /// Returns an error if any library page cannot be fetched or decoded.
    pub fn library(&self, user: UserId) -> Result<LibrarySync> {
        self.library_with_progress(user, |_| {})
    }

    /// Like [`Client::library`], calling `on_page` with each page as it arrives
    /// so the UI can show partial results.
    ///
    /// # Errors
    ///
    /// Returns an error if any library page cannot be fetched or decoded.
    pub fn library_with_progress(
        &self,
        user: UserId,
        mut on_page: impl FnMut(&[LibraryEntry]),
    ) -> Result<LibrarySync> {
        #[derive(Deserialize)]
        struct Data {
            user_books: Vec<RawUserBook>,
        }
        let mut entries = Vec::new();
        let mut unreadable = Vec::new();
        let complete = keyset_pages(
            None,
            |after| {
                let data: Data = self.query(
                    "LibraryPage",
                    queries::LIBRARY_PAGE,
                    json!({ "userId": user.get(), "afterId": after, "limit": LIBRARY_PAGE_SIZE }),
                    QUERY_TIMEOUT,
                )?;
                Ok(data.user_books)
            },
            |raw| raw.id,
            |rows| {
                let mut page = Vec::with_capacity(rows.len());
                for raw in rows {
                    let id = UserBookId::new(raw.id);
                    match raw.into_entry() {
                        Some(entry) => page.push(entry),
                        None => unreadable.push(id),
                    }
                }
                on_page(&page);
                entries.extend(page);
            },
        )?;
        Ok(LibrarySync {
            entries,
            coverage: if complete {
                Coverage::Complete { unreadable }
            } else {
                Coverage::Partial
            },
        })
    }

    /// One of the signed-in user's entries. `None` if it does not exist or belongs
    /// to someone else (ids are global, and other users' public entries are readable).
    ///
    /// # Errors
    ///
    /// Returns an error if the account or entry lookup fails.
    pub fn library_entry(&self, id: UserBookId) -> Result<Option<LibraryEntry>> {
        let me = self.my_id()?;
        self.fetch_entry(
            "UserBookForUser",
            queries::USER_BOOK_FOR_USER,
            json!({ "userId": me.get(), "id": id.get() }),
        )
    }

    /// The user's entry for `book`, whatever its status (including Ignored).
    ///
    /// # Errors
    ///
    /// Returns an error if the account or entry lookup fails.
    pub fn library_entry_for_book(
        &self,
        user: UserId,
        book: BookId,
    ) -> Result<Option<LibraryEntry>> {
        self.fetch_entry(
            "UserBookForBook",
            queries::USER_BOOK_FOR_BOOK,
            json!({ "userId": user.get(), "bookId": book.get() }),
        )
    }

    fn entry_by_id(&self, id: i64) -> Result<Option<LibraryEntry>> {
        #[derive(Deserialize)]
        struct Data {
            user_books_by_pk: Option<RawUserBook>,
        }
        let data: Data = self.query(
            "UserBookById",
            queries::USER_BOOK_BY_ID,
            json!({ "id": id }),
            QUERY_TIMEOUT,
        )?;
        Ok(data.user_books_by_pk.and_then(RawUserBook::into_entry))
    }

    fn fetch_entry(&self, op: &str, doc: &str, vars: Value) -> Result<Option<LibraryEntry>> {
        #[derive(Deserialize)]
        struct Data {
            user_books: Vec<RawUserBook>,
        }
        let data: Data = self.query(op, doc, vars, QUERY_TIMEOUT)?;
        Ok(data
            .user_books
            .into_iter()
            .next()
            .and_then(RawUserBook::into_entry))
    }

    /// Book search (Hardcover's Typesense index, same ranking as the website).
    /// `page` is 1-based; a blank query returns an empty page without a request.
    ///
    /// # Errors
    ///
    /// Returns an error if the search request fails or the search service reports an
    /// error.
    pub fn search_books(&self, query: &str, page: u32) -> Result<SearchPage> {
        let query = query.trim();
        let page = page.max(1);
        if query.is_empty() {
            return Ok(SearchPage::new(Vec::new(), page, SEARCH_PER_PAGE));
        }
        let raw = self.raw_search("SearchBooks", queries::SEARCH_BOOKS, query, page)?;
        Ok(search_page(&raw, page, SEARCH_PER_PAGE))
    }

    /// Author search, ranked as on the website. Like [`Client::search_books`]
    /// otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if the search request fails or the search service reports an
    /// error.
    pub fn search_authors(&self, query: &str, page: u32) -> Result<SearchHits<AuthorHit>> {
        self.search_hits(
            "SearchAuthors",
            queries::SEARCH_AUTHORS,
            query,
            page,
            author_from_search_document,
        )
    }

    /// Series search, ranked as on the website. Like [`Client::search_books`]
    /// otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if the search request fails or the search service reports an
    /// error.
    pub fn search_series(&self, query: &str, page: u32) -> Result<SearchHits<SeriesHit>> {
        self.search_hits(
            "SearchSeries",
            queries::SEARCH_SERIES,
            query,
            page,
            series_from_search_document,
        )
    }

    fn search_hits<T>(
        &self,
        op: &str,
        doc: &str,
        query: &str,
        page: u32,
        parse: impl Fn(&Value, Option<i64>) -> Option<T>,
    ) -> Result<SearchHits<T>> {
        let query = query.trim();
        let page = page.max(1);
        if query.is_empty() {
            return Ok(SearchHits::new(Vec::new(), page, SEARCH_PER_PAGE));
        }
        let raw = self.raw_search(op, doc, query, page)?;
        Ok(search_hits(&raw, page, SEARCH_PER_PAGE, parse))
    }

    fn raw_search(&self, op: &str, doc: &str, query: &str, page: u32) -> Result<RawSearch> {
        #[derive(Deserialize)]
        struct Data {
            search: Option<RawSearch>,
        }
        let data: Data = self.query(
            op,
            doc,
            json!({ "query": query, "page": page, "perPage": SEARCH_PER_PAGE }),
            SEARCH_TIMEOUT,
        )?;
        let raw = data
            .search
            .ok_or_else(|| Error::Decode("`search` returned null".into()))?;
        if let Some(e) = raw.error.as_deref().filter(|e| !e.is_empty()) {
            return Err(Error::Api(e.to_owned()));
        }
        Ok(raw)
    }

    /// The books the most readers finished in `period`, in Hardcover's order,
    /// at most [`TRENDING_LIMIT`]. Two requests: the ranked ids, then the books.
    ///
    /// # Errors
    ///
    /// Returns an error if the ranking or its book details cannot be retrieved.
    pub fn trending_books(&self, period: TrendingPeriod) -> Result<Vec<BookSummary>> {
        #[derive(Deserialize)]
        struct Data {
            books_trending: Option<RawTrending>,
        }
        let data: Data = self.query(
            "BooksTrending",
            queries::BOOKS_TRENDING,
            json!({ "duration": period.api_name(), "limit": TRENDING_LIMIT }),
            QUERY_TIMEOUT,
        )?;
        let raw = data
            .books_trending
            .ok_or_else(|| Error::Decode("`books_trending` returned null".into()))?;
        if let Some(e) = raw.error.as_deref().filter(|e| !e.is_empty()) {
            return Err(Error::Api(e.to_owned()));
        }
        self.books_by_ids(&raw.into_ids())
    }

    /// Full details for the book page. `Error::Api` if the book does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested record does not exist or the catalogue
    /// request fails.
    pub fn book_details(&self, id: BookId) -> Result<BookDetails> {
        #[derive(Deserialize)]
        struct Data {
            books_by_pk: Option<RawBookDetails>,
        }
        let data: Data = self.query(
            "BookDetails",
            queries::BOOK_DETAILS,
            json!({ "id": id.get() }),
            QUERY_TIMEOUT,
        )?;
        data.books_by_pk
            .map(RawBookDetails::into_details)
            .ok_or_else(|| Error::Api(format!("book {id} not found")))
    }

    /// One page of `book`'s editions, most read first. `offset` is the
    /// previous page's [`OffsetPage::next_offset`], 0 for the first.
    ///
    /// # Errors
    ///
    /// Returns an error if the catalogue request fails or its response cannot be
    /// decoded.
    pub fn book_editions(&self, book: BookId, offset: u32) -> Result<OffsetPage<EditionSummary>> {
        #[derive(Deserialize)]
        struct Data {
            editions: Vec<RawEdition>,
        }
        let data: Data = self.query(
            "BookEditions",
            queries::BOOK_EDITIONS,
            json!({ "bookId": book.get(), "limit": EDITIONS_PER_PAGE + 1, "offset": offset }),
            QUERY_TIMEOUT,
        )?;
        Ok(offset_page(data.editions, offset, EDITIONS_PER_PAGE, |e| {
            Some(e.into_summary())
        }))
    }

    /// A series page in one request. `Error::Api` if the series does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested record does not exist or the catalogue
    /// request fails.
    pub fn series(&self, id: SeriesId) -> Result<SeriesDetails> {
        #[derive(Deserialize)]
        struct Data {
            series_by_pk: Option<RawSeriesPage>,
        }
        let data: Data = self.query(
            "SeriesPage",
            queries::SERIES_PAGE,
            json!({ "id": id.get(), "limit": SERIES_ROWS_LIMIT }),
            QUERY_TIMEOUT,
        )?;
        data.series_by_pk
            .map(RawSeriesPage::into_details)
            .ok_or_else(|| Error::Api(format!("series {id} not found")))
    }

    /// An author page in one request. `Error::Api` if the author does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested record does not exist or the catalogue
    /// request fails.
    pub fn author(&self, id: AuthorId) -> Result<AuthorDetails> {
        #[derive(Deserialize)]
        struct Data {
            authors_by_pk: Option<RawAuthorPage>,
        }
        let data: Data = self.query(
            "AuthorPage",
            queries::AUTHOR_PAGE,
            json!({ "id": id.get(), "limit": AUTHOR_BOOKS_LIMIT }),
            QUERY_TIMEOUT,
        )?;
        data.authors_by_pk
            .map(RawAuthorPage::into_details)
            .ok_or_else(|| Error::Api(format!("author {id} not found")))
    }

    /// The books with these ids, in the order given, in one request. Only the
    /// first [`BOOKS_BY_IDS_LIMIT`] ids are used; ids Hardcover does not know
    /// are left out. No ids, no request.
    ///
    /// # Errors
    ///
    /// Returns an error if the catalogue request fails or its response cannot be
    /// decoded.
    pub fn books_by_ids(&self, ids: &[BookId]) -> Result<Vec<BookSummary>> {
        #[derive(Deserialize)]
        struct Data {
            books: Vec<RawBook>,
        }
        let ids: Vec<i64> = ids
            .iter()
            .take(BOOKS_BY_IDS_LIMIT)
            .map(|id| id.get())
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let data: Data = self.query(
            "BooksByIds",
            queries::BOOKS_BY_IDS,
            json!({ "ids": ids, "limit": ids.len() }),
            QUERY_TIMEOUT,
        )?;
        let mut books: Vec<BookSummary> =
            data.books.into_iter().map(RawBook::into_summary).collect();
        books.sort_by_key(|b| ids.iter().position(|id| *id == b.id.get()));
        Ok(books)
    }

    /// Books in these series released after `after`, the soonest first, in
    /// one request. A book listed under several of the series comes once,
    /// naming each. Merged duplicates, compilations and partial books are
    /// left out. No series, no request.
    ///
    /// # Errors
    ///
    /// Returns an error if the catalogue request fails or its response cannot be
    /// decoded.
    pub fn upcoming_in_series(
        &self,
        series: &[SeriesId],
        after: Date,
    ) -> Result<Vec<UpcomingBook>> {
        #[derive(Deserialize)]
        struct Data {
            book_series: Vec<RawUpcomingRow>,
        }
        let mut ids: Vec<i64> = series.iter().map(|s| s.get()).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let data: Data = self.query(
            "UpcomingInSeries",
            queries::UPCOMING_IN_SERIES,
            json!({
                "series": ids,
                "after": after.format("%Y-%m-%d").to_string(),
                "limit": UPCOMING_ROWS_LIMIT,
            }),
            QUERY_TIMEOUT,
        )?;
        Ok(upcoming_books(data.book_series))
    }

    /// The series with these ids and every book of each, [`TRACKED_SERIES_BATCH`]
    /// series per request, in the order Hardcover returns them. Ids Hardcover
    /// doesn't know are left out. No ids, no request.
    ///
    /// A batch that fails doesn't throw away the others: its ids are in
    /// [`TrackedSeriesLoad::missing`], and the first failure in
    /// [`TrackedSeriesLoad::error`]. A remembered rate-limit block refuses
    /// the remaining batches at once, so they cost no requests.
    #[must_use]
    pub fn tracked_series(&self, ids: &[SeriesId]) -> TrackedSeriesLoad {
        #[derive(Deserialize)]
        struct Data {
            series: Vec<RawTrackedSeries>,
        }
        let mut ids: Vec<i64> = ids.iter().map(|s| s.get()).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut load = TrackedSeriesLoad::default();
        for batch in ids.chunks(TRACKED_SERIES_BATCH) {
            let fetched: Result<Data> = self.query(
                "SeriesOverview",
                queries::SERIES_OVERVIEW,
                json!({ "ids": batch, "limit": batch.len(), "rows": TRACKED_SERIES_ROWS }),
                QUERY_TIMEOUT,
            );
            match fetched {
                Ok(data) => load
                    .series
                    .extend(data.series.into_iter().map(RawTrackedSeries::into_tracked)),
                Err(e) => {
                    load.missing
                        .extend(batch.iter().copied().map(SeriesId::new));
                    load.error.get_or_insert(e);
                }
            }
        }
        load
    }

    // ----- writes -----

    /// Adds `book` with `status`, using the account's default privacy. A
    /// read it starts or finishes is dated today on this computer, as
    /// [`Client::set_status`] dates it.
    ///
    /// Adding a book that already has an entry (an Ignored one, say)
    /// changes that entry's status: Hardcover returns the same entry
    /// (verified live).
    ///
    /// # Errors
    ///
    /// Returns an error if the library write fails or the updated entry cannot be
    /// retrieved.
    pub fn add_to_library(&self, book: BookId, status: ReadingStatus) -> Result<LibraryEntry> {
        let mut object = status_fields(status, today());
        object.insert("book_id".into(), json!(book.get()));
        self.insert_user_book(&Value::Object(object))
    }

    /// Like [`Client::add_to_library`] with an explicit privacy for the entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the library write fails or the updated entry cannot be
    /// retrieved.
    pub fn add_to_library_with_privacy(
        &self,
        book: BookId,
        status: ReadingStatus,
        privacy: Privacy,
    ) -> Result<LibraryEntry> {
        let mut object = status_fields(status, today());
        object.insert("book_id".into(), json!(book.get()));
        object.insert("privacy_setting_id".into(), json!(privacy.id()));
        self.insert_user_book(&Value::Object(object))
    }

    fn insert_user_book(&self, object: &Value) -> Result<LibraryEntry> {
        #[derive(Deserialize)]
        struct Data {
            insert_user_book: Option<UserBookIdPayload>,
        }
        let data: Data = self.query(
            "InsertUserBook",
            queries::INSERT_USER_BOOK,
            json!({ "object": object }),
            QUERY_TIMEOUT,
        )?;
        self.user_book_payload(data.insert_user_book, "insert_user_book")
    }

    /// Changes the status. Hardcover starts a read on Currently Reading and
    /// finishes the open read on Read; either is dated today on this
    /// computer, not in the time zone of the account's settings.
    ///
    /// # Errors
    ///
    /// Returns an error if the library write fails or the updated entry cannot be
    /// retrieved.
    pub fn set_status(&self, id: UserBookId, status: ReadingStatus) -> Result<LibraryEntry> {
        self.update_user_book(id, &Value::Object(status_fields(status, today())))
    }

    /// Sets or (with `None`) clears the rating, keeping the entry at `keep_status`.
    ///
    /// Hardcover treats a bare rating as "I read this": it sets the status to Read
    /// and finishes the open read at 100% (verified live). Sending the status the
    /// entry should keep avoids that; pass `ReadingStatus::Read` to get the
    /// server's behaviour on purpose. Pass the status the UI currently shows.
    ///
    /// # Errors
    ///
    /// Returns an error if the library write fails or the updated entry cannot be
    /// retrieved.
    pub fn set_rating(
        &self,
        id: UserBookId,
        rating: Option<Rating>,
        keep_status: ReadingStatus,
    ) -> Result<LibraryEntry> {
        let rating = rating.map(|r| f64::from(r.value()));
        self.update_user_book(
            id,
            &json!({ "rating": rating, "status_id": keep_status.id() }),
        )
    }

    /// Ties the entry to `edition`.
    ///
    /// An open read on another edition moves to `edition` too, in a second
    /// request, with its progress converted at the same percentage: page 150 of
    /// 300 becomes 5 hours of a 10-hour audiobook. Otherwise progress would be
    /// counted against one edition and shown against the other. If that second
    /// request fails, the edition change still stands: the returned entry shows
    /// the read on its old edition, which is consistent on its own, and
    /// [`EditionChange::read_left_behind`] carries the error so the caller can
    /// say so.
    ///
    /// # Errors
    ///
    /// Returns an error if the edition change fails or the updated entry cannot be
    /// retrieved.
    pub fn set_edition(&self, id: UserBookId, edition: EditionId) -> Result<EditionChange> {
        let entry = self.update_user_book(id, &json!({ "edition_id": edition.get() }))?;
        let Some(read) = entry
            .open_read()
            .filter(|r| r.edition_id != Some(edition))
            .cloned()
        else {
            return Ok(EditionChange {
                entry,
                read_left_behind: None,
            });
        };
        let fields = moved_read_fields(&read, edition, entry.edition.as_ref());
        Ok(match self.update_read(read.id, &Value::Object(fields)) {
            Ok(written) => EditionChange {
                entry: self
                    .entry_after_read_write(written, "edition change")
                    .unwrap_or(entry),
                read_left_behind: None,
            },
            Err(e) => EditionChange {
                entry,
                read_left_behind: Some(e),
            },
        })
    }

    /// Sets the entry's private notes; blank text clears them.
    ///
    /// # Errors
    ///
    /// Returns an error if the library write fails or the updated entry cannot be
    /// retrieved.
    pub fn set_private_notes(&self, id: UserBookId, notes: &str) -> Result<LibraryEntry> {
        let notes = notes.trim();
        let notes = (!notes.is_empty()).then_some(notes);
        self.update_user_book(id, &json!({ "private_notes": notes }))
    }

    /// Changes who can see the entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the library write fails or the updated entry cannot be
    /// retrieved.
    pub fn set_privacy(&self, id: UserBookId, privacy: Privacy) -> Result<LibraryEntry> {
        self.update_user_book(id, &json!({ "privacy_setting_id": privacy.id() }))
    }

    /// The review of one of the user's entries; [`Review::default`] if it has
    /// none or the entry is gone.
    ///
    /// # Errors
    ///
    /// Returns an error if the review request fails or its response cannot be
    /// decoded.
    pub fn review(&self, id: UserBookId) -> Result<Review> {
        #[derive(Deserialize)]
        struct Data {
            user_books_by_pk: Option<RawReview>,
        }
        let data: Data = self.query(
            "UserBookReview",
            queries::USER_BOOK_REVIEW,
            json!({ "id": id.get() }),
            QUERY_TIMEOUT,
        )?;
        Ok(data.user_books_by_pk.map_or_default(RawReview::into_review))
    }

    /// Writes the review, in Markdown, dated `reviewed_at`. Blank text is an
    /// error: use [`Client::delete_review`]. Needs the `write:reviews` scope.
    ///
    /// Sends `keep_status` too, as [`Client::set_rating`] does: Hardcover
    /// treats a bare rating as "I read this", and a review may be treated the
    /// same way. Pass the status the UI currently shows.
    ///
    /// # Errors
    ///
    /// Returns an error if the text is blank or Hardcover rejects the review update.
    pub fn set_review(
        &self,
        id: UserBookId,
        markdown: &str,
        has_spoilers: bool,
        reviewed_at: Option<Date>,
        keep_status: ReadingStatus,
    ) -> Result<ReviewedEntry> {
        let markdown = markdown.trim();
        if markdown.is_empty() {
            return Err(Error::InvalidInput("a review needs some text".into()));
        }
        self.update_review(
            id,
            "UpdateUserBookReview",
            queries::UPDATE_USER_BOOK_REVIEW,
            json!({
                "markdown": markdown,
                "spoilers": has_spoilers,
                "reviewedAt": reviewed_at.map(|d| d.to_string()),
            }),
            keep_status,
            Review::new(markdown, has_spoilers, reviewed_at),
        )
    }

    /// Deletes the review: its text in every form the app can write
    /// (`review_markdown` and the website's `review_slate`), its spoiler flag
    /// and its date. The entry stays.
    ///
    /// # Errors
    ///
    /// Returns an error if the request to clear the review fails.
    pub fn delete_review(
        &self,
        id: UserBookId,
        keep_status: ReadingStatus,
    ) -> Result<ReviewedEntry> {
        // Verified live (2026-11-19): Hardcover ignores null or empty
        // `review_markdown`, and null or `{}` for `review_slate`, and keeps
        // the review. An empty slate document clears it: `review_markdown`
        // becomes null, `review_raw` empty and `has_review` false.
        // `reviewed_at` can't be cleared, but means nothing without a review.
        let empty_slate = json!({ "document": { "object": "document", "children": [{
            "object": "block", "type": "paragraph", "data": {},
            "children": [{ "object": "text", "text": "" }],
        }]}});
        self.update_review(
            id,
            "DeleteUserBookReview",
            queries::DELETE_USER_BOOK_REVIEW,
            json!({ "slate": empty_slate }),
            keep_status,
            Review::default(),
        )
    }

    /// `sent` is the review as written, used when the payload has none: a
    /// read right after a write can be stale.
    fn update_review(
        &self,
        id: UserBookId,
        op: &str,
        doc: &str,
        review: Value,
        keep_status: ReadingStatus,
        sent: Review,
    ) -> Result<ReviewedEntry> {
        #[derive(Deserialize)]
        struct Data {
            update_user_book: Option<ReviewedUserBookPayload>,
        }
        let mut vars = json!({ "id": id.get(), "statusId": keep_status.id() });
        if let (Value::Object(vars), Value::Object(review)) = (&mut vars, review) {
            vars.extend(review);
        }
        let data: Data = self.query(op, doc, vars, QUERY_TIMEOUT)?;
        let p = data
            .update_user_book
            .ok_or_else(|| Error::Api("`update_user_book` returned null".into()))?;
        if let Some(e) = p.error() {
            return Err(Error::Api(e));
        }
        if let Some(written) = p.user_book
            && let Some(entry) = written.entry.into_entry()
        {
            return Ok(ReviewedEntry {
                entry: Some(entry),
                review: written.review.into_review(),
            });
        }
        // No entry in the payload. The write succeeded, so a failed read-back
        // only means the entry shown stays as it was.
        let id = p.id.map_or(id, UserBookId::new);
        Ok(ReviewedEntry {
            entry: self.entry_by_id(id.get()).ok().flatten(),
            review: sent,
        })
    }

    fn update_user_book(&self, id: UserBookId, object: &Value) -> Result<LibraryEntry> {
        #[derive(Deserialize)]
        struct Data {
            update_user_book: Option<UserBookIdPayload>,
        }
        let data: Data = self.query(
            "UpdateUserBook",
            queries::UPDATE_USER_BOOK,
            json!({ "id": id.get(), "object": object }),
            QUERY_TIMEOUT,
        )?;
        self.user_book_payload(data.update_user_book, "update_user_book")
    }

    /// Uses the entry returned by the mutation, or re-reads it if the payload had none.
    fn user_book_payload(
        &self,
        payload: Option<UserBookIdPayload>,
        field: &str,
    ) -> Result<LibraryEntry> {
        written(payload, field, RawUserBook::into_entry, |id| {
            self.entry_by_id(id)
        })
    }

    /// Removes the book (and its reads) from the library.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Hardcover does not confirm the
    /// removal.
    pub fn remove_from_library(&self, id: UserBookId) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            delete_user_book: Option<DeletePayload>,
        }
        let data: Data = self.query(
            "DeleteUserBook",
            queries::DELETE_USER_BOOK,
            json!({ "id": id.get() }),
            QUERY_TIMEOUT,
        )?;
        match data.delete_user_book {
            Some(DeletePayload { id: Some(_) }) => Ok(()),
            _ => Err(Error::Api(format!("could not remove library entry {id}"))),
        }
    }

    /// Records reading progress on `read`, or (with `None`) on the latest unfinished read.
    ///
    /// With `None` and no unfinished read: if the entry is not Currently Reading it
    /// is moved there first (Hardcover then starts a read, which gets the progress);
    /// otherwise a read started today is created. This avoids two open reads:
    /// Hardcover starts a *new* read on every move to Currently Reading.
    ///
    /// Prefer passing the `ReadId` from the entry you already hold: a lookup made
    /// right after another write can miss a read the server just started.
    ///
    /// Percentages are converted with the edition's page count (or audio length).
    /// Progress that reaches the end makes Hardcover finish the read and mark the
    /// book Read; the returned entry reflects that.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid percentage, a missing length needed to convert
    /// it, or a failed read or write.
    pub fn update_progress(
        &self,
        id: UserBookId,
        read: Option<ReadId>,
        progress: ProgressUpdate,
    ) -> Result<ProgressChange> {
        if let ProgressUpdate::Percent(pct) = progress
            && !(0.0..=100.0).contains(&pct)
        {
            return Err(Error::InvalidInput(format!(
                "progress {pct}% is outside 0..=100"
            )));
        }
        let needs_context = read.is_none() || matches!(progress, ProgressUpdate::Percent(_));
        let context = if needs_context {
            Some(self.progress_context(id, read)?)
        } else {
            None
        };
        let fields = progress_fields(progress, context.as_ref())?;
        let target = read.or_else(|| context.as_ref().and_then(ProgressContext::open_read));
        // The entry after a move to Currently Reading, which started the read.
        let mut moved = None;
        let target = match (target, &context) {
            (None, Some(ctx)) if ctx.status_id != Some(ReadingStatus::CurrentlyReading.id()) => {
                let entry = self.set_status(id, ReadingStatus::CurrentlyReading)?;
                let started = entry.open_read().map(|r| r.id);
                moved = Some(entry);
                started
            }
            (t, _) => t,
        };
        let written = match target {
            Some(read) => self.update_read(read, &Value::Object(fields)),
            None => {
                let mut object = fields;
                object.insert("started_at".into(), json!(today().to_string()));
                self.insert_read(id, &Value::Object(object))
            }
        };
        match written.and_then(|w| self.entry_after_read_write(w, "progress update")) {
            Ok(entry) => Ok(ProgressChange {
                entry,
                progress_left_out: None,
            }),
            // The move went through: show it, and say the progress didn't.
            Err(e) => match moved {
                Some(entry) => Ok(ProgressChange {
                    entry,
                    progress_left_out: Some(e),
                }),
                None => Err(e),
            },
        }
    }

    /// The entry from a read mutation's payload, or (if the server left it out)
    /// read back by id.
    fn entry_after_read_write(&self, written: ReadRef, what: &str) -> Result<LibraryEntry> {
        let user_book_id = written.user_book_id;
        if let Some(entry) = written.into_entry() {
            return Ok(entry);
        }
        self.entry_by_id(user_book_id)?
            .ok_or_else(|| Error::Decode(format!("entry {user_book_id} not found after {what}")))
    }

    fn progress_context(&self, id: UserBookId, read: Option<ReadId>) -> Result<ProgressContext> {
        #[derive(Deserialize)]
        struct Data {
            user_books: Vec<ProgressContext>,
        }
        let reads = match read {
            Some(r) => json!({ "id": { "_eq": r.get() } }),
            None => json!({ "finished_at": { "_is_null": true } }),
        };
        let data: Data = self.query(
            "ProgressContext",
            queries::PROGRESS_CONTEXT,
            json!({ "id": id.get(), "reads": reads }),
            QUERY_TIMEOUT,
        )?;
        let mut context = data
            .user_books
            .into_iter()
            .next()
            .ok_or_else(|| Error::Api(format!("library entry {id} not found")))?;
        context.sort_reads();
        Ok(context)
    }

    /// Sets both dates of a read exactly: `None` clears that date.
    ///
    /// # Errors
    ///
    /// Returns an error if the finish date precedes the start date or the read cannot
    /// be saved.
    pub fn set_read_dates(
        &self,
        read: ReadId,
        started: Option<Date>,
        finished: Option<Date>,
    ) -> Result<LibraryEntry> {
        if let (Some(s), Some(f)) = (started, finished)
            && f < s
        {
            return Err(Error::InvalidInput(
                "finish date is before the start date".into(),
            ));
        }
        let object = json!({
            "started_at": started.map(|d| d.to_string()),
            "finished_at": finished.map(|d| d.to_string()),
        });
        let written = self.update_read(read, &object)?;
        self.entry_after_read_write(written, "date update")
    }

    /// Adds a finished read, such as a re-read or one from before the account.
    /// A finish date is required so this never opens a second unfinished read.
    /// `edition` is normally the entry's.
    ///
    /// # Errors
    ///
    /// Returns an error if the finish date precedes the start date or the read cannot
    /// be saved.
    pub fn add_read(
        &self,
        id: UserBookId,
        started: Option<Date>,
        finished: Date,
        edition: Option<EditionId>,
    ) -> Result<LibraryEntry> {
        if started.is_some_and(|s| finished < s) {
            return Err(Error::InvalidInput(
                "finish date is before the start date".into(),
            ));
        }
        let mut object = Map::new();
        object.insert("started_at".into(), json!(started.map(|d| d.to_string())));
        object.insert("finished_at".into(), json!(finished.to_string()));
        if let Some(edition) = edition {
            object.insert("edition_id".into(), json!(edition.get()));
        }
        let written = self.insert_read(id, &Value::Object(object))?;
        self.entry_after_read_write(written, "adding a read")
    }

    /// Deletes `read` of entry `id` and returns the entry without it.
    ///
    /// The mutation returns only the id (a deleted row's relations may not
    /// resolve), so the entry is read back. A read-back that is a moment stale
    /// can still list the read, so it is taken out by id.
    ///
    /// # Errors
    ///
    /// Returns an error if deletion fails or the library entry cannot be read back.
    pub fn delete_read(&self, id: UserBookId, read: ReadId) -> Result<LibraryEntry> {
        #[derive(Deserialize)]
        struct Data {
            delete_user_book_read: Option<DeleteReadPayload>,
        }
        let data: Data = self.query(
            "DeleteUserBookRead",
            queries::DELETE_USER_BOOK_READ,
            json!({ "id": read.get() }),
            QUERY_TIMEOUT,
        )?;
        let p = data
            .delete_user_book_read
            .ok_or_else(|| Error::Api("`delete_user_book_read` returned null".into()))?;
        if let Some(e) = p.error() {
            return Err(Error::Api(e));
        }
        if p.id.is_none() {
            return Err(Error::Api(format!("could not delete read {read}")));
        }
        let mut entry = self
            .entry_by_id(id.get())?
            .ok_or_else(|| Error::Decode(format!("entry {id} not found after deleting a read")))?;
        entry.reads.retain(|r| r.id != read);
        Ok(entry)
    }

    fn update_read(&self, read: ReadId, object: &Value) -> Result<ReadRef> {
        #[derive(Deserialize)]
        struct Data {
            update_user_book_read: Option<ReadIdPayload>,
        }
        let data: Data = self.query(
            "UpdateUserBookRead",
            queries::UPDATE_USER_BOOK_READ,
            json!({ "id": read.get(), "read": object }),
            QUERY_TIMEOUT,
        )?;
        read_payload(data.update_user_book_read, "update_user_book_read")
    }

    fn insert_read(&self, id: UserBookId, object: &Value) -> Result<ReadRef> {
        #[derive(Deserialize)]
        struct Data {
            insert_user_book_read: Option<ReadIdPayload>,
        }
        let data: Data = self.query(
            "InsertUserBookRead",
            queries::INSERT_USER_BOOK_READ,
            json!({ "userBookId": id.get(), "read": object }),
            QUERY_TIMEOUT,
        )?;
        read_payload(data.insert_user_book_read, "insert_user_book_read")
    }

    // ----- goals -----

    /// The signed-in user's goals that are not archived, the latest end date
    /// first. Needs the `read:goals` scope.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or its response cannot be decoded.
    pub fn goals(&self) -> Result<Vec<Goal>> {
        #[derive(Deserialize)]
        struct Data {
            goals: Vec<RawGoal>,
        }
        let me = self.my_id()?;
        let data: Data = self.query(
            "Goals",
            queries::GOALS,
            json!({ "userId": me.get() }),
            QUERY_TIMEOUT,
        )?;
        Ok(data
            .goals
            .into_iter()
            .filter_map(RawGoal::into_goal)
            .collect())
    }

    /// Creates a goal for every book (no conditions), with the account's
    /// default privacy. Hardcover calculates its progress right away.
    ///
    /// # Errors
    ///
    /// Returns an error if the target, dates or description are invalid, or the goal
    /// cannot be saved.
    pub fn create_goal(&self, goal: &NewGoal) -> Result<Goal> {
        #[derive(Deserialize)]
        struct Data {
            insert_goal: Option<GoalPayload>,
        }
        validate_goal_dates(goal.target, goal.start, goal.end)?;
        let description = goal_description(Some(&goal.description))?;
        let object = goal_object(
            &goal.metric,
            goal.target,
            goal.start,
            goal.end,
            description,
            &json!({}),
        );
        let data: Data = self.query(
            "InsertGoal",
            queries::INSERT_GOAL,
            json!({ "object": object }),
            QUERY_TIMEOUT,
        )?;
        self.goal_payload(data.insert_goal, "insert_goal")
    }

    /// Changes goal `id`: reads it from Hardcover, lets `edit` change the
    /// fields the user edited, and writes it back. `update_goal` replaces
    /// the whole goal, so everything `edit` leaves alone is sent as
    /// Hardcover has it now, not as the app last loaded it: a change made on
    /// the website since then stays.
    ///
    /// # Errors
    ///
    /// `Error::InvalidInput` if the goal's conditions hold a field this
    /// version can't send back, which the update would drop, or if it has no
    /// description, which Hardcover requires; `Error::Api` if the goal is
    /// gone.
    pub fn update_goal(&self, id: GoalId, edit: impl FnOnce(&mut Goal)) -> Result<Goal> {
        #[derive(Deserialize)]
        struct Data {
            update_goal: Option<GoalPayload>,
        }
        let mut goal = self
            .goal_by_id(id.get())?
            .ok_or_else(|| Error::Api(format!("goal {id} not found")))?;
        edit(&mut goal);
        let conditions = goal.conditions.to_input().ok_or_else(|| {
            Error::InvalidInput(
                "this goal has conditions the app can't keep; change it on hardcover.app".into(),
            )
        })?;
        validate_goal_dates(goal.target, goal.start, goal.end)?;
        let description = goal_description(goal.description.as_deref())?;
        let mut object = goal_object(
            &goal.metric,
            goal.target,
            goal.start,
            goal.end,
            description,
            &conditions,
        );
        object["archived"] = json!(goal.archived);
        // Sent back as read, even an id this version doesn't know: left out,
        // the goal would get the account's default privacy.
        if let Some(privacy) = goal.privacy_id {
            object["privacy_setting_id"] = json!(privacy);
        }
        let data: Data = self.query(
            "UpdateGoal",
            queries::UPDATE_GOAL,
            json!({ "id": id.get(), "object": object }),
            QUERY_TIMEOUT,
        )?;
        self.goal_payload(data.update_goal, "update_goal")
    }

    /// Deletes a reading goal.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the deletion payload reports an
    /// error.
    pub fn delete_goal(&self, id: GoalId) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            delete_goal: Option<GoalPayload>,
        }
        let data: Data = self.query(
            "DeleteGoal",
            queries::DELETE_GOAL,
            json!({ "id": id.get() }),
            QUERY_TIMEOUT,
        )?;
        let p = data
            .delete_goal
            .ok_or_else(|| Error::Api("`delete_goal` returned null".into()))?;
        if let Some(e) = p.error() {
            return Err(Error::Api(e));
        }
        // Hardcover answers a successful delete with `id: null` (verified
        // live), so only `errors` means it failed.
        Ok(())
    }

    /// Uses the goal returned by the mutation, or re-reads it if the payload had none.
    fn goal_payload(&self, payload: Option<GoalPayload>, field: &str) -> Result<Goal> {
        written(payload, field, RawGoal::into_goal, |id| self.goal_by_id(id))
    }

    fn goal_by_id(&self, id: i64) -> Result<Option<Goal>> {
        #[derive(Deserialize)]
        struct Data {
            goals_by_pk: Option<RawGoal>,
        }
        let data: Data = self.query(
            "GoalById",
            queries::GOAL_BY_ID,
            json!({ "id": id }),
            QUERY_TIMEOUT,
        )?;
        Ok(data.goals_by_pk.and_then(RawGoal::into_goal))
    }

    // ----- lists -----

    /// The signed-in user's lists, the Owned list included once it exists,
    /// in the order they were created. Needs `read:lists`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or its response cannot be decoded.
    pub fn lists(&self) -> Result<Vec<UserList>> {
        #[derive(Deserialize)]
        struct Data {
            lists: Vec<RawList>,
        }
        let me = self.my_id()?;
        let mut all = Vec::new();
        let complete = keyset_pages(
            None,
            |after| {
                let data: Data = self.query(
                    "Lists",
                    queries::LISTS,
                    json!({ "userId": me.get(), "afterId": after, "limit": LISTS_PAGE_SIZE }),
                    QUERY_TIMEOUT,
                )?;
                Ok(data.lists)
            },
            |raw| raw.id,
            |rows| all.extend(rows.into_iter().map(RawList::into_list)),
        )?;
        if !complete {
            // Lists aren't cached, so a short result would quietly hide lists.
            return Err(Error::Decode("the lists stopped paging forward".into()));
        }
        Ok(all)
    }

    /// Which of the user's books are on which of their lists, as one index:
    /// one request per 1,000 rows, at most five. Past that the result is
    /// marked incomplete; [`Client::book_list_memberships`] covers one book.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or its response cannot be decoded.
    pub fn list_memberships(&self) -> Result<ListMemberships> {
        #[derive(Deserialize)]
        struct Data {
            list_books: Vec<RawListMembership>,
        }
        let me = self.my_id()?;
        let mut rows = Vec::new();
        let complete = keyset_pages(
            Some(MEMBERSHIPS_MAX_PAGES),
            |after| {
                let data: Data = self.query(
                    "ListMemberships",
                    queries::LIST_MEMBERSHIPS,
                    json!({ "userId": me.get(), "afterId": after, "limit": MEMBERSHIPS_PAGE_SIZE }),
                    QUERY_TIMEOUT,
                )?;
                Ok(data.list_books)
            },
            |raw| raw.id,
            |page| rows.extend(page.into_iter().map(RawListMembership::into_membership)),
        )?;
        Ok(ListMemberships { rows, complete })
    }

    /// The user's lists that `book` is on, one row per list and edition.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or its response cannot be decoded.
    pub fn book_list_memberships(&self, book: BookId) -> Result<Vec<ListMembership>> {
        #[derive(Deserialize)]
        struct Data {
            list_books: Vec<RawListMembership>,
        }
        let me = self.my_id()?;
        let data: Data = self.query(
            "BookListMemberships",
            queries::BOOK_LIST_MEMBERSHIPS,
            json!({ "userId": me.get(), "bookId": book.get(), "limit": MEMBERSHIPS_PAGE_SIZE }),
            QUERY_TIMEOUT,
        )?;
        Ok(data
            .list_books
            .into_iter()
            .map(RawListMembership::into_membership)
            .collect())
    }

    /// One page of a list's books in the list's order. `offset` is the
    /// previous page's [`OffsetPage::next_offset`], 0 for the first. Rows
    /// whose book did not resolve are left out of the items but still move
    /// the offset on, so they neither end the list nor shift the next page.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or its response cannot be decoded.
    pub fn list_books(&self, list: ListId, offset: u32) -> Result<OffsetPage<ListBook>> {
        #[derive(Deserialize)]
        struct Data {
            list_books: Vec<RawListBook>,
        }
        let data: Data = self.query(
            "ListBooks",
            queries::LIST_BOOKS,
            json!({ "listId": list.get(), "limit": LIST_BOOKS_PER_PAGE + 1, "offset": offset }),
            QUERY_TIMEOUT,
        )?;
        Ok(offset_page(
            data.list_books,
            offset,
            LIST_BOOKS_PER_PAGE,
            RawListBook::into_list_book,
        ))
    }

    /// Creates a list. Needs `write:lists`.
    ///
    /// # Errors
    ///
    /// Returns an error if the draft is invalid or the list cannot be saved.
    pub fn create_list(&self, draft: &ListDraft) -> Result<UserList> {
        #[derive(Deserialize)]
        struct Data {
            insert_list: Option<ListPayload>,
        }
        let name = list_name(&draft.name)?;
        let mut object = list_draft_object(name, draft);
        if let Some(privacy) = draft.privacy {
            object["privacy_setting_id"] = json!(privacy.id());
        }
        let data: Data = self.query(
            "InsertList",
            queries::INSERT_LIST,
            json!({ "object": object }),
            QUERY_TIMEOUT,
        )?;
        self.list_payload(data.insert_list, "insert_list")
    }

    /// Changes list `id`: reads it from Hardcover and writes it back with
    /// `edit`'s fields changed. Whether `update_list` keeps fields left out
    /// is unverified (`update_goal` does not), so the rest (its default
    /// view, profile feature, link, and the raw privacy) is sent as
    /// Hardcover has it now, not as the app last loaded it. Refuses the
    /// Owned list, which Hardcover finds by its slug.
    ///
    /// # Errors
    ///
    /// Returns an error if the list is gone or is the Owned list, the name
    /// would be blank, or the list cannot be saved.
    pub fn update_list(&self, id: ListId, edit: &ListEdit) -> Result<UserList> {
        #[derive(Deserialize)]
        struct Data {
            update_list: Option<ListPayload>,
        }
        let list = self
            .list_by_id(id.get())?
            .ok_or_else(|| Error::Api(format!("list {id} not found")))?;
        if list.is_owned() {
            return Err(Error::InvalidInput(
                "the Owned list can't be changed".into(),
            ));
        }
        let name = list_name(edit.name.as_deref().unwrap_or(&list.name))?;
        let description = edit
            .description
            .as_deref()
            .or(list.description.as_deref())
            .unwrap_or_default()
            .trim();
        let mut object = json!({
            "name": name,
            "description": description,
            "ranked": edit.ranked.unwrap_or(list.ranked),
        });
        // Sent back as read when unchanged, even an id this version doesn't know.
        if let Some(privacy) = edit.privacy.map(Privacy::id).or(list.privacy_id) {
            object["privacy_setting_id"] = json!(privacy);
        }
        if let Some(view) = &list.default_view {
            object["default_view"] = json!(view);
        }
        if let Some(featured) = list.featured_profile {
            object["featured_profile"] = json!(featured);
        }
        if let Some(url) = &list.url {
            object["url"] = json!(url);
        }
        let data: Data = self.query(
            "UpdateList",
            queries::UPDATE_LIST,
            json!({ "id": id.get(), "object": object }),
            QUERY_TIMEOUT,
        )?;
        self.list_payload(data.update_list, "update_list")
    }

    /// Deletes a list and takes its books off it; the books stay in the library.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Hardcover does not confirm the
    /// deletion.
    pub fn delete_list(&self, id: ListId) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            delete_list: Option<ListDeletePayload>,
        }
        let data: Data = self.query(
            "DeleteList",
            queries::DELETE_LIST,
            json!({ "id": id.get() }),
            QUERY_TIMEOUT,
        )?;
        match data.delete_list {
            Some(ListDeletePayload {
                success: Some(true),
            }) => Ok(()),
            _ => Err(Error::Api(format!("could not delete list {id}"))),
        }
    }

    /// Puts `book` on `list`, as `edition` if given. Needs `write:lists`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the addition cannot be confirmed.
    pub fn add_to_list(
        &self,
        list: ListId,
        book: BookId,
        edition: Option<EditionId>,
    ) -> Result<ListMembership> {
        #[derive(Deserialize)]
        struct Data {
            insert_list_book: Option<ListBookPayload>,
        }
        let mut object = json!({ "list_id": list.get(), "book_id": book.get() });
        if let Some(edition) = edition {
            object["edition_id"] = json!(edition.get());
        }
        let data: Data = self.query(
            "InsertListBook",
            queries::INSERT_LIST_BOOK,
            json!({ "object": object }),
            QUERY_TIMEOUT,
        )?;
        let p = data
            .insert_list_book
            .ok_or_else(|| Error::Api("`insert_list_book` returned null".into()))?;
        match (p.list_book, p.id) {
            (Some(row), _) => Ok(row.into_membership()),
            // The row's id is all the payload had; the rest is what was sent.
            (None, Some(id)) => Ok(ListMembership {
                id: ListBookId::new(id),
                list_id: list,
                book_id: book,
                edition_id: edition,
            }),
            (None, None) => Err(Error::Api(format!(
                "could not add book {book} to list {list}"
            ))),
        }
    }

    /// Takes one row off its list: a book, or one edition of it on the Owned
    /// list. Needs `write:lists`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Hardcover does not confirm the
    /// deletion.
    pub fn remove_from_list(&self, row: ListBookId) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            delete_list_book: Option<ListBookDeletePayload>,
        }
        let data: Data = self.query(
            "DeleteListBook",
            queries::DELETE_LIST_BOOK,
            json!({ "id": row.get() }),
            QUERY_TIMEOUT,
        )?;
        match data.delete_list_book {
            Some(ListBookDeletePayload { id: Some(_) }) => Ok(()),
            _ => Err(Error::Api(format!("could not remove list book {row}"))),
        }
    }

    /// Puts `edition` on the user's Owned list, which Hardcover creates the
    /// first time. Repeating it is harmless. Needs `write:library`.
    ///
    /// Returns the list row when Hardcover sends it (the Owned list's id may be
    /// new to the caller); `None` if it sent only an id. Taking the edition off
    /// again is [`Client::remove_from_list`] (`write:lists`), and
    /// `user_books.owned` does not follow the list. Hardcover posts the
    /// addition to the user's activity feed.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the addition cannot be confirmed.
    pub fn mark_owned(&self, edition: EditionId) -> Result<Option<ListMembership>> {
        #[derive(Deserialize)]
        struct Data {
            edition_owned: Option<ListBookPayload>,
        }
        let data: Data = self.query(
            "EditionOwned",
            queries::EDITION_OWNED,
            json!({ "id": edition.get() }),
            QUERY_TIMEOUT,
        )?;
        match data.edition_owned {
            Some(ListBookPayload {
                list_book: Some(row),
                ..
            }) => Ok(Some(row.into_membership())),
            Some(ListBookPayload { id: Some(_), .. }) => Ok(None),
            _ => Err(Error::Api(format!(
                "could not add edition {edition} to the Owned list"
            ))),
        }
    }

    /// Uses the list returned by the mutation, or re-reads it if the payload had none.
    fn list_payload(&self, payload: Option<ListPayload>, field: &str) -> Result<UserList> {
        written(
            payload,
            field,
            |raw| Some(RawList::into_list(raw)),
            |id| self.list_by_id(id),
        )
    }

    fn list_by_id(&self, id: i64) -> Result<Option<UserList>> {
        #[derive(Deserialize)]
        struct Data {
            lists_by_pk: Option<RawList>,
        }
        let data: Data = self.query(
            "ListById",
            queries::LIST_BY_ID,
            json!({ "id": id }),
            QUERY_TIMEOUT,
        )?;
        Ok(data.lists_by_pk.map(RawList::into_list))
    }

    // ----- other readers' reviews -----

    /// A page of other readers' reviews of `book`, the most liked first, then
    /// the newest. `me` (the signed-in user) is left out. `offset` is the
    /// previous page's [`ReviewsPage::next_offset`], 0 for the first. Asks
    /// for one row more than a page, so `has_more` is never a guess. Needs
    /// `read:social`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or its response cannot be decoded.
    pub fn book_reviews(&self, book: BookId, me: UserId, offset: u32) -> Result<ReviewsPage> {
        #[derive(Deserialize)]
        struct Data {
            user_books: Vec<RawCommunityReview>,
        }
        let data: Data = self.query(
            "BookReviews",
            queries::BOOK_REVIEWS,
            json!({
                "bookId": book.get(),
                "me": me.get(),
                "limit": REVIEWS_PER_PAGE + 1,
                "offset": offset,
            }),
            QUERY_TIMEOUT,
        )?;
        let page = offset_page(
            data.user_books,
            offset,
            REVIEWS_PER_PAGE,
            RawCommunityReview::into_review,
        );
        Ok(ReviewsPage {
            reviews: page.items,
            next_offset: page.next_offset,
            has_more: page.has_more,
        })
    }

    /// How many reviews [`Client::book_reviews`] pages through.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or its response cannot be decoded.
    pub fn book_reviews_count(&self, book: BookId, me: UserId) -> Result<u32> {
        #[derive(Deserialize)]
        struct Data {
            user_books_aggregate: RawAggregate,
        }
        let data: Data = self.query(
            "BookReviewsCount",
            queries::BOOK_REVIEWS_COUNT,
            json!({ "bookId": book.get(), "me": me.get() }),
            QUERY_TIMEOUT,
        )?;
        Ok(data
            .user_books_aggregate
            .count()
            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)))
    }

    // ----- transport -----

    /// The lane a request waits in for the rate limit.
    fn priority_of(&self, op: &str, doc: &str) -> Priority {
        if self.priority == Priority::Interactive
            || is_mutation(doc)
            || op.starts_with("Search")
            || WRITE_STEP_READS.contains(&op)
        {
            Priority::Interactive
        } else {
            Priority::Background
        }
    }

    fn query<T: DeserializeOwned>(
        &self,
        op: &str,
        doc: &str,
        variables: Value,
        timeout: Duration,
    ) -> Result<T> {
        let mut body = json!({ "operationName": op, "query": doc });
        body["variables"] = variables;
        let priority = self.priority_of(op, doc);
        let mutation = is_mutation(doc);
        let mut auth_retried = false;
        // Set once a 429 is retried: the retry must get out by then.
        let mut retry_deadline: Option<Instant> = None;
        loop {
            let token = self.inner.tokens.access_token()?;
            self.inner.bucket.acquire(priority, retry_deadline)?;
            let req = HttpRequest::new(Method::Post, &self.inner.endpoint)
                .header("authorization", format!("Bearer {token}"))
                .header("accept", "application/json")
                .json(&body)
                .timeout(timeout);
            let resp = match self.inner.http.send(req) {
                Ok(resp) => resp,
                // The request went out but no answer came back: a write may
                // have been applied. Never retried.
                Err(Error::Timeout) if mutation => return Err(Error::OutcomeUnknown),
                Err(e) => return Err(e),
            };
            if resp.status == 401 && !auth_retried && self.inner.tokens.on_unauthorized(&token)? {
                auth_retried = true;
                continue;
            }
            if resp.status == 504 && mutation {
                // The gateway gave up waiting; the write may still have run.
                return Err(Error::OutcomeUnknown);
            }
            if resp.status == 429 {
                // Throttled requests are rejected before they run, so retrying is
                // safe even for mutations. Every waiting caller now respects the
                // block; a long one (the daily limit) is remembered and fails
                // the calls that follow at once. One retry, if it can go out
                // soon enough.
                let wait = retry_after(&resp);
                self.inner.bucket.penalize(wait);
                let short = wait.is_none_or(|w| w <= MAX_AUTO_RETRY_WAIT);
                if retry_deadline.is_none() && short {
                    retry_deadline = Some(Instant::now() + MAX_AUTO_RETRY_WAIT);
                    continue;
                }
            }
            return decode_graphql(&resp);
        }
    }
}

/// Whether a document is a mutation (each holds one operation).
fn is_mutation(doc: &str) -> bool {
    doc.trim_start().starts_with("mutation")
}

/// Keyset paging over rows ordered by `id`, `after` the last id seen.
///
/// Paging stops only on an empty page. A page shorter than asked for is
/// not taken as the last: Hardcover caps some tables' rows at the page sizes
/// used, so a cap lowered later would otherwise end a list early, silently.
/// (Asking for a count alongside each page would save the final empty
/// request, but Hardcover charges a rate-limit token per top-level field,
/// so it would cost more.) `take` gets every page, rows it may not be able
/// to read included.
///
/// Returns whether it paged to the end: false if it stopped at `max_pages`
/// or because the ids stopped growing.
fn keyset_pages<R>(
    max_pages: Option<usize>,
    mut fetch: impl FnMut(i64) -> Result<Vec<R>>,
    id: impl Fn(&R) -> i64,
    mut take: impl FnMut(Vec<R>),
) -> Result<bool> {
    let mut after = 0_i64;
    let mut pages = 0_usize;
    loop {
        if max_pages.is_some_and(|max| pages >= max) {
            return Ok(false);
        }
        pages += 1;
        let rows = fetch(after)?;
        if rows.is_empty() {
            return Ok(true);
        }
        let last = rows.iter().map(&id).fold(after, i64::max);
        take(rows);
        if last <= after {
            // No progress: the same rows would come back forever.
            return Ok(false);
        }
        after = last;
    }
}

/// A page fetched by offset with `limit + 1` rows asked for, so `has_more`
/// is never a guess. The offset moves on by the rows Hardcover sent, so a
/// row `read` can't use (`None`) doesn't shift the next page.
fn offset_page<R, T>(
    mut rows: Vec<R>,
    offset: u32,
    limit: u32,
    read: impl FnMut(R) -> Option<T>,
) -> OffsetPage<T> {
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let sent = u32::try_from(rows.len()).unwrap_or(limit);
    OffsetPage {
        items: rows.into_iter().filter_map(read).collect(),
        next_offset: offset + sent,
        has_more,
    }
}

fn read_payload(payload: Option<ReadIdPayload>, field: &str) -> Result<ReadRef> {
    written(payload, field, Some, |id| {
        Err(Error::Decode(format!(
            "`{field}` returned no read (id {id})"
        )))
    })
}

/// A mutation's payload, whichever of the API's error shapes it has:
/// `error: String` (the library and read mutations), `errors: [String]`
/// (goals and lists), or none. The payloads with no error field and no row
/// (the deletes) are matched where they are used.
trait Payload {
    /// The written row, as the payload nests it.
    type Row;
    /// The payload's error text, if it reports one.
    fn error(&self) -> Option<String>;
    /// The written row's id and the row, either of which may be missing.
    fn into_written(self) -> (Option<i64>, Option<Self::Row>);
}

/// An `error: String` field, if it holds any text.
fn error_text(error: Option<&String>) -> Option<String> {
    error
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(str::to_owned)
}

impl Payload for UserBookIdPayload {
    type Row = RawUserBook;
    fn error(&self) -> Option<String> {
        error_text(self.error.as_ref())
    }
    fn into_written(self) -> (Option<i64>, Option<RawUserBook>) {
        (self.id, self.user_book)
    }
}

impl Payload for ReviewedUserBookPayload {
    type Row = wire::RawReviewedUserBook;
    fn error(&self) -> Option<String> {
        error_text(self.error.as_ref())
    }
    fn into_written(self) -> (Option<i64>, Option<Self::Row>) {
        (self.id, self.user_book)
    }
}

impl Payload for ReadIdPayload {
    type Row = ReadRef;
    fn error(&self) -> Option<String> {
        error_text(self.error.as_ref())
    }
    fn into_written(self) -> (Option<i64>, Option<ReadRef>) {
        (self.id, self.user_book_read)
    }
}

impl Payload for DeleteReadPayload {
    type Row = ();
    fn error(&self) -> Option<String> {
        error_text(self.error.as_ref())
    }
    fn into_written(self) -> (Option<i64>, Option<()>) {
        (self.id, None)
    }
}

impl Payload for GoalPayload {
    type Row = RawGoal;
    fn error(&self) -> Option<String> {
        GoalPayload::error(self)
    }
    fn into_written(self) -> (Option<i64>, Option<RawGoal>) {
        (self.id, self.goal)
    }
}

impl Payload for ListPayload {
    type Row = RawList;
    fn error(&self) -> Option<String> {
        ListPayload::error(self)
    }
    fn into_written(self) -> (Option<i64>, Option<RawList>) {
        (self.id, self.list)
    }
}

/// What a mutation wrote: the row from its payload, else the row read back
/// by the payload's id (a read right after a write can be stale, so the
/// payload's own row wins). An error in the payload is the write's failure.
fn written<P: Payload, T>(
    payload: Option<P>,
    field: &str,
    read: impl FnOnce(P::Row) -> Option<T>,
    read_back: impl FnOnce(i64) -> Result<Option<T>>,
) -> Result<T> {
    let p = payload.ok_or_else(|| Error::Api(format!("`{field}` returned null")))?;
    if let Some(e) = p.error() {
        return Err(Error::Api(e));
    }
    let (id, row) = p.into_written();
    if let Some(found) = row.and_then(read) {
        return Ok(found);
    }
    let id =
        id.ok_or_else(|| Error::Decode(format!("`{field}` returned neither a row nor an id")))?;
    read_back(id)?.ok_or_else(|| Error::Decode(format!("{id} not found after `{field}`")))
}

/// Today on this computer: the date a read started or finished here gets.
fn today() -> Date {
    chrono::Local::now().date_naive()
}

/// The `status_id` to send, with the date that dates the read the change
/// starts (Currently Reading) or finishes (Read). Verified live
/// (2026-11-21): `user_date` sets the new read's `started_at` on Currently
/// Reading and the open read's `finished_at` on Read, on insert and update
/// alike; left out, Hardcover uses today in the account's time zone (UTC
/// without one), which needn't be this computer's. `user_start_date` must
/// not be sent: it is ignored on Currently Reading and, with Read, leaves
/// the read open.
fn status_fields(status: ReadingStatus, today: Date) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("status_id".into(), json!(status.id()));
    if matches!(
        status,
        ReadingStatus::CurrentlyReading | ReadingStatus::Read
    ) {
        m.insert("user_date".into(), json!(today.to_string()));
    }
    m
}

/// The `DatesReadInput` fields for a progress update.
fn progress_fields(
    progress: ProgressUpdate,
    ctx: Option<&ProgressContext>,
) -> Result<Map<String, Value>> {
    let mut m = Map::new();
    match progress {
        ProgressUpdate::Pages(p) => {
            m.insert("progress_pages".into(), json!(p));
        }
        ProgressUpdate::Seconds(s) => {
            m.insert("progress_seconds".into(), json!(s));
        }
        ProgressUpdate::Percent(pct) => {
            if !(0.0..=100.0).contains(&pct) {
                return Err(Error::InvalidInput(format!(
                    "progress {pct}% is outside 0..=100"
                )));
            }
            match ctx.and_then(ProgressContext::length) {
                Some(length) => {
                    return progress_fields(length.position_at(f64::from(pct)), None);
                }
                None => {
                    return Err(Error::InvalidInput(
                        "cannot convert a percentage: the edition's length is unknown; set pages instead".into(),
                    ));
                }
            }
        }
    }
    Ok(m)
}

/// The `DatesReadInput` that moves an open read to `edition`: its progress at
/// the same percentage, in the new edition's unit, with the other unit cleared.
/// Just the edition when the percentage or the new length is unknown.
fn moved_read_fields(
    read: &ReadSession,
    edition: EditionId,
    new_edition: Option<&EditionSummary>,
) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("edition_id".into(), json!(edition.get()));
    let length = new_edition
        .filter(|e| e.id == edition)
        .and_then(EditionSummary::length);
    if let (Some(pct), Some(length)) = (read.progress_percent, length) {
        // Never the end, even from 100%: progress at the end finishes the
        // read, which changing the edition must not do.
        let units = length
            .units_at(f64::from(pct.clamp(0.0, 100.0)))
            .min(length.total().saturating_sub(1));
        let (pages, seconds) = match length {
            Length::Pages(_) => (Some(units), None),
            Length::Seconds(_) => (None, Some(units)),
        };
        m.insert("progress_pages".into(), json!(pages));
        m.insert("progress_seconds".into(), json!(seconds));
    }
    m
}

/// The description to send: Hardcover requires one (`GoalInput.description`
/// is `String!`), and an empty one would be stored as is.
fn goal_description(description: Option<&str>) -> Result<&str> {
    description
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or_else(|| Error::InvalidInput("a goal needs a description".into()))
}

fn validate_goal_dates(target: u32, start: Date, end: Date) -> Result<()> {
    if target == 0 || end < start {
        return Err(Error::InvalidInput(
            "a goal needs a target above zero and an end after its start".into(),
        ));
    }
    Ok(())
}

fn goal_object(
    metric: &GoalMetric,
    target: u32,
    start: Date,
    end: Date,
    description: &str,
    conditions: &Value,
) -> Value {
    json!({
        "metric": metric.api_name(),
        "goal": target,
        "description": description,
        "start_date": start.to_string(),
        "end_date": end.to_string(),
        "conditions": conditions,
    })
}

/// The name to send: Hardcover needs one to make a list's slug.
fn list_name(name: &str) -> Result<&str> {
    Some(name.trim())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| Error::InvalidInput("a list needs a name".into()))
}

fn list_draft_object(name: &str, draft: &ListDraft) -> Value {
    json!({
        "name": name,
        "description": draft.description.trim(),
        "ranked": draft.ranked,
    })
}

fn retry_after(resp: &HttpResponse) -> Option<Duration> {
    resp.header("retry-after")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Maps an API response to data or a typed error.
fn decode_graphql<T: DeserializeOwned>(resp: &HttpResponse) -> Result<T> {
    let body: Option<Value> = serde_json::from_slice(&resp.body).ok();
    let message = || {
        body.as_ref()
            .and_then(error_message)
            .unwrap_or_else(|| snippet(&resp.body))
    };
    match resp.status {
        200..=299 => {}
        401 => return Err(Error::Unauthorized),
        403 => {
            let b = body.as_ref();
            if b.and_then(|b| b.get("error")).and_then(Value::as_str) == Some("insufficient_scope")
            {
                return Err(Error::InsufficientScope {
                    scope: b
                        .and_then(|b| b.get("scope"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
            }
            return Err(Error::Api(message()));
        }
        408 => return Err(Error::Timeout),
        429 => {
            return Err(Error::RateLimited {
                retry_after: retry_after(resp),
            });
        }
        500..=599 => {
            return Err(Error::Network(format!(
                "server error {}: {}",
                resp.status,
                message()
            )));
        }
        s => return Err(Error::Api(format!("HTTP {s}: {}", message()))),
    }
    let body = body.ok_or_else(|| Error::Decode(format!("not JSON: {}", snippet(&resp.body))))?;
    if let Some(errors) = body.get("errors").filter(|e| !e.is_null()) {
        let msg = graphql_errors(errors);
        if !msg.is_empty() {
            return Err(Error::Api(msg));
        }
    }
    let data = body
        .get("data")
        .filter(|d| !d.is_null())
        .ok_or_else(|| Error::Decode("response has no `data`".into()))?;
    T::deserialize(data).map_err(|e| Error::Decode(e.to_string()))
}

/// GraphQL `errors` is normally `[{message, extensions}]`; Hardcover's gateway
/// also sends `["request_exceeds_capacity"]`.
fn graphql_errors(errors: &Value) -> String {
    let list: Vec<String> = match errors {
        Value::Array(items) => items
            .iter()
            .filter_map(|e| match e {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o.get("message").and_then(Value::as_str).map(str::to_owned),
                _ => None,
            })
            .collect(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    };
    list.join("; ")
}

/// Error text from any of the API's error bodies:
/// `{error, error_description}`, `{error, message}`, `{errors: [...], message}`.
fn error_message(body: &Value) -> Option<String> {
    let s = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let errors = body
        .get("errors")
        .map(graphql_errors)
        .filter(|s| !s.is_empty());
    let head = s("error").map(str::to_owned).or(errors);
    let detail = s("error_description").or_else(|| s("message"));
    match (head, detail) {
        (Some(h), Some(d)) => Some(format!("{h}: {d}")),
        (Some(h), None) => Some(h),
        (None, Some(d)) => Some(d.to_owned()),
        (None, None) => None,
    }
}

fn snippet(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        return "empty response".into();
    }
    text.chars().take(200).collect()
}

#[cfg(test)]
mod tests;
