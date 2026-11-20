//! Exporting the library: a Goodreads-style CSV that other services import,
//! and a full JSON backup.
//!
//! [`gather`] fetches what the library sync leaves out (reviews, the lists'
//! books, goals); [`goodreads_csv`] and [`backup_json`] turn a [`Backup`]
//! into a file; [`save`] writes it atomically. Everything here blocks: run it
//! off the UI thread.

mod goodreads;
mod json;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::api::{
    Client, Date, Goal, LIST_BOOKS_PER_PAGE, LibraryEntry, ListBook, Review, Timestamp, User,
    UserBookId, UserList,
};
use crate::{Error, Result};

pub use goodreads::goodreads_csv;
pub use json::{BACKUP_FORMAT, BACKUP_VERSION, backup_json};

/// The two export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Goodreads' export columns, which StoryGraph, BookWyrm and others import.
    GoodreadsCsv,
    /// Everything the app knows, versioned (see [`BACKUP_FORMAT`]).
    Json,
}

impl Format {
    pub const ALL: [Format; 2] = [Self::GoodreadsCsv, Self::Json];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::GoodreadsCsv => "csv",
            Self::Json => "json",
        }
    }

    /// `hardcover-library-YYYY-MM-DD.csv` or `.json`, for the save dialog.
    pub fn file_name(self, day: Date) -> String {
        format!(
            "hardcover-library-{}.{}",
            day.format("%Y-%m-%d"),
            self.extension()
        )
    }

    /// The file's contents.
    pub fn render(self, backup: &Backup) -> String {
        match self {
            Self::GoodreadsCsv => goodreads_csv(backup),
            Self::Json => backup_json(backup),
        }
    }
}

/// One of the user's lists with its books, in the list's order.
#[derive(Debug, Clone, PartialEq)]
pub struct ListContents {
    pub list: UserList,
    pub books: Vec<ListBook>,
}

/// Everything an export is made from.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Backup {
    pub exported_at: Timestamp,
    /// The signed-in user, when known.
    pub user: Option<User>,
    /// The whole library, Ignored entries included.
    pub entries: Vec<LibraryEntry>,
    /// The entries' reviews, by entry. Entries without one are absent.
    pub reviews: HashMap<UserBookId, Review>,
    /// `None` when the session can't read lists.
    pub lists: Option<Vec<ListContents>>,
    /// `None` when the session can't read goals.
    pub goals: Option<Vec<Goal>>,
}

impl Backup {
    /// A backup of `entries` alone: no reviews, and lists and goals left out.
    pub fn new(exported_at: Timestamp, user: Option<User>, entries: Vec<LibraryEntry>) -> Self {
        Self {
            exported_at,
            user,
            entries,
            reviews: HashMap::new(),
            lists: None,
            goals: None,
        }
    }

    /// The parts left out because the session can't read them.
    pub fn omitted(&self) -> Vec<&'static str> {
        let mut omitted = Vec::new();
        if self.lists.is_none() {
            omitted.push("lists");
        }
        if self.goals.is_none() {
            omitted.push("goals");
        }
        omitted
    }
}

/// What [`gather`] fetches besides the reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    /// Needs `read:lists`.
    pub lists: bool,
    /// Needs `read:goals`.
    pub goals: bool,
}

/// Where [`gather`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Reviews fetched so far.
    Reviews(usize),
    /// Lists whose books have been fetched, of `total`.
    Lists {
        done: usize,
        total: usize,
    },
    Goals,
}

/// Fetches the reviews, then (as `parts` asks) every list with all its books
/// and the goals, and adds them to `backup`. One request per 100 reviews, one
/// for the lists, one per 100 books of each list, and one for the goals.
///
/// `cancel` is checked between requests: once set, this returns
/// [`Error::Cancelled`].
pub fn gather(
    client: &Client,
    mut backup: Backup,
    parts: Parts,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Backup> {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let check = || {
        if cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    };

    progress(Progress::Reviews(0));
    let reviews = client.reviews(|count| {
        progress(Progress::Reviews(count));
        !cancelled()
    })?;
    check()?;
    backup.reviews = reviews.into_iter().collect();

    if parts.lists {
        let lists = client.lists()?;
        let total = lists.len();
        let mut contents = Vec::with_capacity(total);
        for (done, list) in lists.into_iter().enumerate() {
            progress(Progress::Lists { done, total });
            check()?;
            let books = list_books(client, &list, &check)?;
            contents.push(ListContents { list, books });
        }
        progress(Progress::Lists { done: total, total });
        backup.lists = Some(contents);
    }

    if parts.goals {
        check()?;
        progress(Progress::Goals);
        backup.goals = Some(client.goals()?);
    }
    check()?;
    Ok(backup)
}

/// Every book on `list`. [`Client::list_books`] leaves out rows whose book
/// didn't resolve, so a short page ends the list only once the offset has
/// passed the list's count.
fn list_books(
    client: &Client,
    list: &UserList,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<ListBook>> {
    let mut books = Vec::new();
    let mut offset = 0;
    loop {
        let page = client.list_books(list.id, offset)?;
        let fetched = page.len();
        books.extend(page);
        offset += LIST_BOOKS_PER_PAGE;
        if fetched == 0 || (fetched < LIST_BOOKS_PER_PAGE as usize && offset >= list.books_count) {
            return Ok(books);
        }
        check()?;
    }
}

/// Where the save dialog starts: the Documents folder if there is one (the
/// XDG one, else `~/Documents`), else the home folder.
pub fn default_directory() -> PathBuf {
    let dirs = directories::UserDirs::new();
    let home = dirs.as_ref().map(|d| d.home_dir().to_path_buf());
    dirs.as_ref()
        .and_then(|d| d.document_dir())
        .map(Path::to_path_buf)
        .filter(|p| p.is_dir())
        .or_else(|| {
            home.as_ref()
                .map(|h| h.join("Documents"))
                .filter(|p| p.is_dir())
        })
        .or(home)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Renders `backup` as `format` and writes it to `path` atomically.
pub fn save(format: Format, backup: &Backup, path: &Path) -> Result<()> {
    write_atomic(path, format.render(backup).as_bytes())
}

/// Writes `bytes` to a temporary file next to `path`, then renames it into
/// place, so a failed write never leaves half a file (or clobbers an older
/// export).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| Error::InvalidInput(format!("{} is not a file name", path.display())))?
        .to_string_lossy();
    // Unique per write, so overlapping writes never share a temporary file.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let serial = WRITES.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".{name}.{}-{serial}.tmp", std::process::id()));
    let written = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    Ok(written?)
}
