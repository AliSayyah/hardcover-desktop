//! The type roles (`hardcover-design`, Tokens → Type) as style helpers.
//!
//! Each role sets its family, its fallback for Persian and Arabic, its weight,
//! its size and its line height, so it reads the same wherever it is used:
//!
//! ```ignore
//! use crate::design::TypeRoles as _;
//! div().page_title().child("Currently reading");
//! div().caption(cx).tabular().child("1,007 pages");
//! ```
//!
//! - The sans roles are Inter, the book roles Literata (Text up to 18 px,
//!   Display from 24 px); Persian and Arabic fall back to Vazirmatn in both.
//! - Line heights are explicit: GPUI's default is 1.618.
//! - `label` and `caption` are muted, so they take the app context for the
//!   colour. The other roles take the colour from their parent.
//! - [`TypeRoles::tabular`] turns on tabular figures for numbers that stack
//!   or change in place. Display roles use lining figures.
//! - Text of two roles in one row shares a baseline through
//!   [`TypeRoles::baseline_on`]: `items_baseline()` can't do it, because
//!   GPUI's text doesn't report its baseline to the layout.

use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, FontFeatures, FontWeight, Styled, rems};

use crate::fonts;

/// Text sizes and their line heights, in px, from the skill's type scale.
mod scale {
    pub const XS: (f32, f32) = (12., 16.);
    pub const SM: (f32, f32) = (14., 20.);
    pub const BASE: (f32, f32) = (16., 24.);
    pub const LG: (f32, f32) = (18., 24.);
    pub const XL: (f32, f32) = (20., 28.);
    pub const XXL: (f32, f32) = (24., 30.);
    pub const XXXL: (f32, f32) = (30., 36.);
    /// Descriptions: the serif at 16 with more air.
    pub const PROSE: (f32, f32) = (16., 26.);
    /// Readers' words: Inter at 14 with more air.
    pub const READER: (f32, f32) = (14., 22.);
}

/// Which bundled family a role is set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Inter, over Vazirmatn.
    Sans,
    /// Literata Text (12 pt cut), over Vazirmatn. Up to 18 px.
    SerifText,
    /// Literata Display (36 pt cut), over Vazirmatn. From 24 px.
    SerifDisplay,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sans => fonts::INTER,
            Self::SerifText => fonts::LITERATA_TEXT,
            Self::SerifDisplay => fonts::LITERATA_DISPLAY,
        }
    }

    /// The family Persian and Arabic fall back to under this one: Vazirmatn
    /// under every family.
    pub fn script_fallback(self) -> &'static str {
        fonts::VAZIRMATN
    }

    /// Half of (ascent − descent), per px of font size: where the baseline
    /// sits below the middle of a line box. GPUI centres a line's ascent and
    /// descent in its line height (`(line − ascent − descent) / 2` above the
    /// ascent), so the line height drops out. From the fonts' `hhea` tables
    /// (Inter 1984/−494 of 2048, Literata 1177/−308 of 1000), checked by a
    /// test.
    fn baseline_below_middle(self) -> f32 {
        match self {
            Self::Sans => (1984. - 494.) / 2048. / 2.,
            Self::SerifText | Self::SerifDisplay => (1177. - 308.) / 1000. / 2.,
        }
    }

    /// The family a run named `family` belongs to: the serif for either
    /// Literata cut, else the sans.
    pub fn of(family: &str) -> Self {
        if family == fonts::LITERATA_DISPLAY {
            Self::SerifDisplay
        } else if family == fonts::LITERATA_TEXT {
            Self::SerifText
        } else {
            Self::Sans
        }
    }
}

/// px to rems, at the system's 16 px rem.
fn r(px: f32) -> gpui_kit::Rems {
    rems(px / 16.)
}

/// A type role as a value, for what depends on its metrics: see
/// [`TypeRoles::baseline_on`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeRole {
    PageTitle,
    Section,
    Heading,
    Body,
    BodyStrong,
    Label,
    Caption,
    Display30,
    Display24,
    Title18,
    Prose,
    Reader,
}

impl TypeRole {
    pub fn family(self) -> Family {
        match self {
            Self::Display30 | Self::Display24 => Family::SerifDisplay,
            Self::Title18 | Self::Prose => Family::SerifText,
            _ => Family::Sans,
        }
    }

