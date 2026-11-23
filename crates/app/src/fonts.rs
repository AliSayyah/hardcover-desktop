//! The app's fonts, bundled in the binary: Inter for the interface, Literata
//! for the book, and Vazirmatn under both for Persian and Arabic text.
//!
//! Every face is a static font. The theme's default family is GPUI's
//! `.SystemUIFont`, which on Linux often lands on a variable font (Adwaita
//! Sans on GNOME, recent Cantarell), and GPUI's Linux text system draws only
//! a variable font's default instance: every medium, semibold and bold run
//! came out regular. Static faces give the real weights on every desktop.
//!
//! The families, as the design system names them (`hardcover-design`, Type):
//! - **Inter** ([`INTER`]): regular, italic (review text), medium, semibold
//!   and bold. Everything in the interface. A weight in between, or bold
//!   italic, is drawn with the nearest of these.
//! - **Literata Text** ([`LITERATA_TEXT`], the 12 pt optical cut): regular,
//!   italic and semibold, for book titles up to 18 px and descriptions.
//! - **Literata Display** ([`LITERATA_DISPLAY`], the 36 pt cut): semibold,
//!   for book titles and figures from 24 px. The two cuts are two families,
//!   because GPUI can't pick an optical size.
//! - **Vazirmatn** ([`VAZIRMATN`]): regular, medium and semibold. Persian and
//!   Arabic, under Inter and under Literata alike.
//!
//! Inter and Literata have no Arabic script, and GPUI's own fallback picks a
//! system font for it (cosmic-text asks for Noto Sans Arabic). So the
//! fallback is named with the family (`design::type_roles::Family`): by each
//! type role, on the shell's root, and by `WrappingText` for the family it
//! inherits, which also covers dialogs and popovers, as they don't inherit
//! from the shell.
//!
//! GPUI loads a family, as the primary font or a fallback, only if it maps
//! the letter "m". Vazirmatn does (it has a Latin set); a family without
//! Latin, such as Noto Naskh Arabic, would be dropped silently.
//!
//! The faces come from the projects' releases: Inter 4, Literata 3.103
//! (`fonts/ttf`) and Vazirmatn 33.003 (`fonts/ttf`). All are under the SIL
//! Open Font License, shipped
//! next to them as `assets/fonts/<family>/LICENSE.txt` and installed by
//! `packaging/linux/package.sh`.

use std::borrow::Cow;
use std::sync::Arc;

use gpui_kit::component::Theme;
use gpui_kit::{App, FontFallbacks, FontStyle, FontWeight, TextSystem, font};

/// The interface family.
pub const INTER: &str = "Inter";
/// Literata's 12 pt optical cut: book type up to 18 px.
pub const LITERATA_TEXT: &str = "Literata";
/// Literata's 36 pt optical cut: book type from 24 px.
pub const LITERATA_DISPLAY: &str = "Literata 36pt";
/// Persian and Arabic, under Inter and Literata.
pub const VAZIRMATN: &str = "Vazirmatn";

macro_rules! face {
    ($path:literal) => {
        include_bytes!(concat!("../../../assets/fonts/", $path))
    };
}

/// Every bundled face. [`init`] registers these, and the test below checks
/// each of them.
const FACES: [&[u8]; 12] = [
    face!("inter/Inter-Regular.ttf"),
    face!("inter/Inter-Italic.ttf"),
    face!("inter/Inter-Medium.ttf"),
    face!("inter/Inter-SemiBold.ttf"),
    face!("inter/Inter-Bold.ttf"),
    face!("literata/Literata-Regular.ttf"),
    face!("literata/Literata-Italic.ttf"),
    face!("literata/Literata-SemiBold.ttf"),
    face!("literata/Literata36pt-SemiBold.ttf"),
    face!("vazirmatn/Vazirmatn-Regular.ttf"),
    face!("vazirmatn/Vazirmatn-Medium.ttf"),
    face!("vazirmatn/Vazirmatn-SemiBold.ttf"),
];

/// Registers the faces and makes Inter the theme's interface font. Runs
/// after `gpui_kit::init`, which creates the theme. If the faces can't be
/// loaded, the system font stays and this returns false.
pub fn init(cx: &mut App) -> bool {
    let faces = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    if let Err(error) = cx.text_system().add_fonts(faces) {
        eprintln!("hardcover: couldn't load the bundled fonts, using the system font: {error:#}");
        return false;
    }
    // The app's theme configs name Inter too (`design::theme`), so a theme
    // change keeps it.
    Theme::update(cx, |theme| theme.font_family = INTER.into());
    true
}

/// Warms the text system's font caches on a thread of its own, while the
/// window opens (GPUI sets up the GPU there, for a few hundred ms).
///
/// The first time a family is shaped at a weight and style, cosmic-text
/// ranks every installed font against it (`get_font_matches`), opening the
/// file of each whose weight differs to look for a variable `wght` axis. With
/// hundreds of system fonts that took a noticeable part of the first frame,
/// which uses several combinations. Done here, it finds them ranked. Each
/// font takes the text system's lock on its own, so a frame that needs it
/// waits for one at most. The most used come first.
pub fn prewarm(cx: &App) {
    let text_system: Arc<TextSystem> = cx.text_system().clone();
    let spawned = std::thread::Builder::new()
        .name("font-prewarm".into())
        .spawn(move || {
            let fallbacks = FontFallbacks::from_fonts(vec![VAZIRMATN.to_owned()]);
            let normal = FontStyle::Normal;
            let italic = FontStyle::Italic;
            let faces = [
                (INTER, FontWeight::NORMAL, normal),
                (INTER, FontWeight::MEDIUM, normal),
                (INTER, FontWeight::SEMIBOLD, normal),
                (LITERATA_DISPLAY, FontWeight::SEMIBOLD, normal),
                (LITERATA_TEXT, FontWeight::SEMIBOLD, normal),
                (LITERATA_TEXT, FontWeight::NORMAL, normal),
                (INTER, FontWeight::NORMAL, italic),
                (LITERATA_TEXT, FontWeight::NORMAL, italic),
            ];
            for (family, weight, style) in faces {
                let mut face = font(family);
                face.weight = weight;
                face.style = style;
                face.fallbacks = Some(fallbacks.clone());
                text_system.prewarm_fonts(&[face]);
            }
        });
    if let Err(error) = spawned {
        eprintln!("hardcover: couldn't start warming the fonts: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table tags of a TrueType or OpenType font.
    fn tables(face: &[u8]) -> Vec<&[u8]> {
        let count = u16::from_be_bytes([face[4], face[5]]) as usize;
        (0..count)
            .map(|i| &face[12 + 16 * i..12 + 16 * i + 4])
            .collect()
    }

    /// The faces must be static: a variable font is what this module avoids.
    /// `fvar` is the table that makes a font variable.
    #[test]
    fn every_bundled_face_is_a_static_font() {
        for (i, face) in FACES.iter().enumerate() {
            let tags = tables(face);
            assert!(
                tags.contains(&&b"glyf"[..]),
                "face {i} is not a TrueType font"
            );
            assert!(!tags.contains(&&b"fvar"[..]), "face {i} is a variable font");
            assert!(!tags.contains(&&b"gvar"[..]), "face {i} has variations");
        }
    }
}
