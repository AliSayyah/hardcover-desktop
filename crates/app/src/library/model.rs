//! The signed-in user's library as an entity: entries, per-status counts, sync
//! state and the writes in flight.
//!
//! Every view of the library reads this model, and every library write goes
//! through [`LibraryModel::write`]. Writes return the whole entry, which
//! replaces the local one (and is upserted into the cache); a sync re-applies
//! the writes it may not have seen (`collection::merge_sync`).
//!
//! The model belongs to one session ([`SessionScope`]): it makes its calls
//! through it and owns the library cache's worker, so the cache is read and
//! written in order, and [`LibraryModel::end_session`] stops everything
//! before the cache is cleared.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Task, Window};
use hardcover_core::auth::scopes::{READ_LIBRARY, WRITE_LIBRARY};
use hardcover_core::store::LibraryCache;
use hardcover_core::{
    BookId, Client, Coverage, EditionChange, Error, LibraryEntry, LibrarySync, ProgressChange,
    ReadingStatus, Review, ReviewedEntry, Timestamp, User, UserBookId, UserId,
};

use super::cache_worker::CacheWorker;
use super::collection::{LocalChange, RecentWrites, merge_sync, same_entries, status_counts};
use crate::app_state::CoverService;
use crate::io::Priority;
use crate::resource::{Binding, Demand, Resource};
use crate::session::SessionScope;

/// How old the last sync (or try) must be before coming back to the window
/// syncs again. A sync is about ten requests, most of the rate limit's burst.
const SYNC_AGAIN_AFTER: Duration = Duration::from_mins(15);

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
    /// Adding a book to the library.
    Add,
}

/// What a library write is about: one write at a time each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteTarget {
    Entry(UserBookId),
    /// A book being added, which has no entry yet.
    Add(BookId),
}

/// What [`LibraryModel::write`] shows when a second write on the same entry
/// is asked for while one runs.
pub const STILL_SAVING: &str =
    "Another change to this book is still being saved. Try again in a moment.";

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

/// The sync found the cache was another account's: it is dropped, and
/// whatever else was shown from it must go too.
pub struct AccountChanged;

impl EventEmitter<AccountChanged> for LibraryModel {}

pub struct LibraryModel {
    session: SessionScope,
    /// The session's cache worker; `None` without a cache, and once the
    /// session ended.
    cache: Option<Rc<CacheWorker>>,
    /// The cached user until the session's own `me()` answers.
    user: Option<User>,
    /// True once `me()` answered for this session: `user` is the account
    /// signed in, not only the one the cache was written for.
    user_verified: bool,
    entries: Vec<LibraryEntry>,
    index: HashMap<UserBookId, usize>,
    by_book: HashMap<BookId, usize>,
    counts: HashMap<ReadingStatus, usize>,
    /// True once the cache or a sync has delivered entries (possibly none).
    /// A first sync that fails leaves it false: never loaded, which the
    /// views show as the failure ([`Self::load_failure`]), not as an empty
    /// library.
    loaded: bool,
    /// Bumped whenever the entries change.
    revision: u64,
    sync: SyncStatus,
    last_synced_at: Option<Timestamp>,
    /// When the last sync started, whether or not it succeeded. Wall-clock
    /// time: `Instant` stops while the computer sleeps.
    sync_started: Option<SystemTime>,
    /// Entries received so far by the running sync.
    sync_fetched: usize,
    /// The running sync shows pages as they arrive (nothing was cached).
    fill_during_sync: bool,
    /// The writes of the last moments, re-applied on top of a sync.
    recent: RecentWrites,
    /// The writes in flight. Each holds its task: removing it (the write
    /// finished, or the session ended) is what ends the write.
    writes: HashMap<WriteTarget, InFlight>,
    /// Each entry's own review, fetched when a book page shows the entry,
    /// not by the sync (review text can be long), and again after the next
    /// sync. A review write's response replaces it, and a fetch that
    /// started before the write is dropped.
    reviews: Resource<UserBookId, Review>,
    sync_task: Option<Task<()>>,
}

/// A write in flight.
struct InFlight {
    kind: WriteKind,
    task: Task<()>,
}

impl LibraryModel {
    pub fn new(session: SessionScope) -> Self {
        let cache = session.cache_worker();
        Self {
            reviews: Resource::new(session.clone(), READ_LIBRARY, Priority::Interactive),
            session,
            cache,
            user: None,
            user_verified: false,
            entries: Vec::new(),
            index: HashMap::new(),
            by_book: HashMap::new(),
            counts: HashMap::new(),
            loaded: false,
            revision: 0,
            sync: SyncStatus::Idle,
            last_synced_at: None,
            sync_started: None,
            sync_fetched: 0,
            fill_during_sync: false,
            recent: RecentWrites::default(),
            writes: HashMap::new(),
            sync_task: None,
        }
    }

