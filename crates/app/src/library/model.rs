//! The signed-in user's library as an entity: entries, per-status counts, sync
//! state and the writes in flight.
//!
//! Every view of the library reads this model. Writes return the whole entry,
//! which replaces the local one (and is upserted into the cache); a full sync
//! that started before a write re-applies that write when it lands.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, SharedString, Task};
use hardcover_core::auth::scopes::READ_LIBRARY;
use hardcover_core::store::LibraryCache;
use hardcover_core::{
    BookId, Error, LibraryEntry, ReadingStatus, Review, Timestamp, User, UserBookId,
};

use super::collection::{LocalChange, merge_sync, status_counts};
use crate::app_state::{CoverService, Services};

/// What a write in flight changes, so only that control shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    Status,
    Rating,
    Progress,
    Dates,
    Edition,
    Privacy,
    Notes,
    Review,
    /// Adding, changing or deleting a read in the reading history.
    Reads,
    Remove,
}

/// An entry's review as far as this run knows it. Reviews are fetched when a
/// book page shows the entry, not by the sync, and kept for the run.
#[derive(Debug, Clone, Default)]
pub struct ReviewSlot {
    /// The last review known, kept while it is fetched again.
    pub review: Option<Review>,
    pub fetching: bool,
    /// Why the last fetch failed.
    pub error: Option<SharedString>,
    /// False until fetched or written, and again after each sync, which may
    /// bring changes made elsewhere.
    current: bool,
}

/// Where the last sync stands.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SyncStatus {
    /// Nothing attempted yet.
    #[default]
    Idle,
    Syncing,
    Synced,
    /// The last sync failed; the entries shown are the cached ones.
    Failed {
        message: SharedString,
        offline: bool,
    },
}

pub struct LibraryModel {
    user: Option<User>,
    entries: Vec<LibraryEntry>,
    index: HashMap<UserBookId, usize>,
    by_book: HashMap<BookId, usize>,
    counts: HashMap<ReadingStatus, usize>,
    /// True once the cache or a sync has delivered entries (possibly none).
    loaded: bool,
    /// Bumped whenever the entries change.
    revision: u64,
    sync: SyncStatus,
    last_synced_at: Option<Timestamp>,
    /// Entries received so far by the running sync.
    sync_fetched: usize,
    /// The running sync shows pages as they arrive (nothing was cached).
    fill_during_sync: bool,
    /// `Some` while a sync runs: writes to re-apply on top of its result.
    changes_during_sync: Option<Vec<LocalChange>>,
    /// Entries whose review was written while the running sync ran: the sync
    /// does not make those reviews stale.
    reviews_during_sync: HashSet<UserBookId>,
    writes: HashMap<UserBookId, WriteKind>,
    adding: HashSet<BookId>,
    reviews: HashMap<UserBookId, ReviewSlot>,
    _sync_task: Option<Task<()>>,
}

impl LibraryModel {
    pub fn new() -> Self {
        Self {
            user: None,
            entries: Vec::new(),
            index: HashMap::new(),
            by_book: HashMap::new(),
            counts: HashMap::new(),
            loaded: false,
            revision: 0,
            sync: SyncStatus::Idle,
            last_synced_at: None,
            sync_fetched: 0,
            fill_during_sync: false,
            changes_during_sync: None,
            reviews_during_sync: HashSet::new(),
            writes: HashMap::new(),
            adding: HashSet::new(),
            reviews: HashMap::new(),
            _sync_task: None,
        }
    }

    // ----- reads -----

    pub fn user(&self) -> Option<&User> {
        self.user.as_ref()
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    pub fn entry(&self, id: UserBookId) -> Option<&LibraryEntry> {
        self.index.get(&id).map(|&ix| &self.entries[ix])
    }

    pub fn entry_for_book(&self, book: BookId) -> Option<&LibraryEntry> {
        self.by_book.get(&book).map(|&ix| &self.entries[ix])
    }

    pub fn count(&self, status: ReadingStatus) -> usize {
        self.counts.get(&status).copied().unwrap_or(0)
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn sync_status(&self) -> &SyncStatus {
        &self.sync
    }

    pub fn sync_fetched(&self) -> usize {
        self.sync_fetched
    }

    pub fn last_synced_at(&self) -> Option<Timestamp> {
        self.last_synced_at
    }

    pub fn pending_write(&self, id: UserBookId) -> Option<WriteKind> {
        self.writes.get(&id).copied()
    }

    pub fn is_adding(&self, book: BookId) -> bool {
        self.adding.contains(&book)
    }

    pub fn review(&self, id: UserBookId) -> Option<&ReviewSlot> {
        self.reviews.get(&id)
    }

    /// True if the entry's review should be fetched: never fetched, or not
    /// since the last sync. A failed fetch waits for [`Self::fetch_review`].
    pub fn wants_review(&self, id: UserBookId) -> bool {
        self.reviews
            .get(&id)
            .is_none_or(|s| !s.current && !s.fetching && s.error.is_none())
    }

    // ----- loading and syncing -----

    /// Shows the cached library, then syncs.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        let cache = Services::global(cx).cache().cloned();
        let task = cx.spawn(async move |this, cx| {
            let cached = match cache {
                Some(cache) => {
                    cx.background_spawn(async move { load_cached(&cache) })
                        .await
                }
                None => None,
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(cached) = cached
                    && !this.loaded
                {
                    this.user = Some(cached.user);
                    this.last_synced_at = cached.last_synced_at;
                    this.set_entries(cached.entries);
                    this.loaded = true;
                    cx.notify();
                }
                this.sync(cx);
            });
        });
        self._sync_task = Some(task);
    }

