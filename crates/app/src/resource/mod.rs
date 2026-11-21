//! Data fetched for the signed-in session, kept per key: one place that
//! decides when to fetch, keeps what was fetched while it is fetched again,
//! and drops what arrives too late.
//!
//! A [`Resource`] holds, for each key, the last value fetched (or written),
//! a [`Stamp`] taken when the request behind it *started*, the request in
//! flight, a write generation and why the last fetch failed. Asking for a
//! key ([`Resource::request`]) with a [`Demand`] is all a caller does; the
//! resource decides:
//!
//! - **Never fetched, or stale** (older than [`STALE_AFTER`], or asked for
//!   before the last Refresh): fetch, keeping the value shown meanwhile.
//! - **In flight:** nothing more, unless the request in flight started
//!   before the last Refresh: then it is started again, so Refresh never
//!   finds a request it can't count.
//! - **Failed:** tried again only when shown ([`Demand::Shown`]: opened,
//!   shown again, Refresh) or asked for ([`Demand::Now`]: "Try again"),
//!   never because the window became active ([`Demand::Active`]), so being
//!   offline doesn't send a request on every return to the window. A failed
//!   fetch counts as a try either way.
//! - **A write** ([`Resource::set`], [`Resource::update`],
//!   [`Resource::note_write`]) bumps the key's generation: a fetch that
//!   started before it is older than the write and is dropped when it lands.
//!   It isn't fetched again at once, since Hardcover can answer a read right
//!   after a write with the old row; owners that need a reload schedule one
//!   ([`Debounce`]).
//! - **The session ended:** a fetch never starts, and one that lands is
//!   dropped (`SessionScope::run`).
//!
//! An equal result only renews the stamp ([`Landing::Same`]), so lists built
//! from it aren't rebuilt and their selection doesn't move.
//!
//! A resource is a plain value inside its owner entity (a model, or a
//! [`Shared`] store), which it reaches through a [`Binding`]: the task of a
//! fetch updates the owner, and the binding's `landed` hook tells the owner
//! what changed. The catalogue's resources, shared by every view, are in
//! [`Catalog`].
//!
//! Some resources outlive the run in the library cache ([`Resource::persist`]):
//! the last good value is shown at the next launch, offline too, and fetched
//! again as stale ([`Resource::hydrate`]).

pub mod catalog;
pub mod follow;
pub mod saved;
mod stamp;

use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::{App, Context, SharedString, Task};
use hardcover_core::Client;

pub use catalog::{Catalog, Paged, ReviewPages, Shared, SharedChanged};
pub use follow::{Debounce, LibraryFollow};
pub use stamp::{STALE_AFTER, Stamp, refresh_all};

use crate::io::Priority;
use crate::session::SessionScope;

/// Why a key is asked for, which decides whether it is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Demand {
    /// Whatever shows it may want it: the window became active, a library
    /// change, a value it depends on landed. Fetches what was never fetched
    /// or is stale; not again after a failure.
    Active,
    /// It was shown: opened, shown again, or Refresh. Also tries again after
    /// a failure.
    Shown,
    /// Asked for in so many words ("Try again", Discover's Refresh button):
    /// fetches now, starting a request in flight again.
    Now,
}

/// Where a key's loading stands, for owners that report one status (a model
/// with one value, or a summary of its keys).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum LoadStatus {
    /// Never loaded, or the session can't read it.
    #[default]
    Idle,
    /// A request is in flight; the value shown, if any, is the previous one.
    Loading,
    Loaded,
    /// The last fetch failed; the value shown, if any, is the previous one.
    Failed(SharedString),
}

/// What happened when a request landed, for the owner's `landed` hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// A new value (or the first).
    Changed,
    /// The same value again: only the stamp was renewed, and a failure
    /// cleared.
    Same,
    /// The fetch failed; the value, if any, stays.
    Failed,
    /// A write came after the request started: its result was dropped.
    Dropped,
}

/// How a resource is reached in its owner `T`, and how the owner hears of
/// a request landing.
pub struct Binding<T, K, V> {
    pub get: fn(&mut T) -> &mut Resource<K, V>,
    pub landed: fn(&mut T, &K, Landing, &mut Context<T>),
}

impl<T, K, V> Clone for Binding<T, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, K, V> Copy for Binding<T, K, V> {}

