//! Rows leaving a list (`hardcover-design`, Motion): when a row leaves a
//! collection while the reader looks at it (the selected book moved to
//! another shelf, a book taken off a list), it stays in its place and fades
//! out over [`ROW_LEAVE`]; only then do the rows below close the gap, at once.
//! Without it the row is gone in one frame and the rows under the pointer
//! jump up, and nothing says which one went.
//!
//! Only a pure removal of a few rows ([`MAX_LEAVING`]) fades: the new rows
//! are the old ones with those taken out, in the same order. Anything else (another shelf, a new filter or
//! sort, rows added or moved, the first load) shows the new rows at once.
//! So does everything under reduced motion.
//!
//! A leaving row loses its focus treatment at once (`ListRow`'s ring is
//! drawn outside the row's opacity), and the selection moves on with the
//! rows when it goes.
//!
//! A delegate keeps a [`Leaving`] beside its rows, works its rows out through
//! [`Leaving::rows`], draws each row at [`Leaving::opacity`], and when
//! `rows` says a fade started, works its rows out again after [`ROW_LEAVE`]:
//!
//! ```ignore
//! let shown = self.leaving.rows(&self.rows, new_rows, animate, now);
//! self.rows = shown.rows;
//! if shown.started {
//!     cx.spawn(async move |this, cx| {
//!         cx.background_executor().timer(ROW_LEAVE).await;
//!         this.update(cx, |this, cx| this.refresh_rows(cx)).ok();
//!     })
//!     .detach();
//! }
//! // In render_item:
//! if let Some(opacity) = self.leaving.opacity(&id, now) {
//!     window.request_animation_frame();
//!     item = item.opacity(opacity);
//! }
//! ```

use std::time::Instant;

use gpui_kit::base::animation::ease_out_cubic;

pub use crate::design::motion::ROW_LEAVE;

/// The rows of a list that are fading out, each with when it started.
#[derive(Debug)]
pub struct Leaving<K> {
    rows: Vec<(K, Instant)>,
}

impl<K> Default for Leaving<K> {
    fn default() -> Self {
        Self { rows: Vec::new() }
    }
}

/// The most rows that fade out at once. More leaving together (a sync, a
/// list fetched again) is not something the reader did here: they go at once.
pub const MAX_LEAVING: usize = 3;

/// What [`Leaving::rows`] worked out.
#[derive(Debug, PartialEq)]
pub struct Shown<K> {
    /// The rows to show: the new ones, with the rows still fading out in
    /// their old places.
    pub rows: Vec<K>,
    /// A row started fading out: work the rows out again after
    /// [`ROW_LEAVE`], to let it go.
    pub started: bool,
}

impl<K: Clone + PartialEq> Leaving<K> {
    /// The rows to show, given the rows shown so far (`old`, leaving rows
    /// included) and the list's rows now (`new`). `animate` is false when
    /// the change isn't to the same rows (another shelf, filter or sort, a
    /// first load) or motion is reduced: then `new` shows at once.
    pub fn rows(&mut self, old: &[K], new: Vec<K>, animate: bool, now: Instant) -> Shown<K> {
        let previous = std::mem::take(&mut self.rows);
        let gone = match departures(old, &new) {
            Some(gone) if animate && !gone.is_empty() && gone.len() <= MAX_LEAVING => gone,
            _ => {
                return Shown {
                    rows: new,
                    started: false,
                };
            }
        };
        let gone_keys = gone.clone();
        let mut started = false;
        for key in gone {
            match previous.iter().find(|(k, _)| *k == key) {
                // Still fading: keep its start, so it doesn't begin again.
                Some((_, start)) if now.duration_since(*start) < ROW_LEAVE => {
                    self.rows.push((key, *start));
                }
                // Done fading: it goes now.
                Some(_) => {}
                None => {
                    self.rows.push((key, now));
                    started = true;
                }
            }
        }
        // `new` is `old` less `gone`: keep the rest, and the gone still fading.
        let rows = old
            .iter()
            .filter(|k| !gone_keys.contains(k) || self.is_leaving(k))
            .cloned()
            .collect();
        Shown { rows, started }
    }

    /// Whether `key` is fading out.
    pub fn is_leaving(&self, key: &K) -> bool {
        self.rows.iter().any(|(k, _)| k == key)
    }

    /// The opacity to draw `key` at, if it is fading out. While it is, the
    /// caller asks for another frame.
    pub fn opacity(&self, key: &K, now: Instant) -> Option<f32> {
        let (_, start) = self.rows.iter().find(|(k, _)| k == key)?;
        let progress = now.duration_since(*start).as_secs_f32() / ROW_LEAVE.as_secs_f32();
        Some(1. - ease_out_cubic(progress))
    }
}

