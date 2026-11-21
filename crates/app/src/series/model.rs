//! The series the reader has started, as an entity: every book of each,
//! asked for a batch at a time (`Client::tracked_series`). Loaded right
//! after the library cache, on Refresh and after signing in again; when a
//! sync or a write changes which series are started, only the series added
//! are asked for, shortly after, and those no longer started are dropped.
//! Each series is kept, and goes stale, on its own (a [`Resource`] keyed by
//! series), and the last good copies are kept in the library cache
//! (`resource::saved`), so they show at the next launch, offline too.
//!
//! It also keeps the summaries the destination and its sidebar row show
//! (`progress::summaries`), worked out once per change of the series, the
//! library, the upcoming books or the day.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{App, Context, Entity, Subscription, Task};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookId, Client, Date, SeriesId, TrackedSeries, TrackedSeriesLoad};

use super::progress::{SeriesSummary, started_series, summaries};
use crate::io::Priority;
use crate::library::model::{AccountChanged, LibraryModel};
use crate::resource::follow::Followed;
use crate::resource::{
    Batch, Binding, Debounce, Demand, Landing, LibraryFollow, LoadStatus, Resource, saved,
};
use crate::session::SessionScope;
use crate::upcoming::model::UpcomingModel;

/// How long the started series must stay changed before loading, so a run
/// of writes costs one load.
const RELOAD_DELAY: Duration = Duration::from_secs(2);

/// The series' rows in the library cache.
const SAVED_AS: &str = "series";

/// What the summaries were worked out from.
type SummaryKey = (u64, u64, u64, Date);

