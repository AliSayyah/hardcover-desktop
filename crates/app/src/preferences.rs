//! The user's preferences: the app-wide [`PreferencesModel`] that Settings
//! edits, and the colour scheme that follows from it.
//!
//! Preferences are read once in `main`, before the window opens, so the first
//! frame already has the right theme and size. Every later write runs in the
//! background.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Window, WindowAppearance};
use hardcover_core::prefs::{Preferences, ThemePreference};

/// `HARDCOVER_THEME=light|dark` forces a theme (a development aid for
/// screenshots). Settings shows the theme choice as unavailable while it is set.
const THEME_ENV: &str = "HARDCOVER_THEME";

pub struct PreferencesModel {
    preferences: Preferences,
    /// `None` when there is no home directory; changes then last for this run.
    path: Option<Arc<Path>>,
    /// The newest preferences, for the background writes to pick up.
    unsaved: Arc<Mutex<Preferences>>,
    /// Held while writing: each write saves whatever is newest when it gets
    /// the lock, so the last write to finish always saves the newest.
    write_lock: Arc<Mutex<()>>,
}

/// The app-wide [`PreferencesModel`].
pub struct AppPreferences(pub Entity<PreferencesModel>);

impl Global for AppPreferences {}

impl AppPreferences {
    pub fn global(cx: &App) -> &Entity<PreferencesModel> {
        &cx.global::<Self>().0
    }
}

/// Blocking: reads the preferences file. Runs in `main` before the window opens.
pub fn load() -> (Preferences, Option<PathBuf>) {
    match Preferences::default_path() {
        Ok(path) => (Preferences::load(&path), Some(path)),
        Err(e) => {
            eprintln!("hardcover: preferences unavailable: {e}");
            (Preferences::default(), None)
        }
    }
}

pub fn init(preferences: Preferences, path: Option<PathBuf>, cx: &mut App) {
    let model = cx.new(|_| PreferencesModel {
        unsaved: Arc::new(Mutex::new(preferences.clone())),
        preferences,
        path: path.map(Arc::from),
        write_lock: Arc::default(),
    });
    cx.set_global(AppPreferences(model));
}

impl PreferencesModel {
    pub fn theme(&self) -> ThemePreference {
        self.preferences.theme
    }

    pub fn automatic_update_checks(&self) -> bool {
        self.preferences.automatic_update_checks
    }

    pub fn set_theme(&mut self, theme: ThemePreference, cx: &mut Context<Self>) {
        self.update(cx, |p| p.theme = theme);
    }

    pub fn set_automatic_update_checks(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.update(cx, |p| p.automatic_update_checks = enabled);
    }

    fn update(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Preferences)) {
        let before = self.preferences.clone();
        change(&mut self.preferences);
        if self.preferences == before {
            return;
        }
        cx.notify();
        let Some(path) = self.path.clone() else {
            return;
        };
        *lock(&self.unsaved) = self.preferences.clone();
        let unsaved = self.unsaved.clone();
        let write_lock = self.write_lock.clone();
        cx.background_spawn(async move {
            let _writing = lock(&write_lock);
            let preferences = lock(&unsaved).clone();
            if let Err(e) = preferences.save(&path) {
                eprintln!("hardcover: could not save preferences: {e}");
            }
        })
        .detach();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

/// The theme forced by `HARDCOVER_THEME`, if any.
pub fn theme_override() -> Option<ThemePreference> {
    match std::env::var(THEME_ENV).ok().as_deref() {
        Some("light") => Some(ThemePreference::Light),
        Some("dark") => Some(ThemePreference::Dark),
        _ => None,
    }
}

/// The light or dark mode for `preference` when the desktop looks like `system`.
pub fn theme_mode(preference: ThemePreference, system: WindowAppearance) -> ThemeMode {
    match preference {
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
        ThemePreference::System => match system {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
        },
    }
}

/// Applies the theme to every window: `HARDCOVER_THEME`, else the preference.
pub fn apply_theme(window: &mut Window, cx: &mut App) {
    let preference =
        theme_override().unwrap_or_else(|| AppPreferences::global(cx).read(cx).theme());
    let mode = theme_mode(preference, window.appearance());
    // Theme::change refreshes every window even when nothing changes.
    if cx.try_global::<Theme>().map(|theme| theme.mode) != Some(mode) {
        Theme::change(mode, Some(window), cx);
    }
}

/// Applies the theme now, and again whenever the desktop's appearance or the
/// preference changes.
pub fn follow_theme(window: &mut Window, cx: &mut App) {
    apply_theme(window, cx);
    window.observe_window_appearance(apply_theme).detach();
    let preferences = AppPreferences::global(cx).clone();
    window
        .observe(&preferences, cx, |_, window, cx| apply_theme(window, cx))
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_choice_ignores_the_desktop() {
        for system in [WindowAppearance::Light, WindowAppearance::Dark] {
            assert_eq!(theme_mode(ThemePreference::Light, system), ThemeMode::Light);
            assert_eq!(theme_mode(ThemePreference::Dark, system), ThemeMode::Dark);
        }
    }

    #[test]
    fn system_follows_the_desktop() {
        assert_eq!(
            theme_mode(ThemePreference::System, WindowAppearance::Dark),
            ThemeMode::Dark
        );
        assert_eq!(
            theme_mode(ThemePreference::System, WindowAppearance::VibrantLight),
            ThemeMode::Light
        );
    }
}
