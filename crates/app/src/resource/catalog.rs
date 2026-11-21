//! The catalogue as this session has fetched it, shared by every view: a
//! book's details, similar books, other readers' reviews and editions,
//! series and author pages, Discover's periods and a list's books.
//!
//! Each is a [`Shared`] store, one entity per kind, holding a [`Resource`]
//! per id. Views ask for what they show ([`Catalog`]'s `request_*`) and
//! subscribe to [`SharedChanged`], re-rendering only for the ids they show;
//! they keep nothing fetched themselves. So the book page opened from a pane
//! shows the pane's details, reviews and similar books at once, with no
//! request and no skeleton.
//!
//! The workspace makes the catalogue when the user signs in
//! ([`Catalog::init`]) and drops it on signing out ([`Catalog::release`]);
//! views find it with [`Catalog::global`].

use std::collections::HashSet;
use std::hash::Hash;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use hardcover_core::auth::scopes::{READ_CATALOG, READ_LISTS, READ_SOCIAL};
use hardcover_core::{
    AuthorDetails, AuthorId, BookDetails, BookId, BookSummary, Client, CommunityReview,
    EditionSummary, ListBook, ListBookId, ListId, OffsetPage, ReviewsPage, SeriesDetails, SeriesId,
    TrendingPeriod, UserBookId, UserId,
};

use super::{Binding, Demand, Resource};
use crate::io::Priority;
use crate::session::SessionScope;

/// A key's value, loading state or failure changed.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedChanged<K>(pub K);

/// One kind of catalogue data, per id, as an entity views subscribe to.
pub struct Shared<K, V> {
    resource: Resource<K, V>,
}

impl<K: 'static, V: 'static> EventEmitter<SharedChanged<K>> for Shared<K, V> {}

impl<K, V> Shared<K, V>
where
    K: Clone + Eq + Hash + 'static,
    V: PartialEq + Send + 'static,
{
    fn new(session: SessionScope, scope: &'static str, priority: Priority) -> Self {
        Self {
            resource: Resource::new(session, scope, priority),
        }
    }

    fn bind() -> Binding<Self, K, V> {
        Binding {
            get: |s| &mut s.resource,
            landed: |_, key, _, cx| cx.emit(SharedChanged(key.clone())),
        }
    }

    pub fn resource(&self) -> &Resource<K, V> {
        &self.resource
    }

    /// [`Resource::request`], telling subscribers it started.
    pub fn request(
        &mut self,
        key: K,
        demand: Demand,
        call: impl FnOnce(&Client) -> hardcover_core::Result<V> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> bool {
        let sent = self
            .resource
            .request(key.clone(), demand, call, Self::bind(), cx);
        if sent {
            cx.emit(SharedChanged(key));
        }
        sent
    }

    /// [`Resource::extend`], telling subscribers it started.
    pub fn extend<R: Send + 'static>(
        &mut self,
        key: K,
        call: impl FnOnce(&Client) -> hardcover_core::Result<R> + Send + 'static,
        merge: fn(&V, R) -> V,
        cx: &mut Context<Self>,
    ) -> bool {
        let sent = self
            .resource
            .extend(key.clone(), call, merge, Self::bind(), cx);
        if sent {
            cx.emit(SharedChanged(key));
        }
        sent
    }

    /// [`Resource::update`]: a write's local effect on `key`.
    pub fn update(&mut self, key: &K, edit: impl FnOnce(&mut V), cx: &mut Context<Self>)
    where
        V: Clone,
    {
        self.resource.update(key, edit);
        cx.emit(SharedChanged(key.clone()));
    }

    /// A value as if fetched (tests).
    #[cfg(test)]
    pub fn set(&mut self, key: K, value: V, cx: &mut Context<Self>) {
        self.resource.set(key.clone(), value, cx);
        cx.emit(SharedChanged(key));
    }

    /// [`Resource::invalidate`]: fetched again when next asked for.
    pub fn invalidate(&mut self, key: &K) {
        self.resource.invalidate(key);
    }

    /// [`Self::invalidate`] for every key.
    pub fn invalidate_all(&mut self) {
        self.resource.invalidate_all_but(|_| false);
    }

    pub fn remove(&mut self, key: &K, cx: &mut Context<Self>) {
        self.resource.remove(key);
        cx.emit(SharedChanged(key.clone()));
    }
}

