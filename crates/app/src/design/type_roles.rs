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

    /// A destination's title: Inter 600, 20/28.
    fn page_title(self) -> Self {
        self.type_role(Family::Sans, FontWeight::SEMIBOLD, scale::XL)
    }

    /// A section on a page: Inter 600, 16/24.
    fn section(self) -> Self {
        self.type_role(Family::Sans, FontWeight::SEMIBOLD, scale::BASE)
    }

    /// A block in a pane: Inter 600, 14/20.
    fn heading(self) -> Self {
        self.type_role(Family::Sans, FontWeight::SEMIBOLD, scale::SM)
    }

    /// Everything by default: Inter 400, 14/20.
    fn body(self) -> Self {
        self.type_role(Family::Sans, FontWeight::NORMAL, scale::SM)
    }

    /// Row titles, values that lead: Inter 500, 14/20.
    fn body_strong(self) -> Self {
        self.type_role(Family::Sans, FontWeight::MEDIUM, scale::SM)
    }

    /// Sidebar and rail section labels: Inter 500, 12/16, muted.
    fn label(self, cx: &App) -> Self {
        self.type_role(Family::Sans, FontWeight::MEDIUM, scale::XS)
            .text_color(cx.theme().muted_foreground)
    }

    /// Counts, trailing figures, timestamps, helper text: Inter 400, 12/16,
    /// muted.
    fn caption(self, cx: &App) -> Self {
        self.type_role(Family::Sans, FontWeight::NORMAL, scale::XS)
            .text_color(cx.theme().muted_foreground)
    }

    /// The book title on the full page, the Home greeting, headline figures:
    /// Literata Display 600, 30/36, lining figures.
    fn display_30(self) -> Self {
        self.type_role(Family::SerifDisplay, FontWeight::SEMIBOLD, scale::XXXL)
            .font_features(features(&["lnum"]))
    }

    /// The book title in the pane and the Home lead: Literata Display 600,
    /// 24/30, lining figures.
    fn display_24(self) -> Self {
        self.type_role(Family::SerifDisplay, FontWeight::SEMIBOLD, scale::XXL)
            .font_features(features(&["lnum"]))
    }

    /// Book titles in Home rows and Up next: Literata Text 600, 18/24.
    fn title_18(self) -> Self {
        self.type_role(Family::SerifText, FontWeight::SEMIBOLD, scale::LG)
    }

    /// Descriptions: Literata Text 400, 16/26. At most
    /// [`super::size::PROSE`] wide.
    fn prose(self) -> Self {
        self.type_role(Family::SerifText, FontWeight::NORMAL, scale::PROSE)
    }

    /// Readers' words (reviews, long notes): Inter 400, 14/22. At most
    /// [`super::size::PROSE`] wide.
    fn reader(self) -> Self {
        self.type_role(Family::Sans, FontWeight::NORMAL, scale::READER)
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
}