    // ----- reads -----

    /// The account whose library this is: the cached one until the first
    /// sync's `me()` answers. Fine to show; to act on the account, see
    /// [`Self::verified_user_id`].
    pub fn user(&self) -> Option<&User> {
        self.user.as_ref()
    }

    /// The signed-in account's id, once this session's `me()` answered.
    pub fn verified_user_id(&self) -> Option<UserId> {
        self.user
            .as_ref()
            .filter(|_| self.user_verified)
            .map(|u| u.id)
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

    /// Why the library has never loaded: nothing was cached and the first
    /// sync failed. The views show it with "Try again" ([`Self::sync`]) in
    /// place of their empty states.
    pub fn load_failure(&self) -> Option<&SharedString> {
        match &self.sync {
            SyncStatus::Failed { message, .. } if !self.loaded => Some(message),
            _ => None,
        }
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
        self.writes.get(&WriteTarget::Entry(id)).map(|w| w.kind)
    }

    pub fn is_adding(&self, book: BookId) -> bool {
        self.writes.contains_key(&WriteTarget::Add(book))
    }

    /// The entries' own reviews, as far as this run knows them.
    pub fn reviews(&self) -> &Resource<UserBookId, Review> {
        &self.reviews
    }

    /// Fetches entry `id`'s own review if `demand` calls for it: never
    /// fetched, or not since the last sync. The review shown, if any, stays
    /// until the new one arrives.
    pub fn request_review(&mut self, id: UserBookId, demand: Demand, cx: &mut Context<Self>) {
        let sent = self.reviews.request(
            id,
            demand,
            move |client| client.review(id),
            Binding {
                get: |m: &mut Self| &mut m.reviews,
                landed: |_, _, _, cx| cx.notify(),
            },
            cx,
        );
        if sent {
            cx.notify();
        }
    }

    // ----- loading and syncing -----

    /// Shows the cached library, then syncs.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        let Some(cache) = &self.cache else {
            self.sync(cx);
            return;
        };
        let cached = cache.ask(load_cached);
        self.sync_task = Some(cx.spawn(async move |this, cx| {
            let cached = cached.await.flatten();
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
        }));
    }

    /// Fetches the whole library and replaces the local copy. No-op while a
    /// sync is already running, and once the session ended.
    pub fn sync(&mut self, cx: &mut Context<Self>) {
        if self.sync == SyncStatus::Syncing || !self.session.is_alive() {
            return;
        }
        self.sync = SyncStatus::Syncing;
        self.sync_started = Some(SystemTime::now());
        self.sync_fetched = 0;
        self.fill_during_sync = self.entries.is_empty();
        cx.notify();

        let session = self.session.clone();
        let me = session.background(Client::me);
        self.sync_task = Some(cx.spawn(async move |this, cx| {
            let Some(me) = me.await else {
                let _ = this.update(cx, Self::sync_stopped);
                return;
            };
            let user = match me {
                Ok(user) => user,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| this.finish_sync(Err(error), cx));
                    return;
                }
            };
            if this
                .update(cx, |this, cx| this.signed_in_as(user.clone(), cx))
                .is_err()
            {
                return;
            }
            let (pages_tx, pages_rx) = async_channel::unbounded::<Vec<LibraryEntry>>();
            let id = user.id;
            // The sender goes with the call, so this loop ends with it.
            let fetch = session.background(move |client| {
                client.library_with_progress(id, |page| {
                    let _ = pages_tx.send_blocking(page.to_vec());
                })
            });
            while let Ok(page) = pages_rx.recv().await {
                if this
                    .update(cx, |this, cx| this.receive_page(page, cx))
                    .is_err()
                {
                    return;
                }
            }
            let Some(fetched) = fetch.await else {
                let _ = this.update(cx, Self::sync_stopped);
                return;
            };
            let _ = this.update(cx, |this, cx| this.finish_sync(fetched, cx));
        }));
    }

    /// The sync's call resolved to nothing: the session ended, which leaves
    /// nothing to do, or the call panicked, which mustn't leave the sync
    /// running for good.
    fn sync_stopped(&mut self, cx: &mut Context<Self>) {
        if self.session.is_alive() {
            self.sync = SyncStatus::Failed {
                message: "The sync stopped unexpectedly. Try again.".into(),
                offline: false,
            };
            cx.notify();
        }
    }

    /// Syncs if the last sync started more than [`SYNC_AGAIN_AFTER`] ago,
    /// so changes made on the website or another device show without
    /// Refresh. Waits for the first sync, which [`Self::start`] runs.
    pub fn sync_if_stale(&mut self, cx: &mut Context<Self>) {
        // A clock set back since (`elapsed` fails) counts as stale too.
        let stale = self.sync_started.is_some_and(|started| {
            started
                .elapsed()
                .map_or(true, |age| age >= SYNC_AGAIN_AFTER)
        });
        if stale {
            self.sync(cx);
        }
    }

    /// The session's `me()` answered. A library cached for another account
    /// is dropped, from the screen and the cache, and the sync fills it again.
    fn signed_in_as(&mut self, user: User, cx: &mut Context<Self>) {
        if self
            .user
            .as_ref()
            .is_some_and(|cached| cached.id != user.id)
        {
            self.set_entries(Vec::new());
            self.loaded = false;
            self.fill_during_sync = true;
            self.last_synced_at = None;
            self.reviews.retain(|_| false);
            self.recent.clear();
            if let Some(cache) = &self.cache {
                cache.write(LibraryCache::clear);
            }
            // What other models showed from the cache goes with it.
            cx.emit(AccountChanged);
        }
        self.user = Some(user);
        self.user_verified = true;
        cx.notify();
    }

    /// A page of a running sync. A first sync (nothing cached) shows pages as
    /// they arrive; otherwise the cached entries stay until the sync completes.
    fn receive_page(&mut self, page: Vec<LibraryEntry>, cx: &mut Context<Self>) {
        self.sync_fetched += page.len();
        if self.fill_during_sync {
            let mut entries = std::mem::take(&mut self.entries);
            // An entry already shown came from a write made during the sync,
            // which is at least as new: keep it, once.
            entries.extend(
                page.into_iter()
                    .filter(|e| !self.index.contains_key(&e.user_book_id)),
            );
            self.set_entries(entries);
            self.loaded = true;
        }
        cx.notify();
    }

    /// Applies a sync result and saves it to the cache.
    fn finish_sync(&mut self, fetched: Result<LibrarySync, Error>, cx: &mut Context<Self>) {
        let now = SystemTime::now();
        let changes = self
            .sync_started
            .map(|started| self.recent.since_before(started))
            .unwrap_or_default();
        self.recent.prune(now);
        match fetched {
            Ok(sync) => {
                let write = match sync.coverage {
                    Coverage::Complete { .. } => CacheWrite::Replace,
                    Coverage::Partial => CacheWrite::Upsert,
                };
                // Entries the sync couldn't read or reach keep their copies.
                let synced = sync.merged_with(&self.entries);
                let merged = merge_sync(synced, &changes);
                // Nothing new keeps the revision, so nothing worked out from
                // the library is redone and the goals aren't fetched again.
                // The cache is still written: a first sync filled the entries
                // page by page, so they match, but nothing was saved yet.
                if !same_entries(&self.entries, &merged) {
                    self.set_entries(merged.clone());
                }
                self.loaded = true;
                self.sync = SyncStatus::Synced;
                self.last_synced_at = Some(chrono::Utc::now());
                // Fetched again when next shown, also after a failure. A
                // review written lately may be read back stale, so it stays.
                let written: HashSet<UserBookId> = changes
                    .iter()
                    .filter_map(|c| match c {
                        LocalChange::Reviewed(id) => Some(*id),
                        _ => None,
                    })
                    .collect();
                self.reviews.invalidate_all_but(|id| written.contains(id));
                // Sent from this update, so the cache sees the writes in the
                // order the model made them.
                if let (Some(cache), Some(user)) = (&self.cache, self.user.clone()) {
                    cache.write(move |cache| {
                        cache.save_user(&user)?;
                        match write {
                            CacheWrite::Replace => cache.replace_library(user.id, &merged),
                            CacheWrite::Upsert => merged.iter().try_for_each(|e| cache.upsert(e)),
                        }
                    });
                }
                CoverService::retry_failed(cx);
            }
            Err(error) => {
                let offline = matches!(error, Error::Network(_) | Error::Timeout);
                self.sync = SyncStatus::Failed {
                    message: self
                        .session
                        .error_message(&error, Some(READ_LIBRARY), cx)
                        .into(),
                    offline,
                };
                // Without a cache there is nothing to show but the failure:
                // the pages a first sync showed so far are only part of the
                // library, so they go too.
                if self.fill_during_sync && self.sync_fetched > 0 {
                    self.set_entries(Vec::new());
                    self.loaded = false;
                }
            }
        }
        cx.notify();
    }

    /// Stops the session's work, first thing on signing out: the sync, the
    /// writes and the review fetches are dropped, and the cache is cleared
    /// after every write already sent to it. Resolves once it is.
    pub fn end_session(&mut self, cx: &mut Context<Self>) -> impl Future<Output = ()> + use<> {
        self.sync_task = None;
        self.writes.clear();
        self.reviews.retain(|_| false);
        self.cache = None;
        let cleared = self.session.clear_cache();
        cx.notify();
        cleared
    }

    // ----- writes -----

    /// Runs one library write: marks it in flight (one write at a time per
    /// entry, since each returns the whole entry), sends `write`'s call at
    /// interactive priority, applies what it returns whatever view asked for
    /// it, and ends the write. Unless the caller reports for itself
    /// ([`LibraryWrite::reported_by_caller`]), a failure, or a write refused
    /// because another is still saving, is shown as a notification.
    ///
    /// The model owns the write: closing the view that asked doesn't cancel
    /// it. The future is for the caller's own follow-up; dropping it changes
    /// nothing.
    pub fn write<T: Written>(
        &mut self,
        write: LibraryWrite<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl Future<Output = WriteOutcome<T>> + use<T> {
        let (tx, rx) = async_channel::bounded(1);
        let outcome = async move { rx.recv().await.unwrap_or(WriteOutcome::Ended) };
        let LibraryWrite {
            target,
            kind,
            failure_title,
            fallback_scope,
            notify,
            call,
        } = write;
        let refused = if !self.session.is_alive() {
            Some(WriteOutcome::Ended)
        } else if let WriteTarget::Add(book) = target
            && self.entry_for_book(book).is_some()
        {
            // Adding it again would overwrite the reader's choice of shelf
            // (Ignored too): a book in the library changes through its entry.
            Some(WriteOutcome::InLibrary)
        } else if self.writes.contains_key(&target) {
            if notify {
                window.push_notification(Notification::info(STILL_SAVING).title(failure_title), cx);
            }
            Some(WriteOutcome::Busy)
        } else {
            None
        };
        if let Some(refused) = refused {
            let _ = tx.try_send(refused);
            return outcome;
        }
        let call = self.session.interactive(call);
        let window = window.window_handle();
        let task = cx.spawn(async move |this, cx| {
            let result = call.await;
            let outcome = this.update(cx, |this, cx| {
                // Whatever happened, the write is over.
                if let Some(write) = this.writes.remove(&target) {
                    write.task.detach();
                }
                cx.notify();
                // The session ended (or the call panicked): nothing to apply.
                let Some(result) = result else {
                    return WriteOutcome::Ended;
                };
                match result {
                    Ok(value) => {
                        this.apply(target, value.applied(), cx);
                        WriteOutcome::Saved(value)
                    }
                    Err(error) => {
                        let message = this.session.error_message(&error, fallback_scope, cx);
                        WriteOutcome::Failed {
                            error,
                            message: message.into(),
                        }
                    }
                }
            });
            let Ok(outcome) = outcome else {
                return;
            };
            if notify && let WriteOutcome::Failed { message, .. } = &outcome {
                let note = Notification::error(message.clone()).title(failure_title);
                let _ =
                    cx.update_window(window, |_, window, cx| window.push_notification(note, cx));
            }
            let _ = tx.try_send(outcome);
        });
        self.writes.insert(target, InFlight { kind, task });
        cx.notify();
        outcome
    }

    fn apply(&mut self, target: WriteTarget, applied: Applied<'_>, cx: &mut Context<Self>) {
        match (applied, target) {
            (Applied::Entry(entry), _) => self.apply_written(entry.clone(), cx),
            (Applied::Removed, WriteTarget::Entry(id)) => self.apply_removed(id, cx),
            (Applied::Reviewed { entry, review }, WriteTarget::Entry(id)) => {
                // Without an entry, the one shown stays.
                if let Some(entry) = entry {
                    self.apply_written(entry.clone(), cx);
                }
                self.apply_review(id, review.clone(), cx);
            }
            (Applied::Removed | Applied::Reviewed { .. }, WriteTarget::Add(_)) => {}
        }
    }

    /// Replaces (or adds) an entry with the one a write returned, and saves it
    /// to the cache.
    fn apply_written(&mut self, entry: LibraryEntry, cx: &mut Context<Self>) {
        self.recent.record(
            SystemTime::now(),
            LocalChange::Upserted(Box::new(entry.clone())),
        );
        let mut entries = std::mem::take(&mut self.entries);
        match self.index.get(&entry.user_book_id) {
            Some(&ix) => entries[ix] = entry.clone(),
            None => entries.push(entry.clone()),
        }
        self.set_entries(entries);
        cx.notify();
        if let Some(cache) = &self.cache {
            cache.write(move |cache| cache.upsert(&entry));
        }
    }

    fn apply_removed(&mut self, id: UserBookId, cx: &mut Context<Self>) {
        self.reviews.remove(&id);
        self.recent
            .record(SystemTime::now(), LocalChange::Removed(id));
        let mut entries = std::mem::take(&mut self.entries);
        entries.retain(|e| e.user_book_id != id);
        self.set_entries(entries);
        cx.notify();
        if let Some(cache) = &self.cache {
            cache.write(move |cache| cache.remove(id));
        }
    }

    /// The review a review write returned.
    fn apply_review(&mut self, id: UserBookId, review: Review, cx: &mut Context<Self>) {
        self.recent
            .record(SystemTime::now(), LocalChange::Reviewed(id));
        self.reviews.set(id, review, cx);
        cx.notify();
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

// ----- the write API -----

/// One library write, for [`LibraryModel::write`].
pub struct LibraryWrite<T> {
    target: WriteTarget,
    kind: WriteKind,
    failure_title: &'static str,
    fallback_scope: Option<&'static str>,
    notify: bool,
    call: WriteCall<T>,
}

/// A write's request, made on the I/O threads.
type WriteCall<T> = Box<dyn FnOnce(&Client) -> Result<T, Error> + Send>;

impl<T: Written> LibraryWrite<T> {
    /// A write on entry `id`. `failure_title` titles the notification of a
    /// failure ("Couldn't change the status").
    pub fn entry(
        id: UserBookId,
        kind: WriteKind,
        failure_title: &'static str,
        call: impl FnOnce(&Client) -> Result<T, Error> + Send + 'static,
    ) -> Self {
        Self {
            target: WriteTarget::Entry(id),
            kind,
            failure_title,
            fallback_scope: Some(WRITE_LIBRARY),
            notify: true,
            call: Box::new(call),
        }
    }

    /// Adding `book`, which isn't in the library.
    pub fn add(
        book: BookId,
        failure_title: &'static str,
        call: impl FnOnce(&Client) -> Result<T, Error> + Send + 'static,
    ) -> Self {
        Self {
            target: WriteTarget::Add(book),
            kind: WriteKind::Add,
            failure_title,
            fallback_scope: Some(WRITE_LIBRARY),
            notify: true,
            call: Box::new(call),
        }
    }

    pub fn kind(&self) -> WriteKind {
        self.kind
    }

    pub fn target(&self) -> WriteTarget {
        self.target
    }

    /// The write needs more than `write:library` (a review also needs
    /// `write:reviews`), so a refusal that names no scope can't be pinned
    /// on one.
    #[must_use]
    pub fn any_scope(mut self) -> Self {
        self.fallback_scope = None;
        self
    }

    /// The caller shows failures itself (the progress dialog, next to its
    /// field), so no notification is shown.
    #[must_use]
    pub fn reported_by_caller(mut self) -> Self {
        self.notify = false;
        self
    }
}

/// How a write ended, for the caller that asked for it.
pub enum WriteOutcome<T> {
    /// Saved; the model has applied it.
    Saved(T),
    /// The request failed. `message` is what to show (a refused scope has
    /// been recorded).
    Failed { error: Error, message: SharedString },
    /// Another write on the same entry is still saving; nothing was sent.
    Busy,
    /// The book to add is already in the library; nothing was sent.
    InLibrary,
    /// The session ended: nothing was sent, or the result was dropped.
    Ended,
}

/// What a write returns, as the model applies it.
pub trait Written: Send + 'static {
    fn applied(&self) -> Applied<'_>;
}

/// What the model applies after a write.
pub enum Applied<'a> {
    /// The entry as of the write: replaces the local one, or adds it.
    Entry(&'a LibraryEntry),
    /// The entry is gone from the library.
    Removed,
    /// A review write: its review, and the entry when the response had one.
    Reviewed {
        entry: Option<&'a LibraryEntry>,
        review: &'a Review,
    },
}

/// What `remove_from_library` returns.
pub struct Removed;

impl Written for LibraryEntry {
    fn applied(&self) -> Applied<'_> {
        Applied::Entry(self)
    }
}

