//! The rating star, shared by the rating control, the library rows and the
//! catalogue's ratings.
//!
//! A solid star with rounded points, bundled as SVG: Lucide's star is thin
//! and pointed, and its outline showed through a half-filled star. An empty
//! star is the same shape in a faint solid, so a half star is a clean split
//! and empty stars don't outweigh the filled ones.
//!
//! The colours are tokens (`design::colors`): `gold` for the user's own
//! rating, nothing else; `fg_muted` for catalogue figures (the Hardcover
//! average, the ratings breakdown); `star_empty` for an unfilled star.

use gpui_kit::component::Icon;

const SVG: &[u8] = include_bytes!("../../../assets/icons/star.svg");

/// The star. Size and color are set by the caller, or come from the text.
pub fn icon() -> Icon {
    Icon::empty().data(SVG)
}
