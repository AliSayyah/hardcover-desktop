//! The series the reader follows, as an entity: every book of each, asked
//! for a batch at a time (`Client::tracked_series`). Loaded once the
//! follows are known (the last launch's, then Hardcover's), on Refresh and
//! after signing in again; when the follows change, only the series added
//! are asked for, and those no longer followed are dropped. Until the
//! follows are known nothing is dropped: not loaded isn't following
//! nothing.
//! Each series is kept, and goes stale, on its own (a [`Resource`] keyed by
//! series), and the last good copies are kept in the library cache
//! (`resource::saved`), so they show at the next launch, offline too.
//!
//! It also keeps the summaries the destination and its sidebar row show
//! (`progress::summaries`), worked out once per change of the series, the
//! library, the upcoming books or the day.

use gpui_kit::{App, Context, Entity, Subscription, Task};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookId, Client, Date, SeriesId, TrackedSeries, TrackedSeriesLoad};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::follows::FollowsModel;
use super::progress::{SeriesSummary, summaries};
use crate::io::Priority;
use crate::library::model::{AccountChanged, LibraryModel};
use crate::resource::{Batch, Binding, Demand, Landing, LoadStatus, Resource, saved};
use crate::session::SessionScope;
use crate::upcoming::model::UpcomingModel;

/// The series' rows in the library cache.
const SAVED_AS: &str = "series";

/// What the summaries were worked out from.
type SummaryKey = (u64, u64, u64, u64, Date);