/// Rows fetched a page at a time by offset.
#[derive(Debug, Clone, PartialEq)]
pub struct Paged<T> {
    pub items: Vec<T>,
    /// Where the next page starts: the rows Hardcover has sent, less any
    /// taken off since. Rows that didn't resolve count too, so this can be
    /// more than `items.len()`.
    pub next_offset: u32,
    pub has_more: bool,
}

impl<T> Paged<T> {
    fn first(page: OffsetPage<T>) -> Self {
        Self {
            items: page.items,
            next_offset: page.next_offset,
            has_more: page.has_more,
        }
    }
}

impl<T: Clone> Paged<T> {
    /// With `page` after these rows, leaving out a row already here (offset
    /// paging repeats one when the order shifted between pages).
    fn then(&self, page: OffsetPage<T>, same: impl Fn(&T, &T) -> bool) -> Self {
        let mut items = self.items.clone();
        for item in page.items {
            if !items.iter().any(|i| same(i, &item)) {
                items.push(item);
            }
        }
        Self {
            items,
            next_offset: page.next_offset,
            has_more: page.has_more,
        }
    }
}

impl Paged<ListBook> {
    /// Takes one row off. The rows after it moved up one on Hardcover.
    pub fn remove(&mut self, row: ListBookId) {
        let before = self.items.len();
        self.items.retain(|b| b.id != row);
        if self.items.len() != before {
            self.next_offset = self.next_offset.saturating_sub(1);
        }
    }
}

/// Other readers' reviews of a book, as many pages as were asked for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReviewPages {
    pub reviews: Vec<CommunityReview>,
    /// How many there are, when Hardcover said.
    pub total: Option<u32>,
    pub next_offset: u32,
    pub has_more: bool,
    /// Pages shown: the first, then each "Show more".
    pub pages: u32,
}

impl ReviewPages {
    fn first(page: ReviewsPage, total: Option<u32>) -> Self {
        Self {
            reviews: page.reviews,
            total,
            next_offset: page.next_offset,
            has_more: page.has_more,
            pages: 1,
        }
    }

    fn then(&self, page: ReviewsPage) -> Self {
        // Likes change between requests, so a review can move into the next
        // page: show it once.
        let seen: HashSet<UserBookId> = self.reviews.iter().map(|r| r.id).collect();
        let mut reviews = self.reviews.clone();
        reviews.extend(page.reviews.into_iter().filter(|r| !seen.contains(&r.id)));
        Self {
            reviews,
            total: self.total,
            next_offset: page.next_offset,
            has_more: page.has_more,
            pages: self.pages + 1,
        }
    }
}

/// Similar books fetched: two rows of lg covers in the side column of a wide
/// window. One `books` field, so one request whatever the count.
pub const SIMILAR_FETCHED: usize = 16;

/// The session's catalogue stores. Cheap to clone.
#[derive(Clone)]
pub struct Catalog {
    pub details: Entity<Shared<BookId, BookDetails>>,
    pub similar: Entity<Shared<BookId, Vec<BookSummary>>>,
    pub reviews: Entity<Shared<BookId, ReviewPages>>,
    pub editions: Entity<Shared<BookId, Paged<EditionSummary>>>,
    pub series: Entity<Shared<SeriesId, SeriesDetails>>,
    pub authors: Entity<Shared<AuthorId, AuthorDetails>>,
    pub trending: Entity<Shared<TrendingPeriod, Vec<BookSummary>>>,
    pub list_books: Entity<Shared<ListId, Paged<ListBook>>>,
}

struct CatalogGlobal(Catalog);

impl Global for CatalogGlobal {}

impl Catalog {
    /// Makes the catalogue for a new session and installs it.
    pub fn init(session: &SessionScope, cx: &mut App) -> Self {
        let s = || session.clone();
        let catalog = Self {
            details: cx.new(|_| Shared::new(s(), READ_CATALOG, Priority::Interactive)),
            similar: cx.new(|_| Shared::new(s(), READ_CATALOG, Priority::Background)),
            reviews: cx.new(|_| Shared::new(s(), READ_SOCIAL, Priority::Background)),
            editions: cx.new(|_| Shared::new(s(), READ_CATALOG, Priority::Interactive)),
            series: cx.new(|_| Shared::new(s(), READ_CATALOG, Priority::Interactive)),
            authors: cx.new(|_| Shared::new(s(), READ_CATALOG, Priority::Interactive)),
            trending: cx.new(|_| Shared::new(s(), READ_CATALOG, Priority::Interactive)),
            list_books: cx.new(|_| Shared::new(s(), READ_LISTS, Priority::Interactive)),
        };
        cx.set_global(CatalogGlobal(catalog.clone()));
        catalog
    }

