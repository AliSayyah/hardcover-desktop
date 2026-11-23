//! The user's preferences, a small JSON file at
//! `$XDG_CONFIG_HOME/hardcover-desktop/settings.json` that survives signing out.
//! Besides the Settings dialog's choices it keeps the series hidden from the
//! Series page: Hardcover has no such thing, so this file is their only copy.
//! It also keeps the books left out of a vibe ("Never recommend…") until
//! Hardcover's list catches up ([`VibeExclusion`]).
//!
//! Reading never fails: a missing, unreadable or garbled file gives the
//! defaults. Each field is read on its own: one the file lacks (an older
//! version wrote it) or holds a value this version doesn't know (a newer one
//! did) takes its default, and the others keep theirs. Writes go to a
//! temporary file that is renamed into place, so a crash never leaves half a
//! file.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::{BookId, Result, SeriesId, Timestamp, Vibe, VibeId};

const PREFERENCES_FILE: &str = "settings.json";

/// The colour scheme the app uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the desktop's light or dark appearance.
    #[default]
    System,
    Light,
    Dark,
}

/// Choices made in the app's Settings. Deserialized field by field (see the
/// module doc): add a new field to `Deserialize` below too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preferences {
    pub theme: ThemePreference,
    /// Check GitHub for new releases on start and every few hours.
    pub automatic_update_checks: bool,
    /// The series hidden from the Series page ("Hide series"), kept sorted.
    pub hidden_series: BTreeSet<SeriesId>,
    /// Books left out of a vibe, hidden until Hardcover rebuilds it.
    pub vibe_exclusions: Vec<VibeExclusion>,
}

/// A book the user left out of a vibe ("Never recommend…"). Hardcover drops
/// it from the vibe's books only when it next builds the vibe, which takes
/// a visit to hardcover.app, so the app hides it from every vibe built
/// before the exclusion ([`VibeExclusion::hides`]) and forgets the entry
/// once the vibe it was sent to is built after it
/// ([`VibeExclusion::is_settled`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VibeExclusion {
    pub book: BookId,
    /// The vibe the exclusion was written to.
    pub vibe: VibeId,
    /// When the write landed.
    pub at: Timestamp,
}

impl VibeExclusion {
    /// Whether the book is hidden from `vibe`: any vibe Hardcover hasn't
    /// built since the exclusion may still list it.
    #[must_use]
    pub fn hides(&self, vibe: &Vibe) -> bool {
        vibe.built_at.is_none_or(|built| built <= self.at)
    }

    /// Whether Hardcover has caught up: the vibe written to, as fetched,
    /// was built after the exclusion. A vibe not loaded settles nothing.
    #[must_use]
    pub fn is_settled(&self, vibe: &Vibe) -> bool {
        vibe.id == self.vibe && vibe.built_at.is_some_and(|built| built > self.at)
    }
}

/// `vibe`'s books less those `exclusions` hide, in the vibe's order.
#[must_use]
pub fn shown_books(vibe: &Vibe, exclusions: &[VibeExclusion]) -> Vec<BookId> {
    vibe.book_ids
        .iter()
        .copied()
        .filter(|book| !exclusions.iter().any(|e| e.book == *book && e.hides(vibe)))
        .collect()
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            automatic_update_checks: true,
            hidden_series: BTreeSet::new(),
            vibe_exclusions: Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for Preferences {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = Map::<String, Value>::deserialize(deserializer)?;
        let defaults = Self::default();
        Ok(Self {
            theme: field(&map, "theme", defaults.theme),
            automatic_update_checks: field(
                &map,
                "automatic_update_checks",
                defaults.automatic_update_checks,
            ),
            hidden_series: field(&map, "hidden_series", defaults.hidden_series),
            vibe_exclusions: field(&map, "vibe_exclusions", defaults.vibe_exclusions),
        })
    }
}

/// `name`'s value in `map`, or `default` if it is missing or not a `T`.
fn field<T: DeserializeOwned>(map: &Map<String, Value>, name: &str, default: T) -> T {
    map.get(name)
        .and_then(|v| T::deserialize(v).ok())
        .unwrap_or(default)
}

