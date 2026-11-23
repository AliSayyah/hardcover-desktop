//! Books still to come in the series the reader follows, as an entity: one
//! request for every followed series. Loaded once the follows are known
//! (the last launch's, then Hardcover's), on Refresh, and again when the
//! follows change. Until they are known nothing is asked or dropped. The
//! last good answer is kept in the library cache (`resource::saved`), so it
//! shows at the next launch, offline too.

use std::collections::HashSet;

use gpui_kit::{Context, Entity, Subscription, Task};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookId, Date, SeriesId, UpcomingBook};

use super::schedule::unreleased;
use crate::io::Priority;
use crate::library::model::{AccountChanged, LibraryModel};
use crate::resource::{Binding, Demand, Landing, LoadStatus, Resource, saved};
use crate::series::follows::FollowsModel;
use crate::session::SessionScope;

/// The books' row in the library cache.
const SAVED_AS: &str = "upcoming";

pub struct UpcomingModel {
    books: Resource<(), Vec<UpcomingBook>>,
    /// The followed series, as last asked about; `None` until the follows
    /// are known.
    series: Option<Vec<SeriesId>>,
    /// Bumped whenever the books change.
    revision: u64,
    _hydrating: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl UpcomingModel {
    pub fn new(
        session: SessionScope,
        library: Entity<LibraryModel>,
        follows: Entity<FollowsModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&follows, |this, follows, cx| {
                let Some(ids) = follows.read(cx).ids() else {
                    return;
                };
                if this.series.as_ref() == Some(&ids) {
                    return;
                }
                let first = this.series.is_none();
                this.series = Some(ids);
                // The books shown are for other series: `Now` replaces them.
                this.request(if first { Demand::Shown } else { Demand::Now }, cx);
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
            books,
            series: None,
            revision: 0,
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

    /// Asks for the books still to come in the followed series where they
    /// are stale or failed (Refresh, a new grant), keeping the ones shown
    /// until the answer arrives. Waits for the follows to be known.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Shown, cx);
    }

    fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        let Some(series) = self.series.clone() else {
            return;
        };
        if series.is_empty() {
            // Following nothing: nothing to come, and nothing to ask.
            if self.books.value(&()).is_none_or(|b| !b.is_empty()) {
                self.books.set((), Vec::new(), cx);
                self.revision += 1;
                cx.notify();
            }
            return;
        }
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
        let follows = cx.update(|cx| FollowsModel::init(session.clone(), &library, cx));
        let upcoming =
            cx.new(|cx| UpcomingModel::new(session, library.clone(), follows.clone(), cx));
        library.update(cx, LibraryModel::start);
        follows.update(cx, FollowsModel::load);
        for _ in 0..3 {
            cx.executor()
                .advance_clock(std::time::Duration::from_secs(4));
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