/// One batch request's answer ([`Resource::request_many`]): a value for
/// every key that loaded, and the keys of the parts that failed, with the
/// first failure.
pub struct Batch<K, V> {
    pub found: Vec<(K, V)>,
    pub failed: Vec<K>,
    pub error: Option<hardcover_core::Error>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    /// Nothing was shown: the message is the content.
    First,
    /// A value was shown and stays: the message is said quietly, if at all.
    Revalidate,
    /// More of a value ([`Resource::extend`]) failed: said beside the value.
    More,
}

#[derive(Debug, Clone)]
struct Failure {
    message: SharedString,
    kind: FailureKind,
}

struct Flight {
    id: u64,
    started: Stamp,
    /// The key's write generation when the request started.
    generation: u64,
    _task: Rc<Task<()>>,
}

struct Slot<V> {
    value: Option<Arc<V>>,
    /// When the request behind `value` (or the last failure) started;
    /// `None` when it must be fetched again (from the disk, invalidated).
    stamp: Option<Stamp>,
    failure: Option<Failure>,
    generation: u64,
    flight: Option<Flight>,
    /// `value` came from the disk and nothing has replaced it this run.
    from_disk: bool,
}

impl<V> Default for Slot<V> {
    fn default() -> Self {
        Self {
            value: None,
            stamp: None,
            failure: None,
            generation: 0,
            flight: None,
            from_disk: false,
        }
    }
}

/// What a request lands with.
enum Outcome<V> {
    Value(V),
    Failed(SharedString),
    /// The session ended, or the call panicked: nothing to apply.
    Ended,
}

/// Whether to fetch a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Skip,
    Start,
    /// Drop the request in flight and start another.
    Restart,
}

/// Decides whether `slot` is fetched for `demand`. Pure; see the module doc.
fn plan<V>(slot: Option<&Slot<V>>, demand: Demand, refresh: u64, now: SystemTime) -> Plan {
    let Some(slot) = slot else {
        return Plan::Start;
    };
    if let Some(flight) = &slot.flight {
        return if demand == Demand::Now || flight.started.predates(refresh) {
            Plan::Restart
        } else {
            Plan::Skip
        };
    }
    if demand == Demand::Now {
        return Plan::Start;
    }
    if slot
        .failure
        .as_ref()
        .is_some_and(|f| f.kind != FailureKind::More)
    {
        return if demand == Demand::Shown {
            Plan::Start
        } else {
            Plan::Skip
        };
    }
    match (&slot.value, slot.stamp) {
        (Some(_), Some(stamp)) if !stamp.is_stale_at(now, refresh) => Plan::Skip,
        _ => Plan::Start,
    }
}

type Persist<K, V> = Box<dyn Fn(&K, Option<&V>)>;

/// Values fetched per key `K`; see the module doc.
pub struct Resource<K, V> {
    session: SessionScope,
    /// What the requests need: a failure for want of it is recorded as a
    /// refused scope, and nothing is asked for without it.
    scope: &'static str,
    priority: Priority,
    slots: HashMap<K, Slot<V>>,
    next_flight: u64,
    persist: Option<Persist<K, V>>,
    /// Requests started, for tests.
    #[cfg(test)]
    started: usize,
}