/// The rows of `old` missing from `new`, if `new` is `old` with only those
/// taken out (the rest in the same order, nothing added); else `None`.
pub fn departures<K: PartialEq + Clone>(old: &[K], new: &[K]) -> Option<Vec<K>> {
    let mut gone = Vec::new();
    let mut next = new.iter().peekable();
    for key in old {
        if next.peek() == Some(&key) {
            next.next();
        } else {
            gone.push(key.clone());
        }
    }
    next.peek().is_none().then_some(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pure_removal_has_departures() {
        assert_eq!(departures(&[1, 2, 3, 4], &[1, 3, 4]), Some(vec![2]));
        assert_eq!(departures(&[1, 2, 3, 4], &[1, 4]), Some(vec![2, 3]));
        assert_eq!(departures(&[1, 2, 3], &[1, 2, 3]), Some(vec![]));
        assert_eq!(departures(&[1, 2, 3], &[]), Some(vec![1, 2, 3]));
        assert_eq!(departures(&[1, 2, 3], &[1, 2, 3, 4]), None, "a row added");
        assert_eq!(departures(&[1, 2, 3], &[2, 1, 3]), None, "rows moved");
        assert_eq!(departures(&[1, 2, 3], &[1, 5]), None, "one out, one in");
    }

    #[test]
    fn a_row_that_leaves_stays_in_its_place_while_it_fades() {
        let start = Instant::now();
        let mut leaving = Leaving::default();
        let shown = leaving.rows(&[1, 2, 3], vec![1, 3], true, start);
        assert_eq!(
            shown,
            Shown {
                rows: vec![1, 2, 3],
                started: true
            }
        );
        assert_eq!(leaving.opacity(&2, start), Some(1.));
        assert_eq!(leaving.opacity(&1, start), None);
        let half = leaving.opacity(&2, start + ROW_LEAVE / 2).unwrap();
        assert!(
            half > 0. && half < 0.5,
            "ease-out: past halfway at half time, {half}"
        );
        assert_eq!(leaving.opacity(&2, start + ROW_LEAVE), Some(0.));

        // Another change meanwhile keeps the fade where it is.
        let later = start + ROW_LEAVE / 3;
        let shown = leaving.rows(&[1, 2, 3], vec![1, 3], true, later);
        assert_eq!(
            shown,
            Shown {
                rows: vec![1, 2, 3],
                started: false
            }
        );
        let opacity = leaving.opacity(&2, later).unwrap();
        assert!(opacity > 0. && opacity < 1., "{opacity}");

        // Once it has faded, it goes.
        let done = start + ROW_LEAVE;
        let shown = leaving.rows(&[1, 2, 3], vec![1, 3], true, done);
        assert_eq!(
            shown,
            Shown {
                rows: vec![1, 3],
                started: false
            }
        );
        assert!(!leaving.is_leaving(&2));
    }

    #[test]
    fn anything_but_a_removal_shows_at_once() {
        let now = Instant::now();
        let mut leaving = Leaving::default();
        leaving.rows(&[1, 2, 3], vec![1, 3], true, now);
        let shown = leaving.rows(&[1, 2, 3], vec![3, 1], true, now);
        assert_eq!(
            shown,
            Shown {
                rows: vec![3, 1],
                started: false
            }
        );
        assert!(!leaving.is_leaving(&2), "a reorder ends the fade");

        let shown = leaving.rows(&[1, 2, 3], vec![1, 3], false, now);
        assert_eq!(
            shown,
            Shown {
                rows: vec![1, 3],
                started: false
            },
            "not animating"
        );

        let shown = leaving.rows(&[1, 2, 3, 4, 5], vec![1], true, now);
        assert_eq!(shown.rows, vec![1], "too many at once");
        assert!(!shown.started);
    }

    #[test]
    fn a_second_row_leaving_fades_on_its_own_clock() {
        let start = Instant::now();
        let mut leaving = Leaving::default();
        leaving.rows(&[1, 2, 3], vec![1, 3], true, start);
        let later = start + ROW_LEAVE / 2;
        let shown = leaving.rows(&[1, 2, 3], vec![1], true, later);
        assert_eq!(
            shown,
            Shown {
                rows: vec![1, 2, 3],
                started: true
            }
        );
        assert!(leaving.opacity(&2, later).unwrap() < leaving.opacity(&3, later).unwrap());
    }
}
