//! The genres and moods of the books on the shelves, for each shelf's
//! filter bar (`library::tag_filter`). Asked for a hundred books at a time
//! (`Client::book_tags`, `read:catalog`), only for the shelves shown this
//! session: a shelf's books are asked for when it is first shown, and
//! those stale or failed when it is shown again or on Refresh.
//!
//! - Each book is kept, and goes stale, on its own (a [`Resource`] keyed by
//!   book), and the last good copies are kept in the library cache
//!   (`resource::saved`), so the chips show at once at the next launch,
//!   offline too.
//! - When a sync or a write changes which books are on the shelves shown,
//!   shortly after (not during a sync), only the books not known yet are
//!   asked for; those that left the library are forgotten.
//! - A batch that fails keeps its books' previous copies; the bar says so,
//!   with "Try again" ([`ShelfTagsModel::retry`]).
//!
//! What the filter knows of each book ([`BookFacts`]) is worked out once,
//! when its tags land.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{Context, Entity, SharedString, Subscription, Task};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::tags::{BookGenres, BookVotes};
use hardcover_core::{BookId, BookTags, BookTagsLoad, Client, ReadingStatus};

use super::model::{AccountChanged, LibraryModel};
use super::tag_filter::BookFacts;
use crate::io::Priority;
use crate::resource::follow::Followed;
use crate::resource::{
    Batch, Binding, Debounce, Demand, Landing, LibraryFollow, LoadStatus, Resource, saved,
};
use crate::session::SessionScope;

/// How long the shelves' books must stay changed before asking, so a run of
/// writes costs one request.
const RELOAD_DELAY: Duration = Duration::from_secs(2);

/// The books' rows in the library cache.
const SAVED_AS: &str = "book_tags";

/// Where a shelf's tags stand, for its filter bar.
#[derive(Debug, Clone, PartialEq)]
pub enum ShelfTags {
    /// Without `read:catalog`: no bar.
    Unavailable,
    /// Nothing known yet for the shelf's books: skeleton chips.
    Loading,
    /// Nothing known, and the request failed.
    Failed(SharedString),
    /// Known for some or all of the books; `failure` says why the rest
    /// aren't.
    Ready { failure: Option<SharedString> },
}

