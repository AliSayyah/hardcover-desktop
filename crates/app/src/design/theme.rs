//! The palette applied to gpui-component's theme, so the kit's components
//! draw in paper and ink.
//!
//! **How:** the app builds its own light and dark `ThemeConfig` from
//! [`super::color`] and installs them as the theme's `light_theme` and
//! `dark_theme`. `Theme::change` loads the config of the mode it switches
//! to, so the palette holds through every mode change: the Settings choice,
//! the desktop's appearance and `HARDCOVER_THEME` all go through
//! `preferences::apply_theme`, which calls `Theme::change`. Both configs are
//! the chosen theme's ([`ThemeFamily`]); [`set_theme_family`] swaps them,
//! and records the choice for [`super::color::colors`].
//!
//! **Why a config, not overriding fields after `Theme::change`:** the kit
//! derives dozens of fields from the ones a config names (button and list
//! states, the danger family, scrollbars, tabs) and keeps a second copy of
//! each colour in `Theme::tokens`, which its components read. Applying a
//! config sets both copies and every derived field in one step, on every
//! `Theme::change`, wherever it is called from. Overriding fields afterwards
//! would have to repeat after each change, update both copies by hand, and
//! would leave derived fields on the kit's palette. The config is built in
//! Rust rather than loaded from JSON in `assets/`, so it reads the same
//! token values as the app's own views, and its field names are checked by
//! the compiler.
//!
//! **Mapping:** the skill's "Implementing in GPUI" table, plus:
//! - Overlays on controls that paint their own fill are blended to solid
//!   colours ([`super::color::solid`]): default buttons on `raised`,
//!   secondary buttons and tabs on `surface`, primary on `accent`. Rows,
//!   menus, ghost buttons and the sidebar take the translucent overlay.
//! - `accent` is the kit's hover fill for menu items and ghost buttons. In
//!   dark mode the kit halves it for ghost buttons, so there it is ink 10%:
//!   ghost buttons hover at 5% as the system says, and menu items at 10%.
//! - Danger buttons have no fill: danger text, the overlays on hover and
//!   press.
//! - `ring` is the solid accent: the kit draws its 3 px ring at 50% of it
//!   (the app's own is 2 px; the kit's width can't be set),
//!   plus a 1 px accent edge, as the system's focus treatment asks.
//! - `switch_thumb` is its own role: `raised` in light mode, `fg` in dark.
//! - `info` and `success` are `fg-muted`: no second accent on a
//!   notification's icon or a success check. `overlay` (the dialog
//!   backdrop) is `scrim`.
//! - `tab_bar` is the segmented bar's track, so a disabled segment reads
//!   as not chosen (the app has no other tab bar).
//!
//! **Where the kit doesn't follow the theme:**
//! - `Theme::input_background()` ignores the config: inputs are paper in
//!   light mode and a tint of the input border in dark. `ui::kit`'s
//!   `RaisedField` puts them on `raised`.
//! - Dialogs paint `background` (paper); `ui::kit::open_dialog` and
//!   `open_alert_dialog` put them on `raised`.
//! - The segmented tab bar's sliding indicator is always `background`,
//!   with the kit's small shadow.
//! - The rest (menus, the Select dropdown, switches, radios, the
//!   `DatePicker`, dialog shadows, focus rings) is listed in `ui::kit`'s
//!   module doc and the system's "Kit limits".

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, SharedString, Window};

use super::color::{ActiveFamily, Palette, ThemeFamily, channel_byte, solid};
use crate::fonts;

/// The radius of controls (`r-control`).
const RADIUS: u8 = 6;
/// The radius of rows, panels, popovers and dialogs (`r-box`).
const RADIUS_LG: u8 = 8;