    pub fn weight(self) -> FontWeight {
        match self {
            Self::Body | Self::Caption | Self::Prose | Self::Reader => FontWeight::NORMAL,
            Self::BodyStrong | Self::Label => FontWeight::MEDIUM,
            _ => FontWeight::SEMIBOLD,
        }
    }

    /// Size and line height, in px.
    pub fn scale(self) -> (f32, f32) {
        match self {
            Self::PageTitle => scale::XL,
            Self::Section => scale::BASE,
            Self::Heading | Self::Body | Self::BodyStrong => scale::SM,
            Self::Label | Self::Caption => scale::XS,
            Self::Display30 => scale::XXXL,
            Self::Display24 => scale::XXL,
            Self::Title18 => scale::LG,
            Self::Prose => scale::PROSE,
            Self::Reader => scale::READER,
        }
    }

    /// How far the baseline sits below the middle of the line box, in px.
    fn baseline_below_middle(self) -> f32 {
        self.family().baseline_below_middle() * self.scale().0
    }

    /// How far down to move text in this role, centred in a row beside text
    /// in `lead`, so the two share a baseline. Negative moves it up.
    pub fn shift_onto(self, lead: TypeRole) -> f32 {
        lead.baseline_below_middle() - self.baseline_below_middle()
    }
}

/// The type roles, on anything styled.
pub trait TypeRoles: Styled + Sized {
    /// Sets `family` with its script fallback.
    fn type_family(mut self, family: Family) -> Self {
        let style = self.text_style();
        style.font_family = Some(family.name().into());
        style.font_fallbacks = Some(gpui_kit::FontFallbacks::from_fonts(vec![
            family.script_fallback().to_owned(),
        ]));
        self
    }

    /// A role: family, weight, and a (size, line height) pair in px.
    fn type_role(self, family: Family, weight: FontWeight, (size, line): (f32, f32)) -> Self {
        self.type_family(family)
            .font_weight(weight)
            .text_size(r(size))
            .line_height(r(line))
    }

    /// A role by value: family, weight, size and line height (not the
    /// colour or features the named helpers add).
    fn type_role_of(self, role: TypeRole) -> Self {
        self.type_role(role.family(), role.weight(), role.scale())
    }

    /// Puts this element's text, set in `role`, on the baseline of `lead`
    /// text beside it in a row that centres its items (`items_center`, the
    /// default for `h_flex`). Use it instead of `items_baseline()`: GPUI's
    /// text doesn't report a baseline to the layout, so that aligns the
    /// bottoms of the line boxes and smaller text sits low.
    ///
    /// ```ignore
    /// h_flex()
    ///     .child(div().page_title().child("Read"))
    ///     .child(div().body().baseline_on(TypeRole::Body, TypeRole::PageTitle).child("9"))
    /// ```
    fn baseline_on(self, role: TypeRole, lead: TypeRole) -> Self {
        self.relative().top(r(role.shift_onto(lead)))
    }

    /// A destination's title: Inter 600, 20/28.
    fn page_title(self) -> Self {
        self.type_role_of(TypeRole::PageTitle)
    }

    /// A section on a page: Inter 600, 16/24.
    fn section(self) -> Self {
        self.type_role_of(TypeRole::Section)
    }

    /// A block in a pane: Inter 600, 14/20.
    fn heading(self) -> Self {
        self.type_role_of(TypeRole::Heading)
    }

    /// Everything by default: Inter 400, 14/20.
    fn body(self) -> Self {
        self.type_role_of(TypeRole::Body)
    }

    /// Row titles, values that lead: Inter 500, 14/20.
    fn body_strong(self) -> Self {
        self.type_role_of(TypeRole::BodyStrong)
    }

    /// Sidebar and rail section labels: Inter 500, 12/16, muted.
    fn label(self, cx: &App) -> Self {
        self.type_role_of(TypeRole::Label)
            .text_color(cx.theme().muted_foreground)
    }

    /// Counts, trailing figures, timestamps, helper text: Inter 400, 12/16,
    /// muted.
    fn caption(self, cx: &App) -> Self {
        self.type_role_of(TypeRole::Caption)
            .text_color(cx.theme().muted_foreground)
    }

    /// The book title on the full page, the Home greeting, headline figures:
    /// Literata Display 600, 30/36, lining figures.
    fn display_30(self) -> Self {
        self.type_role_of(TypeRole::Display30)
            .font_features(features(&["lnum"]))
    }

    /// The book title in the pane and the Home lead: Literata Display 600,
    /// 24/30, lining figures.
    fn display_24(self) -> Self {
        self.type_role_of(TypeRole::Display24)
            .font_features(features(&["lnum"]))
    }