pub struct ShelfTagsModel {
    session: SessionScope,
    library: Entity<LibraryModel>,
    tags: Resource<BookId, BookTags>,
    facts: HashMap<BookId, Rc<BookFacts>>,
    /// The shelves shown this session, whose books are asked for.
    shelves: Vec<ReadingStatus>,
    /// The books on those shelves, as last seen in the library.
    books: LibraryFollow<Vec<BookId>>,
    /// Bumped whenever a book's facts change.
    revision: u64,
    reload: Debounce,
    _hydrating: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl ShelfTagsModel {
    pub fn new(
        session: SessionScope,
        library: Entity<LibraryModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&library, |this, library, cx| {
                let shelves = this.shelves.clone();
                let found = this
                    .books
                    .check(library.read(cx), |l| books_on(l, &shelves));
                match found {
                    // The library loaded: the shelves already shown are asked
                    // for now.
                    Some(Followed::First) => this.load(cx),
                    Some(Followed::Changed) => {
                        this.reload.schedule(RELOAD_DELAY, Self::follow_books, cx);
                    }
                    None => {}
                }
            }),
            cx.subscribe(&library, |this, _, _: &AccountChanged, cx| {
                this._hydrating = Task::ready(());
                this.tags.forget_from_disk();
                this.rebuild_facts();
                cx.notify();
            }),
        ];
        let tags = Resource::new(session.clone(), READ_CATALOG, Priority::Background).persist(
            saved::saver(&session, SAVED_AS, |id: &BookId| id.get().to_string()),
        );
        let hydrating = saved::hydrate(
            &session,
            SAVED_AS,
            |key| key.parse().ok().map(BookId::new),
            Self::bind(),
            cx,
        );
        Self {
            session,
            library,
            tags,
            facts: HashMap::new(),
            shelves: Vec::new(),
            books: LibraryFollow::default(),
            revision: 0,
            reload: Debounce::default(),
            _hydrating: hydrating,
            _subscriptions: subscriptions,
        }
    }

    fn bind() -> Binding<Self, BookId, BookTags> {
        Binding {
            get: |m| &mut m.tags,
            landed: |this, id, landing, cx| {
                if landing == Landing::Changed
                    && let Some(tags) = this.tags.value(id)
                {
                    this.facts.insert(*id, Rc::new(BookFacts::of(tags)));
                    this.revision += 1;
                }
                cx.notify();
            },
        }
    }

    fn rebuild_facts(&mut self) {
        self.facts = self
            .tags
            .values()
            .map(|(id, tags)| (*id, Rc::new(BookFacts::of(tags))))
            .collect();
        self.revision += 1;
    }

    // ----- reads -----

    /// Bumped whenever a book's facts change.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// What the filter knows of `book`, if its tags are known.
    pub fn facts(&self, book: BookId) -> Option<&BookFacts> {
        self.facts.get(&book).map(Rc::as_ref)
    }

    /// Where `status`'s tags stand. Its books are those `books` lists.
    pub fn shelf(&self, books: &[BookId]) -> ShelfTags {
        if !self.session.has_scope(READ_CATALOG) {
            return ShelfTags::Unavailable;
        }
        let known = books.iter().any(|b| self.facts.contains_key(b));
        let status = self.tags.summary(books);
        match (known, status) {
            (true, LoadStatus::Failed(message)) => ShelfTags::Ready {
                failure: Some(message),
            },
            (true, _) => ShelfTags::Ready { failure: None },
            (false, LoadStatus::Failed(message)) => ShelfTags::Failed(message),
            // Every book settled with nothing to say (no votes, unknown to
            // Hardcover), or an empty shelf: rows with nothing to offer.
            (false, _) if books.iter().all(|b| self.tags.has_settled(b)) => {
                ShelfTags::Ready { failure: None }
            }
            (false, _) => ShelfTags::Loading,
        }
    }

    // ----- loading -----

    /// `status` is shown: asks for its books that were never asked for,
    /// are stale or failed.
    pub fn show_shelf(&mut self, status: ReadingStatus, cx: &mut Context<Self>) {
        if !self.shelves.contains(&status) {
            self.shelves.push(status);
        }
        // Until the library loads, its first load asks (`Followed::First`).
        if self.books.seen().is_some() {
            let books = books_on(self.library.read(cx), &[status]);
            self.request(books, Demand::Shown, cx);
        }
    }

    /// The window became active on `status`: asks for its stale books.
    pub fn revalidate(&mut self, status: ReadingStatus, cx: &mut Context<Self>) {
        if self.books.seen().is_some() && self.shelves.contains(&status) {
            let books = books_on(self.library.read(cx), &[status]);
            self.request(books, Demand::Active, cx);
        }
    }

    /// Asks for the books of every shelf shown that are stale or failed
    /// (the library loaded, Refresh, a new grant).
    pub fn load(&mut self, cx: &mut Context<Self>) {
        if self.books.seen().is_none() {
            return;
        }
        let books = books_on(self.library.read(cx), &self.shelves);
        self.request(books, Demand::Shown, cx);
    }

    /// "Try again": asks for `status`'s books whose tags failed or never
    /// came, now.
    pub fn retry(&mut self, status: ReadingStatus, cx: &mut Context<Self>) {
        let books: Vec<BookId> = books_on(self.library.read(cx), &[status])
            .into_iter()
            .filter(|b| self.tags.failure(b).is_some() || !self.tags.has_settled(b))
            .collect();
        self.request(books, Demand::Now, cx);
    }

    /// The shelves' books changed: forgets the books that left the library
    /// and asks only for those not known yet.
    fn follow_books(&mut self, cx: &mut Context<Self>) {
        let library: HashSet<BookId> = self
            .library
            .read(cx)
            .entries()
            .iter()
            .map(|e| e.book.id)
            .collect();
        let before = self.facts.len();
        self.tags.retain(|id| library.contains(id));
        self.facts.retain(|id, _| library.contains(id));
        if self.facts.len() != before {
            self.revision += 1;
            cx.notify();
        }
        let unknown: Vec<BookId> = self
            .books
            .seen()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .copied()
            .filter(|b| self.tags.value(b).is_none())
            .collect();
        self.request(unknown, Demand::Active, cx);
    }

    fn request(&mut self, books: Vec<BookId>, demand: Demand, cx: &mut Context<Self>) {
        let call = |client: &Client, ids: &[BookId]| batch_of(ids, client.book_tags(ids));
        if self
            .tags
            .request_many(books, demand, call, Self::bind(), cx)
            > 0
        {
            cx.notify();
        }
    }
}

