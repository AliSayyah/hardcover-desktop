//! Where the app's blocking work runs: the core's calls (network, keyring,
//! files) on a small set of threads of their own, never on GPUI's.
//!
//! GPUI's background pool has as many threads as the machine has cores, and
//! everything else shares it: a few calls stalled on a slow network (or
//! sleeping in the rate limiter) would hold it and stall every other
//! background task. So the core's calls go to [`IO_THREADS`] threads here,
//! in two lanes: an [`Priority::Interactive`] job (a write, a search, a page
//! the user opened) runs before any waiting [`Priority::Background`] one
//! (syncs, reloads, prefetches), and background jobs never take the last
//! thread, so an interactive one always finds a thread free unless
//! [`IO_THREADS`] interactive ones are already running.
//!
//! [`run`] queues a job and returns a future of its result. Dropping the
//! future before the job starts means it never runs; once running, a job
//! runs to the end (the core's calls can't be interrupted) and its result is
//! dropped. Calls made for the signed-in account go through
//! `crate::session::SessionScope::run`, which adds the session's own checks.
//!
//! Waits measured in minutes (the browser sign-in, an update's download) get
//! a thread of their own instead ([`run_on_own_thread`]), so they never hold
//! one of these.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::thread;

pub use hardcover_core::Priority;

/// Threads serving core calls.
pub const IO_THREADS: usize = 4;

/// Of [`IO_THREADS`], how many may run background jobs at once.
const BACKGROUND_THREADS: usize = IO_THREADS - 1;

type Job = Box<dyn FnOnce() + Send>;

/// Runs `job` on the I/O threads at `priority`. The future resolves to its
/// result, or to `None` if the job was skipped. Dropping the future before
/// the job starts skips it.
pub fn run<R: Send + 'static>(
    priority: Priority,
    job: impl FnOnce() -> R + Send + 'static,
) -> impl Future<Output = Option<R>> + 'static {
    let result = submit(IoPool::global(), priority, job);
    async move { result.recv().await.ok() }
}

/// Runs `job` on the I/O threads at `priority`, with nobody waiting for it
/// (a revoke, a save).
pub fn spawn(priority: Priority, job: impl FnOnce() + Send + 'static) {
    IoPool::global().push(priority, Box::new(job));
}

/// Queues `job` on `pool`; its result arrives on the receiver. Dropping the
/// receiver before the job starts skips it.
pub(crate) fn submit<R: Send + 'static>(
    pool: &IoPool,
    priority: Priority,
    job: impl FnOnce() -> R + Send + 'static,
) -> async_channel::Receiver<R> {
    let (tx, rx) = async_channel::bounded(1);
    pool.push(
        priority,
        Box::new(move || {
            // Nobody waits for it any more: skip it.
            if tx.is_closed() {
                return;
            }
            let _ = tx.try_send(job());
        }),
    );
    rx
}

/// Runs `job` on a new thread of its own, for a wait that may last minutes.
/// The future resolves to its result, or to `None` if the thread couldn't be
/// started or the job panicked.
pub fn run_on_own_thread<R, F>(name: &str, job: F) -> impl Future<Output = Option<R>> + use<R, F>
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    let started = thread::Builder::new().name(name.to_owned()).spawn(move || {
        let _ = tx.try_send(job());
    });
    if let Err(e) = started {
        eprintln!("hardcover: could not start a thread for {name}: {e}");
    }
    async move { rx.recv().await.ok() }
}

/// The I/O threads and their two queues.
pub(crate) struct IoPool {
    queues: Mutex<Queues>,
    /// Signalled when a job is queued or a background job finishes.
    ready: Condvar,
    background_threads: usize,
}

#[derive(Default)]
struct Queues {
    interactive: VecDeque<Job>,
    background: VecDeque<Job>,
    /// Background jobs running now.
    background_running: usize,
}