/// Installs `family`'s light and dark configs, and applies the current
/// mode's. Runs after `gpui_kit::init` and `fonts::init`. `bundled_fonts`
/// says whether the fonts loaded; without them the theme keeps the system
/// font.
pub fn init(bundled_fonts: bool, family: ThemeFamily, cx: &mut App) {
    cx.set_global(ActiveFamily(family));
    let light = Rc::new(config(
        Palette::light(family),
        ThemeMode::Light,
        bundled_fonts,
    ));
    let dark = Rc::new(config(
        Palette::dark(family),
        ThemeMode::Dark,
        bundled_fonts,
    ));
    let mode = Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
        theme.focus_ring = true;
        theme.mode
    });
    // `Theme::update` doesn't reload a swapped config; `change` does, even
    // for the mode already showing.
    Theme::change(mode, None, cx);
}

/// Makes `family` the theme. The new configs keep the installed ones' font,
/// rem and radii (`set_ui_scale`), and the window is redrawn in it at once.
pub fn set_theme_family(family: ThemeFamily, window: Option<&mut Window>, cx: &mut App) {
    if cx.try_global::<ActiveFamily>().map(|active| active.0) == Some(family) {
        return;
    }
    cx.set_global(ActiveFamily(family));
    let mode = Theme::update(cx, |theme| {
        theme.light_theme = Rc::new(swapped(
            &theme.light_theme,
            Palette::light(family),
            ThemeMode::Light,
        ));
        theme.dark_theme = Rc::new(swapped(
            &theme.dark_theme,
            Palette::dark(family),
            ThemeMode::Dark,
        ));
        theme.mode
    });
    Theme::change(mode, window, cx);
}

/// `p`'s config with `installed`'s font, rem and radii.
fn swapped(installed: &ThemeConfig, p: &Palette, mode: ThemeMode) -> ThemeConfig {
    let mut config = config(p, mode, false);
    config.font_family.clone_from(&installed.font_family);
    config.font_size = installed.font_size;
    config.radius = installed.radius;
    config.radius_lg = installed.radius_lg;
    config
}

/// Draws the interface `k` times larger than the design (`ui_scale`): the rem
/// becomes `16 * k` px (gpui-component's `Root` sets the window's rem from the
/// theme's `font_size` on every frame), and the kit's radii, whole pixels in
/// the config, follow it. Both configs are rebuilt and the current mode
/// applied again, so the factor holds through every later `Theme::change`.
pub fn set_ui_scale(k: f32, window: &mut Window, cx: &mut App) {
    let mode = Theme::update(cx, |theme| {
        theme.light_theme = Rc::new(scaled(&theme.light_theme, k));
        theme.dark_theme = Rc::new(scaled(&theme.dark_theme, k));
        theme.mode
    });
    Theme::change(mode, Some(window), cx);
}

/// `config` at factor `k`.
fn scaled(config: &ThemeConfig, k: f32) -> ThemeConfig {
    let mut config = config.clone();
    config.font_size = Some(16. * k);
    let radius = |r: u8| (f32::from(r) * k).round() as usize;
    config.radius = Some(radius(RADIUS));
    config.radius_lg = Some(radius(RADIUS_LG));
    config
}

/// A colour as the theme config reads it: `#RRGGBBAA`.
fn css_color(color: Hsla) -> SharedString {
    let c = Rgba::from(color);
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        channel_byte(c.r),
        channel_byte(c.g),
        channel_byte(c.b),
        channel_byte(c.a)
    )
    .into()
}