pub struct SeriesModel {
    library: Entity<LibraryModel>,
    upcoming: Entity<UpcomingModel>,
    /// Each started series; `None` when Hardcover returned nothing for it
    /// (a merged or emptied series).
    series: Resource<SeriesId, Option<TrackedSeries>>,
    /// The started series, as last seen in the library.
    started: LibraryFollow<Vec<SeriesId>>,
    /// Bumped whenever a series changes.
    revision: u64,
    /// The last summaries worked out, and from what. A cache, filled on
    /// read, so the views can ask from `render` without updating the model.
    summaries: RefCell<Option<(SummaryKey, Rc<Vec<SeriesSummary>>)>>,
    reload: Debounce,
    _hydrating: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SeriesModel {
    pub fn new(
        session: SessionScope,
        library: Entity<LibraryModel>,
        upcoming: Entity<UpcomingModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&library, |this, library, cx| {
                let found = this
                    .started
                    .check(library.read(cx), |l| started_series(l.entries()));
                match found {
                    // The first load goes out with the cached library, without
                    // waiting for the sync that starts with it.
                    Some(Followed::First) => this.load(cx),
                    Some(Followed::Changed) => {
                        this.reload.schedule(RELOAD_DELAY, Self::follow_started, cx);
                    }
                    None => {}
                }
            }),
            cx.subscribe(&library, |this, _, _: &AccountChanged, cx| {
                this._hydrating = Task::ready(());
                this.series.forget_from_disk();
                this.revision += 1;
                cx.notify();
            }),
            cx.observe(&upcoming, |_, _, cx| cx.notify()),
        ];
        let series = Resource::new(session.clone(), READ_CATALOG, Priority::Background).persist(
            saved::saver(&session, SAVED_AS, |id: &SeriesId| id.get().to_string()),
        );
        let hydrating = saved::hydrate(
            &session,
            SAVED_AS,
            |key| key.parse().ok().map(SeriesId::new),
            Self::bind(),
            cx,
        );
        Self {
            library,
            upcoming,
            series,
            started: LibraryFollow::default(),
            revision: 0,
            summaries: RefCell::new(None),
            reload: Debounce::default(),
            _hydrating: hydrating,
            _subscriptions: subscriptions,
        }
    }

    fn bind() -> Binding<Self, SeriesId, Option<TrackedSeries>> {
        Binding {
            get: |m| &mut m.series,
            landed: |this, _, landing, cx| {
                if landing == Landing::Changed {
                    this.revision += 1;
                }
                cx.notify();
            },
        }
    }

    // ----- reads -----

    fn started(&self) -> &[SeriesId] {
        self.started.seen().map_or(&[], Vec::as_slice)
    }

    /// One status for every started series: loading while any is, else the
    /// first failure.
    pub fn status(&self) -> LoadStatus {
        self.series.summary(self.started())
    }

    /// Whether every started series has loaded or failed, with something to
    /// show, so an empty result means no series.
    pub fn is_loaded(&self) -> bool {
        self.started.seen().is_some_and(|ids| {
            ids.iter().all(|id| self.series.has_settled(id))
                && (ids.is_empty() || ids.iter().any(|id| self.series.value(id).is_some()))
        })
    }

    /// Every started series, in group order, each group sorted. Worked out
    /// again only when the series, the library, the upcoming books or the
    /// day changed.
    pub fn summaries(&self, today: Date, cx: &App) -> Rc<Vec<SeriesSummary>> {
        let library = self.library.read(cx);
        let upcoming = self.upcoming.read(cx);
        let key = (
            self.revision,
            library.revision(),
            upcoming.revision(),
            today,
        );
        if let Some((k, summaries)) = &*self.summaries.borrow()
            && *k == key
        {
            return summaries.clone();
        }
        // The upcoming books know the dates of books the series don't.
        let dates: HashMap<BookId, Date> = upcoming
            .books()
            .iter()
            .map(|b| (b.book.id, b.release_date))
            .collect();
        let series: Vec<TrackedSeries> = self
            .started()
            .iter()
            .filter_map(|id| self.series.value(id)?.as_ref().clone())
            .collect();
        let computed = Rc::new(summaries(&series, library.entries(), &dates, today));
        *self.summaries.borrow_mut() = Some((key, computed.clone()));
        computed
    }

    // ----- loading -----

    /// Asks for the started series that are stale or failed (the library
    /// loaded, Refresh, a new grant, "Try again"), keeping the ones shown
    /// until the answers arrive.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let started = self.started().to_vec();
        self.request(started, Demand::Shown, cx);
    }

    /// The started series changed: forgets those no longer started and asks
    /// for the new ones.
    fn follow_started(&mut self, cx: &mut Context<Self>) {
        let started: HashSet<SeriesId> = self.started().iter().copied().collect();
        let before = self.series.values().count();
        self.series.retain(|id| started.contains(id));
        if self.series.values().count() != before {
            self.revision += 1;
            cx.notify();
        }
        let started = self.started().to_vec();
        self.request(started, Demand::Active, cx);
    }

    fn request(&mut self, ids: Vec<SeriesId>, demand: Demand, cx: &mut Context<Self>) {
        let call = |client: &Client, ids: &[SeriesId]| batch_of(ids, client.tracked_series(ids));
        if self
            .series
            .request_many(ids, demand, call, Self::bind(), cx)
            > 0
        {
            cx.notify();
        }
    }
}