pub struct SeriesModel {
    library: Entity<LibraryModel>,
    upcoming: Entity<UpcomingModel>,
    follows: Entity<FollowsModel>,
    /// Each followed series; `None` when Hardcover returned nothing for it
    /// (a merged or emptied series).
    series: Resource<SeriesId, Option<TrackedSeries>>,
    /// The followed series, as last seen; `None` until the follows are
    /// known.
    followed: Option<Vec<SeriesId>>,
    /// Bumped whenever a series changes.
    revision: u64,
    /// The last summaries worked out, and from what. A cache, filled on
    /// read, so the views can ask from `render` without updating the model.
    summaries: RefCell<Option<(SummaryKey, Rc<Vec<SeriesSummary>>)>>,
    _hydrating: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SeriesModel {
    pub fn new(
        session: SessionScope,
        library: Entity<LibraryModel>,
        upcoming: Entity<UpcomingModel>,
        follows: Entity<FollowsModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&follows, |this, follows, cx| {
                let Some(ids) = follows.read(cx).ids() else {
                    return;
                };
                if this.followed.as_ref() == Some(&ids) {
                    return;
                }
                let first = this.followed.is_none();
                this.followed = Some(ids);
                if first {
                    this.load(cx);
                } else {
                    this.follow_changed(cx);
                }
            }),
            cx.observe(&library, |_, _, cx| cx.notify()),
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
            follows,
            series,
            followed: None,
            revision: 0,
            summaries: RefCell::new(None),
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

    fn followed(&self) -> &[SeriesId] {
        self.followed.as_deref().unwrap_or_default()
    }

    /// One status for the follows and every followed series: loading while
    /// any is, else the first failure.
    pub fn status(&self, cx: &App) -> LoadStatus {
        match self.follows.read(cx).status() {
            LoadStatus::Loaded | LoadStatus::Idle => self.series.summary(self.followed()),
            status => status,
        }
    }

    /// Whether the follows are known and every followed series has loaded
    /// or failed, with something to show, so an empty result means no
    /// series.
    pub fn is_loaded(&self) -> bool {
        self.followed.as_ref().is_some_and(|ids| {
            ids.iter().all(|id| self.series.has_settled(id))
                && (ids.is_empty() || ids.iter().any(|id| self.series.value(id).is_some()))
        })
    }

    /// Every followed series, in group order, each group sorted. Worked out
    /// again only when the series, the follows, the library, the upcoming
    /// books or the day changed.
    pub fn summaries(&self, today: Date, cx: &App) -> Rc<Vec<SeriesSummary>> {
        let library = self.library.read(cx);
        let upcoming = self.upcoming.read(cx);
        let follows = self.follows.read(cx);
        let key = (
            self.revision,
            follows.revision(),
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
            .followed()
            .iter()
            .filter_map(|id| self.series.value(id)?.as_ref().clone())
            .collect();
        let computed = Rc::new(summaries(
            &series,
            library.entries(),
            &dates,
            &follows.followed_on(),
            today,
        ));
        *self.summaries.borrow_mut() = Some((key, computed.clone()));
        computed
    }

    // ----- loading -----

    /// Asks for the follows, and the followed series that are stale or
    /// failed (Refresh, a new grant, "Try again"), keeping the ones shown
    /// until the answers arrive.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.follows.update(cx, FollowsModel::load);
        let followed = self.followed().to_vec();
        self.request(followed, Demand::Shown, cx);
    }

    /// The followed series changed: forgets those no longer followed and
    /// asks for the new ones.
    fn follow_changed(&mut self, cx: &mut Context<Self>) {
        let followed: HashSet<SeriesId> = self.followed().iter().copied().collect();
        let before = self.series.values().count();
        self.series.retain(|id| followed.contains(id));
        if self.series.values().count() != before {
            self.revision += 1;
        }
        cx.notify();
        let followed = self.followed().to_vec();
        self.request(followed, Demand::Active, cx);
    }

    /// The page is shown (`shown_again`) or the window came back: asks for
    /// the follows and the followed series that are stale (`resource`).
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        let demand = if shown_again {
            Demand::Shown
        } else {
            Demand::Active
        };
        self.follows.update(cx, |f, cx| f.request(demand, cx));
        let followed = self.followed().to_vec();
        self.request(followed, demand, cx);
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

    /// Starts a library, the follows and the series on `client`, and lets
    /// them load.
    fn launch(
        client: hardcover_core::Client,
        cache: Option<std::sync::Arc<hardcover_core::store::LibraryCache>>,
        cx: &mut TestAppContext,
    ) -> (
        Entity<SeriesModel>,
        Entity<FollowsModel>,
        Entity<LibraryModel>,
    ) {
        let session = SessionScope::for_tests(client, cache);
        cx.update(|cx| session.install(cx));
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let follows = cx.update(|cx| FollowsModel::init(session.clone(), &library, cx));
        let upcoming =
            cx.new(|cx| UpcomingModel::new(session.clone(), library.clone(), follows.clone(), cx));
        let model =
            cx.new(|cx| SeriesModel::new(session, library.clone(), upcoming, follows.clone(), cx));
        library.update(cx, LibraryModel::start);
        follows.update(cx, FollowsModel::load);
        for _ in 0..3 {
            cx.executor()
                .advance_clock(std::time::Duration::from_secs(4));
            cx.run_until_parked();
        }
        (model, follows, library)
    }

    fn names(m: &SeriesModel) -> Vec<String> {
        let mut names: Vec<String> = m
            .series
            .values()
            .filter_map(|(_, s)| s.as_ref().as_ref().map(|s| s.name.clone()))
            .collect();
        names.sort();
        names
    }

    /// B7: offline, the follows and the series of the last launch show,
    /// with the failure; not loaded is never "follows nothing".
    #[gpui_kit::test]
    fn the_last_series_show_offline(cx: &mut TestAppContext) {
        use hardcover_core::store::LibraryCache;
        use std::sync::Arc;
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let (online, follows, _) = launch(hardcover_core::demo::client(), Some(cache.clone()), cx);
        let shown = online.read_with(cx, |m, _| names(m));
        assert_eq!(shown.len(), 7, "the demo follows seven series");
        let followed = follows.read_with(cx, |f, _| f.ids());

        let faults = hardcover_core::demo::Faults::parse("offline").unwrap();
        let (offline, follows, _) =
            launch(hardcover_core::demo::client_with(&faults), Some(cache), cx);
        // From the cache: known, and every view keyed on the revision moves.
        assert_eq!(follows.read_with(cx, |f, _| f.ids()), followed);
        assert!(follows.read_with(cx, |f, _| f.revision()) > 0);
        offline.read_with(cx, |m, cx| {
            assert_eq!(names(m), shown);
            assert!(!m.summaries(crate::dates::today(), cx).is_empty());
            assert!(m.is_loaded());
            assert!(matches!(m.status(cx), LoadStatus::Failed(_)));
        });
    }

    /// Following a series asks only for it; unfollowing one drops it and
    /// asks for nothing.
    #[gpui_kit::test]
    fn a_change_asks_only_for_the_series_added(cx: &mut TestAppContext) {
        let (model, follows, library) = launch(hardcover_core::demo::client(), None, cx);
        let asked = model.read_with(cx, |m, _| m.series.started());
        let id = cx.read(|cx| {
            let followed = follows.read(cx).id_set();
            super::super::progress::unfollowed(library.read(cx).entries(), &followed)[0].id
        });
        let task = follows.update(cx, |f, cx| f.follow(id, cx));
        cx.run_until_parked();
        assert!(matches!(gpui_kit::block_on(task), Some(Ok(()))));
        model.read_with(cx, |m, _| {
            assert_eq!(m.series.started(), asked + 1, "only the new one");
            assert!(m.series.value(&id).is_some());
            assert!(m.is_loaded());
        });
        let task = follows.update(cx, |f, cx| f.unfollow(id, cx));
        cx.run_until_parked();
        assert!(matches!(gpui_kit::block_on(task), Some(Ok(()))));
        model.read_with(cx, |m, _| {
            assert_eq!(m.series.started(), asked + 1, "nothing asked for");
            assert!(m.series.value(&id).is_none(), "dropped");
        });
    }
}