/// The theme config for `mode`, in the palette `p`.
pub fn config(p: &Palette, mode: ThemeMode, bundled_fonts: bool) -> ThemeConfig {
    let clear = p.fg.opacity(0.);
    // The kit's hover fill for menu items and ghost buttons; see the module
    // doc for why dark mode doubles it.
    let kit_accent = if mode.is_dark() {
        p.fg.opacity(0.10)
    } else {
        p.hover
    };

    let mut config = ThemeConfig::default();
    config.name = if mode.is_dark() {
        "Hardcover Dark"
    } else {
        "Hardcover Light"
    }
    .into();
    config.mode = mode;
    config.font_size = Some(16.);
    if bundled_fonts {
        config.font_family = Some(fonts::INTER.into());
    }
    config.radius = Some(usize::from(RADIUS));
    config.radius_lg = Some(usize::from(RADIUS_LG));
    // Controls are flat: elevation is for covers and popovers only.
    config.shadow = Some(false);

    let css = |color| Some(css_color(color));
    let c = &mut config.colors;
    // Paper, ink and hairlines.
    c.background = css(p.bg);
    c.foreground = css(p.fg);
    c.border = css(p.border);
    c.input = css(p.border_strong);
    c.muted = css(p.surface);
    c.muted_foreground = css(p.fg_muted);
    c.window_border = css(p.border);
    c.accordion = css(p.bg);

    // Hover fills.
    c.accent = css(kit_accent);
    c.accent_foreground = css(p.fg);

    // The accent.
    c.primary = css(p.accent);
    c.primary_foreground = css(p.accent_fg);
    c.primary_hover = css(solid(p.accent, p.hover));
    c.primary_active = css(solid(p.accent, p.selected));
    c.progress_bar = css(p.accent);
    c.ring = css(p.accent);
    c.link = css(p.accent);
    c.link_hover = css(p.accent);
    c.link_active = css(p.accent);
    c.caret = css(p.accent);
    c.selection = css(p.accent.opacity(0.25));
    c.drag_border = css(p.accent);
    c.drop_target = css(p.accent_soft);
    c.slider_bar = css(p.accent);
    c.slider_thumb = css(p.raised);

    // Panels.
    c.secondary = css(p.surface);
    c.secondary_foreground = css(p.fg);
    c.secondary_hover = css(solid(p.surface, p.hover));
    c.secondary_active = css(solid(p.surface, p.selected));
    c.group_box = css(p.surface);
    c.group_box_foreground = css(p.fg);

    // Default buttons paint `raised`, so their overlays are blended.
    c.button = css(p.raised);
    c.button_foreground = css(p.fg);
    c.button_hover = css(solid(p.raised, p.hover));
    c.button_active = css(solid(p.raised, p.selected));
    // Danger: the text, no fill.
    c.danger = css(p.danger);
    c.danger_foreground = css(p.bg);
    c.danger_hover = css(solid(p.danger, p.hover));
    c.danger_active = css(solid(p.danger, p.selected));
    c.button_danger = css(clear);
    c.button_danger_foreground = css(p.danger);
    c.button_danger_hover = css(p.hover);
    c.button_danger_active = css(p.selected);
    c.warning = css(p.warning);
    c.warning_foreground = css(p.bg);
    // The kit's info and success (a notification's icon, "Up to date") are
    // neutral ink: the accent is spent elsewhere, and a green check would
    // be a second accent.
    c.info = css(p.fg_muted);
    c.info_foreground = css(p.bg);
    c.info_hover = css(solid(p.fg_muted, p.hover));
    c.info_active = css(solid(p.fg_muted, p.selected));
    c.success = css(p.fg_muted);
    c.success_foreground = css(p.bg);
    c.success_hover = css(solid(p.fg_muted, p.hover));
    c.success_active = css(solid(p.fg_muted, p.selected));
    // The dialog backdrop.
    c.overlay = css(p.scrim);

    apply_collection_colors(c, p);

    config
}

