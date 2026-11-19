//! Hardcover Desktop: a Linux client for hardcover.app.

mod about;
mod app_state;
mod book;
mod format;
mod library;
mod preferences;
mod search;
mod settings_view;
mod shell;
mod sign_in;
mod update;
mod window_state;

use gpui_kit::{App, AppContext as _, KeyBinding, TitlebarOptions, WindowOptions, actions};

use crate::book::star_rating;

actions!(
    hardcover,
    [
        Quit,
        Refresh,
        FocusSearch,
        ShowCurrentlyReading,
        ShowWantToRead,
        ShowRead,
        ShowPaused,
        ShowDidNotFinish,
        OpenSettings,
        ShowAbout,
    ]
);

const APP_ID: &str = "io.github.alisayyah.HardcoverDesktop";

/// Must run before any menu is built (none yet), and after `gpui_kit::init`
/// so these bindings win over the components' own `ctrl-f`.
fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-r", Refresh, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-f", FocusSearch, None),
        // Input binds ctrl-f to its own search; inside a text field it still
        // means "go to Search".
        KeyBinding::new("ctrl-f", FocusSearch, Some("Input")),
        KeyBinding::new("ctrl-1", ShowCurrentlyReading, None),
        KeyBinding::new("ctrl-2", ShowWantToRead, None),
        KeyBinding::new("ctrl-3", ShowRead, None),
        KeyBinding::new("ctrl-4", ShowPaused, None),
        KeyBinding::new("ctrl-5", ShowDidNotFinish, None),
        KeyBinding::new(
            "right",
            star_rating::IncreaseRating,
            Some(star_rating::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "up",
            star_rating::IncreaseRating,
            Some(star_rating::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "left",
            star_rating::DecreaseRating,
            Some(star_rating::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "down",
            star_rating::DecreaseRating,
            Some(star_rating::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "backspace",
            star_rating::ClearRating,
            Some(star_rating::KEY_CONTEXT),
        ),
        KeyBinding::new(
            "delete",
            star_rating::ClearRating,
            Some(star_rating::KEY_CONTEXT),
        ),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

fn main() {
    if std::env::args()
        .skip(1)
        .any(|a| a == "--version" || a == "-V")
    {
        println!("hardcover-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // Resolved before anything else: once an update replaces the binary,
    // /proc/self/exe names the deleted file, and restarting must use this path.
    let exe = std::env::current_exe().ok();
    // Two small files, read before the event loop starts (the one exception
    // to keeping core calls off the UI thread): the first frame needs the
    // theme and the window size.
    let (prefs, prefs_path) = preferences::load();
    let (window_state, window_state_path) = window_state::load();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            bind_keys(cx);
            if let Some(exe) = &exe {
                cx.set_restart_path(exe.clone());
            }
            preferences::init(prefs.clone(), prefs_path.clone(), cx);
            update::init(exe.clone(), cx);

            let options = WindowOptions {
                window_bounds: Some(window_state::initial_bounds(window_state, cx)),
                window_min_size: Some(window_state::min_size()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Hardcover".into()),
                    ..Default::default()
                }),
                app_id: Some(APP_ID.to_owned()),
                ..Default::default()
            };
            let saver =
                window_state::WindowStateSaver::new(window_state_path.clone(), window_state);
            gpui_kit::open_window(options, cx, |window, cx| {
                preferences::follow_theme(window, cx);
                cx.new(|cx| shell::Shell::new(saver, window, cx))
            })
            .expect("failed to open window");
        });
}