    /// Fetches the whole library and replaces the local copy. No-op while a
    /// sync is already running.
    pub fn sync(&mut self, cx: &mut Context<Self>) {
        if self.sync == SyncStatus::Syncing {
            return;
        }
        self.sync = SyncStatus::Syncing;
        self.sync_fetched = 0;
        self.fill_during_sync = self.entries.is_empty();
        self.changes_during_sync = Some(Vec::new());
        self.reviews_during_sync.clear();
        cx.notify();

        let services = Services::global(cx).clone();
        self._sync_task = Some(cx.spawn(async move |this, cx| {
            let client = services.client().clone();
            let (pages_tx, pages_rx) = async_channel::unbounded::<Vec<LibraryEntry>>();
            let fetch = cx.background_spawn(async move {
                let user = client.me()?;
                let entries = client.library_with_progress(user.id, |page| {
                    let _ = pages_tx.send_blocking(page.to_vec());
                })?;
                Ok::<_, Error>((user, entries))
            });
            // The sender is dropped when the fetch ends, which ends this loop.
            while let Ok(page) = pages_rx.recv().await {
                if this
                    .update(cx, |this, cx| this.receive_page(page, cx))
                    .is_err()
                {
                    return;
                }
            }
            let fetched = fetch.await;
            let saved = this.update(cx, |this, cx| this.finish_sync(fetched, cx));
            if let Ok(Some((user, entries))) = saved
                && let Some(cache) = services.cache().cloned()
            {
                cx.background_spawn(async move {
                    if let Err(e) = cache
                        .save_user(&user)
                        .and_then(|()| cache.replace_library(user.id, &entries))
                    {
                        eprintln!("hardcover: could not update the library cache: {e}");
                    }
                })
                .await;
            }
        }));
    }

    /// A page of a running sync. A first sync (nothing cached) shows pages as
    /// they arrive; otherwise the cached entries stay until the sync completes.
    fn receive_page(&mut self, page: Vec<LibraryEntry>, cx: &mut Context<Self>) {
        self.sync_fetched += page.len();
        if self.fill_during_sync {
            let mut entries = std::mem::take(&mut self.entries);
            entries.extend(page);
            self.set_entries(entries);
            self.loaded = true;
        }
        cx.notify();
    }

    /// Applies a sync result. Returns what to write to the cache.
    fn finish_sync(
        &mut self,
        fetched: Result<(User, Vec<LibraryEntry>), Error>,
        cx: &mut Context<Self>,
    ) -> Option<(User, Vec<LibraryEntry>)> {
        let changes = self.changes_during_sync.take().unwrap_or_default();
        match fetched {
            Ok((user, synced)) => {
                let merged = merge_sync(synced, &changes);
                self.user = Some(user.clone());
                self.set_entries(merged.clone());
                self.loaded = true;
                self.sync = SyncStatus::Synced;
                self.last_synced_at = Some(chrono::Utc::now());
                // Fetched again when next shown, also after a failure.
                let written = std::mem::take(&mut self.reviews_during_sync);
                for (id, slot) in &mut self.reviews {
                    if !written.contains(id) {
                        slot.current = false;
                        slot.error = None;
                    }
                }
                CoverService::retry_failed(cx);
                cx.notify();
                Some((user, merged))
            }
            Err(error) => {
                let offline = matches!(error, Error::Network(_) | Error::Timeout);
                self.sync = SyncStatus::Failed {
                    message: Services::error_message(&error, Some(READ_LIBRARY), cx).into(),
                    offline,
                };
                // Without a cache there is nothing to show but the failure.
                self.loaded = true;
                cx.notify();
                None
            }
        }
    }

    // ----- writes -----

