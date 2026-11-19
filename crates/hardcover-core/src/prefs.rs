//! Small JSON files for the app's own state: the user's preferences and the
//! main window's size.
//!
//! - [`Preferences`] live in `$XDG_CONFIG_HOME/hardcover-desktop/settings.json`.
//! - [`WindowState`] lives in `$XDG_STATE_HOME/hardcover-desktop/window.json`.
//!
//! Both survive signing out. Reading never fails: a missing, unreadable or
//! garbled file gives the defaults, and fields a file lacks (an older version
//! wrote it) take their default values. Writes go to a temporary file that is
//! renamed into place, so a crash never leaves half a file.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Result;

const PREFERENCES_FILE: &str = "settings.json";
const WINDOW_STATE_FILE: &str = "window.json";

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

/// Choices made in the app's Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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

impl Preferences {
    /// `$XDG_CONFIG_HOME/hardcover-desktop/settings.json`.
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::project_dirs()?.config_dir().join(PREFERENCES_FILE))
    }

    /// Blocking. The saved preferences, or the defaults.
    pub fn load(path: &Path) -> Self {
        read_json(path).unwrap_or_default()
    }

    /// Blocking. Creates the directory if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_json(path, self)
    }
}

/// The main window's size, in logical pixels.
///
/// There is no position: Wayland doesn't tell clients where their windows
/// are, nor let them choose, so the window opens centred. There is no
/// maximized state either: tiling compositors report tiled windows as
/// maximized, and asking them to maximize a new window covers the others.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1200.,
            height: 800.,
        }
    }
}

impl WindowState {
    /// The smallest window the layout supports.
    pub const MIN_WIDTH: f32 = 800.;
    pub const MIN_HEIGHT: f32 = 560.;
    /// Larger than any real display; anything bigger is a garbled file.
    const MAX_SIDE: f32 = 16_384.;

    /// `$XDG_STATE_HOME/hardcover-desktop/window.json`, or the data directory
    /// where there is no state directory.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = crate::project_dirs()?;
        let dir = dirs.state_dir().unwrap_or_else(|| dirs.data_dir());
        Ok(dir.join(WINDOW_STATE_FILE))
    }

    /// Blocking. The saved state, or the default size.
    pub fn load(path: &Path) -> Self {
        read_json::<Self>(path).unwrap_or_default().sanitized()
    }

    /// Blocking. Creates the directory if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_json(path, &self.sanitized())
    }

    /// A size the window can actually open at: finite, no smaller than the
    /// minimum and no larger than any display. Garbage gives the default.
    pub fn sanitized(self) -> Self {
        let side = |value: f32, min: f32, default: f32| {
            if value.is_finite() && value > 0. {
                value.clamp(min, Self::MAX_SIDE)
            } else {
                default
            }
        };
        let default = Self::default();
        Self {
            width: side(self.width, Self::MIN_WIDTH, default.width),
            height: side(self.height, Self::MIN_HEIGHT, default.height),
        }
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
    let dir = path
        .parent()
        .ok_or_else(|| crate::Error::Storage(format!("{} has no parent", path.display())))?;
    fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| crate::Error::Storage(format!("could not encode settings: {e}")))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // Unique per write, so overlapping writes never share a temporary file.
    static WRITES: AtomicU64 = AtomicU64::new(0);
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
        assert_eq!(
            WindowState::load(&dir.path().join("none.json")),
            WindowState::default()
        );
    }

    #[test]
    fn garbled_files_give_defaults() {
        let dir = temp();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        fs::write(&path, br#"{"theme": "sepia"}"#).unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        fs::write(&path, br#"{"width": "wide"}"#).unwrap();
        assert_eq!(WindowState::load(&path), WindowState::default());
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

    #[test]
    fn window_state_round_trips() {
        let dir = temp();
        let path = dir.path().join("window.json");
        let state = WindowState {
            width: 1440.,
            height: 900.,
        };
        state.save(&path).unwrap();
        assert_eq!(WindowState::load(&path), state);
    }

    #[test]
    fn window_sizes_are_clamped_to_something_openable() {
        let state = |width, height| WindowState { width, height };
        assert_eq!(state(100., 100.).sanitized(), state(800., 560.));
        assert_eq!(state(1e9, 900.).sanitized(), state(16_384., 900.));
        assert_eq!(
            state(f32::NAN, -5.).sanitized(),
            state(1200., 800.),
            "garbage falls back to the default size"
        );
        assert_eq!(state(f32::INFINITY, 0.).sanitized(), state(1200., 800.));
    }
}
