//! Paper and ink: the colour tokens of both themes (`hardcover-design`,
//! Tokens → Colour).
//!
//! The values are the skill's tables, written once here. [`super::theme`]
//! hands the roles the kit reads to gpui-component's theme; views read the
//! app-only roles through [`colors`]:
//!
//! ```ignore
//! let colors = design::colors(cx);
//! div().bg(colors.surface).text_color(colors.fg_faint)
//! ```
//!
//! `hover`, `selected` and `chip` are ink overlays: translucent ink, so they
//! step the same way on paper, the sidebar or a panel. GPUI replaces a
//! background rather than layering one, so a control that paints its own
//! fill needs the overlay blended into that fill: [`solid`].

use std::sync::LazyLock;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, BoxShadow, Hsla, Rgba, point, px};

/// Nearest byte for a normalized sRGB or alpha channel.
pub(super) fn channel_byte(channel: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamping and rounding produce a byte in 0..=255"
    )]
    let byte = (channel.clamp(0., 1.) * 255.).round() as u8;
    byte
}

/// An opaque colour from `0xRRGGBB`.
const fn hex(rgb: u32) -> Rgba {
    hexa(rgb, 1.)
}

/// A colour from `0xRRGGBB` at `alpha`.
const fn hexa(rgb: u32, alpha: f32) -> Rgba {
    let [_, r, g, b] = rgb.to_be_bytes();
    Rgba {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: alpha,
    }
}

/// `overlay` painted over `base`, as one opaque colour: for an ink overlay
/// on a control that paints its own background (a default button on
/// `raised`). Composited in sRGB, as the mockup's CSS does.
pub fn solid(base: Hsla, overlay: Hsla) -> Hsla {
    let base = Rgba::from(base);
    let overlay = Rgba::from(overlay);
    let a = overlay.a.clamp(0., 1.);
    let mix = |b: f32, o: f32| b * (1. - a) + o * a;
    Rgba {
        r: mix(base.r, overlay.r),
        g: mix(base.g, overlay.g),
        b: mix(base.b, overlay.b),
        a: 1.,
    }
    .into()
}

/// One layer of a box shadow, in CSS terms: `0 <y> <blur> <spread> <color>`.
#[derive(Clone, Copy, Debug)]
struct ShadowLayer {
    y: f32,
    blur: f32,
    spread: f32,
    color: Rgba,
}

impl ShadowLayer {
    const fn new(y: f32, blur: f32, spread: f32, color: Rgba) -> Self {
        Self {
            y,
            blur,
            spread,
            color,
        }
    }

    fn box_shadow(self) -> BoxShadow {
        BoxShadow {
            color: self.color.into(),
            offset: point(px(0.), px(self.y)),
            blur_radius: px(self.blur),
            spread_radius: px(self.spread),
            inset: false,
        }
    }
}

/// The eight book-cloth colours of the cloth cover, the same in both themes.
/// cloth-ink on each is at least 5:1.
const CLOTH: [Rgba; 8] = [
    hex(0x6A_4A_39),
    hex(0x3A_57_4B),
    hex(0x41_4C_6B),
    hex(0x74_38_38),
    hex(0x7A_5D_30),
    hex(0x39_59_6A),
    hex(0x59_45_67),
    hex(0x4E_53_56),
];

/// Type and stamp on a cloth cover, in both themes.
const CLOTH_INK: Rgba = hex(0xF4_EC_DD);

/// The empty star's ink overlay, in both themes.
const STAR_EMPTY: f32 = 0.10;

/// One theme's tokens as written in the skill.
struct Tokens {
    bg: Rgba,
    sidebar: Rgba,
    surface: Rgba,
    raised: Rgba,
    skeleton: Rgba,
    border: Rgba,
    border_strong: Rgba,
    track: Rgba,
    switch_thumb: Rgba,
    fg: Rgba,
    fg_muted: Rgba,
    fg_faint: Rgba,
    accent: Rgba,
    accent_fg: Rgba,
    /// The accent-soft alpha (10% light, 14% dark).
    accent_soft: f32,
    /// The chip overlay's alpha (7% light, 8% dark).
    chip: f32,
    gold: Rgba,
    /// The dialog backdrop.
    scrim: Rgba,
    danger: Rgba,
    warning: Rgba,
    cover_edge: Rgba,
    cover_hinge: Rgba,
    shadow_cover: [ShadowLayer; 2],
}

