//! The cover pipeline's threads: a queue served by dedicated threads, the
//! cover drawn most recently first, and the downloader on top of it.
//!
//! Downloads block for up to half a minute each. On GPUI's background pool
//! (as many threads as cores) a few stalled ones held every thread, and
//! sync, search and writes waited behind them. On their own threads they
//! hold only themselves, and the asset futures that wait for them only
//! await a channel.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use super::usage::Usage;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A job for a [`Queue`]: the cover it is for, if any, gives its priority.
pub(super) trait Job: Send + 'static {
    fn url(&self) -> Option<&str>;
}

struct State<J> {
    jobs: Vec<J>,
    /// Threads serving the queue. With none, nothing is queued.
    workers: usize,
}

/// Jobs served by dedicated threads, the job for the cover drawn most
/// recently first ([`Usage::priority`], read when a thread takes a job, so a
/// cover scrolled back into view moves up); jobs for no cover, or one never
/// drawn, last; among equals, the oldest first.
pub(super) struct Queue<J> {
    state: Mutex<State<J>>,
    ready: Condvar,
    usage: Arc<Mutex<Usage>>,
}

impl<J: Job> Queue<J> {
    /// Starts `workers` threads named `name`, each running `run` on one job
    /// at a time. A job that panics is dropped (its reply channel closes) and
    /// the thread goes on.
    pub(super) fn spawn(
        name: &str,
        workers: usize,
        usage: Arc<Mutex<Usage>>,
        run: impl Fn(J) + Send + Sync + 'static,
    ) -> Arc<Self> {
        let queue = Arc::new(Self {
            state: Mutex::new(State {
                jobs: Vec::new(),
                workers: 0,
            }),
            ready: Condvar::new(),
            usage,
        });
        let run = Arc::new(run);
        for _ in 0..workers {
            let (worker, run) = (queue.clone(), run.clone());
            let spawned = thread::Builder::new().name(name.into()).spawn(move || {
                loop {
                    let job = worker.take();
                    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| run(job)));
                }
            });
            match spawned {
                Ok(_) => lock(&queue.state).workers += 1,
                Err(e) => eprintln!("hardcover: could not start a {name} thread: {e}"),
            }
        }
        queue
    }

    /// Queues `job`, or hands it back when no thread serves the queue.
    pub(super) fn push(&self, job: J) -> Result<(), J> {
        let mut state = lock(&self.state);
        if state.workers == 0 {
            return Err(job);
        }
        state.jobs.push(job);
        self.ready.notify_one();
        Ok(())
    }

    /// Waits for a job and takes the one wanted most.
    fn take(&self) -> J {
        let mut state = lock(&self.state);
        while state.jobs.is_empty() {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        let usage = lock(&self.usage);
        let mut best = 0;
        let mut best_priority = 0;
        for (i, job) in state.jobs.iter().enumerate() {
            let priority = job.url().map_or(0, |url| usage.priority(url));
            if priority > best_priority {
                (best, best_priority) = (i, priority);
            }
        }
        drop(usage);
        state.jobs.remove(best)
    }
}

/// How a download went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fetched {
    /// The file, or every scaled copy, is on disk.
    Done,
    /// Something a later try may not hit (the network, say).
    Failed,
    /// Not an image the app can show, or not a URL it fetches: trying again
    /// won't help.
    Unsupported,
}

type Waiters = Arc<Mutex<HashMap<String, Vec<async_channel::Sender<Fetched>>>>>;

/// One URL to download; its waiters are in [`Downloader::waiters`].
struct Download(String);

impl Job for Download {
    fn url(&self) -> Option<&str> {
        Some(&self.0)
    }
}

/// Downloads covers on their own threads, one download per URL however many
/// ask for it (two sizes of one cover, say), skipping a URL nobody waits
/// for any more by the time a thread gets to it.
#[derive(Clone)]
pub(super) struct Downloader {
    queue: Arc<Queue<Download>>,
    /// Who waits for each URL that is queued or downloading.
    waiters: Waiters,
}

impl Downloader {
    /// Starts `workers` download threads, each calling `fetch` (blocking) for
    /// one URL at a time.
    pub(super) fn spawn(
        workers: usize,
        usage: Arc<Mutex<Usage>>,
        fetch: impl Fn(&str) -> Fetched + Send + Sync + 'static,
    ) -> Self {
        let waiters = Waiters::default();
        let queue = Queue::spawn("cover-download", workers, usage, {
            let waiters = waiters.clone();
            move |Download(url)| run(&waiters, &url, &fetch)
        });
        Self { queue, waiters }
    }

    /// Asks for `url`. The answer comes on the receiver; dropping it before
    /// the download starts withdraws the request.
    pub(super) fn request(&self, url: &str) -> async_channel::Receiver<Fetched> {
        let (reply, answer) = async_channel::bounded(1);
        let first = match lock(&self.waiters).entry(url.to_owned()) {
            Entry::Occupied(mut waiting) => {
                waiting.get_mut().push(reply);
                false
            }
            Entry::Vacant(none) => {
                none.insert(vec![reply]);
                true
            }
        };
        if first && self.queue.push(Download(url.to_owned())).is_err() {
            answer_all(&self.waiters, url, Fetched::Failed);
        }
        answer
    }

    pub(super) async fn download(&self, url: &str) -> Fetched {
        self.request(url).recv().await.unwrap_or(Fetched::Failed)
    }
}

