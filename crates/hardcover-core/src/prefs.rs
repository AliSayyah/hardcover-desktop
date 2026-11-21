//! The user's preferences, a small JSON file at
//! `$XDG_CONFIG_HOME/hardcover-desktop/settings.json` that survives signing out.
//!
//! Reading never fails: a missing, unreadable or garbled file gives the
//! defaults. Each field is read on its own: one the file lacks (an older
//! version wrote it) or holds a value this version doesn't know (a newer one
//! did) takes its default, and the others keep theirs. Writes go to a
//! temporary file that is renamed into place, so a crash never leaves half a
//! file.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::Result;

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
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            automatic_update_checks: true,
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
        };
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), prefs);
        // Overwrites in place, leaving no temporary files behind.
        Preferences::default().save(&path).unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        let entries = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(entries, 1);
    }
}