impl Written for Removed {
    fn applied(&self) -> Applied<'_> {
        Applied::Removed
    }
}

/// Whatever went through is shown, even if the progress didn't.
impl Written for ProgressChange {
    fn applied(&self) -> Applied<'_> {
        Applied::Entry(&self.entry)
    }
}

impl Written for EditionChange {
    fn applied(&self) -> Applied<'_> {
        Applied::Entry(&self.entry)
    }
}

impl Written for ReviewedEntry {
    fn applied(&self) -> Applied<'_> {
        Applied::Reviewed {
            entry: self.entry.as_ref(),
            review: &self.review,
        }
    }
}

struct Cached {
    user: User,
    entries: Vec<LibraryEntry>,
    last_synced_at: Option<Timestamp>,
}

/// Blocking.
fn load_cached(cache: &LibraryCache) -> Option<Cached> {
    let user = cache.load_user().ok().flatten()?;
    let entries = cache.load_library(user.id).ok()?;
    let last_synced_at = cache.last_synced_at(user.id).ok().flatten();
    Some(Cached {
        user,
        entries,
        last_synced_at,
    })
}

/// How a finished sync is saved. A complete one replaces the cached
/// library, which drops entries that are gone from Hardcover; one that
/// stopped early only adds and updates, so nothing is deleted on its
/// strength.
enum CacheWrite {
    Replace,
    Upsert,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Entity;
    use std::sync::Arc;