    /// Marks a write on `id` as in flight. `false` if one already is: writes on
    /// one entry run one at a time, since each returns the whole entry.
    pub fn begin_write(&mut self, id: UserBookId, kind: WriteKind, cx: &mut Context<Self>) -> bool {
        if self.writes.contains_key(&id) {
            return false;
        }
        self.writes.insert(id, kind);
        cx.notify();
        true
    }

    pub fn end_write(&mut self, id: UserBookId, cx: &mut Context<Self>) {
        if self.writes.remove(&id).is_some() {
            cx.notify();
        }
    }

    pub fn begin_add(&mut self, book: BookId, cx: &mut Context<Self>) -> bool {
        let started = self.adding.insert(book);
        cx.notify();
        started
    }

    pub fn end_add(&mut self, book: BookId, cx: &mut Context<Self>) {
        self.adding.remove(&book);
        cx.notify();
    }

    /// Replaces (or adds) an entry with the one a write returned, and saves it
    /// to the cache.
    pub fn apply_written(&mut self, entry: LibraryEntry, cx: &mut Context<Self>) {
        if let Some(changes) = &mut self.changes_during_sync {
            changes.push(LocalChange::Upserted(Box::new(entry.clone())));
        }
        let mut entries = std::mem::take(&mut self.entries);
        match self.index.get(&entry.user_book_id) {
            Some(&ix) => entries[ix] = entry.clone(),
            None => entries.push(entry.clone()),
        }
        self.set_entries(entries);
        cx.notify();
        if let Some(cache) = Services::global(cx).cache().cloned() {
            persist(cx, move || cache.upsert(&entry));
        }
    }

    /// Fetches the entry's review in the background. The review shown, if
    /// any, stays until the new one arrives.
    pub fn fetch_review(&mut self, id: UserBookId, cx: &mut Context<Self>) {
        let slot = self.reviews.entry(id).or_default();
        if slot.fetching {
            return;
        }
        slot.fetching = true;
        slot.error = None;
        cx.notify();
        let client = Services::global(cx).client().clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { client.review(id) }).await;
            let _ = this.update(cx, |this, cx| {
                let error = match &result {
                    Err(e) => Some(Services::error_message(e, Some(READ_LIBRARY), cx).into()),
                    Ok(_) => None,
                };
                let Some(slot) = this.reviews.get_mut(&id) else {
                    // The entry was removed meanwhile.
                    return;
                };
                slot.fetching = false;
                // A review write that landed meanwhile is newer.
                if !slot.current {
                    match result {
                        Ok(review) => {
                            slot.review = Some(review);
                            slot.current = true;
                        }
                        Err(_) => slot.error = error,
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The review a review write returned.
    pub fn apply_review(&mut self, id: UserBookId, review: Review, cx: &mut Context<Self>) {
        if self.changes_during_sync.is_some() {
            self.reviews_during_sync.insert(id);
        }
        let slot = self.reviews.entry(id).or_default();
        slot.review = Some(review);
        slot.current = true;
        slot.error = None;
        cx.notify();
    }

    pub fn apply_removed(&mut self, id: UserBookId, cx: &mut Context<Self>) {
        self.reviews.remove(&id);
        if let Some(changes) = &mut self.changes_during_sync {
            changes.push(LocalChange::Removed(id));
        }
        let mut entries = std::mem::take(&mut self.entries);
        entries.retain(|e| e.user_book_id != id);
        self.set_entries(entries);
        cx.notify();
        if let Some(cache) = Services::global(cx).cache().cloned() {
            persist(cx, move || cache.remove(id));
        }
    }

    fn set_entries(&mut self, entries: Vec<LibraryEntry>) {
        self.index = entries
            .iter()
            .enumerate()
            .map(|(ix, e)| (e.user_book_id, ix))
            .collect();
        self.by_book = entries
            .iter()
            .enumerate()
            .map(|(ix, e)| (e.book.id, ix))
            .collect();
        self.counts = status_counts(&entries);
        self.entries = entries;
        self.revision += 1;
    }
}

struct Cached {
    user: User,
    entries: Vec<LibraryEntry>,
    last_synced_at: Option<Timestamp>,
}

/// Blocking.
fn load_cached(cache: &Arc<LibraryCache>) -> Option<Cached> {
    let user = cache.load_user().ok().flatten()?;
    let entries = cache.load_library(user.id).ok()?;
    let last_synced_at = cache.last_synced_at(user.id).ok().flatten();
    Some(Cached {
        user,
        entries,
        last_synced_at,
    })
}

/// Runs a cache write in the background; failures are logged, not shown.
fn persist(
    cx: &mut Context<LibraryModel>,
    write: impl FnOnce() -> hardcover_core::Result<()> + Send + 'static,
) {
    cx.background_spawn(async move {
        if let Err(e) = write() {
            eprintln!("hardcover: could not update the library cache: {e}");
        }
    })
    .detach();
}