impl<K, V> Resource<K, V>
where
    K: Clone + Eq + Hash + 'static,
    V: PartialEq + 'static,
{
    pub fn new(session: SessionScope, scope: &'static str, priority: Priority) -> Self {
        Self {
            session,
            scope,
            priority,
            slots: HashMap::new(),
            next_flight: 0,
            persist: None,
            #[cfg(test)]
            started: 0,
        }
    }

    /// Saves each new value (and forgets a removed one) with `save`, which
    /// the owner points at the library cache.
    #[must_use]
    pub fn persist(mut self, save: impl Fn(&K, Option<&V>) + 'static) -> Self {
        self.persist = Some(Box::new(save));
        self
    }

    pub fn session(&self) -> &SessionScope {
        &self.session
    }

    // ----- reads -----

    pub fn value(&self, key: &K) -> Option<&Arc<V>> {
        self.slots.get(key)?.value.as_ref()
    }

    /// Every key with a value.
    pub fn values(&self) -> impl Iterator<Item = (&K, &Arc<V>)> {
        self.slots
            .iter()
            .filter_map(|(k, s)| s.value.as_ref().map(|v| (k, v)))
    }

    pub fn status(&self, key: &K) -> LoadStatus {
        let Some(slot) = self.slots.get(key) else {
            return LoadStatus::Idle;
        };
        match (&slot.flight, &slot.failure, &slot.value) {
            (Some(_), _, _) => LoadStatus::Loading,
            (None, Some(f), _) if f.kind != FailureKind::More => {
                LoadStatus::Failed(f.message.clone())
            }
            (None, _, Some(_)) => LoadStatus::Loaded,
            _ => LoadStatus::Idle,
        }
    }

    /// One status for the keys `keys` holds: loading while any is, else
    /// failed with the first failure, else loaded once any has settled.
    pub fn summary<'a>(&self, keys: impl IntoIterator<Item = &'a K>) -> LoadStatus
    where
        K: 'a,
    {
        let mut status = LoadStatus::Idle;
        for key in keys {
            match self.status(key) {
                LoadStatus::Loading => return LoadStatus::Loading,
                failed @ LoadStatus::Failed(_) if !matches!(status, LoadStatus::Failed(_)) => {
                    status = failed;
                }
                LoadStatus::Loaded if status == LoadStatus::Idle => status = LoadStatus::Loaded,
                _ => {}
            }
        }
        status
    }

    /// A request for `key` is in flight.
    pub fn is_loading(&self, key: &K) -> bool {
        self.slots.get(key).is_some_and(|s| s.flight.is_some())
    }

    /// Why the last fetch of `key` failed when nothing was shown, so the
    /// message stands in for the value.
    pub fn error(&self, key: &K) -> Option<&SharedString> {
        let failure = self.slots.get(key)?.failure.as_ref()?;
        (failure.kind == FailureKind::First).then_some(&failure.message)
    }

    /// Why the last fetch of `key` failed, whether or not a value is shown.
    pub fn failure(&self, key: &K) -> Option<&SharedString> {
        let failure = self.slots.get(key)?.failure.as_ref()?;
        (failure.kind != FailureKind::More).then_some(&failure.message)
    }

    /// Why fetching more of `key` failed ([`Self::extend`]).
    pub fn more_error(&self, key: &K) -> Option<&SharedString> {
        let failure = self.slots.get(key)?.failure.as_ref()?;
        (failure.kind == FailureKind::More).then_some(&failure.message)
    }

    /// A fetch of `key` has finished (or failed) at least once, or a value
    /// came from the disk: an empty value means there is nothing.
    pub fn has_settled(&self, key: &K) -> bool {
        self.slots
            .get(key)
            .is_some_and(|s| s.value.is_some() || s.failure.is_some())
    }

    /// The value of `key` would be fetched by [`Demand::Active`]: never
    /// fetched, or stale (and not failed).
    pub fn is_stale(&self, key: &K, cx: &App) -> bool {
        plan(
            self.slots.get(key),
            Demand::Active,
            stamp::generation(cx),
            SystemTime::now(),
        ) != Plan::Skip
            && !self.is_loading(key)
    }

    #[cfg(test)]
    pub fn started(&self) -> usize {
        self.started
    }

    // ----- fetching -----

    /// Fetches `key` with `call` if `demand` calls for it (see the module
    /// doc). True if a request went out.
    pub fn request<T: 'static>(
        &mut self,
        key: K,
        demand: Demand,
        call: impl FnOnce(&Client) -> hardcover_core::Result<V> + Send + 'static,
        bind: Binding<T, K, V>,
        cx: &mut Context<T>,
    ) -> bool
    where
        V: Send,
    {
        if !self.wants(&key, demand, cx) {
            return false;
        }
        let id = self.flight_id();
        let fetch = self.session.run(self.priority, call);
        let task = cx.spawn({
            let key = key.clone();
            async move |this, cx| {
                let result = fetch.await;
                let _ = this.update(cx, |owner, cx| {
                    let resource = (bind.get)(owner);
                    let outcome = match result {
                        None => Outcome::Ended,
                        Some(Ok(value)) => Outcome::Value(value),
                        Some(Err(e)) => Outcome::Failed(resource.message(&e, cx)),
                    };
                    let resource = (bind.get)(owner);
                    if let Some(landing) = resource.land(&key, id, outcome) {
                        (bind.landed)(owner, &key, landing, cx);
                    }
                });
            }
        });
        self.fly(&key, id, Rc::new(task), cx);
        true
    }

    /// Fetches, in one request, every key of `keys` that `demand` calls for,
    /// with `call` (given those keys). Its [`Batch`] must name each key it
    /// was given, as found or failed. How many keys were asked for.
    pub fn request_many<T: 'static>(
        &mut self,
        keys: impl IntoIterator<Item = K>,
        demand: Demand,
        call: impl FnOnce(&Client, &[K]) -> Batch<K, V> + Send + 'static,
        bind: Binding<T, K, V>,
        cx: &mut Context<T>,
    ) -> usize
    where
        K: Send,
        V: Send,
    {
        let keys: Vec<K> = keys
            .into_iter()
            .filter(|k| self.wants(k, demand, cx))
            .collect();
        if keys.is_empty() {
            return 0;
        }
        let id = self.flight_id();
        let asked = keys.clone();
        let fetch = self
            .session
            .run(self.priority, move |client| call(client, &asked));
        let task = Rc::new(cx.spawn({
            let keys = keys.clone();
            async move |this, cx| {
                let result = fetch.await;
                let _ = this.update(cx, |owner, cx| {
                    let resource = (bind.get)(owner);
                    let mut outcomes: Vec<(K, Outcome<V>)> = Vec::new();
                    match result {
                        None => outcomes.extend(keys.into_iter().map(|k| (k, Outcome::Ended))),
                        Some(batch) => {
                            let message = batch.error.as_ref().map(|e| resource.message(e, cx));
                            outcomes.extend(
                                batch.found.into_iter().map(|(k, v)| (k, Outcome::Value(v))),
                            );
                            let message = message.unwrap_or_else(|| "Couldn't load this.".into());
                            outcomes.extend(
                                batch
                                    .failed
                                    .into_iter()
                                    .map(|k| (k, Outcome::Failed(message.clone()))),
                            );
                        }
                    }
                    for (key, outcome) in outcomes {
                        let resource = (bind.get)(owner);
                        if let Some(landing) = resource.land(&key, id, outcome) {
                            (bind.landed)(owner, &key, landing, cx);
                        }
                    }
                    // A key the batch didn't name ends its request quietly.
                    let resource = (bind.get)(owner);
                    for slot in resource.slots.values_mut() {
                        if slot.flight.as_ref().is_some_and(|f| f.id == id) {
                            slot.flight = None;
                        }
                    }
                });
            }
        }));
        let count = keys.len();
        for key in keys {
            self.fly(&key, id, task.clone(), cx);
        }
        count
    }

    /// Fetches more of `key`'s value (its next page) with `call`, and adds it
    /// with `merge`. Only once there is a value and nothing is in flight; a
    /// failure is kept apart ([`Self::more_error`]) and leaves the value and
    /// its stamp as they are. True if a request went out.
    pub fn extend<T: 'static, R: Send + 'static>(
        &mut self,
        key: K,
        call: impl FnOnce(&Client) -> hardcover_core::Result<R> + Send + 'static,
        merge: fn(&V, R) -> V,
        bind: Binding<T, K, V>,
        cx: &mut Context<T>,
    ) -> bool {
        if !self.session.allows(self.scope) {
            return false;
        }
        match self.slots.get(&key) {
            Some(slot) if slot.value.is_some() && slot.flight.is_none() => {}
            _ => return false,
        }
        let id = self.flight_id();
        let fetch = self.session.run(self.priority, call);
        let task = cx.spawn({
            let key = key.clone();
            async move |this, cx| {
                let result = fetch.await;
                let _ = this.update(cx, |owner, cx| {
                    let resource = (bind.get)(owner);
                    let outcome = match result {
                        None => Outcome::Ended,
                        Some(Ok(more)) => Outcome::Value(more),
                        Some(Err(e)) => Outcome::Failed(resource.message(&e, cx)),
                    };
                    let resource = (bind.get)(owner);
                    if let Some(landing) = resource.land_more(&key, id, outcome, merge) {
                        (bind.landed)(owner, &key, landing, cx);
                    }
                });
            }
        });
        self.fly(&key, id, Rc::new(task), cx);
        let slot = self.slots.get_mut(&key).expect("just flown");
        if slot
            .failure
            .as_ref()
            .is_some_and(|f| f.kind == FailureKind::More)
        {
            slot.failure = None;
        }
        true
    }

    fn wants(&self, key: &K, demand: Demand, cx: &App) -> bool {
        self.session.allows(self.scope)
            && plan(
                self.slots.get(key),
                demand,
                stamp::generation(cx),
                SystemTime::now(),
            ) != Plan::Skip
    }

    fn flight_id(&mut self) -> u64 {
        self.next_flight += 1;
        self.next_flight
    }

    /// Records the request `id` for `key`, replacing (and so dropping) one
    /// already in flight.
    fn fly(&mut self, key: &K, id: u64, task: Rc<Task<()>>, cx: &App) {
        #[cfg(test)]
        {
            self.started += 1;
        }
        let started = Stamp::now(cx);
        let slot = self.slots.entry(key.clone()).or_default();
        slot.flight = Some(Flight {
            id,
            started,
            generation: slot.generation,
            _task: task,
        });
    }

    fn message(&self, error: &hardcover_core::Error, cx: &mut App) -> SharedString {
        self.session
            .error_message(error, Some(self.scope), cx)
            .into()
    }

    /// Applies what request `id` brought for `key`. `None` when there is
    /// nothing to tell: the request was replaced, or the session ended.
    fn land(&mut self, key: &K, id: u64, outcome: Outcome<V>) -> Option<Landing> {
        let slot = self.slots.get_mut(key)?;
        if slot.flight.as_ref().map(|f| f.id) != Some(id) {
            return None;
        }
        let flight = slot.flight.take()?;
        if matches!(outcome, Outcome::Ended) {
            return None;
        }
        if flight.generation != slot.generation {
            return Some(Landing::Dropped);
        }
        slot.stamp = Some(flight.started);
        match outcome {
            Outcome::Value(value) => {
                slot.failure = None;
                slot.from_disk = false;
                if slot.value.as_deref() == Some(&value) {
                    return Some(Landing::Same);
                }
                let value = Arc::new(value);
                slot.value = Some(value.clone());
                if let Some(save) = &self.persist {
                    save(key, Some(&value));
                }
                Some(Landing::Changed)
            }
            Outcome::Failed(message) => {
                let kind = if slot.value.is_some() {
                    FailureKind::Revalidate
                } else {
                    FailureKind::First
                };
                slot.failure = Some(Failure { message, kind });
                Some(Landing::Failed)
            }
            Outcome::Ended => None,
        }
    }

    /// [`Self::land`] for [`Self::extend`]: the stamp stays.
    fn land_more<R>(
        &mut self,
        key: &K,
        id: u64,
        outcome: Outcome<R>,
        merge: fn(&V, R) -> V,
    ) -> Option<Landing> {
        let slot = self.slots.get_mut(key)?;
        if slot.flight.as_ref().map(|f| f.id) != Some(id) {
            return None;
        }
        let flight = slot.flight.take()?;
        if matches!(outcome, Outcome::Ended) {
            return None;
        }
        if flight.generation != slot.generation {
            return Some(Landing::Dropped);
        }
        match outcome {
            Outcome::Value(more) => {
                let value = Arc::new(merge(slot.value.as_deref()?, more));
                slot.value = Some(value.clone());
                slot.failure = None;
                if let Some(save) = &self.persist {
                    save(key, Some(&value));
                }
                Some(Landing::Changed)
            }
            Outcome::Failed(message) => {
                slot.failure = Some(Failure {
                    message,
                    kind: FailureKind::More,
                });
                Some(Landing::Failed)
            }
            Outcome::Ended => None,
        }
    }

    // ----- writes -----

    /// A write's own result: replaces `key`'s value as fresh. A fetch in
    /// flight is older, and dropped when it lands.
    pub fn set(&mut self, key: K, value: V, cx: &App) {
        let stamp = Stamp::now(cx);
        let slot = self.slots.entry(key.clone()).or_default();
        let value = Arc::new(value);
        slot.value = Some(value.clone());
        slot.stamp = Some(stamp);
        slot.failure = None;
        slot.generation += 1;
        slot.from_disk = false;
        if let Some(save) = &self.persist {
            save(&key, Some(&value));
        }
    }

    /// Changes `key`'s value in place (a write's local effect). Nothing
    /// happens without a value. A fetch in flight is dropped when it lands.
    pub fn update(&mut self, key: &K, edit: impl FnOnce(&mut V))
    where
        V: Clone,
    {
        let Some(slot) = self.slots.get_mut(key) else {
            return;
        };
        let Some(value) = slot.value.as_mut() else {
            return;
        };
        edit(Arc::make_mut(value));
        slot.generation += 1;
        if let Some(save) = &self.persist {
            save(key, Some(value));
        }
    }

    /// Changes every value in place (a write that touches several keys).
    pub fn update_all(&mut self, mut edit: impl FnMut(&K, &mut V))
    where
        V: Clone,
    {
        for (key, slot) in &mut self.slots {
            if let Some(value) = slot.value.as_mut() {
                edit(key, Arc::make_mut(value));
                slot.generation += 1;
                if let Some(save) = &self.persist {
                    save(key, Some(value));
                }
            }
        }
    }

    /// A write that changes `key` is starting or has ended: a fetch in
    /// flight is older than it, and dropped when it lands.
    pub fn note_write(&mut self, key: &K) {
        self.slots.entry(key.clone()).or_default().generation += 1;
    }

    /// Marks `key` to be fetched again on the next request, keeping its
    /// value shown meanwhile.
    pub fn invalidate(&mut self, key: &K) {
        if let Some(slot) = self.slots.get_mut(key) {
            slot.stamp = None;
            slot.failure = None;
        }
    }

    /// [`Self::invalidate`] for every key but those `keep` holds.
    pub fn invalidate_all_but(&mut self, keep: impl Fn(&K) -> bool) {
        for (key, slot) in &mut self.slots {
            if !keep(key) {
                slot.stamp = None;
                slot.failure = None;
            }
        }
    }

    /// Forgets `key`, dropping a fetch in flight.
    pub fn remove(&mut self, key: &K) {
        if self.slots.remove(key).is_some()
            && let Some(save) = &self.persist
        {
            save(key, None);
        }
    }

    /// Keeps only the keys `keep` holds.
    pub fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        let gone: Vec<K> = self.slots.keys().filter(|k| !keep(k)).cloned().collect();
        for key in gone {
            self.remove(&key);
        }
    }

    // ----- the disk -----

    /// A value saved at an earlier launch: shown, but fetched again on the
    /// next request. Never replaces a value fetched or written this run.
    /// True if it is shown.
    pub fn hydrate(&mut self, key: K, value: V) -> bool {
        let slot = self.slots.entry(key).or_default();
        if slot.value.is_some() {
            return false;
        }
        slot.value = Some(Arc::new(value));
        slot.from_disk = true;
        // A first fetch that failed meanwhile is now said beside the value.
        if let Some(failure) = &mut slot.failure
            && failure.kind == FailureKind::First
        {
            failure.kind = FailureKind::Revalidate;
        } else if slot.flight.is_none() {
            slot.stamp = None;
        }
        true
    }

    /// Forgets the values that came from the disk and nothing replaced: the
    /// cache turned out to be another account's.
    pub fn forget_from_disk(&mut self) {
        self.slots.retain(|_, slot| !slot.from_disk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext as _, TestAppContext};
    use hardcover_core::Error;

    fn slot(value: Option<i32>, stamp: Option<Stamp>) -> Slot<i32> {
        Slot {
            value: value.map(Arc::new),
            stamp,
            ..Slot::default()
        }
    }

    #[test]
    fn what_each_demand_fetches() {
        let now = SystemTime::now();
        let fresh = Some(Stamp::at(now, 1));
        let old = Some(Stamp::at(now - STALE_AFTER, 1));
        let p = |s: &Slot<i32>, d| plan(Some(s), d, 1, now);

        assert_eq!(plan::<i32>(None, Demand::Active, 1, now), Plan::Start);
        assert_eq!(p(&slot(Some(1), fresh), Demand::Active), Plan::Skip);
        assert_eq!(p(&slot(Some(1), fresh), Demand::Shown), Plan::Skip);
        assert_eq!(p(&slot(Some(1), fresh), Demand::Now), Plan::Start);
        assert_eq!(p(&slot(Some(1), old), Demand::Active), Plan::Start);
        // From the disk, or invalidated: fetched again.
        assert_eq!(p(&slot(Some(1), None), Demand::Active), Plan::Start);
        // A Refresh since.
        assert_eq!(
            plan(Some(&slot(Some(1), fresh)), Demand::Active, 2, now),
            Plan::Start
        );

        let mut failed = slot(None, fresh);
        failed.failure = Some(Failure {
            message: "offline".into(),
            kind: FailureKind::First,
        });
        assert_eq!(p(&failed, Demand::Active), Plan::Skip, "not on activation");
        assert_eq!(p(&failed, Demand::Shown), Plan::Start, "shown again");
        // Failing to fetch more doesn't make the value due.
        failed.value = Some(Arc::new(1));
        failed.failure.as_mut().unwrap().kind = FailureKind::More;
        assert_eq!(p(&failed, Demand::Shown), Plan::Skip);
    }

    /// The owner of a test resource, which counts what landed.
    struct Owner {
        numbers: Resource<u32, i32>,
        landings: Vec<(u32, Landing)>,
    }

    fn bind() -> Binding<Owner, u32, i32> {
        Binding {
            get: |o| &mut o.numbers,
            landed: |o, k, l, _| o.landings.push((*k, l)),
        }
    }

    fn owner(cx: &mut TestAppContext) -> gpui_kit::Entity<Owner> {
        let session = SessionScope::for_tests(hardcover_core::demo::client(), None);
        cx.new(|_| Owner {
            numbers: Resource::new(session, "read:catalog", Priority::Background),
            landings: Vec::new(),
        })
    }

    fn ask(
        owner: &gpui_kit::Entity<Owner>,
        demand: Demand,
        result: hardcover_core::Result<i32>,
        cx: &mut TestAppContext,
    ) -> bool {
        owner.update(cx, |o, cx| {
            o.numbers.request(1, demand, move |_| result, bind(), cx)
        })
    }

    #[gpui_kit::test]
    fn a_key_in_flight_is_asked_for_once(cx: &mut TestAppContext) {
        let owner = owner(cx);
        assert!(ask(&owner, Demand::Shown, Ok(1), cx));
        assert!(!ask(&owner, Demand::Shown, Ok(2), cx), "in flight");
        cx.run_until_parked();
        owner.read_with(cx, |o, _| {
            assert_eq!(o.numbers.value(&1).map(|v| **v), Some(1));
            assert_eq!(o.numbers.started(), 1);
            assert_eq!(o.landings, [(1, Landing::Changed)]);
        });
        assert!(!ask(&owner, Demand::Shown, Ok(1), cx), "fresh");
    }

    /// The refresh-skipped-while-in-flight bug: a request that started
    /// before Refresh doesn't make the value count as fresh.
    #[gpui_kit::test]
    fn refresh_while_in_flight_fetches_again(cx: &mut TestAppContext) {
        let owner = owner(cx);
        ask(&owner, Demand::Shown, Ok(1), cx);
        cx.update(refresh_all);
        assert!(ask(&owner, Demand::Shown, Ok(2), cx), "started again");
        cx.run_until_parked();
        owner.read_with(cx, |o, _| {
            assert_eq!(o.numbers.value(&1).map(|v| **v), Some(2));
            // The first request's answer was dropped with it.
            assert_eq!(o.landings, [(1, Landing::Changed)]);
        });
        assert!(
            !ask(&owner, Demand::Shown, Ok(3), cx),
            "fresh after Refresh"
        );

        // Landed first, then Refresh: stale.
        cx.update(refresh_all);
        assert!(ask(&owner, Demand::Active, Ok(3), cx));
    }

    #[gpui_kit::test]
    fn a_result_that_predates_a_write_is_dropped(cx: &mut TestAppContext) {
        let owner = owner(cx);
        ask(&owner, Demand::Shown, Ok(1), cx);
        owner.update(cx, |o, cx| o.numbers.set(1, 5, cx));
        cx.run_until_parked();
        owner.read_with(cx, |o, _| {
            assert_eq!(o.numbers.value(&1).map(|v| **v), Some(5));
            assert_eq!(o.landings, [(1, Landing::Dropped)]);
        });
        // The write's value counts as fresh: no read right behind it.
        assert!(!ask(&owner, Demand::Active, Ok(1), cx));
    }

    #[gpui_kit::test]
    fn a_failure_keeps_the_value_and_waits_to_be_shown(cx: &mut TestAppContext) {
        let owner = owner(cx);
        ask(&owner, Demand::Shown, Err(Error::Timeout), cx);
        cx.run_until_parked();
        owner.read_with(cx, |o, _| {
            assert!(o.numbers.error(&1).is_some());
            assert!(matches!(o.numbers.status(&1), LoadStatus::Failed(_)));
        });
        assert!(!ask(&owner, Demand::Active, Ok(1), cx), "activation");
        assert!(ask(&owner, Demand::Shown, Ok(1), cx), "shown again");
        cx.run_until_parked();

        cx.update(refresh_all);
        ask(&owner, Demand::Shown, Err(Error::Timeout), cx);
        cx.run_until_parked();
        owner.read_with(cx, |o, _| {
            assert_eq!(o.numbers.value(&1).map(|v| **v), Some(1), "kept");
            assert_eq!(o.numbers.error(&1), None, "said quietly");
            assert!(o.numbers.failure(&1).is_some());
        });
    }

    #[gpui_kit::test]
    fn an_equal_answer_only_renews_the_stamp(cx: &mut TestAppContext) {
        let owner = owner(cx);
        ask(&owner, Demand::Shown, Ok(1), cx);
        cx.run_until_parked();
        cx.update(refresh_all);
        ask(&owner, Demand::Shown, Ok(1), cx);
        cx.run_until_parked();
        owner.read_with(cx, |o, _| {
            assert_eq!(o.landings, [(1, Landing::Changed), (1, Landing::Same)]);
        });
    }

    #[gpui_kit::test]
    fn a_value_from_the_disk_shows_and_is_fetched_again(cx: &mut TestAppContext) {
        let owner = owner(cx);
        owner.update(cx, |o, _| o.numbers.hydrate(1, 7));
        assert!(ask(&owner, Demand::Active, Err(Error::Timeout), cx));
        cx.run_until_parked();
        owner.update(cx, |o, _| {
            assert_eq!(o.numbers.value(&1).map(|v| **v), Some(7), "offline: kept");
            // Never over a value of this run.
            o.numbers.hydrate(1, 9);
            assert_eq!(o.numbers.value(&1).map(|v| **v), Some(7));
            o.numbers.forget_from_disk();
            assert_eq!(o.numbers.value(&1), None);
        });
    }

    #[gpui_kit::test]
    fn nothing_goes_out_once_the_session_ended(cx: &mut TestAppContext) {
        let owner = owner(cx);
        owner.read_with(cx, |o, _| o.numbers.session().end());
        assert!(!ask(&owner, Demand::Now, Ok(1), cx));
    }

    #[gpui_kit::test]
    fn a_batch_lands_per_key(cx: &mut TestAppContext) {
        let owner = owner(cx);
        let asked = owner.update(cx, |o, cx| {
            o.numbers.request_many(
                [1, 2, 3],
                Demand::Shown,
                |_, keys| Batch {
                    found: keys
                        .iter()
                        .filter(|k| **k != 2)
                        .map(|k| (*k, *k as i32))
                        .collect(),
                    failed: vec![2],
                    error: Some(Error::Timeout),
                },
                bind(),
                cx,
            )
        });
        assert_eq!(asked, 3);
        cx.run_until_parked();
        owner.read_with(cx, |o, _| {
            assert_eq!(o.numbers.value(&3).map(|v| **v), Some(3));
            assert!(o.numbers.error(&2).is_some());
            assert!(!o.numbers.is_loading(&2));
        });
        // Only what isn't fresh goes out again.
        let asked = owner.update(cx, |o, cx| {
            o.numbers.request_many(
                [1, 2, 3, 4],
                Demand::Shown,
                |_, keys| Batch {
                    found: keys.iter().map(|k| (*k, 0)).collect(),
                    failed: Vec::new(),
                    error: None,
                },
                bind(),
                cx,
            )
        });
        assert_eq!(asked, 2, "the failed one and the new one");
    }
}