const LIGHT: Tokens = Tokens {
    bg: hex(0xFA_F8_F4),
    sidebar: hex(0xF1_EE_E7),
    surface: hex(0xF3_F0_E9),
    raised: hex(0xFF_FF_FF),
    skeleton: hex(0xE9_E4_DA),
    border: hex(0xE4_DE_D3),
    // At least 3:1 on `raised`, paper, `surface` and the sidebar: a field's
    // outline is all that marks it.
    border_strong: hex(0x91_88_79),
    track: hex(0xE0_D9_CC),
    // `raised`.
    switch_thumb: hex(0xFF_FF_FF),
    fg: hex(0x1F_1C_17),
    fg_muted: hex(0x5F_57_4B),
    fg_faint: hex(0x7A_72_66),
    accent: hex(0x3B_4A_9C),
    accent_fg: hex(0xFF_FF_FF),
    accent_soft: 0.10,
    chip: 0.07,
    // Ochre: at least 3:1 against an empty star on paper, hovered or
    // selected (amber 500 was 1.5:1, so a rating barely showed).
    gold: hex(0xA1_62_07),
    scrim: hexa(0x1F_1C_17, 0.16),
    danger: hex(0xB4_23_18),
    warning: hex(0xB5_47_08),
    cover_edge: hexa(0x1E_14_0A, 0.16),
    cover_hinge: hexa(0x1E_14_0A, 0.16),
    shadow_cover: [
        ShadowLayer::new(1., 2., 0., hexa(0x28_1C_10, 0.18)),
        ShadowLayer::new(8., 20., 0., hexa(0x28_1C_10, 0.14)),
    ],
};

const DARK: Tokens = Tokens {
    bg: hex(0x17_16_14),
    sidebar: hex(0x10_0F_0E),
    surface: hex(0x1E_1C_1A),
    raised: hex(0x26_23_20),
    skeleton: hex(0x2B_28_25),
    border: hex(0x29_26_23),
    border_strong: hex(0x77_70_6A),
    track: hex(0x35_31_2C),
    // `fg`: a `raised` thumb on `track` is 1.2:1 in dark mode, gone once a
    // disabled switch fades its track.
    switch_thumb: hex(0xEE_EA_E3),
    fg: hex(0xEE_EA_E3),
    fg_muted: hex(0xAB_A4_98),
    fg_faint: hex(0x8D_87_7C),
    accent: hex(0x98_A5_F4),
    accent_fg: hex(0x12_14_2A),
    accent_soft: 0.14,
    chip: 0.08,
    // Amber 400.
    gold: hex(0xFB_BF_24),
    scrim: hexa(0x00_00_00, 0.48),
    // At least 4.5:1 on a pressed danger button in a dialog.
    danger: hex(0xFB_81_78),
    warning: hex(0xFD_B0_22),
    cover_edge: hexa(0xFF_FF_FF, 0.12),
    cover_hinge: hexa(0x00_00_00, 0.28),
    shadow_cover: [
        ShadowLayer::new(1., 2., 0., hexa(0x00_00_00, 0.55)),
        ShadowLayer::new(10., 24., 0., hexa(0x00_00_00, 0.45)),
    ],
};

/// Every colour role of one theme. Fields are named after the skill's roles
/// (`fg-muted` is `fg_muted`).
#[derive(Clone, Debug)]
pub struct Palette {
    /// Paper: the work area.
    pub bg: Hsla,
    /// The sidebar.
    pub sidebar: Hsla,
    /// The one inset panel in a pane (progress, Not started).
    pub surface: Hsla,
    /// Controls, inputs, popovers, dialogs.
    pub raised: Hsla,
    /// Ink overlay (5%): the pointer over a row, nav item, menu item or ghost
    /// button.
    pub hover: Hsla,
    /// Ink overlay (10%): the current row or destination; a pressed control.
    pub selected: Hsla,
    /// Ink overlay (7% light, 8% dark): a chip's fill.
    pub chip: Hsla,
    /// Something still loading.
    pub skeleton: Hsla,
    /// Hairlines between regions and sections.
    pub border: Hsla,
    /// Control outlines.
    pub border_strong: Hsla,
    /// The empty part of a meter; an off switch.
    pub track: Hsla,
    /// A switch's thumb: `raised` in light mode, `fg` in dark, so it shows
    /// on `track` and on `accent`, enabled or disabled.
    pub switch_thumb: Hsla,
    /// Ink: text.
    pub fg: Hsla,
    /// Secondary text that carries information.
    pub fg_muted: Hsla,
    /// Empty-state icons and other marks nobody must read: at least 3:1 on
    /// every fill, never text.
    pub fg_faint: Hsla,
    /// Ink blue, the one accent.
    pub accent: Hsla,
    /// Text on accent.
    pub accent_fg: Hsla,
    /// The avatar's fill.
    pub accent_soft: Hsla,
    /// The focus ring's colour (accent at 50%), for focus drawn by the app.
    /// The kit's controls take the solid accent from the theme and apply the
    /// 50% themselves.
    pub ring: Hsla,
    /// The user's own rating, nothing else. The rating control's hover
    /// preview draws it as an outline over the empty star (`crate::star`).
    pub gold: Hsla,
    /// An unfilled star (ink 10%).
    pub star_empty: Hsla,
    /// The dialog backdrop: ink 16% in light mode, black 48% in dark.
    pub scrim: Hsla,
    /// Destructive commands and errors.
    pub danger: Hsla,
    /// A content warning's icon; warnings.
    pub warning: Hsla,
    /// The cover's 1 px edge.
    pub cover_edge: Hsla,
    /// The hinge gradient's dark stop.
    pub cover_hinge: Hsla,
    /// Type and stamp on a cloth cover.
    pub cloth_ink: Hsla,
    /// The eight cloth colours; [`Palette::cloth_for`] picks one.
    pub cloth: [Hsla; 8],
    shadow_cover: [ShadowLayer; 2],
}

