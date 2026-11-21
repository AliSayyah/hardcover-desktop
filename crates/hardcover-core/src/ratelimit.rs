//! Client-side rate limiting: a token bucket mirroring Hardcover's limits
//! (burst 10, 60 a minute), the server's 429 blocks, and two priorities.
//!
//! We run slightly under the server's burst (8) so other clients of the same
//! account (the limit is per user, not per token: the website, a phone) leave
//! us some headroom.
//!
//! Waiting callers queue in two FIFO lanes. An [`Priority::Interactive`]
//! caller (a write, a search) is admitted before any
//! [`Priority::Background`] one (a sync page, a series batch, a reload)
//! that is still waiting.
//!
//! A 429 is a block, not a debt: no request goes out before
//! `max(blocked until, now + Retry-After)`, however many 429s arrive, and
//! callers already waiting wake up and respect a block that ends later than
//! their slot. A block longer than [`LONG_BLOCK`] (the daily limit) is
//! remembered, and every call fails at once with [`Error::RateLimited`]
//! until it ends, rather than each one sending its own request to be refused.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::Error;

pub(crate) const DEFAULT_BURST: u32 = 8;
pub(crate) const DEFAULT_REFILL_PER_SEC: f64 = 1.0;

/// A block the server asks for that is longer than this is not waited out:
/// calls fail until it ends. Hardcover's per-minute window asks for at most
/// a minute; its daily limit asks for hours.
pub(crate) const LONG_BLOCK: Duration = Duration::from_secs(60);

/// Which callers go first when requests have to wait for the rate limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Priority {
    /// Something the user is waiting on: a write, a search.
    Interactive,
    /// Everything else: syncs, series batches, reloads, prefetches.
    #[default]
    Background,
}

impl Priority {
    fn lane(self) -> usize {
        match self {
            Priority::Interactive => 0,
            Priority::Background => 1,
        }
    }
}

#[derive(Debug)]
pub(crate) struct TokenBucket {
    capacity: f64,
    refill_per_sec: f64,
    state: Mutex<State>,
    /// Signalled whenever a token is taken, a waiter leaves or a block changes.
    changed: Condvar,
}

#[derive(Debug)]
struct State {
    /// Never negative: a caller takes a token only when one is there.
    tokens: f64,
    last: Instant,
    /// No request goes out before this (a 429's Retry-After).
    blocked_until: Option<Instant>,
    /// A block too long to wait out: calls fail until it ends.
    refused_until: Option<Instant>,
    next_ticket: u64,
    /// Waiting callers' tickets, per [`Priority::lane`], in arrival order.
    lanes: [VecDeque<u64>; 2],
}

/// What a waiting caller does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admit {
    /// A token was taken: send now.
    Go,
    /// Ask again after this long, or sooner if woken.
    Wait(Duration),
    /// Fail with [`Error::RateLimited`]: the server's long block ends in this long.
    Refused(Duration),
}

/// A caller's place in a lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket {
    id: u64,
    priority: Priority,
}

