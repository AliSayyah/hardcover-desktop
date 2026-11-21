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
mod ui_scale;
mod upcoming;
mod update;

use gpui_kit::{
    App, AppContext as _, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, actions,
};

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
    // The outputs' sizes, read on a thread of their own while the app starts:
    // the window's size depends on them (`ui_scale`).
    let scale = ui_scale::Probe::start();
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
            // An AppImage's restart path replaces this once the first update
            // check has worked out how the app was installed (`update`).
            if let Some(path) = &exe {
                cx.set_restart_path(path.clone());
            }
            preferences::init(prefs.clone(), prefs_path.clone(), cx);
            update::init(exe.clone(), cx);

            // Window geometry is a platform boundary: logical pixels, not rems.
            // The design's 1200 × 800 and 800 × 560, at the factor of the
            // monitor the window is expected on (`ui_scale::policy::opening`).
            let scale = scale.finish();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(scale.window_size(), cx)),
                window_min_size: Some(scale.min_window_size()),
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
                    ui_scale::follow(scale, window, cx);
                    cx.new(|cx| ui::gallery::Gallery::new(window, cx))
                })
                .expect("failed to open window");
                return;
            }
            gpui_kit::open_window(options, cx, |window, cx| {
                preferences::follow_theme(window, cx);
                ui_scale::follow(scale, window, cx);
                cx.new(|cx| shell::Shell::new(window, cx))
            })
            .expect("failed to open window");
        });
}