    /// Forgets the catalogue, on signing out.
    pub fn release(cx: &mut App) {
        if cx.has_global::<CatalogGlobal>() {
            cx.remove_global::<CatalogGlobal>();
        }
    }

    /// The session's catalogue, once signed in.
    pub fn global(cx: &App) -> Option<Self> {
        cx.try_global::<CatalogGlobal>().map(|g| g.0.clone())
    }

    // ----- books -----

    pub fn request_details(&self, id: BookId, demand: Demand, cx: &mut App) {
        self.details.update(cx, |s, cx| {
            s.request(id, demand, move |c| c.book_details(id), cx);
        });
    }

    /// Asks for the books Hardcover lists as similar to `id`, once its
    /// details (which name them) are in.
    pub fn request_similar(&self, id: BookId, demand: Demand, cx: &mut App) {
        let ids: Vec<BookId> = match self.details.read(cx).resource().value(&id) {
            Some(details) => details
                .similar_book_ids
                .iter()
                .copied()
                .filter(|b| *b != id)
                .take(SIMILAR_FETCHED)
                .collect(),
            None => return,
        };
        if ids.is_empty() {
            return;
        }
        self.similar.update(cx, |s, cx| {
            s.request(id, demand, move |c| c.books_by_ids(&ids), cx);
        });
    }

    /// Asks for the first page of other readers' reviews of `book`, with the
    /// count when there is more than a page. `me` is the signed-in account
    /// once known (its own review is left out); else `me()` tells. While the
    /// book stays on screen ([`Demand::Active`]), pages the reader opened
    /// are not folded back into a fresh first page.
    pub fn request_reviews(&self, book: BookId, me: Option<UserId>, demand: Demand, cx: &mut App) {
        let paged = self
            .reviews
            .read(cx)
            .resource()
            .value(&book)
            .is_some_and(|r| r.pages > 1);
        if demand == Demand::Active && paged {
            return;
        }
        self.reviews.update(cx, |s, cx| {
            s.request(
                book,
                demand,
                move |client| {
                    let me = match me {
                        Some(me) => me,
                        None => client.me()?.id,
                    };
                    let page = client.book_reviews(book, me, 0)?;
                    // A first page that holds them all is its own count; a
                    // count that fails costs only the caption.
                    let total = if page.has_more {
                        client.book_reviews_count(book, me).ok()
                    } else {
                        u32::try_from(page.reviews.len()).ok()
                    };
                    Ok(ReviewPages::first(page, total))
                },
                cx,
            );
        });
    }

    /// Asks for the next page of reviews of `book`.
    pub fn more_reviews(&self, book: BookId, me: Option<UserId>, cx: &mut App) {
        let Some(offset) = self
            .reviews
            .read(cx)
            .resource()
            .value(&book)
            .map(|r| r.next_offset)
        else {
            return;
        };
        self.reviews.update(cx, |s, cx| {
            s.extend(
                book,
                move |client| {
                    let me = match me {
                        Some(me) => me,
                        None => client.me()?.id,
                    };
                    client.book_reviews(book, me, offset)
                },
                ReviewPages::then,
                cx,
            );
        });
    }

    /// Asks for the first page of `book`'s editions.
    pub fn request_editions(&self, book: BookId, demand: Demand, cx: &mut App) {
        self.editions.update(cx, |s, cx| {
            s.request(
                book,
                demand,
                move |c| c.book_editions(book, 0).map(Paged::first),
                cx,
            );
        });
    }

    /// Asks for the next page of `book`'s editions.
    pub fn more_editions(&self, book: BookId, cx: &mut App) {
        let Some(offset) = self
            .editions
            .read(cx)
            .resource()
            .value(&book)
            .map(|p| p.next_offset)
        else {
            return;
        };
        self.editions.update(cx, |s, cx| {
            s.extend(
                book,
                move |c| c.book_editions(book, offset),
                |paged, page| paged.then(page, |a, b| a.id == b.id),
                cx,
            );
        });
    }