impl Palette {
    fn new(t: &Tokens) -> Self {
        let fg: Hsla = t.fg.into();
        let accent: Hsla = t.accent.into();
        Self {
            bg: t.bg.into(),
            sidebar: t.sidebar.into(),
            surface: t.surface.into(),
            raised: t.raised.into(),
            hover: fg.opacity(0.05),
            selected: fg.opacity(0.10),
            chip: fg.opacity(t.chip),
            skeleton: t.skeleton.into(),
            border: t.border.into(),
            border_strong: t.border_strong.into(),
            track: t.track.into(),
            switch_thumb: t.switch_thumb.into(),
            fg,
            fg_muted: t.fg_muted.into(),
            fg_faint: t.fg_faint.into(),
            accent,
            accent_fg: t.accent_fg.into(),
            accent_soft: accent.opacity(t.accent_soft),
            ring: accent.opacity(0.5),
            gold: t.gold.into(),
            star_empty: fg.opacity(STAR_EMPTY),
            scrim: t.scrim.into(),
            danger: t.danger.into(),
            warning: t.warning.into(),
            cover_edge: t.cover_edge.into(),
            cover_hinge: t.cover_hinge.into(),
            cloth_ink: CLOTH_INK.into(),
            cloth: CLOTH.map(Hsla::from),
            shadow_cover: t.shadow_cover,
        }
    }

    /// The light theme's palette.
    pub fn light() -> &'static Self {
        static PALETTE: LazyLock<Palette> = LazyLock::new(|| Palette::new(&LIGHT));
        &PALETTE
    }

    /// The dark theme's palette.
    pub fn dark() -> &'static Self {
        static PALETTE: LazyLock<Palette> = LazyLock::new(|| Palette::new(&DARK));
        &PALETTE
    }

    /// `shadow-cover`: on a cover that leads its row or block.
    pub fn shadow_cover(&self) -> Vec<BoxShadow> {
        self.shadow_cover.map(ShadowLayer::box_shadow).to_vec()
    }

    /// The cloth colour of the book titled `title`.
    pub fn cloth_for(&self, title: &str) -> Hsla {
        self.cloth[cloth_index(title)]
    }
}

/// The current theme's palette.
pub fn colors(cx: &App) -> &'static Palette {
    if cx.theme().is_dark() {
        Palette::dark()
    } else {
        Palette::light()
    }
}

/// A colour that comes as data, such as a reviewer's Hardcover colour
/// (`cached_image.color`): not a UI colour, so it has no token.
pub fn data_color([r, g, b]: [u8; 3]) -> Hsla {
    Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    }
    .into()
}

/// Text on a data colour ([`data_color`]): near-black ink or white,
/// whichever has the higher contrast on it, in both themes.
pub fn text_on_data(rgb: [u8; 3]) -> Hsla {
    let fill = data_color(rgb);
    let (ink, white): (Hsla, Hsla) = (LIGHT.fg.into(), LIGHT.raised.into());
    if contrast(ink, fill) >= contrast(white, fill) {
        ink
    } else {
        white
    }
}