impl IoPool {
    pub(crate) fn global() -> &'static Arc<IoPool> {
        static POOL: OnceLock<Arc<IoPool>> = OnceLock::new();
        POOL.get_or_init(|| IoPool::start(IO_THREADS, BACKGROUND_THREADS))
    }

    /// A pool of `threads` threads, at most `background_threads` of them
    /// running background jobs at once.
    pub(crate) fn start(threads: usize, background_threads: usize) -> Arc<Self> {
        assert!(
            background_threads < threads,
            "one thread is kept for interactive jobs"
        );
        let pool = Arc::new(Self {
            queues: Mutex::new(Queues::default()),
            ready: Condvar::new(),
            background_threads,
        });
        for n in 0..threads {
            let pool = pool.clone();
            thread::Builder::new()
                .name(format!("core-io-{n}"))
                .spawn(move || pool.work())
                .expect("start an I/O thread");
        }
        pool
    }

    pub(crate) fn push(&self, priority: Priority, job: Job) {
        let mut queues = self.lock();
        match priority {
            Priority::Interactive => queues.interactive.push_back(job),
            Priority::Background => queues.background.push_back(job),
        }
        drop(queues);
        self.ready.notify_one();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queues> {
        self.queues.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn work(&self) {
        loop {
            let (job, background) = {
                let mut queues = self.lock();
                loop {
                    if let Some(job) = queues.interactive.pop_front() {
                        break (job, false);
                    }
                    if queues.background_running < self.background_threads
                        && let Some(job) = queues.background.pop_front()
                    {
                        queues.background_running += 1;
                        break (job, true);
                    }
                    queues = self
                        .ready
                        .wait(queues)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            // A job that panics loses only its own result: its sender is
            // dropped, so its future resolves to `None`.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            if background {
                self.lock().background_running -= 1;
                // A background job may be waiting for this slot.
                self.ready.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// A background job that waits until `release` sends.
    fn blocker(pool: &IoPool) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel::<()>();
        pool.push(
            Priority::Background,
            Box::new(move || {
                let _ = held.recv();
            }),
        );
        release
    }

    #[test]
    fn an_interactive_job_runs_while_background_jobs_fill_their_threads() {
        let pool = IoPool::start(2, 1);
        // One background job holds the only background slot, another waits
        // behind it.
        let first = blocker(&pool);
        let second = blocker(&pool);
        let done = submit(&pool, Priority::Interactive, || 42);
        let (answer_tx, answer) = mpsc::channel();
        std::thread::spawn(move || answer_tx.send(done.recv_blocking()));
        assert_eq!(
            answer.recv_timeout(Duration::from_secs(5)),
            Ok(Ok(42)),
            "the interactive job ran with the background ones blocked"
        );
        first.send(()).unwrap();
        second.send(()).unwrap();
    }

    #[test]
    fn a_job_whose_result_nobody_waits_for_never_runs() {
        let pool = IoPool::start(2, 1);
        let release = blocker(&pool);
        let ran = Arc::new(Mutex::new(false));
        let skipped = submit(&pool, Priority::Background, {
            let ran = ran.clone();
            move || *ran.lock().unwrap() = true
        });
        // Given up on while queued behind the blocked one.
        drop(skipped);
        release.send(()).unwrap();
        // A later background job runs after it, so by then it was skipped.
        let later = submit(&pool, Priority::Background, || ());
        assert_eq!(later.recv_blocking(), Ok(()));
        assert!(!*ran.lock().unwrap());
    }

    #[test]
    fn a_job_that_panics_resolves_to_nothing() {
        let pool = IoPool::start(2, 1);
        let failed = submit(&pool, Priority::Interactive, || -> u8 { panic!("boom") });
        assert!(failed.recv_blocking().is_err());
        // The thread survived.
        let ok = submit(&pool, Priority::Interactive, || 1);
        assert_eq!(ok.recv_blocking(), Ok(1));
    }
}
