//! The interface font: Inter, bundled in the binary.
//!
//! The theme's default family is GPUI's `.SystemUIFont`. On Linux that often
//! lands on a variable font (Adwaita Sans on GNOME, recent Cantarell), and
//! GPUI's Linux text system draws only a variable font's default instance:
//! every medium, semibold and bold run came out regular, so titles and
//! section headings looked like body text. Static faces give the real
//! weights on every desktop. Inter is also what Adwaita Sans is drawn from,
//! so GNOME users see a familiar face.
//!
//! Only the faces the app and its components use are bundled: regular,
//! italic (review text), medium, semibold and bold. A weight in between, or
//! bold italic, is drawn with the nearest of these. Scripts Inter lacks fall
//! back to the system's fonts glyph by glyph. Inter is under the SIL Open
//! Font License, shipped next to it as `assets/fonts/inter/LICENSE.txt`.

use std::borrow::Cow;

use gpui_kit::App;
use gpui_kit::component::Theme;

/// The family the faces below register under.
pub const FAMILY: &str = "Inter";

const FACES: [&[u8]; 5] = [
    include_bytes!("../../../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Italic.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Bold.ttf"),
];

/// Registers Inter and makes it the theme's interface font. Runs after
/// `gpui_kit::init`, which creates the theme. If the faces can't be loaded,
/// the system font stays.
pub fn init(cx: &mut App) {
    let faces = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    if let Err(error) = cx.text_system().add_fonts(faces) {
        eprintln!("hardcover: couldn't load the bundled font, using the system font: {error:#}");
        return;
    }
    // A theme change keeps the family: the built-in themes don't name one.
    Theme::update(cx, |theme| theme.font_family = FAMILY.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The faces must be static: a variable font is what this module avoids.
    /// `fvar` is the table that makes a font variable.
    #[test]
    fn every_bundled_face_is_a_static_font() {
        for face in FACES {
            let tables = u16::from_be_bytes([face[4], face[5]]) as usize;
            let tags: Vec<&[u8]> = (0..tables)
                .map(|i| &face[12 + 16 * i..12 + 16 * i + 4])
                .collect();
            assert!(tags.contains(&&b"glyf"[..]), "not a TrueType font");
            assert!(!tags.contains(&&b"fvar"[..]), "a variable font");
        }
    }
}