    fn review(text: &str) -> Review {
        Review::new(text, false, None)
    }

    /// A model over the demo client, with an empty cache in memory. Its
    /// calls run inline (`SessionScope::for_tests`).
    fn demo_model(
        cx: &mut gpui_kit::TestAppContext,
    ) -> (Entity<LibraryModel>, SessionScope, Arc<LibraryCache>) {
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let session = SessionScope::for_tests(hardcover_core::demo::client(), Some(cache.clone()));
        cx.update(|cx| session.install(cx));
        let model = cx.new(|_| LibraryModel::new(session.clone()));
        (model, session, cache)
    }

    /// A fetch of the reader's own review that started before a write of it
    /// is dropped, even after a sync made every review stale meanwhile.
    #[gpui_kit::test]
    fn an_own_review_fetch_older_than_a_write_is_dropped(cx: &mut gpui_kit::TestAppContext) {
        let (model, _, _) = demo_model(cx);
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let id = model.read_with(cx, |m, _| m.entries()[0].user_book_id);
        model.update(cx, |m, cx| {
            m.request_review(id, Demand::Shown, cx);
            m.apply_review(id, review("Written here"), cx);
            m.reviews.invalidate_all_but(|_| false);
        });
        cx.run_until_parked();
        model.read_with(cx, |m, _| {
            let shown = m.reviews().value(&id).map(|r| (**r).clone());
            assert_eq!(shown, Some(review("Written here")));
        });
    }