impl TokenBucket {
    pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
        Self::new_at(capacity, refill_per_sec, Instant::now())
    }

    fn new_at(capacity: u32, refill_per_sec: f64, now: Instant) -> Self {
        assert!(capacity > 0 && refill_per_sec > 0.0);
        Self {
            capacity: f64::from(capacity),
            refill_per_sec,
            state: Mutex::new(State {
                tokens: f64::from(capacity),
                last: now,
                blocked_until: None,
                refused_until: None,
                next_ticket: 0,
                lanes: [VecDeque::new(), VecDeque::new()],
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn refill(&self, s: &mut State, now: Instant) {
        let elapsed = now.saturating_duration_since(s.last).as_secs_f64();
        s.tokens = (s.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        s.last = now;
    }

    /// Joins the back of `priority`'s lane.
    pub(crate) fn enqueue(&self, priority: Priority) -> Ticket {
        let mut s = self.lock();
        let id = s.next_ticket;
        s.next_ticket += 1;
        s.lanes[priority.lane()].push_back(id);
        Ticket { id, priority }
    }

    /// Leaves the lane without a token (refused, gave up, or panicked).
    fn leave(&self, ticket: Ticket) {
        let mut s = self.lock();
        s.lanes[ticket.priority.lane()].retain(|t| *t != ticket.id);
        drop(s);
        self.changed.notify_all();
    }

    /// The decision for `ticket` at `now`; takes the token on [`Admit::Go`],
    /// which also takes the ticket out of its lane. Pure but for the state.
    pub(crate) fn admit_at(&self, ticket: Ticket, now: Instant) -> Admit {
        let mut s = self.lock();
        let admit = self.decide(&mut s, ticket, now);
        if admit == Admit::Go {
            s.tokens -= 1.0;
            s.lanes[ticket.priority.lane()].retain(|t| *t != ticket.id);
            drop(s);
            self.changed.notify_all();
        }
        admit
    }

    fn decide(&self, s: &mut State, ticket: Ticket, now: Instant) -> Admit {
        self.refill(s, now);
        match s.refused_until {
            Some(until) if until > now => return Admit::Refused(until - now),
            Some(_) => s.refused_until = None,
            None => {}
        }
        if let Some(until) = s.blocked_until {
            if until > now {
                return Admit::Wait(until - now);
            }
            s.blocked_until = None;
        }
        let one_token = Duration::from_secs_f64(1.0 / self.refill_per_sec);
        let ahead = match ticket.priority {
            // Interactive callers only queue behind each other.
            Priority::Interactive => s.lanes[0].front() != Some(&ticket.id),
            Priority::Background => {
                !s.lanes[0].is_empty() || s.lanes[1].front() != Some(&ticket.id)
            }
        };
        if ahead {
            // Someone goes first; they wake us when they take their token.
            return Admit::Wait(one_token);
        }
        if s.tokens >= 1.0 {
            Admit::Go
        } else {
            Admit::Wait(Duration::from_secs_f64(
                (1.0 - s.tokens) / self.refill_per_sec,
            ))
        }
    }

    /// Blocks until a token is free for a caller of `priority`, then takes it.
    ///
    /// # Errors
    ///
    /// [`Error::RateLimited`] while a long block is remembered, or if the
    /// token can't be had by `deadline` (then `retry_after` is how long the
    /// known block still runs, if one does).
    pub fn acquire(&self, priority: Priority, deadline: Option<Instant>) -> Result<(), Error> {
        let ticket = self.enqueue(priority);
        // Leaves the lane however this returns, so nobody waits behind a ghost.
        struct Leave<'a>(&'a TokenBucket, Ticket, bool);
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                if !self.2 {
                    self.0.leave(self.1);
                }
            }
        }
        let mut guard = Leave(self, ticket, false);
        loop {
            let now = Instant::now();
            let wait = match self.admit_at(ticket, now) {
                Admit::Go => {
                    guard.2 = true;
                    return Ok(());
                }
                Admit::Refused(left) => {
                    return Err(Error::RateLimited {
                        retry_after: Some(left),
                    });
                }
                Admit::Wait(wait) => wait,
            };
            let wait = match deadline {
                Some(deadline) if now + wait > deadline => {
                    // The known wait ends past the deadline: give up now,
                    // unless only queueing (which may move faster) is left.
                    let blocked = self.lock().blocked_until.filter(|b| *b > deadline);
                    if blocked.is_some() || now >= deadline {
                        return Err(Error::RateLimited {
                            retry_after: blocked.map(|b| b.saturating_duration_since(now)),
                        });
                    }
                    deadline - now
                }
                _ => wait,
            };
            let s = self.lock();
            // Woken early by a token taken, a waiter leaving or a new block.
            let _ = self
                .changed
                .wait_timeout(s, wait)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The server said 429. Empties the bucket and blocks every request
    /// until `retry_after` from now (one token's time without a header),
    /// never shortening a block already set. A block longer than
    /// [`LONG_BLOCK`] is remembered and refuses calls until it ends.
    pub fn penalize_at(&self, retry_after: Option<Duration>, now: Instant) {
        let mut s = self.lock();
        self.refill(&mut s, now);
        s.tokens = 0.0;
        let wait =
            retry_after.unwrap_or_else(|| Duration::from_secs_f64(1.0 / self.refill_per_sec));
        let until = now + wait;
        let slot = if wait > LONG_BLOCK {
            &mut s.refused_until
        } else {
            &mut s.blocked_until
        };
        *slot = Some(slot.map_or(until, |old| old.max(until)));
        drop(s);
        self.changed.notify_all();
    }

    pub fn penalize(&self, retry_after: Option<Duration>) {
        self.penalize_at(retry_after, Instant::now());
    }
}

impl Default for TokenBucket {
    fn default() -> Self {
        Self::new(DEFAULT_BURST, DEFAULT_REFILL_PER_SEC)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    /// Takes a token at `now` if one is free; how long to wait otherwise.
    fn take(b: &TokenBucket, p: Priority, now: Instant) -> Admit {
        let t = b.enqueue(p);
        let admit = b.admit_at(t, now);
        if admit != Admit::Go {
            b.leave(t);
        }
        admit
    }

    #[test]
    fn burst_then_one_per_second() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(8, 1.0, t0);
        for _ in 0..8 {
            assert_eq!(take(&b, Priority::Background, t0), Admit::Go);
        }
        assert_eq!(take(&b, Priority::Background, t0), Admit::Wait(secs(1.0)));
        let t = t0 + secs(0.25);
        assert_eq!(take(&b, Priority::Background, t), Admit::Wait(secs(0.75)));
        assert_eq!(take(&b, Priority::Background, t0 + secs(1.0)), Admit::Go);
    }

    #[test]
    fn refills_over_time_up_to_capacity() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(8, 1.0, t0);
        for _ in 0..8 {
            take(&b, Priority::Background, t0);
        }
        // 3 seconds later 3 tokens are back.
        let t3 = t0 + secs(3.0);
        for _ in 0..3 {
            assert_eq!(take(&b, Priority::Background, t3), Admit::Go);
        }
        assert!(matches!(take(&b, Priority::Background, t3), Admit::Wait(_)));
        // A long idle period never exceeds the capacity.
        let b = TokenBucket::new_at(8, 1.0, t0);
        let later = t0 + secs(3600.0);
        for _ in 0..8 {
            assert_eq!(take(&b, Priority::Background, later), Admit::Go);
        }
        assert_eq!(
            take(&b, Priority::Background, later),
            Admit::Wait(secs(1.0))
        );
    }

    #[test]
    fn penalize_blocks_for_retry_after() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(8, 1.0, t0);
        b.penalize_at(Some(secs(5.0)), t0);
        assert_eq!(take(&b, Priority::Background, t0), Admit::Wait(secs(5.0)));
        // After the block the bucket refilled for those 5 seconds.
        assert_eq!(take(&b, Priority::Background, t0 + secs(5.0)), Admit::Go);
        // No header: one token's time.
        let b = TokenBucket::new_at(8, 1.0, t0);
        b.penalize_at(None, t0);
        assert_eq!(take(&b, Priority::Background, t0), Admit::Wait(secs(1.0)));
    }

    /// The review's probe: four queued callers each collecting a 429 (10, 9,
    /// 8, 7 and 6 s) made the next request wait 41 s. Blocks don't add up:
    /// the next request goes out when the longest one ends.
    #[test]
    fn penalties_do_not_stack() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(8, 1.0, t0);
        for (i, wait) in [10.0, 9.0, 8.0, 7.0, 6.0].into_iter().enumerate() {
            b.penalize_at(Some(secs(wait)), t0 + secs(i as f64 * 0.1));
        }
        assert_eq!(take(&b, Priority::Background, t0), Admit::Wait(secs(10.0)));
        assert_eq!(take(&b, Priority::Background, t0 + secs(10.0)), Admit::Go);
        // A later block that ends sooner doesn't shorten it either.
        let b = TokenBucket::new_at(8, 1.0, t0);
        b.penalize_at(Some(secs(10.0)), t0);
        b.penalize_at(Some(secs(1.0)), t0 + secs(2.0));
        assert_eq!(
            take(&b, Priority::Background, t0 + secs(2.0)),
            Admit::Wait(secs(8.0))
        );
    }

    #[test]
    fn a_long_block_refuses_calls_until_it_ends() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(8, 1.0, t0);
        b.penalize_at(Some(Duration::from_hours(3)), t0);
        assert_eq!(
            take(&b, Priority::Interactive, t0 + secs(60.0)),
            Admit::Refused(Duration::from_hours(3) - secs(60.0))
        );
        assert_eq!(
            take(&b, Priority::Background, t0 + Duration::from_hours(3)),
            Admit::Go
        );
    }

    #[test]
    fn interactive_callers_go_before_waiting_background_ones() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(1, 1.0, t0);
        assert_eq!(take(&b, Priority::Background, t0), Admit::Go);
        let sync = b.enqueue(Priority::Background);
        let later_sync = b.enqueue(Priority::Background);
        assert_eq!(b.admit_at(sync, t0), Admit::Wait(secs(1.0)));
        let write = b.enqueue(Priority::Interactive);
        let t1 = t0 + secs(1.0);
        // The token that comes back goes to the write, though it came last.
        assert!(matches!(b.admit_at(sync, t1), Admit::Wait(_)));
        assert_eq!(b.admit_at(write, t1), Admit::Go);
        // Then the background lane, in order.
        let t2 = t0 + secs(2.0);
        assert!(matches!(b.admit_at(later_sync, t2), Admit::Wait(_)));
        assert_eq!(b.admit_at(sync, t2), Admit::Go);
        assert_eq!(b.admit_at(later_sync, t0 + secs(3.0)), Admit::Go);
    }

    #[test]
    fn acquire_blocks_until_a_token_is_free() {
        let b = TokenBucket::new(1, 20.0);
        let start = Instant::now();
        b.acquire(Priority::Background, None).unwrap();
        b.acquire(Priority::Background, None).unwrap(); // must wait ~50ms
        assert!(start.elapsed() >= Duration::from_millis(45));
    }

    #[test]
    fn waiting_callers_respect_a_block_set_after_they_queued() {
        let b = Arc::new(TokenBucket::new(1, 10.0));
        b.acquire(Priority::Background, None).unwrap();
        let start = Instant::now();
        let waiter = {
            let b = b.clone();
            // Its slot is ~100 ms away when it starts waiting.
            std::thread::spawn(move || b.acquire(Priority::Background, None))
        };
        std::thread::sleep(Duration::from_millis(20));
        b.penalize(Some(Duration::from_millis(300)));
        waiter.join().unwrap().unwrap();
        assert!(start.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn acquire_gives_up_at_the_deadline() {
        let b = TokenBucket::new(1, 1.0);
        b.penalize(Some(Duration::from_secs(10)));
        let start = Instant::now();
        let deadline = start + Duration::from_millis(50);
        let Err(Error::RateLimited { retry_after }) =
            b.acquire(Priority::Interactive, Some(deadline))
        else {
            panic!("expected RateLimited");
        };
        // The block is known to outlast the deadline: no waiting at all.
        assert!(start.elapsed() < Duration::from_millis(40));
        assert!(retry_after.is_some_and(|r| r > Duration::from_secs(9)));
        // A refused caller left its lane: the next one isn't stuck behind it.
        let s = b.lock();
        assert!(s.lanes.iter().all(VecDeque::is_empty));
    }

    #[test]
    fn a_remembered_long_block_fails_fast() {
        let b = TokenBucket::new(8, 1.0);
        b.penalize(Some(Duration::from_hours(5)));
        let start = Instant::now();
        assert!(matches!(
            b.acquire(Priority::Background, None),
            Err(Error::RateLimited { retry_after: Some(r) }) if r > Duration::from_hours(4)
        ));
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
