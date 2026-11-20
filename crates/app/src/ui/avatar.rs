//! Avatar (`hardcover-design`, Reviews and Sidebar): a person as a circle.
//!
//! In order of preference:
//! 1. their picture: a raster one through the cover pipeline (decoded and
//!    scaled on the cover-decoder thread, like a list cover), an SVG (many
//!    Hardcover avatars) drawn from its cached file by GPUI;
//! 2. their initials on their Hardcover colour (`cached_image.color`), in
//!    white or ink, whichever reads ([`data_color`], [`text_on_data`]);
//! 3. their initials in `accent` on `accent-soft`.
//!
//! Initials are 12/600 at the 32 px size. A loading picture shows the
//! initials until it lands.
//!
//! ```ignore
//! Avatar::new(name)
//!     .image(user.avatar.as_ref().map(|a| a.url.clone()))
//!     .data_color(reviewer.color)
//! ```

use gpui_kit::{
    App, FontWeight, ImageSource, IntoElement, ObjectFit, ParentElement as _, RenderOnce,
    SharedString, Styled as _, StyledImage as _, Window, div, img,
};

use crate::app_state::{cover_file, cover_thumbnail};
use crate::design::color::{data_color, text_on_data};
use crate::design::type_roles::Family;
use crate::design::{TypeRoles as _, colors, size};

/// How big an avatar is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvatarSize {
    /// 32: beside a review, the sidebar's account.
    #[default]
    Md,
    /// 128: a person's page, where a book has its xl cover.
    Xl,
}

/// A person's avatar. See the module doc.
#[derive(IntoElement)]
pub struct Avatar {
    name: SharedString,
    image: Option<SharedString>,
    color: Option<[u8; 3]>,
    size: AvatarSize,
}

impl Avatar {
    /// The avatar of `name`, which gives the initials and the accessible
    /// name.
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            image: None,
            color: None,
            size: AvatarSize::Md,
        }
    }

    /// The picture's URL: a raster image or an SVG.
    pub fn image(mut self, url: Option<impl Into<SharedString>>) -> Self {
        self.image = url.map(Into::into);
        self
    }

    /// Their Hardcover colour, as RGB, for the initials' circle.
    pub fn data_color(mut self, color: Option<[u8; 3]>) -> Self {
        self.color = color;
        self
    }

    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let side = match self.size {
            AvatarSize::Md => size::AVATAR,
            AvatarSize::Xl => size::AVATAR_LARGE,
        };
        let picture = self.image.as_deref().and_then(|url| {
            if is_svg(url) {
                cover_file(url, window, cx).map(ImageSource::from)
            } else {
                cover_thumbnail(url, window, cx).map(ImageSource::from)
            }
        });
        let circle = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(side)
            .rounded_full()
            .overflow_hidden();
        if let Some(picture) = picture {
            return circle.child(
                img(picture)
                    .size_full()
                    .rounded_full()
                    .object_fit(ObjectFit::Cover),
            );
        }
        let (fill, ink) = match self.color {
            Some(rgb) => (data_color(rgb), text_on_data(rgb)),
            None => (colors.accent_soft, colors.accent),
        };
        // Initials: 12/600 at md; at xl, the type scale's 30/36 step.
        let (font_size, line) = match self.size {
            AvatarSize::Md => (12., 16.),
            AvatarSize::Xl => (30., 36.),
        };
        circle.bg(fill).child(
            div()
                .type_role(Family::Sans, FontWeight::SEMIBOLD, (font_size, line))
                .text_color(ink)
                .child(initials(&self.name)),
        )
    }
}

/// Whether `url` names an SVG, which the cover pipeline can't decode.
fn is_svg(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    path.to_ascii_lowercase().ends_with(".svg")
}

/// One or two letters for an avatar: the first letter of the first and last
/// words. "@user" counts from the letter after the "@".
pub fn initials(name: &str) -> String {
    let first_letter = |word: &str| word.chars().find(|c| c.is_alphanumeric());
    let words: Vec<&str> = name.split_whitespace().collect();
    let mut out = String::new();
    if let Some(c) = words.first().and_then(|w| first_letter(w)) {
        out.extend(c.to_uppercase());
    }
    if words.len() > 1
        && let Some(c) = words.last().and_then(|w| first_letter(w))
    {
        out.extend(c.to_uppercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_the_first_and_last_words() {
        assert_eq!(initials("Jane Quill"), "JQ");
        assert_eq!(initials("Jane van der Quill"), "JQ");
        assert_eq!(initials("@quill_reads"), "Q");
        assert_eq!(initials("شیرین"), "ش");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn svg_pictures_skip_the_cover_pipeline() {
        assert!(is_svg(
            "https://assets.hardcover.app/users/1/avatar.SVG?v=2"
        ));
        assert!(!is_svg("https://assets.hardcover.app/users/1/avatar.png"));
        assert!(!is_svg("https://assets.hardcover.app/svg/avatar.jpg#x.svg"));
    }
}
