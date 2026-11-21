//! One running app per data directory.
//!
//! The first launch holds `instance.lock` in the data directory (a
//! [`FileLock`]) and listens on `instance.sock` beside it. A later launch
//! finds the lock taken, asks the running copy over the socket to show its
//! window, and exits. Two copies would share the keyring session, the
//! library cache and the update's staging directory.
//!
//! The data directory is what an instance is, so a run with its own
//! `XDG_DATA_HOME` is an instance of its own. Programs that only use the
//! keyring (the examples, `api_check`) never claim it; the token refresh has
//! its own lock ([`crate::auth::TokenStore::lock`]).

use std::io::{self, ErrorKind, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::lock::FileLock;
use crate::{Error, Result};

const LOCK_FILE: &str = "instance.lock";
const SOCKET_FILE: &str = "instance.sock";
const SHOW: &[u8; 5] = b"show\n";
const SHOWN: &[u8; 3] = b"ok\n";
/// How long a later launch tries to reach the running copy, which binds its
/// socket just after taking the lock.
const CONNECT_FOR: Duration = Duration::from_secs(2);
/// For each read and write on a connection, both ways.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// What a launch found.
#[derive(Debug)]
pub enum Claim {
    /// This is the only copy: keep the [`Primary`] for the whole run.
    Primary(Primary),
    /// Another copy is running. `shown` says whether it confirmed it would
    /// show its window.
    Running { shown: bool },
}

/// The running copy's claim: the lock, and the socket later launches call.
#[derive(Debug)]
pub struct Primary {
    _lock: FileLock,
    listener: UnixListener,
}

/// Claims the instance for the default data directory.
///
/// # Errors
///
/// Returns an error if the directory, the lock or the socket can't be set
/// up; the caller decides whether to run anyway.
pub fn claim() -> Result<Claim> {
    let dirs = crate::project_dirs()?;
    claim_in(dirs.data_dir())
        .map_err(|e| Error::Storage(format!("could not check for a running copy: {e}")))
}

/// Claims the instance for `dir`.
///
/// # Errors
///
/// Returns an error if the directory, the lock or the socket can't be set up.
pub fn claim_in(dir: &Path) -> io::Result<Claim> {
    let socket = dir.join(SOCKET_FILE);
    let Some(lock) = FileLock::try_acquire(&dir.join(LOCK_FILE))? else {
        return Ok(Claim::Running {
            shown: ask_to_show(&socket),
        });
    };
    // The lock is ours, so a socket file here was left by a copy that
    // ended without removing it.
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(&socket)?;
    Ok(Claim::Primary(Primary {
        _lock: lock,
        listener,
    }))
}

/// Asks the running copy to show its window; true once it confirmed.
fn ask_to_show(socket: &Path) -> bool {
    let deadline = Instant::now() + CONNECT_FOR;
    let mut stream = loop {
        match UnixStream::connect(socket) {
            Ok(stream) => break stream,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    };
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut reply = [0u8; SHOWN.len()];
    stream.write_all(SHOW).is_ok() && stream.read_exact(&mut reply).is_ok() && &reply == SHOWN
}

impl Primary {
    /// Answers later launches on a thread of its own for the rest of the
    /// run, calling `on_show` for each that asks for the window. The claim
    /// moves to that thread and is never released before the process ends,
    /// even if the socket fails.
    pub fn serve(self, mut on_show: impl FnMut() + Send + 'static) {
        let spawned = std::thread::Builder::new()
            .name("instance".into())
            .spawn(move || {
                loop {
                    match self.listener.accept() {
                        Ok((stream, _)) => {
                            if answer(stream) {
                                on_show();
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::Interrupted => {}
                        Err(e) => {
                            eprintln!("hardcover: no longer answering later launches: {e}");
                            break;
                        }
                    }
                }
                // Keep the lock: another copy must still not start.
                let _claim = self;
                loop {
                    std::thread::park();
                }
            });
        if let Err(e) = spawned {
            eprintln!("hardcover: could not answer later launches: {e}");
        }
    }
}

/// Reads one request; true if it asked for the window (and was answered).
fn answer(mut stream: UnixStream) -> bool {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut request = [0u8; SHOW.len()];
    if stream.read_exact(&mut request).is_err() || &request != SHOW {
        return false;
    }
    let _ = stream.write_all(SHOWN);
    true
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn a_second_launch_asks_the_first_to_show_itself() {
        let dir = tempfile::tempdir().unwrap();
        let Claim::Primary(primary) = claim_in(dir.path()).unwrap() else {
            panic!("the first launch must be the primary");
        };
        let (tx, rx) = mpsc::channel();
        primary.serve(move || tx.send(()).unwrap());

        assert!(matches!(
            claim_in(dir.path()).unwrap(),
            Claim::Running { shown: true }
        ));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // And again: the first copy keeps answering.
        assert!(matches!(
            claim_in(dir.path()).unwrap(),
            Claim::Running { shown: true }
        ));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn a_socket_left_by_a_crash_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        // A socket file nobody listens on, and no lock held.
        drop(UnixListener::bind(dir.path().join(SOCKET_FILE)).unwrap());
        assert!(dir.path().join(SOCKET_FILE).exists());
        assert!(matches!(claim_in(dir.path()).unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn the_claim_ends_with_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let first = claim_in(dir.path()).unwrap();
        assert!(matches!(first, Claim::Primary(_)));
        drop(first);
        assert!(matches!(claim_in(dir.path()).unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn other_requests_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let Claim::Primary(primary) = claim_in(dir.path()).unwrap() else {
            panic!("the first launch must be the primary");
        };
        let (tx, rx) = mpsc::channel();
        primary.serve(move || tx.send(()).unwrap());
        let mut stream = UnixStream::connect(dir.path().join(SOCKET_FILE)).unwrap();
        stream.write_all(b"quit\n").unwrap();
        drop(stream);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