    /// Signs out as the workspace does: the session ends, then the model.
    fn sign_out(
        model: &Entity<LibraryModel>,
        session: &SessionScope,
        cx: &mut gpui_kit::TestAppContext,
    ) {
        session.end();
        let cleared = model.update(cx, LibraryModel::end_session);
        cx.update(SessionScope::uninstall);
        cx.foreground_executor().spawn(cleared).detach();
    }

    #[gpui_kit::test]
    fn a_first_sync_is_saved_and_an_unchanged_one_keeps_the_revision(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (model, _, cache) = demo_model(cx);
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let (user, revision, shown) = model.read_with(cx, |m, _| {
            assert_eq!(*m.sync_status(), SyncStatus::Synced);
            (m.user().unwrap().id, m.revision(), m.entries().to_vec())
        });
        // The entries were filled page by page, so the result matched them:
        // it must be saved all the same.
        let saved = cache.load_library(user).unwrap();
        assert!(!saved.is_empty());
        assert!(same_entries(&saved, &shown));

        model.update(cx, LibraryModel::sync);
        cx.run_until_parked();
        model.read_with(cx, |m, _| {
            assert_eq!(*m.sync_status(), SyncStatus::Synced);
            assert_eq!(m.revision(), revision, "nothing new: same revision");
        });

        // Just synced: coming back to the window doesn't sync again.
        model.update(cx, |m, cx| {
            m.sync_if_stale(cx);
            assert_eq!(*m.sync_status(), SyncStatus::Synced);
        });
    }

