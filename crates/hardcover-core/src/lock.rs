//! Advisory file locks (`flock`) shared between processes.
//!
//! A lock is held on an open file and released when the [`FileLock`] is
//! dropped, or when the process ends however it ends, so a crash never
//! leaves one behind. Two opens of the same file conflict even within one
//! process. Files are opened close-on-exec, so programs the app starts
//! (the browser, a restart) don't inherit a lock.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::Path;

/// An exclusive lock on a file, held until dropped.
#[derive(Debug)]
pub struct FileLock {
    _file: File,
}

impl FileLock {
    /// Waits until the lock on `path` is free, then takes it. Creates the file
    /// (and its directory, private to the user) if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be created or locked.
    pub fn acquire(path: &Path) -> io::Result<Self> {
        let file = open(path)?;
        file.lock()?;
        Ok(Self { _file: file })
    }

    /// Takes the lock on `path` if it is free; `None` while another holder has it.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be created or locked.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

fn open(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_is_refused_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/refresh.lock");
        let held = FileLock::acquire(&path).unwrap();
        // A second open of the same file, as another process would make.
        assert!(FileLock::try_acquire(&path).unwrap().is_none());
        drop(held);
        assert!(FileLock::try_acquire(&path).unwrap().is_some());
    }

    #[test]
    fn acquire_waits_for_the_holder() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refresh.lock");
        let held = FileLock::acquire(&path).unwrap();
        let released = Arc::new(AtomicBool::new(false));
        let waiter = {
            let path = path.clone();
            let released = released.clone();
            std::thread::spawn(move || {
                let _lock = FileLock::acquire(&path).unwrap();
                released.load(Ordering::SeqCst)
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        released.store(true, Ordering::SeqCst);
        drop(held);
        // The waiter got the lock only after it was released.
        assert!(waiter.join().unwrap());
    }
}