impl Preferences {
    /// `$XDG_CONFIG_HOME/hardcover-desktop/settings.json`.
    ///
    /// # Errors
    ///
    /// Returns an error if the user configuration directory cannot be located.
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::project_dirs()?.config_dir().join(PREFERENCES_FILE))
    }

    /// Blocking. The saved preferences, or the defaults.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        read_json(path).unwrap_or_default()
    }

    /// Blocking. Creates the directory if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the preferences cannot be serialized or the file cannot be
    /// replaced.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_json(path, self)
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("hardcover: could not read {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("hardcover: ignoring {}: {e}", path.display());
            None
        }
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    // Unique per write, so overlapping writes never share a temporary file.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let dir = path
        .parent()
        .ok_or_else(|| crate::Error::Storage(format!("{} has no parent", path.display())))?;
    fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| crate::Error::Storage(format!("could not encode settings: {e}")))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let serial = WRITES.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".{name}.{}-{serial}.tmp", std::process::id()));
    let written = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(&json)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    Ok(written?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn missing_files_give_defaults() {
        let dir = temp();
        assert_eq!(
            Preferences::load(&dir.path().join("none.json")),
            Preferences::default()
        );
    }

    #[test]
    fn garbled_files_give_defaults() {
        let dir = temp();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        fs::write(&path, b"[1, 2]").unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
    }

    /// A value this version doesn't know (a newer one wrote it) resets that
    /// field only: automatic updates stay off.
    #[test]
    fn an_unknown_value_resets_only_its_own_field() {
        let dir = temp();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"theme": "sepia", "automatic_update_checks": false}"#,
        )
        .unwrap();
        assert_eq!(
            Preferences::load(&path),
            Preferences {
                theme: ThemePreference::System,
                automatic_update_checks: false,
                ..Preferences::default()
            }
        );
        fs::write(
            &path,
            br#"{"theme": "dark", "automatic_update_checks": "sometimes"}"#,
        )
        .unwrap();
        assert_eq!(
            Preferences::load(&path),
            Preferences {
                theme: ThemePreference::Dark,
                automatic_update_checks: true,
                ..Preferences::default()
            }
        );
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_fields_are_ignored() {
        let dir = temp();
        let path = dir.path().join("settings.json");
        fs::write(&path, br#"{"theme": "dark", "from_the_future": 1}"#).unwrap();
        assert_eq!(
            Preferences::load(&path),
            Preferences {
                theme: ThemePreference::Dark,
                automatic_update_checks: true,
                ..Preferences::default()
            }
        );
    }

    #[test]
    fn preferences_round_trip_and_create_the_directory() {
        let dir = temp();
        let path = dir.path().join("nested/dir/settings.json");
        let prefs = Preferences {
            theme: ThemePreference::Light,
            automatic_update_checks: false,
            hidden_series: [SeriesId::new(7), SeriesId::new(3)].into(),
            vibe_exclusions: vec![VibeExclusion {
                book: BookId::new(12),
                vibe: VibeId::new(4),
                at: "2026-11-23T10:00:00Z".parse().unwrap(),
            }],
        };
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), prefs);
        // Overwrites in place, leaving no temporary files behind.
        Preferences::default().save(&path).unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        let entries = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(entries, 1);
    }

    /// The hidden series survive a save, and a garbled list resets only
    /// itself.
    #[test]
    fn hidden_series_are_kept_and_a_bad_list_resets_only_itself() {
        let dir = temp();
        let path = dir.path().join("settings.json");
        fs::write(&path, br#"{"theme": "dark", "hidden_series": [12, 4, 12]}"#).unwrap();
        let loaded = Preferences::load(&path);
        assert_eq!(loaded.theme, ThemePreference::Dark);
        assert_eq!(
            loaded.hidden_series.into_iter().collect::<Vec<_>>(),
            [SeriesId::new(4), SeriesId::new(12)]
        );
        fs::write(&path, br#"{"theme": "dark", "hidden_series": "all"}"#).unwrap();
        let loaded = Preferences::load(&path);
        assert_eq!(loaded.theme, ThemePreference::Dark);
        assert!(loaded.hidden_series.is_empty());
    }

    fn vibe(id: i64, built: Option<&str>, books: &[i64]) -> Vibe {
        let mut v = Vibe::new(VibeId::new(id), crate::VibeKind::Recommendations, "Recs");
        v.built_at = built.map(|b| b.parse().unwrap());
        v.book_ids = books.iter().copied().map(BookId::new).collect();
        v
    }

    /// An excluded book stays hidden from every vibe built before the
    /// exclusion, and the entry goes once its own vibe is built after it.
    #[test]
    fn an_excluded_book_hides_until_its_vibe_is_rebuilt() {
        let excluded = VibeExclusion {
            book: BookId::new(2),
            vibe: VibeId::new(1),
            at: "2026-11-23T10:00:00Z".parse().unwrap(),
        };
        let exclusions = [excluded.clone()];
        let ids = |books: Vec<BookId>| books.into_iter().map(BookId::get).collect::<Vec<_>>();

        // Built before the exclusion, or at the same moment: hidden.
        let before = vibe(1, Some("2026-11-22T08:00:00Z"), &[1, 2, 3]);
        assert_eq!(ids(shown_books(&before, &exclusions)), [1, 3]);
        assert!(!excluded.is_settled(&before));
        let same = vibe(1, Some("2026-11-23T10:00:00Z"), &[1, 2, 3]);
        assert!(excluded.hides(&same) && !excluded.is_settled(&same));
        // Another vibe built before it hides the book too, and settles
        // nothing.
        let other = vibe(7, Some("2026-11-24T08:00:00Z"), &[2, 5]);
        assert_eq!(ids(shown_books(&other, &exclusions)), [2, 5], "built since");
        assert!(!excluded.is_settled(&other));
        let other_old = vibe(7, Some("2026-11-01T08:00:00Z"), &[2, 5]);
        assert_eq!(ids(shown_books(&other_old, &exclusions)), [5]);

        // Rebuilt after it: Hardcover's list is what counts, and the entry
        // can go.
        let rebuilt = vibe(1, Some("2026-11-23T10:00:01Z"), &[1, 3, 4]);
        assert!(!excluded.hides(&rebuilt));
        assert!(excluded.is_settled(&rebuilt));
        assert_eq!(ids(shown_books(&rebuilt, &exclusions)), [1, 3, 4]);
    }

    #[test]
    fn a_bad_exclusion_list_resets_only_itself() {
        let dir = temp();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"theme": "dark", "vibe_exclusions": [{"book": 3}]}"#,
        )
        .unwrap();
        let loaded = Preferences::load(&path);
        assert_eq!(loaded.theme, ThemePreference::Dark);
        assert!(loaded.vibe_exclusions.is_empty());
    }
}
