//! How long catalogue data fetched during a run stays current: book details,
//! similar books, other readers' reviews, series and author pages, Discover.
//!
//! Each is kept per id for the run, as a [`Fetched`] value. One older than
//! [`STALE_AFTER`], or fetched before the last Refresh ([`refresh_all`]), is
//! stale: the next time it is asked for, it is fetched again while the old
//! value stays on screen. A refetch that fails keeps the old value and shows
//! nothing, and counts as a try, so being offline doesn't send a request on
//! every trigger; Refresh tries again.
//!
//! Views can't tell whether they are on screen, so the shell asks the visible
//! ones to revalidate: on Refresh, when the window becomes active, and when a
//! page or destination is shown again (`Workspace::revalidate_visible`).

use std::time::{Duration, Instant};

use gpui_kit::{App, Global};

/// How long a value fetched this run is shown before it is fetched again.
pub const STALE_AFTER: Duration = Duration::from_mins(30);

/// Bumped by Refresh: everything fetched before it is stale.
#[derive(Default)]
struct RefreshGeneration(u64);

impl Global for RefreshGeneration {}

fn generation(cx: &App) -> u64 {
    cx.try_global::<RefreshGeneration>().map_or(0, |g| g.0)
}

/// Makes every value fetched so far stale (Refresh).
pub fn refresh_all(cx: &mut App) {
    let next = generation(cx) + 1;
    cx.set_global(RefreshGeneration(next));
}

/// When a value was last fetched or tried, and in which Refresh generation.
#[derive(Debug, Clone, Copy)]
pub struct Stamp {
    at: Instant,
    generation: u64,
}

impl Stamp {
    pub fn now(cx: &App) -> Self {
        Self {
            at: Instant::now(),
            generation: generation(cx),
        }
    }

    pub fn is_stale(self, cx: &App) -> bool {
        self.is_stale_at(Instant::now(), generation(cx))
    }

    fn is_stale_at(self, now: Instant, generation: u64) -> bool {
        self.generation != generation || now.saturating_duration_since(self.at) >= STALE_AFTER
    }
}

/// A value fetched this run, with when it was last fetched or tried.
#[derive(Debug, Clone)]
pub struct Fetched<T> {
    value: T,
    stamp: Stamp,
}

impl<T: PartialEq> Fetched<T> {
    pub fn new(value: T, cx: &App) -> Self {
        Self {
            value,
            stamp: Stamp::now(cx),
        }
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn is_stale(&self, cx: &App) -> bool {
        self.stamp.is_stale(cx)
    }

    /// Applies a refetch: a different value replaces this one, an equal one
    /// or a failure only counts as a try. True if the value changed.
    pub fn refetched<E>(&mut self, result: Result<T, E>, cx: &App) -> bool {
        self.stamp = Stamp::now(cx);
        match result {
            Ok(value) if value != self.value => {
                self.value = value;
                true
            }
            Ok(_) | Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_goes_stale_with_age_or_a_refresh() {
        let at = Instant::now();
        let stamp = Stamp { at, generation: 3 };
        assert!(!stamp.is_stale_at(at, 3));
        assert!(!stamp.is_stale_at(at + STALE_AFTER - Duration::from_secs(1), 3));
        assert!(stamp.is_stale_at(at + STALE_AFTER, 3));
        assert!(stamp.is_stale_at(at, 4), "a Refresh since");
        // A clock read before the stamp (another thread's) is not negative age.
        assert!(!stamp.is_stale_at(at - Duration::from_secs(1), 3));
    }

    #[gpui_kit::test]
    fn a_refetch_replaces_only_a_different_value(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            let mut fetched = Fetched::new(1, cx);
            assert!(!fetched.is_stale(cx));
            refresh_all(cx);
            assert!(fetched.is_stale(cx));

            assert!(!fetched.refetched(Ok::<_, ()>(1), cx));
            assert!(!fetched.is_stale(cx), "an equal value counts as fresh");

            refresh_all(cx);
            assert!(!fetched.refetched(Err("offline"), cx));
            assert_eq!(*fetched.value(), 1, "a failure keeps the value");
            assert!(!fetched.is_stale(cx), "and counts as a try");

            refresh_all(cx);
            assert!(fetched.refetched(Ok::<_, ()>(2), cx));
            assert_eq!(*fetched.value(), 2);
        });
    }
}