/// WCAG relative luminance of an opaque colour.
pub fn luminance(color: Hsla) -> f32 {
    let c = Rgba::from(color);
    let lin = |v: f32| {
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio between two opaque colours (1 to 21). Composite a
/// translucent one onto its host first ([`solid`]).
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The cloth colour for a title: a stable hash of its code points, the same
/// as the mockup's (`h = 7; h = h * 31 + c mod 2^32`), so a book always gets
/// the same cloth.
pub fn cloth_index(title: &str) -> usize {
    let hash = title
        .chars()
        .fold(7u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32));
    hash as usize % CLOTH.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(color: Hsla) -> [u8; 3] {
        let c = Rgba::from(color);
        [c.r, c.g, c.b].map(channel_byte)
    }

    #[test]
    fn hex_reads_channels_in_order() {
        let c = hex(0x3B_4A_9C);
        assert_eq!([c.r, c.g, c.b].map(channel_byte), [0x3B, 0x4A, 0x9C]);
        assert_eq!(c.a, 1.);
    }

    #[test]
    fn the_palette_keeps_the_skill_values() {
        assert_eq!(rgb(Palette::light().bg), [0xFA, 0xF8, 0xF4]);
        assert_eq!(rgb(Palette::dark().bg), [0x17, 0x16, 0x14]);
        assert_eq!(rgb(Palette::light().accent), [0x3B, 0x4A, 0x9C]);
        assert_eq!(rgb(Palette::dark().gold), [0xFB, 0xBF, 0x24]);
        assert!((Palette::light().hover.a - 0.05).abs() < 1e-6);
        assert!((Palette::dark().chip.a - 0.08).abs() < 1e-6);
        assert!((Palette::light().star_empty.a - 0.10).abs() < 1e-6);
    }

    /// The overlay composited by hand: 255 * 0.95 + 31 * 0.05 = 243.8, and
    /// so on per channel.
    #[test]
    fn an_overlay_blends_into_a_solid_colour() {
        let light = Palette::light();
        let hovered = solid(light.raised, light.hover);
        assert_eq!(hovered.a, 1.);
        assert_eq!(rgb(hovered), [244, 244, 243]);
        let pressed = solid(light.raised, light.selected);
        assert_eq!(rgb(pressed), [233, 232, 232]);
        // Dark ink is light: the overlay lightens a dark control.
        let dark = Palette::dark();
        let hovered = solid(dark.raised, dark.hover);
        assert_eq!(rgb(hovered), [48, 45, 42]);
    }

    #[test]
    fn a_clear_overlay_leaves_the_base_and_an_opaque_one_replaces_it() {
        let light = Palette::light();
        assert_eq!(rgb(solid(light.bg, light.fg.opacity(0.))), rgb(light.bg));
        assert_eq!(rgb(solid(light.bg, light.accent)), rgb(light.accent));
    }

    /// Indexes from the mockup's own hash (`docs/design/mockup.html`), run in
    /// Node.
    #[test]
    fn the_cloth_hash_matches_the_mockup() {
        assert_eq!(cloth_index(""), 7);
        assert_eq!(cloth_index("Dune"), 7);
        assert_eq!(cloth_index("Untitled Empyrean #4"), 4);
        assert_eq!(cloth_index("ملت عشق"), 3);
    }

    #[test]
    fn data_colours_get_whichever_text_reads_better() {
        let ink = [0x1F, 0x1C, 0x17];
        let white = [0xFF, 0xFF, 0xFF];
        assert_eq!(rgb(text_on_data([0xf5, 0xe6, 0x80])), ink);
        assert_eq!(rgb(text_on_data([0x33, 0x66, 0x99])), white);
        // Cyan: white is 2.3:1 on it, ink 7.4:1 (a luma threshold picked white).
        assert_eq!(rgb(text_on_data([0x00, 0xBC, 0xD4])), ink);
        assert_eq!(rgb(data_color([0x33, 0x66, 0x99])), [0x33, 0x66, 0x99]);
        for fill in [
            [0x00, 0xBC, 0xD4],
            [0x3f, 0x6e, 0x8c],
            [0x77, 0x77, 0x77],
            [0xE9, 0x1E, 0x63],
        ] {
            let on = data_color(fill);
            let chosen = contrast(text_on_data(fill), on);
            let other = contrast(LIGHT.fg.into(), on).min(contrast(LIGHT.raised.into(), on));
            assert!(chosen >= other, "{fill:?}: {chosen} < {other}");
        }
    }

    #[test]
    fn contrast_is_the_wcag_ratio() {
        let black: Hsla = hex(0x00_00_00).into();
        let white: Hsla = hex(0xFF_FF_FF).into();
        assert!((contrast(black, white) - 21.).abs() < 0.01);
        assert!((contrast(white, white) - 1.).abs() < 1e-6);
        // #767676 on white is the classic 4.54:1.
        assert!((contrast(hex(0x76_76_76).into(), white) - 4.54).abs() < 0.01);
    }

    #[test]
    fn every_title_gets_a_cloth_and_the_same_one_each_time() {
        for title in ["Dune", "The Way of Kings", "", "ملت عشق"] {
            let index = cloth_index(title);
            assert!(index < 8);
            assert_eq!(index, cloth_index(title));
        }
    }
}
