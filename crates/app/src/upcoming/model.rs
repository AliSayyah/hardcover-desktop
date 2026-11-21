//! Books still to come in the reader's series, as an entity: one request for
//! every series in the library. Loaded right after the library cache, on
//! Refresh, and shortly after a sync or a write changes which series the
//! library has. The last good answer is kept in the library cache
//! (`resource::saved`), so it shows at the next launch, offline too.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::{Context, Entity, Subscription, Task};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookId, Date, SeriesId, UpcomingBook};

use super::schedule::{unreleased, watched_series};
use crate::io::Priority;
use crate::library::model::{AccountChanged, LibraryModel};
use crate::resource::follow::Followed;
use crate::resource::{
    Binding, Debounce, Demand, Landing, LibraryFollow, LoadStatus, Resource, saved,
};
use crate::session::SessionScope;

/// How long the series must stay changed before reloading, so a run of
/// writes costs one request.
const RELOAD_DELAY: Duration = Duration::from_secs(2);

/// The books' row in the library cache.
const SAVED_AS: &str = "upcoming";

pub struct UpcomingModel {
    library: Entity<LibraryModel>,
    books: Resource<(), Vec<UpcomingBook>>,
    /// The library's series, as last asked about.
    series: LibraryFollow<Vec<SeriesId>>,
    /// Bumped whenever the books change.
    revision: u64,
    reload: Debounce,
    _hydrating: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl UpcomingModel {
    pub fn new(
        session: SessionScope,
        library: Entity<LibraryModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&library, |this, library, cx| {
                let found = this
                    .series
                    .check(library.read(cx), |l| watched_series(l.entries()));
                match found {
                    // The first load goes out with the cached library, without
                    // waiting for the sync that starts with it.
                    Some(Followed::First) => this.load(cx),
                    Some(Followed::Changed) => this.reload.schedule(RELOAD_DELAY, Self::reload, cx),
                    None => {}
                }
            }),
            cx.subscribe(&library, |this, _, _: &AccountChanged, cx| {
                this._hydrating = Task::ready(());
                this.books.forget_from_disk();
                this.revision += 1;
                cx.notify();
            }),
        ];
        let books = Resource::new(session.clone(), READ_CATALOG, Priority::Background)
            .persist(saved::saver(&session, SAVED_AS, saved::single));
        let hydrating = saved::hydrate(&session, SAVED_AS, saved::parse_single, Self::bind(), cx);
        Self {
            library,
            books,
            series: LibraryFollow::default(),
            revision: 0,
            reload: Debounce::default(),
            _hydrating: hydrating,
            _subscriptions: subscriptions,
        }
    }

    fn bind() -> Binding<Self, (), Vec<UpcomingBook>> {
        Binding {
            get: |m| &mut m.books,
            landed: |this, _, landing, cx| {
                if landing == Landing::Changed {
                    this.revision += 1;
                }
                cx.notify();
            },
        }
    }

    // ----- reads -----

    pub fn books(&self) -> &[UpcomingBook] {
        self.books.value(&()).map_or(&[], |b| b.as_slice())
    }

    pub fn status(&self) -> LoadStatus {
        self.books.status(&())
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The books not out yet on `today`, for Home's "Up next".
    pub fn unreleased(&self, today: Date) -> HashSet<BookId> {
        unreleased(self.books(), today)
    }

    // ----- loading -----

    /// Asks for the books still to come in the library's series where they
    /// are stale or failed (the library loaded, Refresh, a new grant),
    /// keeping the ones shown until the answer arrives. Waits for the
    /// library to load.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Shown, cx);
    }

    /// The library's series changed: the books shown are for others.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Now, cx);
    }

    fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        let library = self.library.read(cx);
        if !library.is_loaded() {
            return;
        }
        let series = watched_series(library.entries());
        let today = crate::dates::today();
        let call = move |client: &hardcover_core::Client| client.upcoming_in_series(&series, today);
        // A request still running is for older series: `Now` replaces it.
        if self.books.request((), demand, call, Self::bind(), cx) {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext as _, TestAppContext};
    use hardcover_core::store::LibraryCache;
    use std::sync::Arc;

    fn launch(
        client: hardcover_core::Client,
        cache: &Arc<LibraryCache>,
        cx: &mut TestAppContext,
    ) -> Entity<UpcomingModel> {
        let session = SessionScope::for_tests(client, Some(cache.clone()));
        cx.update(|cx| session.install(cx));
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let upcoming = cx.new(|cx| UpcomingModel::new(session, library.clone(), cx));
        library.update(cx, LibraryModel::start);
        for _ in 0..3 {
            cx.executor().advance_clock(RELOAD_DELAY * 2);
            cx.run_until_parked();
        }
        upcoming
    }

    /// B7: offline, the books of the last launch show, with the failure.
    #[gpui_kit::test]
    fn the_last_upcoming_books_show_offline(cx: &mut TestAppContext) {
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let online = launch(hardcover_core::demo::client(), &cache, cx);
        let shown = online.read_with(cx, |m, _| m.books().to_vec());
        assert!(!shown.is_empty());

        let faults = hardcover_core::demo::Faults::parse("offline").unwrap();
        let offline = launch(hardcover_core::demo::client_with(&faults), &cache, cx);
        offline.read_with(cx, |m, _| {
            assert_eq!(m.books(), shown.as_slice());
            assert!(matches!(m.status(), LoadStatus::Failed(_)));
        });
    }
}
