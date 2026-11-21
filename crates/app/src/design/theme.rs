//! The palette applied to gpui-component's theme, so the kit's components
//! draw in paper and ink.
//!
//! **How:** the app builds its own light and dark `ThemeConfig` from
//! [`super::color`] and installs them as the theme's `light_theme` and
//! `dark_theme`. `Theme::change` loads the config of the mode it switches
//! to, so the palette holds through every mode change: the Settings choice,
//! the desktop's appearance and `HARDCOVER_THEME` all go through
//! `preferences::apply_theme`, which calls `Theme::change`.
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
//! - `ring` is the solid accent: the kit draws the 3 px ring at 50% of it,
//!   plus a 1 px accent edge, as the system's focus treatment asks.
//! - `switch_thumb` is its own role: `raised` in light mode, `fg` in dark.
//!
//! **Where the kit doesn't follow the theme:**
//! - `Theme::input_background()` ignores the config: inputs are paper in
//!   light mode and a tint of the input border in dark. `ui::kit`'s
//!   `RaisedField` puts them on `raised`.
//! - Dialogs paint `background` (paper); `ui::kit::open_dialog` and
//!   `open_alert_dialog` put them on `raised`.
//! - The sidebar's menu hover is `sidebar_accent` at 80%, so it is 8%, not
//!   5%, two steps from the 10% selected fill.
//! - The segmented tab bar's sliding indicator is always `background`.
//! - The rest (menus, the Select dropdown, switches, radios, the
//!   `DatePicker`, dialog shadows, a disabled segment) is listed in `ui::kit`'s module doc and
//!   the system's "Kit limits".

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, SharedString};

use super::color::{Palette, channel_byte, solid};
use crate::fonts;

/// The radius of controls (`r-control`).
const RADIUS: u8 = 6;
/// The radius of rows, panels, popovers and dialogs (`r-box`).
const RADIUS_LG: u8 = 8;

/// Installs the app's light and dark themes and applies the current mode's.
/// Runs after `gpui_kit::init` and `fonts::init`. `bundled_fonts` says
/// whether the fonts loaded; without them the theme keeps the system font.
pub fn init(bundled_fonts: bool, cx: &mut App) {
    let light = Rc::new(config(ThemeMode::Light, bundled_fonts));
    let dark = Rc::new(config(ThemeMode::Dark, bundled_fonts));
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

/// The theme config for `mode`.
pub fn config(mode: ThemeMode, bundled_fonts: bool) -> ThemeConfig {
    let p = if mode.is_dark() {
        Palette::dark()
    } else {
        Palette::light()
    };
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
    c.scrollbar_thumb_hover = css(p.fg_faint);

    // Tabs: a segmented bar is a chip-filled track.
    c.tab = css(p.bg);
    c.tab_active = css(p.bg);
    c.tab_bar = css(p.bg);
    c.tab_bar_segmented = css(solid(p.bg, p.chip));
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
        for mode in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::Light] {
            theme.apply_config(&Rc::new(config(mode, true)));
            let p = if mode.is_dark() {
                Palette::dark()
            } else {
                Palette::light()
            };
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
            ] {
                assert!(close(got, want), "{mode:?} {field}: {got:?} != {want:?}");
                // The kit's components read the token copy.
            }
            assert!(close(theme.tokens.button_hover.color, theme.button_hover));
            assert!(close(theme.tokens.list_hover.color, p.hover));
            assert_eq!(theme.radius, gpui_kit::px(f32::from(RADIUS)));
            assert_eq!(theme.radius_lg, gpui_kit::px(f32::from(RADIUS_LG)));
            assert_eq!(theme.font_family.as_ref(), fonts::INTER);
        }
    }

    #[test]
    fn a_colour_is_written_with_its_alpha() {
        let p = Palette::light();
        assert_eq!(css_color(p.bg).as_ref(), "#FAF8F4FF");
        assert_eq!(css_color(p.hover).as_ref(), "#1F1C170D");
    }
}