    #[gpui_kit::test]
    fn a_failed_first_sync_is_never_loaded_not_an_empty_library(cx: &mut gpui_kit::TestAppContext) {
        let offline = hardcover_core::demo::Faults::parse("offline").unwrap();
        let session = SessionScope::for_tests(hardcover_core::demo::client_with(&offline), None);
        cx.update(|cx| session.install(cx));
        let model = cx.new(|_| LibraryModel::new(session));
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let never_loaded = |m: &LibraryModel| {
            assert!(!m.is_loaded(), "nothing has loaded");
            assert!(m.load_failure().is_some(), "the views show why");
            assert!(m.entries().is_empty());
        };
        model.read_with(cx, |m, _| never_loaded(m));

        // Try again (a sync) that fails too leaves it the same.
        model.update(cx, LibraryModel::sync);
        cx.run_until_parked();
        model.read_with(cx, |m, _| never_loaded(m));
    }

    #[gpui_kit::test]
    fn a_book_in_the_library_is_never_added_again(cx: &mut gpui_kit::TestAppContext) {
        let (model, _, _) = demo_model(cx);
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let had = model.read_with(cx, |m, _| m.entries()[0].book.id);
        let cx = cx.add_empty_window();
        let outcome = cx.update(|window, cx| {
            model.update(cx, |m, cx| {
                let add = LibraryWrite::add(had, "Couldn't add the book", move |client| {
                    client.add_to_library(had, ReadingStatus::WantToRead)
                });
                let outcome = m.write(add, window, cx);
                assert!(!m.is_adding(had));
                outcome
            })
        });
        let outcome = cx.foreground_executor().spawn(outcome);
        cx.run_until_parked();
        assert!(matches!(
            gpui_kit::block_on(outcome),
            WriteOutcome::InLibrary
        ));
    }