    /// Book titles in Home rows and Up next: Literata Text 600, 18/24.
    fn title_18(self) -> Self {
        self.type_role_of(TypeRole::Title18)
    }

    /// Descriptions: Literata Text 400, 16/26. At most
    /// [`super::size::PROSE`] wide.
    fn prose(self) -> Self {
        self.type_role_of(TypeRole::Prose)
    }

    /// Readers' words (reviews, long notes): Inter 400, 14/22. At most
    /// [`super::size::PROSE`] wide.
    fn reader(self) -> Self {
        self.type_role_of(TypeRole::Reader)
    }

    /// Tabular (and lining) figures, for numbers that stack or change in
    /// place: percentages, page counts, sidebar counts, dates in lists.
    /// Replaces the element's other font features.
    fn tabular(self) -> Self {
        self.font_features(features(&["tnum", "lnum"]))
    }
}

impl<T: Styled> TypeRoles for T {}

/// Font features with each of `tags` turned on.
fn features(tags: &[&str]) -> FontFeatures {
    FontFeatures(Arc::new(
        tags.iter().map(|tag| ((*tag).to_owned(), 1)).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_belongs_to_its_family() {
        assert_eq!(Family::of(fonts::LITERATA_DISPLAY), Family::SerifDisplay);
        assert_eq!(Family::of(fonts::LITERATA_TEXT), Family::SerifText);
        assert_eq!(Family::of(fonts::INTER), Family::Sans);
        assert_eq!(Family::of(".SystemUIFont"), Family::Sans);
        assert_eq!(Family::SerifDisplay.script_fallback(), fonts::VAZIRMATN);
        assert_eq!(Family::Sans.script_fallback(), fonts::VAZIRMATN);
    }

    /// `(ascent, descent, units per em)` from a font's `head` and `hhea`
    /// tables, the metrics GPUI centres in the line height.
    fn vertical_metrics(font: &[u8]) -> (f32, f32, f32) {
        let u16_at = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]);
        let i16_at = |at: usize| i16::from_be_bytes([font[at], font[at + 1]]);
        let table = |tag: &[u8; 4]| {
            (0..u16_at(4) as usize)
                .map(|i| 12 + 16 * i)
                .find(|&record| &font[record..record + 4] == tag)
                .map(|record| {
                    u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize
                })
                .expect("table")
        };
        let (head, hhea) = (table(b"head"), table(b"hhea"));
        (
            f32::from(i16_at(hhea + 4)),
            -f32::from(i16_at(hhea + 6)),
            f32::from(u16_at(head + 18)),
        )
    }

    #[test]
    fn baseline_metrics_match_the_bundled_fonts() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/");
        for (family, file) in [
            (Family::Sans, "inter/Inter-Regular.ttf"),
            (Family::Sans, "inter/Inter-SemiBold.ttf"),
            (Family::SerifText, "literata/Literata-SemiBold.ttf"),
            (Family::SerifDisplay, "literata/Literata36pt-SemiBold.ttf"),
        ] {
            let font = std::fs::read(format!("{dir}{file}")).unwrap();
            let (ascent, descent, upm) = vertical_metrics(&font);
            let expected = (ascent - descent) / upm / 2.;
            assert!(
                (family.baseline_below_middle() - expected).abs() < 1e-4,
                "{file}: {} vs {expected}",
                family.baseline_below_middle()
            );
        }
    }

    #[test]
    fn smaller_text_moves_down_onto_the_lead_baseline() {
        // A page title's count: 20 px against 14 px Inter.
        let shift = TypeRole::Body.shift_onto(TypeRole::PageTitle);
        assert!((shift - 6. * 1490. / 4096.).abs() < 1e-4, "{shift}");
        // A sidebar count beside its label.
        assert!(TypeRole::Caption.shift_onto(TypeRole::Body) > 0.);
        // The same role needs no shift; the lead itself never moves.
        assert_eq!(TypeRole::Body.shift_onto(TypeRole::BodyStrong), 0.);
        assert_eq!(TypeRole::Display30.shift_onto(TypeRole::Display30), 0.);
        // Across families: body text beside a Literata figure.
        let across = TypeRole::Body.shift_onto(TypeRole::Display30);
        assert!(
            (across - (30. * 0.4345 - 14. * 1490. / 4096.)).abs() < 1e-3,
            "{across}"
        );
    }
}
