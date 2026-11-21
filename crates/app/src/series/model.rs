//! The series the reader has started, as an entity: every book of each,
//! asked for a batch at a time (`Client::tracked_series`). Loaded right
//! after the library cache, on Refresh, after signing in again, and shortly
//! after a sync or a write changes which series are started. Kept for the
//! session, not cached on disk.
//!
//! It also keeps the summaries the destination and its sidebar row show
//! (`progress::summaries`), worked out once per change of the series, the
//! library, the upcoming books or the day.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Entity, SharedString, Subscription, Task};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookId, Date, SeriesId, TrackedSeries, TrackedSeriesLoad};

use super::progress::{SeriesSummary, started_series, summaries};
use crate::app_state::Services;
use crate::library::model::{LibraryModel, SyncStatus};
use crate::upcoming::model::UpcomingModel;

/// How long the started series must stay changed before reloading, so a run
/// of writes costs one load.
const RELOAD_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SeriesStatus {
    /// Not loaded yet, or the session can't read the catalogue.
    #[default]
    Idle,
    Loading,
    Loaded,
    /// The last load failed; the series shown (if any) are the previous ones.
    Failed(SharedString),
}

/// What the summaries were worked out from.
type SummaryKey = (u64, u64, u64, Date);

pub struct SeriesModel {
    library: Entity<LibraryModel>,
    upcoming: Entity<UpcomingModel>,
    series: Vec<TrackedSeries>,
    status: SeriesStatus,
    /// The series of the last load, finished or not.
    requested: Option<Vec<SeriesId>>,
    /// Bumped whenever the series change.
    revision: u64,
    summaries: Option<(SummaryKey, Rc<Vec<SeriesSummary>>)>,
    load_task: Option<Task<()>>,
    reload_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl SeriesModel {
    pub fn new(
        library: Entity<LibraryModel>,
        upcoming: Entity<UpcomingModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&library, |this, library, cx| {
                let library = library.read(cx);
                if !library.is_loaded() {
                    return;
                }
                // The first load goes out with the cached library, without
                // waiting for the sync that starts with it.
                if this.requested.is_none() {
                    this.load(cx);
                    return;
                }
                // Pages of a running sync are not news; its result is.
                if *library.sync_status() == SyncStatus::Syncing {
                    return;
                }
                if this.requested.as_deref() != Some(&started_series(library.entries())) {
                    this.schedule_reload(cx);
                }
            }),
            cx.observe(&upcoming, |_, _, cx| cx.notify()),
        ];
        Self {
            library,
            upcoming,
            series: Vec::new(),
            status: SeriesStatus::Idle,
            requested: None,
            revision: 0,
            summaries: None,
            load_task: None,
            reload_task: None,
            _subscriptions: subscriptions,
        }
    }

    // ----- reads -----

    pub fn status(&self) -> &SeriesStatus {
        &self.status
    }

    /// Whether a load has finished, so an empty result means no series.
    pub fn is_loaded(&self) -> bool {
        self.revision > 0
    }

    /// Every started series, in group order, each group sorted.
    pub fn summaries(&mut self, today: Date, cx: &App) -> Rc<Vec<SeriesSummary>> {
        let library = self.library.read(cx);
        let upcoming = self.upcoming.read(cx);
        let key = (
            self.revision,
            library.revision(),
            upcoming.revision(),
            today,
        );
        if let Some((k, summaries)) = &self.summaries
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
        let computed = Rc::new(summaries(&self.series, library.entries(), &dates, today));
        self.summaries = Some((key, computed.clone()));
        computed
    }

    // ----- loading -----

    /// Asks for the started series, keeping the ones shown until the answer
    /// arrives. Waits for the library to load.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let library = self.library.read(cx);
        if !library.is_loaded() || !Services::global(cx).has_scope(READ_CATALOG) {
            return;
        }
        let ids = started_series(library.entries());
        self.requested = Some(ids.clone());
        self.reload_task = None;
        self.status = SeriesStatus::Loading;
        cx.notify();
        let client = Services::global(cx).client().clone();
        // Replacing the task drops a load still running: its series are older.
        // A load that lands after signing out finds the model gone.
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let load = cx
                .background_spawn(async move { client.tracked_series(&ids) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let failed = load.error.as_ref().map(|e| {
                    SharedString::from(Services::error_message(e, Some(READ_CATALOG), cx))
                });
                if let Some(series) = merge_load(&this.series, load) {
                    this.series = series;
                    this.revision += 1;
                }
                this.status = failed.map_or(SeriesStatus::Loaded, SeriesStatus::Failed);
                cx.notify();
            });
        }));
    }

    fn schedule_reload(&mut self, cx: &mut Context<Self>) {
        self.reload_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_DELAY).await;
            let _ = this.update(cx, SeriesModel::load);
        }));
    }
}

/// The series to show after a load: what loaded, plus the previous copy of
/// each series in a batch that failed, so a failure doesn't make a series
/// vanish. `None` when nothing loaded and a batch failed: keep what's shown.
fn merge_load(previous: &[TrackedSeries], load: TrackedSeriesLoad) -> Option<Vec<TrackedSeries>> {
    if load.error.is_some() && load.series.is_empty() {
        return None;
    }
    let mut series = load.series;
    series.extend(
        previous
            .iter()
            .filter(|s| load.missing.contains(&s.id))
            .cloned(),
    );
    Some(series)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn ids(series: Option<Vec<TrackedSeries>>) -> Option<Vec<i64>> {
        series.map(|s| s.iter().map(|s| s.id.get()).collect())
    }

    /// One failed batch doesn't make its series vanish from the tracker.
    #[test]
    fn a_failed_batch_keeps_its_series_previous_copies() {
        let previous = [tracked(1), tracked(2), tracked(3)];
        assert_eq!(
            ids(merge_load(&previous, load(&[1, 4], &[2, 5], true))),
            Some(vec![1, 4, 2])
        );
        // A clean load replaces everything: 2 and 3 are no longer started.
        assert_eq!(
            ids(merge_load(&previous, load(&[1], &[], false))),
            Some(vec![1])
        );
        // Nothing loaded: keep what's shown.
        assert_eq!(
            ids(merge_load(&previous, load(&[], &[1, 2, 3], true))),
            None
        );
    }
}
