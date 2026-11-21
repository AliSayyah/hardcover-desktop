//! SQLite cache of the signed-in user and their library, so the app can show
//! something instantly on start and work offline.
//!
//! Entries are stored as JSON blobs of [`LibraryEntry`] keyed by `user_book_id`.
//! The database carries a schema version (`PRAGMA user_version`). An older
//! file is upgraded in place by `MIGRATIONS`, so the library still shows
//! instantly after an update; any other version is wiped and rebuilt, since
//! everything in it can be re-downloaded.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;

use crate::api::{LibraryEntry, Timestamp, User, UserBookId, UserId};
use crate::{Error, Result};

/// Bump when the table layout or the JSON shape of the stored models changes,
/// and add a step to `MIGRATIONS` for the version being left.
pub const SCHEMA_VERSION: i64 = 2;

type Migration = fn(&Transaction<'_>) -> rusqlite::Result<()>;

/// In-place upgrades, as `(version upgraded from, step)`, in order. A file
/// older than the first step, or newer than this build, is wiped instead.
const MIGRATIONS: &[(i64, Migration)] = &[(1, v1_to_v2)];

const FILE_NAME: &str = "library.sqlite3";

pub struct LibraryCache {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for LibraryCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryCache")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl LibraryCache {
    /// `$XDG_DATA_HOME/hardcover-desktop/library.sqlite3`.
    ///
    /// # Errors
    ///
    /// Returns an error if the user data directory cannot be located.
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::project_dirs()?.data_dir().join(FILE_NAME))
    }

    /// Opens (creating if needed) the cache at [`LibraryCache::default_path`].
    ///
    /// # Errors
    ///
    /// Returns an error if the default path cannot be located or the library cache
    /// cannot be opened.
    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_path()?)
    }

    /// Opens (creating if needed) the cache at `path`, creating parent directories.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or its schema cannot be
    /// initialized.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn, Some(path.to_owned()))
    }

    /// Opens a temporary library cache in memory.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot initialize the in-memory cache.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(mut conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != SCHEMA_VERSION && migrate(&mut conn, version).is_err() {
            conn.execute_batch(
                "DROP TABLE IF EXISTS library_entries;
                 DROP TABLE IF EXISTS sync_state;
                 DROP TABLE IF EXISTS current_user;",
            )?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS current_user (
                 slot INTEGER PRIMARY KEY CHECK (slot = 0),
                 user_id INTEGER NOT NULL,
                 json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS library_entries (
                 user_book_id INTEGER PRIMARY KEY,
                 user_id INTEGER NOT NULL,
                 json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS library_entries_user ON library_entries (user_id);
             CREATE TABLE IF NOT EXISTS sync_state (
                 user_id INTEGER PRIMARY KEY,
                 last_synced_at TEXT NOT NULL
             );",
        )?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The last signed-in user, or `None` if the row is absent or unreadable as user JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot read the cached user row.
    pub fn load_user(&self) -> Result<Option<User>> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT json FROM current_user WHERE slot = 0", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Saves the current account in the library cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the user cannot be serialized or stored.
    pub fn save_user(&self, user: &User) -> Result<()> {
        self.conn().execute(
            "INSERT INTO current_user (slot, user_id, json) VALUES (0, ?1, ?2)
             ON CONFLICT (slot) DO UPDATE SET user_id = excluded.user_id, json = excluded.json",
            params![user.id.get(), to_json(user)?],
        )?;
        Ok(())
    }

    /// `user`'s cached entries, ordered by `user_book_id`. Rows that no longer
    /// decode are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot read the library rows.
    pub fn load_library(&self, user: UserId) -> Result<Vec<LibraryEntry>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT json FROM library_entries WHERE user_id = ?1 ORDER BY user_book_id")?;
        let rows = stmt.query_map([user.get()], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for json in rows {
            if let Ok(entry) = serde_json::from_str(&json?) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Replaces everything cached for `user` with `entries` (a full sync) and
    /// records the sync time.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the database transaction fails.
    pub fn replace_library(&self, user: UserId, entries: &[LibraryEntry]) -> Result<()> {
        self.replace_library_at(user, entries, chrono::Utc::now())
    }

    fn replace_library_at(
        &self,
        user: UserId,
        entries: &[LibraryEntry],
        now: Timestamp,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM library_entries WHERE user_id = ?1",
            [user.get()],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO library_entries (user_book_id, user_id, json) VALUES (?1, ?2, ?3)",
            )?;
            for e in entries {
                stmt.execute(params![e.user_book_id.get(), user.get(), to_json(e)?])?;
            }
        }
        tx.execute(
            "INSERT INTO sync_state (user_id, last_synced_at) VALUES (?1, ?2)
             ON CONFLICT (user_id) DO UPDATE SET last_synced_at = excluded.last_synced_at",
            params![user.get(), now.to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Inserts or replaces one entry (e.g. after a mutation), filed under its `user_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry cannot be serialized or stored.
    pub fn upsert(&self, entry: &LibraryEntry) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO library_entries (user_book_id, user_id, json) VALUES (?1, ?2, ?3)",
            params![entry.user_book_id.get(), entry.user_id.get(), to_json(entry)?],
        )?;
        Ok(())
    }

    /// Deletes a library entry from the cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot remove the entry.
    pub fn remove(&self, id: UserBookId) -> Result<()> {
        self.conn().execute(
            "DELETE FROM library_entries WHERE user_book_id = ?1",
            [id.get()],
        )?;
        Ok(())
    }

    /// When [`LibraryCache::replace_library`] last ran for `user`.
    ///
    /// # Errors
    ///
    /// Returns an error if the sync timestamp cannot be read.
    pub fn last_synced_at(&self, user: UserId) -> Result<Option<Timestamp>> {
        let s: Option<String> = self
            .conn()
            .query_row(
                "SELECT last_synced_at FROM sync_state WHERE user_id = ?1",
                [user.get()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(
            s.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|t| t.with_timezone(&chrono::Utc)),
        )
    }

    /// Forgets everything (use on sign-out).
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot clear the cache tables.
    pub fn clear(&self) -> Result<()> {
        self.conn().execute_batch(
            "DELETE FROM library_entries; DELETE FROM sync_state; DELETE FROM current_user;",
        )?;
        Ok(())
    }
}

/// Upgrades a file at `version` to [`SCHEMA_VERSION`] in one transaction.
/// `Err` (nothing changed) when there is no path from `version`.
fn migrate(conn: &mut Connection, version: i64) -> rusqlite::Result<()> {
    let first = MIGRATIONS.iter().position(|(from, _)| *from == version);
    let Some(first) = first.filter(|_| version < SCHEMA_VERSION) else {
        return Err(rusqlite::Error::InvalidQuery);
    };
    let tx = conn.transaction()?;
    for (_, step) in &MIGRATIONS[first..] {
        step(&tx)?;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()
}

/// Rewrites each entry's JSON in place with `edit`. Rows that are not JSON
/// objects are left alone; loading skips them.
fn rewrite_entries(
    tx: &Transaction<'_>,
    edit: impl Fn(&mut serde_json::Map<String, Value>),
) -> rusqlite::Result<()> {
    let rows: Vec<(i64, String)> = tx
        .prepare("SELECT user_book_id, json FROM library_entries")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut update = tx.prepare("UPDATE library_entries SET json = ?2 WHERE user_book_id = ?1")?;
    for (id, json) in rows {
        let Ok(Value::Object(mut entry)) = serde_json::from_str(&json) else {
            continue;
        };
        edit(&mut entry);
        update.execute(params![id, Value::Object(entry).to_string()])?;
    }
    Ok(())
}

/// 0.1.x kept only the latest read, as `current_read`; entries now keep every
/// read in `reads`. The other new fields default when absent.
fn v1_to_v2(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    rewrite_entries(tx, |entry| {
        let reads = match entry.remove("current_read") {
            Some(read @ Value::Object(_)) => vec![read],
            _ => Vec::new(),
        };
        entry.insert("reads".into(), Value::Array(reads));
    })
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| Error::Storage(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::*;

    fn entry(id: i64, user: i64, status: ReadingStatus) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(100 + id), format!("Synthetic {id}"));
        book.authors = vec!["Jane Quill".into()];
        book.cover = Some(Cover::new("https://assets.hardcover.app/x.jpg"));
        let mut e = LibraryEntry::new(UserBookId::new(id), UserId::new(user), book, status);
        e.rating = Rating::new(3.5);
        let mut read = ReadSession::new(ReadId::new(900 + id));
        read.started_at = Date::from_ymd_opt(2024, 1, 2);
        read.progress_percent = Some(12.5);
        e.upsert_read(read);
        let mut past = ReadSession::new(ReadId::new(950 + id));
        past.started_at = Date::from_ymd_opt(2019, 5, 1);
        past.finished_at = Date::from_ymd_opt(2019, 6, 1);
        e.upsert_read(past);
        e.private_notes = Some("Lent to a friend".into());
        e.privacy = Some(Privacy::FollowersOnly);
        e.date_added = Date::from_ymd_opt(2023, 12, 31);
        e.updated_at = Some(chrono::Utc::now());
        e
    }

    #[test]
    fn round_trip() {
        let c = LibraryCache::open_in_memory().unwrap();
        let me = UserId::new(1);
        assert_eq!(c.load_library(me).unwrap(), []);
        assert!(c.last_synced_at(me).unwrap().is_none());

        let entries = vec![
            entry(2, 1, ReadingStatus::Read),
            entry(1, 1, ReadingStatus::CurrentlyReading),
        ];
        c.replace_library(me, &entries).unwrap();
        let loaded = c.load_library(me).unwrap();
        assert_eq!(loaded, vec![entries[1].clone(), entries[0].clone()]);
        assert!(c.last_synced_at(me).unwrap().is_some());

        let mut changed = entries[0].clone();
        changed.status = ReadingStatus::Paused;
        c.upsert(&changed).unwrap();
        c.upsert(&entry(3, 1, ReadingStatus::WantToRead)).unwrap();
        c.upsert(&entry(4, 2, ReadingStatus::WantToRead)).unwrap(); // another user
        let loaded = c.load_library(me).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[1].status, ReadingStatus::Paused);

        c.remove(UserBookId::new(3)).unwrap();
        assert_eq!(c.load_library(me).unwrap().len(), 2);

        // A full sync replaces only that user's rows.
        c.replace_library(me, &[entry(5, 1, ReadingStatus::Read)])
            .unwrap();
        assert_eq!(c.load_library(me).unwrap().len(), 1);
        assert_eq!(c.load_library(UserId::new(2)).unwrap().len(), 1);
    }

    #[test]
    fn user_round_trip_and_clear() {
        let c = LibraryCache::open_in_memory().unwrap();
        assert!(c.load_user().unwrap().is_none());
        let mut u = User::new(UserId::new(1), "reader");
        u.name = Some("A. Reader".into());
        c.save_user(&u).unwrap();
        c.save_user(&u).unwrap();
        assert_eq!(c.load_user().unwrap(), Some(u));
        c.replace_library(UserId::new(1), &[entry(1, 1, ReadingStatus::Read)])
            .unwrap();
        c.clear().unwrap();
        assert!(c.load_user().unwrap().is_none());
        assert_eq!(c.load_library(UserId::new(1)).unwrap(), []);
        assert!(c.last_synced_at(UserId::new(1)).unwrap().is_none());
    }

    #[test]
    fn persists_on_disk_and_wipes_on_version_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/library.sqlite3");
        {
            let c = LibraryCache::open(&path).unwrap();
            c.replace_library(UserId::new(1), &[entry(1, 1, ReadingStatus::Read)])
                .unwrap();
        }
        {
            let c = LibraryCache::open(&path).unwrap();
            assert_eq!(c.load_library(UserId::new(1)).unwrap().len(), 1);
            c.conn().pragma_update(None, "user_version", 999).unwrap();
        }
        let c = LibraryCache::open(&path).unwrap();
        assert_eq!(c.load_library(UserId::new(1)).unwrap(), []);
    }

    /// A cache as 0.1.x wrote it: version 1, one `current_read` per entry.
    fn write_v1_cache(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE current_user (
                   slot INTEGER PRIMARY KEY CHECK (slot = 0),
                   user_id INTEGER NOT NULL,
                   json TEXT NOT NULL
               );
               CREATE TABLE library_entries (
                   user_book_id INTEGER PRIMARY KEY,
                   user_id INTEGER NOT NULL,
                   json TEXT NOT NULL
               );
               CREATE INDEX library_entries_user ON library_entries (user_id);
               CREATE TABLE sync_state (
                   user_id INTEGER PRIMARY KEY,
                   last_synced_at TEXT NOT NULL
               );
               INSERT INTO current_user VALUES (0, 7, '{"id":7,"username":"reader","name":null,"avatar":null}');
               INSERT INTO sync_state VALUES (7, '2026-10-01T08:00:00+00:00');
               INSERT INTO library_entries VALUES (1, 7, '{"user_book_id":1,"user_id":7,"book":{"id":101,"title":"Synthetic One","subtitle":null,"authors":["Jane Quill"],"cover":null,"pages":320,"release_year":1999,"slug":"synthetic-one","average_rating":3.9,"users_count":12},"status":"CurrentlyReading","rating":null,"edition":{"id":5001,"title":null,"pages":400,"audio_seconds":null,"cover":null,"format":"Paperback"},"current_read":{"id":901,"started_at":"2026-09-01","finished_at":null,"progress_pages":120,"progress_seconds":null,"progress_percent":30.0,"edition_id":5001},"date_added":"2026-08-30","updated_at":"2026-09-02T10:00:00Z"}');
               INSERT INTO library_entries VALUES (2, 7, '{"user_book_id":2,"user_id":7,"book":{"id":102,"title":"Synthetic Two","subtitle":null,"authors":[],"cover":null,"pages":null,"release_year":null,"slug":null,"average_rating":null,"users_count":null},"status":"WantToRead","rating":null,"edition":null,"current_read":null,"date_added":"2026-08-31","updated_at":null}');
               INSERT INTO library_entries VALUES (3, 7, 'not json');
               PRAGMA user_version = 1;"#,
        )
        .unwrap();
    }

    #[test]
    fn a_v1_cache_is_upgraded_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        write_v1_cache(&path);

        let c = LibraryCache::open(&path).unwrap();
        let version: i64 = c
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let me = UserId::new(7);
        assert_eq!(c.load_user().unwrap().unwrap().username, "reader");
        assert!(c.last_synced_at(me).unwrap().is_some());

        let loaded = c.load_library(me).unwrap();
        assert_eq!(
            loaded.len(),
            2,
            "the row that was not JSON is still skipped"
        );
        let reading = &loaded[0];
        assert_eq!(reading.reads.len(), 1);
        let read = reading.current_read().unwrap();
        assert_eq!(read.id, ReadId::new(901));
        assert_eq!(read.progress_pages, Some(120));
        assert_eq!(read.edition_id, Some(EditionId::new(5001)));
        assert_eq!(
            reading.edition.as_ref().unwrap().format.as_deref(),
            Some("Paperback")
        );
        assert_eq!(reading.privacy, None);
        assert_eq!(reading.private_notes, None);
        assert_eq!(loaded[1].reads, []);

        // Reopening finds the current version and changes nothing.
        drop(c);
        let c = LibraryCache::open(&path).unwrap();
        assert_eq!(c.load_library(me).unwrap(), loaded);
    }

    #[test]
    fn undecodable_rows_are_skipped() {
        let c = LibraryCache::open_in_memory().unwrap();
        c.upsert(&entry(1, 1, ReadingStatus::Read)).unwrap();
        c.conn()
            .execute(
                "INSERT INTO library_entries VALUES (2, 1, '{\"old\":true}')",
                [],
            )
            .unwrap();
        assert_eq!(c.load_library(UserId::new(1)).unwrap().len(), 1);
    }
}
