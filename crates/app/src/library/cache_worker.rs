//! The library cache, driven from one thread of its own: reads and writes
//! run one at a time, in the order the model sent them. So a sync's replace
//! can't overtake an upsert sent after it (and delete the row it wrote), and
//! a sign-out's clear runs after every write sent before it.
//!
//! The [`LibraryModel`](super::model::LibraryModel) owns the worker for one
//! session. Dropping it lets the thread finish what was sent, then end.

use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use hardcover_core::store::LibraryCache;

use crate::session::Io;

type Job = Box<dyn FnOnce(&LibraryCache) + Send>;

pub struct CacheWorker {
    run: Run,
}

enum Run {
    Thread(mpsc::Sender<Job>),
    /// On the calling thread (tests).
    #[cfg(test)]
    Inline(Arc<LibraryCache>),
}

impl CacheWorker {
    /// `None` if its thread couldn't be started: the app then works without
    /// the cache, as when it can't be opened.
    pub fn start(cache: Arc<LibraryCache>, io: Io) -> Option<Self> {
        let run = match io {
            Io::Threads => {
                let (tx, rx) = mpsc::channel::<Job>();
                let started = thread::Builder::new()
                    .name("library-cache".to_owned())
                    .spawn(move || {
                        for job in rx {
                            job(&cache);
                        }
                    });
                if let Err(e) = started {
                    eprintln!("hardcover: could not start the library cache's thread: {e}");
                    return None;
                }
                Run::Thread(tx)
            }
            #[cfg(test)]
            Io::Inline => Run::Inline(cache),
        };
        Some(Self { run })
    }

    /// Runs `write` after everything sent before; a failure is logged, not
    /// shown: the cache only spares a sync at the next launch.
    pub fn write(
        &self,
        write: impl FnOnce(&LibraryCache) -> hardcover_core::Result<()> + Send + 'static,
    ) {
        self.send(Box::new(move |cache| {
            if let Err(e) = write(cache) {
                eprintln!("hardcover: could not update the library cache: {e}");
            }
        }));
    }

    /// Runs `read` after everything sent before. The future resolves to its
    /// result, or `None` if the worker stopped first.
    pub fn ask<R, F>(&self, read: F) -> impl Future<Output = Option<R>> + use<R, F>
    where
        R: Send + 'static,
        F: FnOnce(&LibraryCache) -> R + Send + 'static,
    {
        let (tx, rx) = async_channel::bounded(1);
        self.send(Box::new(move |cache| {
            let _ = tx.try_send(read(cache));
        }));
        async move { rx.recv().await.ok() }
    }

    fn send(&self, job: Job) {
        match &self.run {
            Run::Thread(tx) => {
                if tx.send(job).is_err() {
                    eprintln!("hardcover: the library cache's thread has stopped");
                }
            }
            #[cfg(test)]
            Run::Inline(cache) => job(cache),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{
        BookId, BookSummary, LibraryEntry, ReadingStatus, User, UserBookId, UserId,
    };

    fn entry(id: i64) -> LibraryEntry {
        LibraryEntry::new(
            UserBookId::new(id),
            UserId::new(1),
            BookSummary::new(BookId::new(id), "A book"),
            ReadingStatus::Read,
        )
    }

    /// On its real thread: an upsert sent after a replace is not deleted by
    /// it, and a read sees every write sent before it.
    #[test]
    fn writes_run_in_the_order_they_were_sent() {
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let worker = CacheWorker::start(cache.clone(), Io::Threads).unwrap();
        let user = User::new(UserId::new(1), "reader");
        worker.write({
            let user = user.clone();
            move |c| {
                c.save_user(&user)?;
                c.replace_library(user.id, &[entry(1)])
            }
        });
        worker.write(|c| c.upsert(&entry(2)));
        let ids = gpui_kit::block_on(worker.ask(move |c| {
            let mut ids: Vec<i64> = c
                .load_library(user.id)
                .unwrap()
                .iter()
                .map(|e| e.user_book_id.get())
                .collect();
            ids.sort_unstable();
            ids
        }));
        assert_eq!(ids, Some(vec![1, 2]));
    }
}
