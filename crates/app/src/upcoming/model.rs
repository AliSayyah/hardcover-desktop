//! Books still to come in the reader's series, as an entity: one request for
//! every series in the library. Loaded right after the library cache, on
//! Refresh, and shortly after a sync or a write changes which series the
//! library has. Not cached on disk.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::{AppContext as _, Context, Entity, SharedString, Subscription, Task};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookId, Date, SeriesId, UpcomingBook};

use super::schedule::{unreleased, watched_series};
use crate::app_state::Services;
use crate::library::model::{LibraryModel, SyncStatus};

/// How long the series must stay changed before reloading, so a run of
/// writes costs one request.
const RELOAD_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Default)]
pub enum UpcomingStatus {
    /// Not loaded yet, or the session can't read the catalogue.
    #[default]
    Idle,
    Loading,
    Loaded,
    /// The last load failed; the books shown (if any) are the previous ones.
    Failed(SharedString),
}

pub struct UpcomingModel {
    library: Entity<LibraryModel>,
    books: Vec<UpcomingBook>,
    status: UpcomingStatus,
    /// The series of the last load, finished or not.
    requested: Option<Vec<SeriesId>>,
    /// Bumped whenever the books change.
    revision: u64,
    _load: Option<Task<()>>,
    _reload: Option<Task<()>>,
    _subscription: Subscription,
}

impl UpcomingModel {
    pub fn new(library: Entity<LibraryModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&library, |this, library, cx| {
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
            if this.requested.as_deref() != Some(&watched_series(library.entries())) {
                this.schedule_reload(cx);
            }
        });
        Self {
            library,
            books: Vec::new(),
            status: UpcomingStatus::Idle,
            requested: None,
            revision: 0,
            _load: None,
            _reload: None,
            _subscription: subscription,
        }
    }

    // ----- reads -----

    pub fn books(&self) -> &[UpcomingBook] {
        &self.books
    }

    pub fn status(&self) -> &UpcomingStatus {
        &self.status
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The books not out yet on `today`, for Home's "Up next".
    pub fn unreleased(&self, today: Date) -> HashSet<BookId> {
        unreleased(&self.books, today)
    }

    // ----- loading -----

    /// Asks for the books still to come in the library's series, keeping the
    /// ones shown until the answer arrives. Waits for the library to load.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let library = self.library.read(cx);
        if !library.is_loaded() || !Services::global(cx).has_scope(READ_CATALOG) {
            return;
        }
        let series = watched_series(library.entries());
        self.requested = Some(series.clone());
        self._reload = None;
        self.status = UpcomingStatus::Loading;
        cx.notify();
        let client = Services::global(cx).client().clone();
        let today = chrono::Local::now().date_naive();
        // Replacing the task drops a load still running: its series are older.
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.upcoming_in_series(&series, today) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(books) => {
                        this.books = books;
                        this.revision += 1;
                        this.status = UpcomingStatus::Loaded;
                    }
                    Err(e) => {
                        let message = Services::error_message(&e, Some(READ_CATALOG), cx);
                        this.status = UpcomingStatus::Failed(message.into());
                    }
                }
                cx.notify();
            });
        }));
    }

    fn schedule_reload(&mut self, cx: &mut Context<Self>) {
        self._reload = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_DELAY).await;
            let _ = this.update(cx, |this, cx| this.load(cx));
        }));
    }
}