    // ----- pages -----

    pub fn request_series(&self, id: SeriesId, demand: Demand, cx: &mut App) {
        self.series.update(cx, |s, cx| {
            s.request(id, demand, move |c| c.series(id), cx);
        });
    }

    pub fn request_author(&self, id: AuthorId, demand: Demand, cx: &mut App) {
        self.authors.update(cx, |s, cx| {
            s.request(id, demand, move |c| c.author(id), cx);
        });
    }

    pub fn request_trending(&self, period: TrendingPeriod, demand: Demand, cx: &mut App) {
        self.trending.update(cx, |s, cx| {
            s.request(period, demand, move |c| c.trending_books(period), cx);
        });
    }

    /// Asks for the first page of `list`'s books.
    pub fn request_list_books(&self, list: ListId, demand: Demand, cx: &mut App) {
        self.list_books.update(cx, |s, cx| {
            s.request(
                list,
                demand,
                move |c| c.list_books(list, 0).map(Paged::first),
                cx,
            );
        });
    }

    /// Asks for the next page of `list`'s books.
    pub fn more_list_books(&self, list: ListId, cx: &mut App) {
        let Some(offset) = self
            .list_books
            .read(cx)
            .resource()
            .value(&list)
            .filter(|p| p.has_more)
            .map(|p| p.next_offset)
        else {
            return;
        };
        self.list_books.update(cx, |s, cx| {
            s.extend(
                list,
                move |c| c.list_books(list, offset),
                // A ranked list's row moved while paging shows once.
                |paged, page| paged.then(page, |a, b| a.id == b.id),
                cx,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    fn catalog(cx: &mut TestAppContext) -> Catalog {
        let session = SessionScope::for_tests(hardcover_core::demo::client(), None);
        cx.update(|cx| {
            session.install(cx);
            Catalog::init(&session, cx)
        })
    }

    /// Project Hail Mary in the demo catalogue: more than a page of reviews.
    const BOOK: BookId = BookId::new(427_578);

    #[gpui_kit::test]
    fn reviews_page_on_and_fold_back_only_when_shown(cx: &mut TestAppContext) {
        let catalog = catalog(cx);
        let pages = |cx: &mut TestAppContext| {
            catalog
                .reviews
                .read_with(cx, |s, _| s.resource().value(&BOOK).map(|r| r.pages))
        };
        cx.update(|cx| catalog.request_reviews(BOOK, None, Demand::Shown, cx));
        cx.run_until_parked();
        assert_eq!(pages(cx), Some(1));
        cx.update(|cx| catalog.more_reviews(BOOK, None, cx));
        cx.run_until_parked();
        assert_eq!(pages(cx), Some(2));

        // The window became active: the pages the reader opened stay.
        cx.update(|cx| {
            super::super::refresh_all(cx);
            catalog.request_reviews(BOOK, None, Demand::Active, cx);
        });
        cx.run_until_parked();
        assert_eq!(pages(cx), Some(2));

        // Refresh, or the book shown again: a fresh first page replaces them.
        cx.update(|cx| catalog.request_reviews(BOOK, None, Demand::Shown, cx));
        cx.run_until_parked();
        assert_eq!(pages(cx), Some(1));
        catalog.reviews.read_with(cx, |s, _| {
            let reviews = s.resource().value(&BOOK).unwrap();
            assert!(reviews.has_more);
            assert!(!reviews.reviews.is_empty());
            assert!(reviews.total.is_some());
        });
    }

    #[test]
    fn a_list_row_taken_off_moves_the_next_page_back_one() {
        use hardcover_core::BookId;
        let book = |row: i64| {
            ListBook::new(
                ListBookId::new(row),
                BookSummary::new(BookId::new(row + 1000), format!("Book {row}")),
            )
        };
        // 100 rows sent, one of them unresolved.
        let mut first = OffsetPage::default();
        first.items = (1..=99).map(book).collect();
        first.next_offset = 100;
        first.has_more = true;
        let mut paged = Paged::first(first);
        assert_eq!((paged.items.len(), paged.next_offset), (99, 100));
        paged.remove(ListBookId::new(5));
        assert_eq!(paged.next_offset, 99);
        // A row not loaded here changes nothing.
        paged.remove(ListBookId::new(500));
        assert_eq!(paged.next_offset, 99);
    }
}