fn apply_collection_colors(c: &mut gpui_kit::component::ThemeConfigColors, p: &Palette) {
    let css = |color| Some(css_color(color));
    let clear = p.fg.opacity(0.);
    // Lists and menus.
    c.list = css(p.bg);
    c.list_even = css(p.bg);
    c.list_head = css(p.bg);
    c.list_hover = css(p.hover);
    c.list_active = css(p.selected);
    c.list_active_border = css(p.accent);
    c.list_highlight_foreground = css(p.accent);
    c.popover = css(p.raised);
    c.popover_foreground = css(p.fg);

    // The sidebar.
    c.sidebar = css(p.sidebar);
    c.sidebar_foreground = css(p.fg);
    c.sidebar_border = css(p.border);
    c.sidebar_primary = css(p.accent);
    c.sidebar_primary_foreground = css(p.accent_fg);
    c.sidebar_accent = css(p.selected);
    c.sidebar_accent_foreground = css(p.fg);

    // Loading, switches, scrollbars.
    c.skeleton = css(p.skeleton);
    c.switch = css(p.track);
    c.switch_thumb = css(p.switch_thumb);
    c.scrollbar = css(clear);
    c.scrollbar_thumb = css(p.border_strong);
    c.scrollbar_thumb_hover = css(p.fg_muted);

    // Tabs: a segmented bar is a chip-filled track. The app's only tab
    // bars are segmented, and the kit paints a disabled segment with
    // `tab_bar`: the track, so it reads as not chosen.
    let segment_track = css(solid(p.bg, p.chip));
    c.tab = css(p.bg);
    c.tab_active = css(p.bg);
    c.tab_bar = segment_track.clone();
    c.tab_bar_segmented = segment_track;
    c.tab_foreground = css(p.fg_muted);
    c.tab_active_foreground = css(p.fg);

    // Charts (Stats, the ratings breakdown): accent on track.
    for chart in [
        &mut c.chart_1,
        &mut c.chart_2,
        &mut c.chart_3,
        &mut c.chart_4,
        &mut c.chart_5,
    ] {
        *chart = css(p.accent);
    }
    c.chart_grid = css(p.track);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Hsla, b: Hsla) -> bool {
        let (a, b) = (Rgba::from(a), Rgba::from(b));
        [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a]
            .iter()
            .all(|d| d.abs() < 1. / 255.)
    }

    /// An unparsable colour in a config silently falls back to the kit's, so
    /// check what the theme resolves to, both ways round.
    #[test]
    fn the_configs_resolve_to_the_tokens_through_mode_changes() {
        let mut theme = Theme::default();
        let modes = ThemeFamily::ALL.into_iter().flat_map(|family| {
            [
                (ThemeMode::Light, Palette::light(family)),
                (ThemeMode::Dark, Palette::dark(family)),
            ]
        });
        for (mode, p) in modes {
            theme.apply_config(&Rc::new(config(p, mode, true)));
            assert_eq!(theme.mode, mode);
            for (field, got, want) in [
                ("background", theme.background, p.bg),
                ("foreground", theme.foreground, p.fg),
                ("sidebar", theme.sidebar, p.sidebar),
                ("sidebar_border", theme.sidebar_border, p.border),
                ("sidebar_accent", theme.sidebar_accent, p.selected),
                ("secondary", theme.secondary, p.surface),
                ("group_box", theme.group_box, p.surface),
                ("popover", theme.popover, p.raised),
                ("button", theme.button, p.raised),
                ("border", theme.border, p.border),
                ("input", theme.input, p.border_strong),
                ("muted_foreground", theme.muted_foreground, p.fg_muted),
                ("primary", theme.primary, p.accent),
                ("primary_foreground", theme.primary_foreground, p.accent_fg),
                ("progress_bar", theme.progress_bar, p.accent),
                ("ring", theme.ring, p.accent),
                ("link", theme.link, p.accent),
                ("caret", theme.caret, p.accent),
                ("list_hover", theme.list_hover, p.hover),
                ("list_active", theme.list_active, p.selected),
                ("button_hover", theme.button_hover, solid(p.raised, p.hover)),
                (
                    "button_active",
                    theme.button_active,
                    solid(p.raised, p.selected),
                ),
                ("skeleton", theme.skeleton, p.skeleton),
                ("switch", theme.switch, p.track),
                ("switch_thumb", theme.switch_thumb, p.switch_thumb),
                ("danger", theme.danger, p.danger),
                ("warning", theme.warning, p.warning),
                ("info", theme.info, p.fg_muted),
                ("success", theme.success, p.fg_muted),
                ("overlay", theme.overlay, p.scrim),
                ("scrollbar_thumb", theme.scrollbar_thumb, p.border_strong),
                (
                    "scrollbar_thumb_hover",
                    theme.scrollbar_thumb_hover,
                    p.fg_muted,
                ),
                ("tab_bar", theme.tab_bar, solid(p.bg, p.chip)),
            ] {
                assert!(close(got, want), "{mode:?} {field}: {got:?} != {want:?}");
                // The kit's components read the token copy.
            }
            assert!(close(theme.tokens.button_hover.color, theme.button_hover));
            assert!(close(theme.tokens.list_hover.color, p.hover));
            // The kit's success button and notification read these copies.
            assert!(close(theme.tokens.success.color, p.fg_muted));
            assert!(close(theme.tokens.info.color, p.fg_muted));
            assert!(close(theme.tokens.tab_bar.color, solid(p.bg, p.chip)));
            assert_eq!(theme.radius, gpui_kit::px(f32::from(RADIUS)));
            assert_eq!(theme.radius_lg, gpui_kit::px(f32::from(RADIUS_LG)));
            assert_eq!(theme.font_family.as_ref(), fonts::INTER);
        }
    }

    #[test]
    fn the_ui_scale_sets_the_rem_and_the_radii() {
        let mut theme = Theme::default();
        theme.apply_config(&Rc::new(scaled(
            &config(Palette::light(ThemeFamily::Paper), ThemeMode::Light, true),
            1.25,
        )));
        assert_eq!(theme.font_size, gpui_kit::px(20.));
        assert_eq!(theme.radius, gpui_kit::px(8.));
        assert_eq!(theme.radius_lg, gpui_kit::px(10.));
        // The palette is untouched.
        assert!(close(
            theme.background,
            Palette::light(ThemeFamily::Paper).bg
        ));
        theme.apply_config(&Rc::new(scaled(
            &config(Palette::dark(ThemeFamily::Paper), ThemeMode::Dark, true),
            1.,
        )));
        assert_eq!(theme.font_size, gpui_kit::px(16.));
        assert_eq!(theme.radius, gpui_kit::px(f32::from(RADIUS)));
    }

    /// Settings switches the theme while the app runs: the kit's theme and
    /// `colors` move together, in the mode showing, at the interface scale
    /// already set, and the other mode follows when it is shown.
    #[gpui_kit::test]
    fn switching_the_theme_keeps_the_scale(cx: &mut gpui_kit::TestAppContext) {
        use ThemeFamily::{Graphite, Hardcover, Paper};
        use gpui_kit::component::ActiveTheme as _;

        cx.update(|cx| {
            gpui_kit::init(cx);
            init(true, Graphite, cx);
            Theme::change(ThemeMode::Dark, None, cx);
            assert!(close(cx.theme().background, Palette::dark(Graphite).bg));
            assert!(close(
                super::super::colors(cx).bg,
                Palette::dark(Graphite).bg
            ));

            // As `set_ui_scale` leaves it.
            Theme::update(cx, |theme| {
                theme.light_theme = Rc::new(scaled(&theme.light_theme, 1.25));
                theme.dark_theme = Rc::new(scaled(&theme.dark_theme, 1.25));
            });
            set_theme_family(Paper, None, cx);
            assert!(close(cx.theme().background, Palette::dark(Paper).bg));
            assert!(close(super::super::colors(cx).bg, Palette::dark(Paper).bg));
            assert_eq!(cx.theme().font_size, gpui_kit::px(20.));
            assert_eq!(cx.theme().radius, gpui_kit::px(8.));
            assert_eq!(cx.theme().font_family.as_ref(), fonts::INTER);

            Theme::change(ThemeMode::Light, None, cx);
            assert!(close(cx.theme().background, Palette::light(Paper).bg));
            set_theme_family(Hardcover, None, cx);
            assert!(close(cx.theme().background, Palette::light(Hardcover).bg));
            assert!(close(
                super::super::colors(cx).bg,
                Palette::light(Hardcover).bg
            ));
            assert_eq!(cx.theme().font_size, gpui_kit::px(20.));
            Theme::change(ThemeMode::Dark, None, cx);
            assert!(close(cx.theme().background, Palette::dark(Hardcover).bg));
            assert_eq!(cx.theme().font_size, gpui_kit::px(20.));
        });
    }

    #[test]
    fn a_colour_is_written_with_its_alpha() {
        let p = Palette::light(ThemeFamily::Paper);
        assert_eq!(css_color(p.bg).as_ref(), "#FAF8F4FF");
        assert_eq!(css_color(p.hover).as_ref(), "#1F1C170D");
    }
}
