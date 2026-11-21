//! How long fetched data stays current. A value is stale once it is older
//! than [`STALE_AFTER`], or was asked for before the last Refresh
//! ([`refresh_all`]). Its age counts from when its request *started*, so a
//! request that was in flight when Refresh was pressed doesn't count as
//! fresh when it lands.

use std::time::{Duration, SystemTime};

use gpui_kit::{App, Global};

/// How long a value is shown before it is fetched again.
pub const STALE_AFTER: Duration = Duration::from_mins(30);

/// Bumped by Refresh: everything asked for before it is stale.
#[derive(Default)]
struct RefreshGeneration(u64);

impl Global for RefreshGeneration {}

pub(super) fn generation(cx: &App) -> u64 {
    cx.try_global::<RefreshGeneration>().map_or(0, |g| g.0)
}

/// Makes every value fetched so far stale (Refresh).
pub fn refresh_all(cx: &mut App) {
    let next = generation(cx) + 1;
    cx.set_global(RefreshGeneration(next));
}

/// When a request started, and in which Refresh generation. Wall-clock
/// time, not `Instant`: on Linux that stops while the computer sleeps, so a
/// laptop opened in the morning would find everything fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    at: SystemTime,
    generation: u64,
}

impl Stamp {
    pub fn now(cx: &App) -> Self {
        Self {
            at: SystemTime::now(),
            generation: generation(cx),
        }
    }

    #[cfg(test)]
    pub(super) fn at(at: SystemTime, generation: u64) -> Self {
        Self { at, generation }
    }

    pub fn is_stale(self, cx: &App) -> bool {
        self.is_stale_at(SystemTime::now(), generation(cx))
    }

    /// Asked for before the Refresh generation `generation`.
    pub(super) fn predates(self, generation: u64) -> bool {
        self.generation != generation
    }

    /// A clock set back since the stamp counts as stale: how old the value
    /// is can't be known.
    pub(super) fn is_stale_at(self, now: SystemTime, generation: u64) -> bool {
        self.predates(generation)
            || now
                .duration_since(self.at)
                .is_ok_and(|age| age >= STALE_AFTER)
            || now < self.at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_goes_stale_with_age_or_a_refresh() {
        let at = SystemTime::now();
        let stamp = Stamp { at, generation: 3 };
        assert!(!stamp.is_stale_at(at, 3));
        assert!(!stamp.is_stale_at(at + STALE_AFTER - Duration::from_secs(1), 3));
        assert!(stamp.is_stale_at(at + STALE_AFTER, 3));
        assert!(stamp.is_stale_at(at, 4), "a Refresh since");
        assert!(
            stamp.is_stale_at(at - Duration::from_secs(1), 3),
            "the clock was set back"
        );
    }
}
