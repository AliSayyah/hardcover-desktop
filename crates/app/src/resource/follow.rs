//! How models that depend on the library follow it: what changed, past a
//! running sync's pages ([`LibraryFollow`]), and one reload after a run of
//! changes ([`Debounce`]).

use std::time::Duration;

use gpui_kit::{Context, Task};

use crate::library::model::{LibraryModel, SyncStatus};

/// Runs a method on its owner once calls stop for a while: the last call
/// wins, and dropping it cancels the wait.
#[derive(Default)]
pub struct Debounce {
    task: Option<Task<()>>,
}

impl Debounce {
    /// Calls `run` on the owner `delay` after the last call.
    pub fn schedule<T: 'static>(
        &mut self,
        delay: Duration,
        run: fn(&mut T, &mut Context<T>),
        cx: &mut Context<T>,
    ) {
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, run);
        }));
    }
}

/// What [`LibraryFollow::check`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// The library is loaded (cached or synced) for the first time: the
    /// key's starting point.
    First,
    /// The key changed since it was last seen.
    Changed,
}

/// Follows one key worked out from the library (a revision, the series it
/// has): seen first once the library is loaded, then reported when it
/// changes, but not while a sync runs (its pages are not news; its result
/// is).
pub struct LibraryFollow<K> {
    seen: Option<K>,
}

impl<K> Default for LibraryFollow<K> {
    fn default() -> Self {
        Self { seen: None }
    }
}

impl<K: PartialEq> LibraryFollow<K> {
    /// Checks `library` against the key last seen.
    pub fn check(
        &mut self,
        library: &LibraryModel,
        key: impl FnOnce(&LibraryModel) -> K,
    ) -> Option<Followed> {
        if !library.is_loaded() {
            return None;
        }
        if self.seen.is_none() {
            self.seen = Some(key(library));
            return Some(Followed::First);
        }
        if *library.sync_status() == SyncStatus::Syncing {
            return None;
        }
        let key = key(library);
        if self.seen.as_ref() == Some(&key) {
            return None;
        }
        self.seen = Some(key);
        Some(Followed::Changed)
    }

    /// A follow that has already seen `key` (tests).
    #[cfg(test)]
    pub fn seen_as(key: K) -> Self {
        Self { seen: Some(key) }
    }

    /// The key as last seen.
    pub fn seen(&self) -> Option<&K> {
        self.seen.as_ref()
    }
}
