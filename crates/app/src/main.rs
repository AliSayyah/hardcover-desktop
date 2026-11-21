//! Hardcover Desktop: a Linux client for hardcover.app.

mod about;
mod app_state;
mod bidi;
mod book;
mod catalog;
mod chip;
mod cover;
#[cfg(test)]
mod cover_quality_tests;
mod design;
mod fonts;
mod format;
mod freshness;
mod goals;
mod home;
mod icons;
mod library;
mod lists;
mod preferences;
mod search;
mod series;
mod settings_view;
mod shell;
mod sign_in;
mod star;
mod stats;
mod ui;
mod upcoming;
mod update;

use gpui_kit::{
    App, AppContext as _, KeyBinding, Size, TitlebarOptions, WindowBounds, WindowOptions, actions,
};

use hardcover_core::update::InstallKind;

use crate::book::star_rating;
use crate::search::search_view;

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
        ShowIgnored,
        OpenSettings,
        ShowAbout,
        GoBack,
        ShowDiscover,
        ShowSeries,
        ShowStats,
        ShowHome,
        RemoveFromList,
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
        KeyBinding::new("ctrl-6", ShowIgnored, None),
        KeyBinding::new("ctrl-shift-d", ShowDiscover, None),
        // Ctrl+Shift+E selects to the end of the line in a text field.
        KeyBinding::new("ctrl-shift-r", ShowSeries, None),
        KeyBinding::new("ctrl-shift-s", ShowStats, None),
        KeyBinding::new("ctrl-0", ShowHome, None),
        // Down in Search's query field moves into the results.
        KeyBinding::new(
            "down",
            search_view::EnterResults,
            Some(&format!("{} > Input", search_view::KEY_CONTEXT)),
        ),
        KeyBinding::new("alt-left", GoBack, Some("Workspace")),
        // On a catalogue page; lists, menus and fields inside it use Escape first.
        KeyBinding::new("escape", GoBack, Some(shell::PAGE_CONTEXT)),
        KeyBinding::new(
            "delete",
            RemoveFromList,
            Some(lists::list_view::KEY_CONTEXT),
        ),
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

/// `None` if another copy is running: it was asked to show its window, and
/// this one should exit. `Some(None)` if the check itself failed: the app
/// runs anyway, as it did before there was one.
///
/// The demo build shares nothing with a real one (no keyring, no cache, no
/// settings), so it never claims the instance and runs beside it.
#[cfg(not(feature = "demo"))]
fn claim_instance() -> Option<Option<hardcover_core::instance::Primary>> {
    use hardcover_core::instance::{self, Claim};
    match instance::claim() {
        Ok(Claim::Primary(primary)) => Some(Some(primary)),
        Ok(Claim::Running { shown }) => {
            if shown {
                eprintln!("hardcover: already running; showing that window");
            } else {
                eprintln!("hardcover: already running, but it didn't answer");
            }
            None
        }
        Err(e) => {
            eprintln!("hardcover: {e}; starting anyway");
            Some(None)
        }
    }
}

/// Answers later launches for the rest of the run by activating the window.
/// On Wayland the compositor decides: without a recent input event in this
/// app it may only mark the window as wanting attention.
#[cfg(not(feature = "demo"))]
fn show_on_later_launch(primary: hardcover_core::instance::Primary, cx: &mut App) {
    let (shown, requests) = async_channel::unbounded::<()>();
    primary.serve(move || {
        let _ = shown.send_blocking(());
    });
    cx.spawn(async move |cx| {
        while requests.recv().await.is_ok() {
            cx.update(|cx| {
                if let Some(window) = cx.windows().into_iter().next() {
                    let _ = window.update(cx, |_, window, _| window.activate_window());
                }
            });
        }
    })
    .detach();
}

fn main() {
    if std::env::args()
        .skip(1)
        .any(|a| a == "--version" || a == "-V")
    {
        println!("hardcover-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // One copy per data directory: a later launch shows this one's window
    // and exits. Claimed before the event loop (like the preferences below):
    // a second copy must not get as far as the keyring or the cache.
    #[cfg(not(feature = "demo"))]
    let Some(instance) = claim_instance() else {
        return;
    };
    // Resolved before anything else: once an update replaces the binary,
    // /proc/self/exe names the deleted file, and restarting must use this path.
    let exe = std::env::current_exe().ok();
    // A small file, read before the event loop starts (the one exception to
    // keeping core calls off the UI thread): the first frame needs the theme.
    let (prefs, prefs_path) = preferences::load();
    gpui_kit::application()
        .with_assets(icons::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let bundled_fonts = fonts::init(cx);
            design::theme::init(bundled_fonts, cx);
            bind_keys(cx);
            ui::kit::bind_keys(cx);
            let install = exe
                .as_deref()
                .map_or(InstallKind::Other, InstallKind::detect);
            // An AppImage restarts from its file: the executable inside it is
            // gone once this process exits.
            if let Some(path) = install.restart_path().or(exe.as_deref()) {
                cx.set_restart_path(path.to_owned());
            }
            preferences::init(prefs.clone(), prefs_path.clone(), cx);
            update::init(install, cx);
            #[cfg(not(feature = "demo"))]
            if let Some(primary) = instance {
                show_on_later_launch(primary, cx);
            }

            // Window geometry is a platform boundary: logical pixels, not rems.
            let size = Size::new(gpui_kit::px(1200.), gpui_kit::px(800.));
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size, cx)),
                window_min_size: Some(Size::new(gpui_kit::px(800.), gpui_kit::px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Hardcover".into()),
                    ..Default::default()
                }),
                app_id: Some(APP_ID.to_owned()),
                ..Default::default()
            };
            // The demo build can show the shared components instead of the
            // app (`ui::gallery`).
            #[cfg(feature = "demo")]
            if std::env::var_os("HARDCOVER_GALLERY").is_some() {
                gpui_kit::open_window(options, cx, |window, cx| {
                    preferences::follow_theme(window, cx);
                    cx.new(|cx| ui::gallery::Gallery::new(window, cx))
                })
                .expect("failed to open window");
                return;
            }
            gpui_kit::open_window(options, cx, |window, cx| {
                preferences::follow_theme(window, cx);
                cx.new(|cx| shell::Shell::new(window, cx))
            })
            .expect("failed to open window");
        });
}