/// A load's answer per series: each that loaded, `None` for one Hardcover
/// returned nothing for, and those of a batch that failed, which keep their
/// previous copies.
fn batch_of(asked: &[SeriesId], load: TrackedSeriesLoad) -> Batch<SeriesId, Option<TrackedSeries>> {
    let missing: HashSet<SeriesId> = load.missing.iter().copied().collect();
    let mut found: Vec<(SeriesId, Option<TrackedSeries>)> =
        load.series.into_iter().map(|s| (s.id, Some(s))).collect();
    let answered: HashSet<SeriesId> = found.iter().map(|(id, _)| *id).collect();
    found.extend(
        asked
            .iter()
            .filter(|id| !answered.contains(id) && !missing.contains(id))
            .map(|id| (*id, None)),
    );
    Batch {
        found,
        failed: load.missing,
        error: load.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext as _, TestAppContext};
    use hardcover_core::Error;

    fn tracked(id: i64) -> TrackedSeries {
        TrackedSeries::new(SeriesId::new(id), format!("Series {id}"))
    }

    fn load(series: &[i64], missing: &[i64], failed: bool) -> TrackedSeriesLoad {
        let mut l = TrackedSeriesLoad::default();
        l.series = series.iter().copied().map(tracked).collect();
        l.missing = missing.iter().copied().map(SeriesId::new).collect();
        l.error = failed.then_some(Error::Timeout);
        l
    }

    fn ids(xs: &[i64]) -> Vec<SeriesId> {
        xs.iter().copied().map(SeriesId::new).collect()
    }

    /// One failed batch doesn't make its series vanish from the tracker: they
    /// fail (keeping their copies), the others load, and a series Hardcover
    /// returned nothing for is known to have nothing.
    #[test]
    fn a_load_answers_for_every_series_asked() {
        let batch = batch_of(&ids(&[1, 2, 3, 4]), load(&[1, 4], &[2], true));
        let found: Vec<(i64, bool)> = batch
            .found
            .iter()
            .map(|(id, s)| (id.get(), s.is_some()))
            .collect();
        assert_eq!(found, [(1, true), (4, true), (3, false)]);
        assert_eq!(batch.failed, ids(&[2]));
        assert!(batch.error.is_some());
    }

    /// B7: offline, the series of the last launch show, with the failure.
    #[gpui_kit::test]
    fn the_last_series_show_offline(cx: &mut TestAppContext) {
        use hardcover_core::store::LibraryCache;
        use std::sync::Arc;
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let launch = |client, cx: &mut TestAppContext| {
            let session = SessionScope::for_tests(client, Some(cache.clone()));
            cx.update(|cx| session.install(cx));
            let library = cx.new(|_| LibraryModel::new(session.clone()));
            let upcoming = cx.new(|cx| UpcomingModel::new(session.clone(), library.clone(), cx));
            let model = cx.new(|cx| SeriesModel::new(session, library.clone(), upcoming, cx));
            library.update(cx, LibraryModel::start);
            for _ in 0..3 {
                cx.executor().advance_clock(RELOAD_DELAY * 2);
                cx.run_until_parked();
            }
            model
        };
        let names = |m: &SeriesModel| -> Vec<String> {
            let mut names: Vec<String> = m
                .series
                .values()
                .filter_map(|(_, s)| s.as_ref().as_ref().map(|s| s.name.clone()))
                .collect();
            names.sort();
            names
        };
        let online = launch(hardcover_core::demo::client(), cx);
        let shown = online.read_with(cx, |m, _| names(m));
        assert!(!shown.is_empty());

        let faults = hardcover_core::demo::Faults::parse("offline").unwrap();
        let offline = launch(hardcover_core::demo::client_with(&faults), cx);
        offline.read_with(cx, |m, _| {
            assert_eq!(names(m), shown);
            assert!(m.is_loaded());
            assert!(matches!(m.status(), LoadStatus::Failed(_)));
        });
    }

    /// Only the series added to the started ones are asked for; those no
    /// longer started are dropped.
    #[gpui_kit::test]
    fn a_change_asks_only_for_the_series_added(cx: &mut TestAppContext) {
        let session = SessionScope::for_tests(hardcover_core::demo::client(), None);
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let upcoming = cx.new(|cx| UpcomingModel::new(session.clone(), library.clone(), cx));
        let model = cx.new(|cx| SeriesModel::new(session, library, upcoming, cx));
        let follow = |started: &[i64], cx: &mut TestAppContext| {
            model.update(cx, |m, cx| {
                m.started = LibraryFollow::seen_as(ids(started));
                m.follow_started(cx);
            });
            cx.run_until_parked();
        };
        follow(&[1, 2], cx);
        assert_eq!(model.read_with(cx, |m, _| m.series.started()), 2);
        follow(&[1, 2, 3], cx);
        model.read_with(cx, |m, _| {
            assert_eq!(m.series.started(), 3, "only the new one");
            assert!(m.is_loaded());
        });
        follow(&[2, 3], cx);
        model.read_with(cx, |m, _| {
            assert_eq!(m.series.started(), 3, "nothing asked for");
            assert!(m.series.value(&SeriesId::new(1)).is_none(), "dropped");
        });
    }
}