    /// The test for the crash and the cache refill of a sign-out (review
    /// D2): a write and a sync still running when the user signs out land
    /// afterwards. Neither may panic, change the library shown, or write
    /// the old account back into the cache.
    #[gpui_kit::test]
    fn a_write_and_a_sync_landing_after_sign_out_are_dropped(cx: &mut gpui_kit::TestAppContext) {
        let (model, session, cache) = demo_model(cx);
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let (entry, revision) = model.read_with(cx, |m, _| (m.entries()[0].clone(), m.revision()));
        assert!(cache.load_user().unwrap().is_some());

        // Both calls run inline: their results wait to be picked up.
        let window = cx.add_empty_window();
        let id = entry.user_book_id;
        let status = if entry.status == ReadingStatus::Read {
            ReadingStatus::Paused
        } else {
            ReadingStatus::Read
        };
        let outcome = window.update(|window, cx| {
            model.update(cx, |m, cx| {
                m.sync(cx);
                let write = LibraryWrite::entry(
                    id,
                    WriteKind::Status,
                    "Couldn't change the status",
                    move |client| client.set_status(id, status),
                );
                m.write(write, window, cx)
            })
        });
        assert_eq!(
            model.read_with(window, |m, _| m.pending_write(id)),
            Some(WriteKind::Status)
        );

        sign_out(&model, &session, window);
        let outcome = window.foreground_executor().spawn(outcome);
        window.run_until_parked();

        assert!(matches!(gpui_kit::block_on(outcome), WriteOutcome::Ended));
        model.read_with(window, |m, _| {
            assert_eq!(m.pending_write(id), None);
            assert_eq!(m.revision(), revision, "nothing applied");
            assert_eq!(m.entry(id).map(|e| e.status), Some(entry.status));
        });
        assert_eq!(cache.load_user().unwrap(), None, "the cache stays cleared");

        // The next sign-in on the same cache starts from nothing.
        let next = SessionScope::for_tests(hardcover_core::demo::client(), Some(cache.clone()));
        let fresh = window.new(|_| LibraryModel::new(next));
        let cached = fresh.read_with(window, |m, _| m.cache.as_ref().map(|c| c.ask(load_cached)));
        let cached = gpui_kit::block_on(cached.unwrap()).flatten();
        assert!(cached.is_none());
    }

    /// A demo entry and a status it isn't on.
    fn entry_and_other_status(
        model: &Entity<LibraryModel>,
        cx: &mut gpui_kit::TestAppContext,
    ) -> (LibraryEntry, ReadingStatus) {
        let entry = model.read_with(cx, |m, _| m.entries()[0].clone());
        let status = if entry.status == ReadingStatus::Read {
            ReadingStatus::Paused
        } else {
            ReadingStatus::Read
        };
        (entry, status)
    }

    fn status_write(id: UserBookId, status: ReadingStatus) -> LibraryWrite<LibraryEntry> {
        LibraryWrite::entry(
            id,
            WriteKind::Status,
            "Couldn't change the status",
            move |client| client.set_status(id, status),
        )
        .reported_by_caller()
    }

    #[gpui_kit::test]
    fn a_write_is_applied_saved_and_ended(cx: &mut gpui_kit::TestAppContext) {
        let (model, _, cache) = demo_model(cx);
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let (entry, status) = entry_and_other_status(&model, cx);
        let id = entry.user_book_id;
        let window = cx.add_empty_window();
        let (saved, busy) = window.update(|window, cx| {
            model.update(cx, |m, cx| {
                let saved = m.write(status_write(id, status), window, cx);
                // One write per entry at a time.
                let busy = m.write(status_write(id, ReadingStatus::WantToRead), window, cx);
                (saved, busy)
            })
        });
        let (saved, busy) = (
            window.foreground_executor().spawn(saved),
            window.foreground_executor().spawn(busy),
        );
        window.run_until_parked();
        assert!(matches!(gpui_kit::block_on(busy), WriteOutcome::Busy));
        assert!(matches!(gpui_kit::block_on(saved), WriteOutcome::Saved(e) if e.status == status));
        model.read_with(window, |m, _| {
            assert_eq!(m.pending_write(id), None);
            assert_eq!(m.entry(id).map(|e| e.status), Some(status));
        });
        let user = model.read_with(window, |m, _| m.verified_user_id().unwrap());
        let cached = cache.load_library(user).unwrap();
        assert_eq!(
            cached
                .iter()
                .find(|e| e.user_book_id == id)
                .map(|e| e.status),
            Some(status)
        );
    }

