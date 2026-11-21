//! When each cover was last drawn, at each size: what orders the download
//! and decode queues (the cover drawn most recently goes first), and what
//! picks the loaded covers to release once there are more than a budget.
//!
//! Views stamp a cover each time they draw it (`cover_pipeline::draw`).
//! A view that sits still doesn't draw again, so an old stamp doesn't mean
//! a cover is off screen: covers are released by rank, past a budget far
//! above a screenful, never by age.

use std::collections::HashMap;

/// A size a cover is loaded at, one GPUI asset each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// 144 px: xs and sm covers, md avatars.
    Thumbnail,
    /// 288 px: md and lg covers, the xl avatar.
    Medium,
    /// 480 px: the book pane's cover, released by the pane itself.
    Detail,
    /// The file, undecoded: an SVG avatar.
    File,
}

impl Kind {
    const ALL: [Self; 4] = [Self::Thumbnail, Self::Medium, Self::Detail, Self::File];

    fn index(self) -> usize {
        self as usize
    }
}

/// How many covers of a kind may stay loaded once they are no longer drawn,
/// and how far past that before any is released.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budget {
    pub keep: usize,
    pub slack: usize,
}

/// 600 thumbnails (about 55 KB each, in memory and again on the GPU) and
/// 150 medium covers (about 220 KB each): some 33 MB of each, in memory and
/// on the GPU, many screenfuls.
pub(crate) const THUMBNAIL_BUDGET: Budget = Budget {
    keep: 600,
    slack: 32,
};
pub(crate) const MEDIUM_BUDGET: Budget = Budget {
    keep: 150,
    slack: 16,
};

/// See the module doc.
#[derive(Debug)]
pub(crate) struct Usage {
    /// Per URL, when it was last drawn at each [`Kind`] (by [`Kind::index`]);
    /// 0 for never, or released since.
    drawn: HashMap<String, [u64; 4]>,
    /// How many URLs are drawn at each kind.
    counts: [usize; 4],
    /// Increases with every stamp.
    clock: u64,
    thumbnails: Budget,
    medium: Budget,
}

impl Default for Usage {
    fn default() -> Self {
        Self::new(THUMBNAIL_BUDGET, MEDIUM_BUDGET)
    }
}

impl Usage {
    pub(crate) fn new(thumbnails: Budget, medium: Budget) -> Self {
        Self {
            drawn: HashMap::new(),
            counts: [0; 4],
            clock: 0,
            thumbnails,
            medium,
        }
    }

    fn budget(&self, kind: Kind) -> Option<Budget> {
        match kind {
            Kind::Thumbnail => Some(self.thumbnails),
            Kind::Medium => Some(self.medium),
            Kind::Detail | Kind::File => None,
        }
    }

    /// Records that `url` was drawn at `kind` just now. True when that
    /// takes the kind past its budget, so some should be released.
    pub(crate) fn stamp(&mut self, url: &str, kind: Kind) -> bool {
        self.clock += 1;
        let clock = self.clock;
        let slot = if let Some(stamps) = self.drawn.get_mut(url) {
            &mut stamps[kind.index()]
        } else {
            &mut self.drawn.entry(url.to_owned()).or_default()[kind.index()]
        };
        let new = *slot == 0;
        *slot = clock;
        if new {
            self.counts[kind.index()] += 1;
        }
        new && self
            .budget(kind)
            .is_some_and(|b| self.counts[kind.index()] > b.keep + b.slack)
    }

    /// How much `url` is wanted: when it was last drawn at any size, 0 if
    /// never. Higher goes first.
    pub(crate) fn priority(&self, url: &str) -> u64 {
        self.drawn
            .get(url)
            .map_or(0, |stamps| stamps.iter().copied().max().unwrap_or(0))
    }

    /// Forgets that `url` is loaded at `kind` (it was released, or will be
    /// loaded again). True if it was drawn at `kind`.
    pub(crate) fn forget(&mut self, url: &str, kind: Kind) -> bool {
        let Some(stamps) = self.drawn.get_mut(url) else {
            return false;
        };
        let drawn = stamps[kind.index()] != 0;
        if drawn {
            stamps[kind.index()] = 0;
            self.counts[kind.index()] -= 1;
        }
        if stamps.iter().all(|&s| s == 0) {
            self.drawn.remove(url);
        }
        drawn
    }

    /// [`Self::forget`] at every kind.
    pub(crate) fn forget_all(&mut self, url: &str) {
        for kind in Kind::ALL {
            self.forget(url, kind);
        }
    }

    /// Past `kind`'s budget, the URLs drawn least recently at it, down to
    /// the budget's `keep`, forgotten here: the caller releases them.
    pub(crate) fn evict(&mut self, kind: Kind) -> Vec<String> {
        let Some(budget) = self.budget(kind) else {
            return Vec::new();
        };
        let count = self.counts[kind.index()];
        if count <= budget.keep {
            return Vec::new();
        }
        let mut drawn: Vec<(u64, &String)> = self
            .drawn
            .iter()
            .map(|(url, stamps)| (stamps[kind.index()], url))
            .filter(|(stamp, _)| *stamp != 0)
            .collect();
        drawn.sort_unstable();
        let evicted: Vec<String> = drawn
            .into_iter()
            .take(count - budget.keep)
            .map(|(_, url)| url.clone())
            .collect();
        for url in &evicted {
            self.forget(url, kind);
        }
        evicted
    }

    #[cfg(test)]
    pub(crate) fn count(&self, kind: Kind) -> usize {
        self.counts[kind.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> Usage {
        Usage::new(Budget { keep: 3, slack: 1 }, Budget { keep: 1, slack: 0 })
    }

    #[test]
    fn past_the_budget_the_covers_drawn_longest_ago_are_released() {
        let mut usage = small();
        for url in ["a", "b", "c", "d"] {
            assert!(!usage.stamp(url, Kind::Thumbnail), "within the slack");
        }
        // Drawn again: no longer the oldest.
        assert!(!usage.stamp("a", Kind::Thumbnail));
        assert!(usage.stamp("e", Kind::Thumbnail), "past keep + slack");
        assert_eq!(usage.evict(Kind::Thumbnail), ["b", "c"]);
        assert_eq!(usage.count(Kind::Thumbnail), 3);
        assert_eq!(usage.priority("b"), 0);
        assert!(usage.evict(Kind::Thumbnail).is_empty());
    }

    #[test]
    fn kinds_have_their_own_budgets_and_detail_covers_none() {
        let mut usage = small();
        usage.stamp("a", Kind::Medium);
        usage.stamp("a", Kind::Thumbnail);
        assert!(usage.stamp("b", Kind::Medium));
        assert_eq!(usage.evict(Kind::Medium), ["a"]);
        // Still loaded as a thumbnail.
        assert!(usage.priority("a") > 0);
        for i in 0..10 {
            assert!(!usage.stamp(&i.to_string(), Kind::Detail));
        }
        assert!(usage.evict(Kind::Detail).is_empty());
    }

    #[test]
    fn the_cover_drawn_last_goes_first() {
        let mut usage = small();
        usage.stamp("a", Kind::Thumbnail);
        usage.stamp("b", Kind::Detail);
        assert!(usage.priority("b") > usage.priority("a"));
        usage.stamp("a", Kind::Thumbnail);
        assert!(usage.priority("a") > usage.priority("b"));
        assert_eq!(usage.priority("never"), 0);
        usage.forget_all("a");
        assert_eq!(usage.priority("a"), 0);
        assert_eq!(usage.count(Kind::Thumbnail), 0);
    }
}
