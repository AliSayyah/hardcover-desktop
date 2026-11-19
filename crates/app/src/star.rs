//! The rating star, shared by the rating control, the library rows and the
//! catalogue's ratings.
//!
//! A solid star with rounded points, bundled as SVG: Lucide's star is thin
//! and pointed, and its outline showed through a half-filled star. An empty
//! star is the same shape in a faint solid, so a half star is a clean split
//! and empty stars don't outweigh the filled ones.
//!
//! Gold is kept for the user's own rating; catalogue figures (the Hardcover
//! average, the ratings breakdown) use the star in the text's muted colour.

use gpui_kit::component::{ActiveTheme as _, Icon, amber_400, amber_500};
use gpui_kit::{App, Hsla};

const SVG: &[u8] = include_bytes!("../../../assets/icons/star.svg");

/// The star. Size and color are set by the caller, or come from the text.
pub fn icon() -> Icon {
    Icon::empty().data(SVG)
}

/// The color of a filled star in the user's own rating.
pub fn color(cx: &App) -> Hsla {
    // Amber reads as gold on both backgrounds; the theme's `yellow` is a dull
    // mustard in light mode.
    if cx.theme().is_dark() {
        amber_400()
    } else {
        amber_500()
    }
}

/// The color of an empty star: a faint shade of the text.
pub fn empty_color(cx: &App) -> Hsla {
    cx.theme().foreground.opacity(0.13)
}