    #[gpui_kit::test]
    fn a_failed_write_changes_nothing_and_ends(cx: &mut gpui_kit::TestAppContext) {
        let (model, _, _) = demo_model(cx);
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let (entry, _) = entry_and_other_status(&model, cx);
        let id = entry.user_book_id;
        let window = cx.add_empty_window();
        let failed = window.update(|window, cx| {
            model.update(cx, |m, cx| {
                let write = LibraryWrite::entry(
                    id,
                    WriteKind::Status,
                    "Couldn't change the status",
                    |_| Err::<LibraryEntry, _>(Error::Api("refused".into())),
                )
                .reported_by_caller();
                m.write(write, window, cx)
            })
        });
        let failed = window.foreground_executor().spawn(failed);
        window.run_until_parked();
        assert!(matches!(
            gpui_kit::block_on(failed),
            WriteOutcome::Failed {
                error: Error::Api(_),
                ..
            }
        ));
        model.read_with(window, |m, _| {
            assert_eq!(m.pending_write(id), None, "a failure ends the write too");
            assert_eq!(m.entry(id), Some(&entry));
        });
    }

    /// The session ended, and the results land before the model is told
    /// (`end_session`): they are dropped on their own.
    #[gpui_kit::test]
    fn results_landing_after_the_session_ended_are_dropped_by_the_session(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (model, session, cache) = demo_model(cx);
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let (entry, status) = entry_and_other_status(&model, cx);
        let id = entry.user_book_id;
        let user = model.read_with(cx, |m, _| m.verified_user_id().unwrap());
        let before = cache.load_library(user).unwrap();
        let revision = model.read_with(cx, |m, _| m.revision());
        let window = cx.add_empty_window();
        let outcome = window.update(|window, cx| {
            model.update(cx, |m, cx| {
                m.sync(cx);
                m.write(status_write(id, status), window, cx)
            })
        });
        session.end();
        let outcome = window.foreground_executor().spawn(outcome);
        window.run_until_parked();
        assert!(matches!(gpui_kit::block_on(outcome), WriteOutcome::Ended));
        model.read_with(window, |m, _| {
            assert_eq!(m.pending_write(id), None);
            assert_eq!(m.revision(), revision, "nothing applied");
        });
        assert!(
            same_entries(&cache.load_library(user).unwrap(), &before),
            "nothing cached"
        );
    }

    #[gpui_kit::test]
    fn a_library_cached_for_another_account_is_dropped(cx: &mut gpui_kit::TestAppContext) {
        let (model, _, cache) = demo_model(cx);
        let stranger = User::new(UserId::new(987_654), "someone-else");
        let theirs = LibraryEntry::new(
            UserBookId::new(987_654),
            stranger.id,
            hardcover_core::BookSummary::new(BookId::new(987_654), "Their book"),
            ReadingStatus::Read,
        );
        cache.save_user(&stranger).unwrap();
        cache
            .replace_library(stranger.id, std::slice::from_ref(&theirs))
            .unwrap();

        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        model.read_with(cx, |m, _| {
            let me = m.verified_user_id().expect("me() answered");
            assert_ne!(me, stranger.id);
            assert!(m.entry(theirs.user_book_id).is_none());
            assert!(!m.entries().is_empty(), "the demo reader's library");
        });
        assert!(cache.load_library(stranger.id).unwrap().is_empty());
        assert_ne!(cache.load_user().unwrap().map(|u| u.id), Some(stranger.id));
    }

    #[gpui_kit::test]
    fn a_write_during_the_first_sync_shows_its_entry_once(cx: &mut gpui_kit::TestAppContext) {
        let (model, _, _) = demo_model(cx);
        let book = |id| {
            LibraryEntry::new(
                UserBookId::new(id),
                UserId::new(1),
                hardcover_core::BookSummary::new(BookId::new(id), "A book"),
                ReadingStatus::WantToRead,
            )
        };
        model.update(cx, |m, cx| {
            m.fill_during_sync = true;
            let mut written = book(1);
            written.status = ReadingStatus::CurrentlyReading;
            m.apply_written(written, cx);
            // The page was read before the write landed.
            m.receive_page(vec![book(1), book(2)], cx);
            assert_eq!(m.entries().len(), 2);
            assert_eq!(m.count(ReadingStatus::CurrentlyReading), 1);
            assert_eq!(m.count(ReadingStatus::WantToRead), 1);
        });
    }
}