/// A download thread's work on `url`.
fn run(waiters: &Waiters, url: &str, fetch: &impl Fn(&str) -> Fetched) {
    {
        let mut waiting = lock(waiters);
        let wanted = waiting
            .get(url)
            .is_some_and(|w| w.iter().any(|reply| !reply.is_closed()));
        if !wanted {
            // Nobody waits any more (the cover scrolled away and was
            // released): skip it. A later request queues it again.
            waiting.remove(url);
            return;
        }
    }
    // Answers `Failed` should `fetch` panic, so no request waits forever.
    struct Answer<'a> {
        waiters: &'a Waiters,
        url: &'a str,
        fetched: Fetched,
    }
    impl Drop for Answer<'_> {
        fn drop(&mut self) {
            answer_all(self.waiters, self.url, self.fetched);
        }
    }
    let mut answer = Answer {
        waiters,
        url,
        fetched: Fetched::Failed,
    };
    answer.fetched = fetch(url);
}

fn answer_all(waiters: &Waiters, url: &str, fetched: Fetched) {
    let waiting = lock(waiters).remove(url).unwrap_or_default();
    for reply in waiting {
        let _ = reply.try_send(fetched);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::cover_pipeline::usage::Kind;

    struct Named(&'static str, mpsc::Sender<&'static str>);

    impl Job for Named {
        fn url(&self) -> Option<&str> {
            (!self.0.starts_with("tidy")).then_some(self.0)
        }
    }

    #[test]
    fn the_cover_drawn_last_is_served_first_and_tidy_jobs_last() {
        let usage = Arc::new(Mutex::new(Usage::default()));
        // Holds the one thread until every job is queued.
        let (gate, wait) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let queue = Queue::spawn("test-queue", 1, usage.clone(), move |Named(name, done)| {
            if name == "gate" {
                let _ = lock(&wait).recv();
            }
            let _ = done.send(name);
        });
        let (done, order) = mpsc::channel();
        assert!(queue.push(Named("gate", done.clone())).is_ok());
        std::thread::sleep(Duration::from_millis(50));
        for name in ["tidy-1", "row-1", "row-2", "tidy-2", "row-3"] {
            assert!(queue.push(Named(name, done.clone())).is_ok());
        }
        {
            let mut usage = lock(&usage);
            usage.stamp("row-1", Kind::Thumbnail);
            usage.stamp("row-3", Kind::Thumbnail);
            usage.stamp("row-2", Kind::Thumbnail);
            // Scrolled back to row 1.
            usage.stamp("row-1", Kind::Thumbnail);
        }
        gate.send(()).unwrap();
        let served: Vec<_> = (0..6)
            .map(|_| order.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(
            served,
            ["gate", "row-1", "row-2", "row-3", "tidy-1", "tidy-2"]
        );
    }

    /// A downloader whose one thread waits for `gate` before each fetch,
    /// counting fetches.
    fn gated() -> (Downloader, Arc<AtomicUsize>, mpsc::Sender<()>) {
        let fetches = Arc::new(AtomicUsize::new(0));
        let (gate, wait) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let downloader = Downloader::spawn(1, Arc::default(), {
            let fetches = fetches.clone();
            move |url| {
                let _ = lock(&wait).recv();
                fetches.fetch_add(1, Ordering::SeqCst);
                if url.ends_with(".avif") {
                    Fetched::Unsupported
                } else {
                    Fetched::Done
                }
            }
        });
        (downloader, fetches, gate)
    }

    #[test]
    fn two_requests_for_one_cover_share_one_download() {
        let (downloader, fetches, gate) = gated();
        let thumbnail = downloader.request("https://x.example/a.jpg");
        let medium = downloader.request("https://x.example/a.jpg");
        gate.send(()).unwrap();
        assert_eq!(thumbnail.recv_blocking(), Ok(Fetched::Done));
        assert_eq!(medium.recv_blocking(), Ok(Fetched::Done));
        assert_eq!(fetches.load(Ordering::SeqCst), 1);

        // Once done, a new request downloads again (the fetch itself finds
        // the file on disk).
        let again = downloader.request("https://x.example/a.jpg");
        gate.send(()).unwrap();
        assert_eq!(again.recv_blocking(), Ok(Fetched::Done));
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_cover_nobody_waits_for_is_not_downloaded() {
        let (downloader, fetches, gate) = gated();
        let busy = downloader.request("https://x.example/busy.jpg");
        std::thread::sleep(Duration::from_millis(50));
        // Scrolled away and released before the thread got to it.
        drop(downloader.request("https://x.example/gone.jpg"));
        let unsupported = downloader.request("https://x.example/c.avif");
        gate.send(()).unwrap();
        gate.send(()).unwrap();
        assert_eq!(busy.recv_blocking(), Ok(Fetched::Done));
        assert_eq!(unsupported.recv_blocking(), Ok(Fetched::Unsupported));
        assert_eq!(fetches.load(Ordering::SeqCst), 2, "gone.jpg was skipped");

        // Asked for again, it is downloaded.
        let back = downloader.request("https://x.example/gone.jpg");
        gate.send(()).unwrap();
        assert_eq!(back.recv_blocking(), Ok(Fetched::Done));
    }

    #[test]
    fn a_fetch_that_panics_answers_and_the_thread_goes_on() {
        let downloader = Downloader::spawn(1, Arc::default(), |url| {
            assert!(!url.contains("boom"), "a decoder bug");
            Fetched::Done
        });
        assert_eq!(
            downloader
                .request("https://x.example/boom.jpg")
                .recv_blocking(),
            Ok(Fetched::Failed)
        );
        assert_eq!(
            downloader
                .request("https://x.example/ok.jpg")
                .recv_blocking(),
            Ok(Fetched::Done)
        );
    }

    #[test]
    fn with_no_thread_a_request_fails_at_once() {
        let downloader = Downloader::spawn(0, Arc::default(), |_| Fetched::Done);
        assert_eq!(
            downloader
                .request("https://x.example/a.jpg")
                .recv_blocking(),
            Ok(Fetched::Failed)
        );
    }
}