/// The books on `shelves`, in id order, each once.
fn books_on(library: &LibraryModel, shelves: &[ReadingStatus]) -> Vec<BookId> {
    let mut books: Vec<BookId> = library
        .entries()
        .iter()
        .filter(|e| shelves.contains(&e.status))
        .map(|e| e.book.id)
        .collect();
    books.sort_unstable_by_key(|b| b.get());
    books.dedup();
    books
}

/// A load's answer per book: each that loaded, no tags for one Hardcover
/// left out (it knows nothing of it, so asking again would say the same),
/// and those of a batch that failed, which keep their previous copies.
fn batch_of(asked: &[BookId], load: BookTagsLoad) -> Batch<BookId, BookTags> {
    let missing: HashSet<BookId> = load.missing.iter().copied().collect();
    let mut found: Vec<(BookId, BookTags)> = load.books.into_iter().map(|t| (t.book, t)).collect();
    let answered: HashSet<BookId> = found.iter().map(|(id, _)| *id).collect();
    found.extend(
        asked
            .iter()
            .filter(|id| !answered.contains(id) && !missing.contains(id))
            .map(|id| {
                (
                    *id,
                    BookTags::new(*id, BookGenres::default(), BookVotes::default()),
                )
            }),
    );
    Batch {
        found,
        failed: load.missing,
        error: load.error,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::{AppContext as _, TestAppContext};
    use hardcover_core::store::LibraryCache;
    use hardcover_core::tags::Mood;

    use super::*;
    use crate::library::tag_filter::{TagPick, facets};

    fn ids(xs: &[i64]) -> Vec<BookId> {
        xs.iter().copied().map(BookId::new).collect()
    }

    fn tags(id: i64) -> BookTags {
        BookTags::new(
            BookId::new(id),
            BookGenres::from_tags([("Fantasy", 10)]),
            BookVotes::default(),
        )
    }

    /// Every book asked for is answered: found, failed, or known to have
    /// nothing.
    #[test]
    fn a_load_answers_for_every_book_asked() {
        let mut load = BookTagsLoad::default();
        load.books = vec![tags(1), tags(4)];
        load.missing = ids(&[2]);
        load.error = Some(hardcover_core::Error::Timeout);
        let batch = batch_of(&ids(&[1, 2, 3, 4]), load);
        let found: Vec<(i64, bool)> = batch
            .found
            .iter()
            .map(|(id, t)| (id.get(), t.genres.is_empty()))
            .collect();
        assert_eq!(found, [(1, false), (4, false), (3, true)]);
        assert_eq!(batch.failed, ids(&[2]));
        assert!(batch.error.is_some());
    }

    fn launch(
        client: Client,
        cache: Option<Arc<LibraryCache>>,
        cx: &mut TestAppContext,
    ) -> (Entity<LibraryModel>, Entity<ShelfTagsModel>) {
        let session = SessionScope::for_tests(client, cache);
        cx.update(|cx| session.install(cx));
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let model = cx.new(|cx| ShelfTagsModel::new(session, library.clone(), cx));
        model.update(cx, |m, cx| m.show_shelf(ReadingStatus::WantToRead, cx));
        library.update(cx, LibraryModel::start);
        for _ in 0..3 {
            cx.executor().advance_clock(RELOAD_DELAY * 2);
            cx.run_until_parked();
        }
        (library, model)
    }

    /// The demo's Want to read combines: three Science Fiction books are
    /// Reflective, four Fantasy books Adventurous.
    #[gpui_kit::test]
    fn want_to_read_combines_genres_and_moods(cx: &mut TestAppContext) {
        let (library, model) = launch(hardcover_core::demo::client(), None, cx);
        let count = |genre: &str, mood: Mood, cx: &mut TestAppContext| {
            let books = library.read_with(cx, |l, _| books_on(l, &[ReadingStatus::WantToRead]));
            model.read_with(cx, |m, _| {
                let mut pick = TagPick::default();
                pick.press_mood(mood);
                let f = facets(books.iter().map(|b| m.facts(*b)), &pick);
                f.genres
                    .iter()
                    .find(|c| c.name == genre)
                    .map_or(0, |c| c.count)
            })
        };
        assert_eq!(count("Science Fiction", Mood::Reflective, cx), 3);
        assert_eq!(count("Fantasy", Mood::Adventurous, cx), 4);
        let books = library.read_with(cx, |l, _| books_on(l, &[ReadingStatus::WantToRead]));
        model.read_with(cx, |m, _| {
            assert_eq!(m.shelf(&books), ShelfTags::Ready { failure: None });
        });
    }

    /// Offline, the tags of the last launch show, with the failure.
    #[gpui_kit::test]
    fn the_last_tags_show_offline(cx: &mut TestAppContext) {
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let (_, online) = launch(hardcover_core::demo::client(), Some(cache.clone()), cx);
        let known = online.read_with(cx, |m, _| m.facts.len());
        assert!(known > 0);

        let faults = hardcover_core::demo::Faults::parse("offline").unwrap();
        let (library, offline) =
            launch(hardcover_core::demo::client_with(&faults), Some(cache), cx);
        let books = library.read_with(cx, |l, _| books_on(l, &[ReadingStatus::WantToRead]));
        offline.read_with(cx, |m, _| {
            assert_eq!(m.facts.len(), known);
            assert!(matches!(
                m.shelf(&books),
                ShelfTags::Ready { failure: Some(_) }
            ));
        });
    }

    /// With nothing known, a failure takes the rows' place.
    #[gpui_kit::test]
    fn a_first_failure_says_so(cx: &mut TestAppContext) {
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        launch(hardcover_core::demo::client(), Some(cache.clone()), cx);
        // The library stays cached; the tags don't.
        cx.run_until_parked();
        let saved = cache.load_companions::<BookTags>(SAVED_AS).unwrap();
        assert!(!saved.is_empty());
        for (key, _) in saved {
            cache.remove_companion(SAVED_AS, &key).unwrap();
        }
        let faults = hardcover_core::demo::Faults::parse("offline").unwrap();
        let (library, model) = launch(hardcover_core::demo::client_with(&faults), Some(cache), cx);
        let books = library.read_with(cx, |l, _| books_on(l, &[ReadingStatus::WantToRead]));
        assert!(!books.is_empty());
        model.read_with(cx, |m, _| {
            assert!(matches!(m.shelf(&books), ShelfTags::Failed(_)));
        });
    }

    /// Only the books not known yet are asked for when the shelves change;
    /// those that left the library are forgotten.
    #[gpui_kit::test]
    fn a_change_asks_only_for_the_books_added(cx: &mut TestAppContext) {
        let (library, model) = launch(hardcover_core::demo::client(), None, cx);
        let shelf = library.read_with(cx, |l, _| books_on(l, &[ReadingStatus::WantToRead]));
        let started = model.read_with(cx, |m, _| m.tags.started());
        assert_eq!(started, shelf.len(), "each book once");

        // A book of another shelf joins the ones followed.
        let other = library.read_with(cx, |l, _| books_on(l, &[ReadingStatus::Read]));
        let mut seen = shelf.clone();
        seen.push(other[0]);
        model.update(cx, |m, cx| {
            m.books = LibraryFollow::seen_as(seen);
            m.follow_books(cx);
        });
        cx.run_until_parked();
        model.read_with(cx, |m, _| {
            assert_eq!(m.tags.started(), started + 1, "only the new one");
            assert!(m.tags.value(&other[0]).is_some());
        });

        // Shown again: nothing is stale, nothing is asked for.
        model.update(cx, |m, cx| m.show_shelf(ReadingStatus::WantToRead, cx));
        cx.run_until_parked();
        assert_eq!(model.read_with(cx, |m, _| m.tags.started()), started + 1);
    }
}
