//! Search results as a plain state machine: the current query and its
//! revision, the pages received so far (deduplicated by id) and whether more
//! can be loaded. One per kind of result: books, authors, series.

use std::collections::HashSet;
use std::hash::Hash;

use hardcover_core::{AuthorHit, AuthorId, BookId, BookSummary, SearchHits, SeriesHit, SeriesId};

/// A search result with an id that identifies it across pages.
pub trait Hit {
    type Key: Copy + Eq + Hash;
    fn key(&self) -> Self::Key;
}

impl Hit for BookSummary {
    type Key = BookId;
    fn key(&self) -> BookId {
        self.id
    }
}

impl Hit for AuthorHit {
    type Key = AuthorId;
    fn key(&self) -> AuthorId {
        self.id
    }
}

impl Hit for SeriesHit {
    type Key = SeriesId;
    fn key(&self) -> SeriesId {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Phase {
    /// No query.
    #[default]
    Idle,
    /// Waiting for the first page.
    Searching,
    Loaded,
    /// Waiting for a further page; earlier pages stay visible.
    LoadingMore,
    Failed(String),
}

#[derive(Debug)]
pub struct SearchResults<T: Hit> {
    query: String,
    revision: u64,
    hits: Vec<T>,
    seen: HashSet<T::Key>,
    page: u32,
    total: Option<u32>,
    has_more: bool,
    phase: Phase,
}

impl<T: Hit> Default for SearchResults<T> {
    fn default() -> Self {
        Self {
            query: String::new(),
            revision: 0,
            hits: Vec::new(),
            seen: HashSet::new(),
            page: 0,
            total: None,
            has_more: false,
            phase: Phase::Idle,
        }
    }
}

impl<T: Hit> SearchResults<T> {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn hits(&self) -> &[T] {
        &self.hits
    }

    pub fn total(&self) -> Option<u32> {
        self.total
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn has_more(&self) -> bool {
        self.has_more && self.phase == Phase::Loaded
    }

    /// Starts a new query, dropping the old results. Returns the revision a
    /// response must carry to be accepted, or `None` for a blank query.
    pub fn start(&mut self, query: String) -> Option<u64> {
        self.revision += 1;
        self.hits.clear();
        self.seen.clear();
        self.page = 0;
        self.total = None;
        self.has_more = false;
        if query.trim().is_empty() {
            self.query.clear();
            self.phase = Phase::Idle;
            return None;
        }
        self.query = query;
        self.phase = Phase::Searching;
        Some(self.revision)
    }

    /// The next page to request, marking it as loading. `None` if there is no
    /// further page or a request is already running.
    pub fn next_page_request(&mut self) -> Option<(u64, String, u32)> {
        if !self.has_more() {
            return None;
        }
        self.phase = Phase::LoadingMore;
        Some((self.revision, self.query.clone(), self.page + 1))
    }

    /// Applies a response. Returns `false` (and changes nothing) if it belongs
    /// to an older query.
    pub fn receive(
        &mut self,
        revision: u64,
        result: Result<SearchHits<T>, impl Into<String>>,
    ) -> bool {
        if revision != self.revision {
            return false;
        }
        match result {
            Ok(page) => {
                let fresh = page.hits.len();
                self.has_more = page.has_more() && fresh > 0;
                self.page = page.page;
                self.total = page.total.or(self.total);
                for hit in page.hits {
                    if self.seen.insert(hit.key()) {
                        self.hits.push(hit);
                    }
                }
                self.phase = Phase::Loaded;
            }
            Err(message) => {
                // No automatic retry of a failed page; a new query starts over.
                self.has_more = false;
                self.phase = Phase::Failed(message.into());
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::SearchPage;

    fn page(ids: &[i64], page_no: u32, total: u32) -> SearchHits<BookSummary> {
        let books = ids
            .iter()
            .map(|&id| BookSummary::new(BookId::new(id), format!("Book {id}")))
            .collect();
        let mut p = SearchPage::new(books, page_no, 2);
        p.total = Some(total);
        p.into()
    }

    #[test]
    fn blank_query_is_idle_without_request() {
        let mut r = SearchResults::<BookSummary>::default();
        assert_eq!(r.start("   ".into()), None);
        assert_eq!(r.phase(), &Phase::Idle);
    }

    #[test]
    fn stale_responses_are_rejected() {
        let mut r = SearchResults::<BookSummary>::default();
        let old = r.start("dun".into()).unwrap();
        let new = r.start("dune".into()).unwrap();
        assert!(!r.receive(old, Ok::<_, String>(page(&[1, 2], 1, 10))));
        assert!(r.hits().is_empty());
        assert_eq!(r.phase(), &Phase::Searching);
        assert!(r.receive(new, Ok::<_, String>(page(&[3, 4], 1, 10))));
        assert_eq!(r.hits().len(), 2);
        assert_eq!(r.total(), Some(10));
    }

    #[test]
    fn pages_append_without_duplicates() {
        let mut r = SearchResults::<BookSummary>::default();
        let rev = r.start("dune".into()).unwrap();
        r.receive(rev, Ok::<_, String>(page(&[1, 2], 1, 3)));
        assert!(r.has_more());
        let (rev2, query, next) = r.next_page_request().unwrap();
        assert_eq!((rev2, query.as_str(), next), (rev, "dune", 2));
        // A second request while one runs is refused.
        assert!(r.next_page_request().is_none());
        r.receive(rev, Ok::<_, String>(page(&[2, 3], 2, 3)));
        let ids: Vec<i64> = r.hits().iter().map(|b| b.id.get()).collect();
        assert_eq!(ids, [1, 2, 3]);
        assert!(!r.has_more());
    }

    #[test]
    fn failure_keeps_earlier_pages_and_stops_paging() {
        let mut r = SearchResults::<BookSummary>::default();
        let rev = r.start("dune".into()).unwrap();
        r.receive(rev, Ok::<_, String>(page(&[1, 2], 1, 9)));
        r.next_page_request().unwrap();
        r.receive(rev, Err("rate limited"));
        assert_eq!(r.hits().len(), 2);
        assert_eq!(r.phase(), &Phase::Failed("rate limited".into()));
        assert!(!r.has_more());
    }

    #[test]
    fn author_pages_dedupe_by_author_id() {
        let mut r = SearchResults::<AuthorHit>::default();
        let rev = r.start("quill".into()).unwrap();
        let hits = |ids: &[i64]| {
            SearchHits::new(
                ids.iter()
                    .map(|&id| AuthorHit::new(AuthorId::new(id), format!("Author {id}")))
                    .collect(),
                1,
                25,
            )
        };
        r.receive(rev, Ok::<_, String>(hits(&[1, 2, 1])));
        let ids: Vec<i64> = r.hits().iter().map(|a| a.id.get()).collect();
        assert_eq!(ids, [1, 2]);
        assert!(!r.has_more(), "a short page is the last");
    }
}
