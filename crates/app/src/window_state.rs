//! Remembers the main window's size.
//!
//! The window opens centred: Wayland neither reports a window's position nor
//! lets a client place it. [`WindowStateSaver`] watches the window's bounds,
//! and writes the file in the background shortly after the last change, and
//! again when the window closes or the app quits.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Bounds, Pixels, Size, Task, Window, WindowBounds, px};
use hardcover_core::prefs::WindowState;

/// How long the bounds must stay put before they are written. Resizing
/// changes them on every frame.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// Blocking: reads the saved window state. Runs in `main` before the window opens.
pub fn load() -> (WindowState, Option<PathBuf>) {
    match WindowState::default_path() {
        Ok(path) => (WindowState::load(&path), Some(path)),
        Err(e) => {
            eprintln!("hardcover: window state unavailable: {e}");
            (WindowState::default(), None)
        }
    }
}

/// Where the window opens: the saved size, no larger than the display, centred.
pub fn initial_bounds(state: WindowState, cx: &App) -> WindowBounds {
    let mut size = Size::new(px(state.width), px(state.height));
    if let Some(display) = cx.primary_display() {
        let available = display.bounds().size;
        size.width = size.width.min(available.width);
        size.height = size.height.min(available.height);
    }
    WindowBounds::Windowed(Bounds::centered(None, size, cx))
}

/// The smallest window the layout supports.
pub fn min_size() -> Size<Pixels> {
    Size::new(px(WindowState::MIN_WIDTH), px(WindowState::MIN_HEIGHT))
}

/// The state worth saving, or `None` in fullscreen, which is a mode for one
/// session rather than a size to come back to.
///
/// `bounds` is GPUI's view (the restore size when maximized), `current` the
/// window's actual size and `display` the size of its display. A maximized
/// window saves its restore size, so it reopens as the window it was before.
/// Some compositors report a window as maximized when it isn't (Hyprland keeps the
/// maximized state on tiled windows, even after floating them), so a
/// "maximized" window counts as one only when it fills its display.
fn state_for(
    bounds: WindowBounds,
    current: Size<Pixels>,
    display: Option<Size<Pixels>>,
) -> Option<WindowState> {
    let fills_display = display.is_none_or(|display| {
        current.width >= display.width * 0.9 && current.height >= display.height * 0.8
    });
    let size = match bounds {
        WindowBounds::Fullscreen(_) => return None,
        WindowBounds::Maximized(restore) if fills_display => restore.size,
        WindowBounds::Maximized(_) | WindowBounds::Windowed(_) => current,
    };
    Some(
        WindowState {
            width: f32::from(size.width),
            height: f32::from(size.height),
        }
        .sanitized(),
    )
}

/// Saves the window state as it changes. Owned by the window's root view.
pub struct WindowStateSaver {
    path: Option<Arc<Path>>,
    /// The newest state, written or about to be.
    latest: WindowState,
    _pending: Option<Task<()>>,
}

impl WindowStateSaver {
    pub fn new(path: Option<PathBuf>, saved: WindowState) -> Self {
        Self {
            path: path.map(Arc::from),
            latest: saved,
            _pending: None,
        }
    }

    /// Call when the window's bounds change; schedules a write.
    pub fn bounds_changed(&mut self, window: &Window, cx: &App) {
        let display = window.display(cx).map(|display| display.bounds().size);
        let Some(state) = state_for(window.window_bounds(), window.bounds().size, display) else {
            return;
        };
        if state == self.latest {
            return;
        }
        self.latest = state;
        let Some(path) = self.path.clone() else {
            return;
        };
        let executor = cx.background_executor().clone();
        // Replaces (and so cancels) any write still waiting.
        self._pending = Some(cx.background_spawn(async move {
            executor.timer(SAVE_DELAY).await;
            save(&path, state);
        }));
    }

    /// Writes the newest state if a write may still be waiting, blocking. For
    /// quitting and closing, when a delayed write would never run.
    pub fn flush(&mut self) {
        if self._pending.take().is_some()
            && let Some(path) = &self.path
        {
            save(path, self.latest);
        }
    }
}

fn save(path: &Path, state: WindowState) {
    if let Err(e) = state.save(path) {
        eprintln!("hardcover: could not save the window size: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::point;

    fn size(width: f32, height: f32) -> Size<Pixels> {
        Size::new(px(width), px(height))
    }

    fn bounds(width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(width, height))
    }

    fn state(width: f32, height: f32) -> Option<WindowState> {
        Some(WindowState { width, height })
    }

    const DISPLAY: Option<Size<Pixels>> = Some(Size {
        width: px(2560.),
        height: px(1440.),
    });

    #[test]
    fn a_window_saves_its_size() {
        assert_eq!(
            state_for(
                WindowBounds::Windowed(bounds(1000., 700.)),
                size(1000., 700.),
                DISPLAY
            ),
            state(1000., 700.)
        );
    }

    #[test]
    fn a_maximized_window_saves_its_restore_size() {
        assert_eq!(
            state_for(
                WindowBounds::Maximized(bounds(1300., 900.)),
                size(2536., 1390.),
                DISPLAY
            ),
            state(1300., 900.)
        );
    }

    #[test]
    fn a_window_reported_maximized_that_does_not_fill_the_display_saves_its_size() {
        assert_eq!(
            state_for(
                WindowBounds::Maximized(bounds(2560., 1440.)),
                size(1400., 900.),
                DISPLAY
            ),
            state(1400., 900.)
        );
    }

    #[test]
    fn fullscreen_is_not_saved() {
        assert_eq!(
            state_for(
                WindowBounds::Fullscreen(bounds(2560., 1440.)),
                size(2560., 1440.),
                DISPLAY
            ),
            None
        );
    }

    #[test]
    fn tiny_windows_are_saved_at_the_minimum() {
        assert_eq!(
            state_for(
                WindowBounds::Windowed(bounds(10., 10.)),
                size(10., 10.),
                None
            ),
            state(WindowState::MIN_WIDTH, WindowState::MIN_HEIGHT)
        );
    }
}
