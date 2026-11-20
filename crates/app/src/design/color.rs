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

/// An opaque colour from `0xRRGGBB`.
const fn hex(rgb: u32) -> Rgba {
    hexa(rgb, 1.)
}

/// A colour from `0xRRGGBB` at `alpha`.
const fn hexa(rgb: u32, alpha: f32) -> Rgba {
    Rgba {
        r: ((rgb >> 16) & 0xff) as f32 / 255.,
        g: ((rgb >> 8) & 0xff) as f32 / 255.,
        b: (rgb & 0xff) as f32 / 255.,
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
    hex(0x6A4A39),
    hex(0x3A574B),
    hex(0x414C6B),
    hex(0x743838),
    hex(0x7A5D30),
    hex(0x39596A),
    hex(0x594567),
    hex(0x4E5356),
];

/// Type and stamp on a cloth cover, in both themes.
const CLOTH_INK: Rgba = hex(0xF4ECDD);

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
    danger: Rgba,
    warning: Rgba,
    cover_edge: Rgba,
    cover_hinge: Rgba,
    shadow_cover: [ShadowLayer; 2],
    shadow_popover: [ShadowLayer; 2],
}

const LIGHT: Tokens = Tokens {
    bg: hex(0xFAF8F4),
    sidebar: hex(0xF1EEE7),
    surface: hex(0xF3F0E9),
    raised: hex(0xFFFFFF),
    skeleton: hex(0xE9E4DA),
    border: hex(0xE4DED3),
    border_strong: hex(0xD0C8BA),
    track: hex(0xE0D9CC),
    fg: hex(0x1F1C17),
    fg_muted: hex(0x5F574B),
    fg_faint: hex(0x857D70),
    accent: hex(0x3B4A9C),
    accent_fg: hex(0xFFFFFF),
    accent_soft: 0.10,
    chip: 0.07,
    // Amber 500.
    gold: hex(0xF59E0B),
    danger: hex(0xB42318),
    warning: hex(0xB54708),
    cover_edge: hexa(0x1E140A, 0.16),
    cover_hinge: hexa(0x1E140A, 0.16),
    shadow_cover: [
        ShadowLayer::new(1., 2., 0., hexa(0x281C10, 0.18)),
        ShadowLayer::new(8., 20., 0., hexa(0x281C10, 0.14)),
    ],
    shadow_popover: [
        ShadowLayer::new(0., 0., 1., hexa(0x1E140A, 0.10)),
        ShadowLayer::new(4., 12., 0., hexa(0x281C10, 0.12)),
    ],
};

const DARK: Tokens = Tokens {
    bg: hex(0x171614),
    sidebar: hex(0x100F0E),
    surface: hex(0x1E1C1A),
    raised: hex(0x262320),
    skeleton: hex(0x23211E),
    border: hex(0x292623),
    border_strong: hex(0x3D3934),
    track: hex(0x35312C),
    fg: hex(0xEEEAE3),
    fg_muted: hex(0xABA498),
    fg_faint: hex(0x807A6F),
    accent: hex(0x98A5F4),
    accent_fg: hex(0x12142A),
    accent_soft: 0.14,
    chip: 0.08,
    // Amber 400.
    gold: hex(0xFBBF24),
    danger: hex(0xF97066),
    warning: hex(0xFDB022),
    cover_edge: hexa(0xFFFFFF, 0.12),
    cover_hinge: hexa(0x000000, 0.28),
    shadow_cover: [
        ShadowLayer::new(1., 2., 0., hexa(0x000000, 0.55)),
        ShadowLayer::new(10., 24., 0., hexa(0x000000, 0.45)),
    ],
    shadow_popover: [
        ShadowLayer::new(0., 0., 1., hexa(0xFFFFFF, 0.08)),
        ShadowLayer::new(4., 12., 0., hexa(0x000000, 0.5)),
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
    /// Ink: text.
    pub fg: Hsla,
    /// Secondary text that carries information.
    pub fg_muted: Hsla,
    /// Placeholders, disabled hints, empty-state icons; nothing a reader must
    /// read.
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
    /// The user's own rating, nothing else.
    pub gold: Hsla,
    /// An unfilled star (ink 14%).
    pub star_empty: Hsla,
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
    shadow_popover: [ShadowLayer; 2],
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
            fg,
            fg_muted: t.fg_muted.into(),
            fg_faint: t.fg_faint.into(),
            accent,
            accent_fg: t.accent_fg.into(),
            accent_soft: accent.opacity(t.accent_soft),
            ring: accent.opacity(0.5),
            gold: t.gold.into(),
            star_empty: fg.opacity(0.14),
            danger: t.danger.into(),
            warning: t.warning.into(),
            cover_edge: t.cover_edge.into(),
            cover_hinge: t.cover_hinge.into(),
            cloth_ink: CLOTH_INK.into(),
            cloth: CLOTH.map(Hsla::from),
            shadow_cover: t.shadow_cover,
            shadow_popover: t.shadow_popover,
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

    /// `shadow-popover`: menus, popovers and dialogs the app draws itself.
    pub fn shadow_popover(&self) -> Vec<BoxShadow> {
        self.shadow_popover.map(ShadowLayer::box_shadow).to_vec()
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

/// The cloth colour for a title: a stable hash of its code points, the same
/// as the mockup's (`h = 7; h = h * 31 + c mod 2^32`), so a book always gets
/// the same cloth.
pub fn cloth_index(title: &str) -> usize {
    let hash = title
        .chars()
        .fold(7u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32));
    (hash % CLOTH.len() as u32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(color: Hsla) -> [u8; 3] {
        let c = Rgba::from(color);
        [c.r, c.g, c.b].map(|v| (v * 255.).round() as u8)
    }

    #[test]
    fn hex_reads_channels_in_order() {
        let c = hex(0x3B4A9C);
        assert_eq!(
            [c.r, c.g, c.b].map(|v| (v * 255.).round() as u8),
            [0x3B, 0x4A, 0x9C]
        );
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
        assert!((Palette::light().star_empty.a - 0.14).abs() < 1e-6);
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
    fn every_title_gets_a_cloth_and_the_same_one_each_time() {
        for title in ["Dune", "The Way of Kings", "", "ملت عشق"] {
            let index = cloth_index(title);
            assert!(index < 8);
            assert_eq!(index, cloth_index(title));
        }
    }
}
