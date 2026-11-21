//! The one cover component (`hardcover-design`, Covers). Never draw a cover
//! box by hand: every book cover in the app is a [`Cover`].
//!
//! - Six sizes ([`CoverSize`]), always 2:3. Corners are 2 px for xs and sm,
//!   3 px from md; the image is clipped to them.
//! - A 1 px `cover-edge` over the image at every size, so a white cover
//!   doesn't melt into paper. From md up, the hinge on the spine side, and
//!   `shadow-cover` where the cover leads its row ([`Cover::shadow`]).
//! - A skeleton while the image loads, and the cloth cover when the book has
//!   no image or it failed: a cloth colour picked by the title, the initial
//!   (xs, sm), the stamped frame, the title and the author (md and up).
//!
//! Images go through the cover pipeline (`app_state`): a list cover asks for
//! the thumbnail itself ([`Cover::thumbnail`]), decoded and scaled on the
//! cover-decoder thread, 144 px high for xs and sm and 288 px from md
//! ([`CoverSize::image_height`]), so each stays sharp on a 2x or 3x screen. The book pane loads and releases its own detail
//! cover and passes it in ([`Cover::art`], [`detail_art`]).
//!
//! ```ignore
//! Cover::new(CoverSize::Sm, book.title.clone())
//!     .thumbnail(book.cover.as_ref().map(|c| c.url.clone()))
//!     .opens(("row-cover", book.id.get()), Page::Book(book.clone()))
//! ```
//!
//! A click opens the book, but the cover is never the only keyboard path to
//! it: its title is a button too. Like that title, it keeps the arrow
//! cursor: the pointing hand is for links (`desktop-practice.md`).
//!
//! The cloth cover's title and author are broken into lines here
//! (`WrappingText::width`), between words, at most four lines for the title
//! and two for the author, the last ending in "…" when more is cut. A word
//! wider than the cover's text column is set smaller, down to three
//! quarters of the size, so it stays whole; only a word too long even then
//! is cut between letters, never clipped.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, Pixels, RenderImage, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img,
    linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, rems,
};

use crate::app_state::{
    CoverImage, CoverMedium, CoverThumbnail, MEDIUM_COVER_HEIGHT, THUMBNAIL_HEIGHT,
};
use crate::bidi::{WrappingText, is_rtl};
use crate::catalog::navigation::{self, Page};
use crate::design::type_roles::Family;
use crate::design::{TypeRoles as _, colors};

/// Lines a cloth cover's title may take.
const CLOTH_TITLE_LINES: usize = 4;
/// Lines a cloth cover's author may take (lg and up).
const CLOTH_AUTHOR_LINES: usize = 2;
/// The smallest a cloth cover's type gets, as a share of its size, to keep
/// a long word whole.
const CLOTH_MIN_SCALE: f32 = 0.75;

/// The six cover sizes. Use the one the system names for the place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverSize {
    /// 20 × 30: compact rows, menus.
    // The system's size for a compact row or a menu; no screen has one yet.
    // The gallery and the tests draw it.
    #[cfg_attr(not(any(test, feature = "demo")), allow(dead_code))]
    Xs,
    /// 32 × 48: list rows (shelves, search, Discover, lists, series and
    /// author pages), upcoming rows.
    Sm,
    /// 64 × 96: Home rows, Up next, a rail's shelf.
    Md,
    /// 80 × 120: similar books.
    Lg,
    /// 128 × 192: the book pane's header; the Home lead.
    Xl,
    /// 160 × 240: the full book page's header.
    Xxl,
}

impl CoverSize {
    /// The width in px; the height is 1.5 times it.
    pub fn width_px(self) -> f32 {
        match self {
            Self::Xs => 20.,
            Self::Sm => 32.,
            Self::Md => 64.,
            Self::Lg => 80.,
            Self::Xl => 128.,
            Self::Xxl => 160.,
        }
    }

    pub fn width(self) -> gpui_kit::Rems {
        rems(self.width_px() / 16.)
    }

    pub fn height(self) -> gpui_kit::Rems {
        rems(self.width_px() * 1.5 / 16.)
    }

    /// The height [`Cover::thumbnail`] decodes this size at, in image pixels:
    /// at least 2.4 times the cover's own height up to lg.
    pub(crate) fn image_height(self) -> u32 {
        if self.is_small() {
            THUMBNAIL_HEIGHT
        } else {
            MEDIUM_COVER_HEIGHT
        }
    }

    /// xs and sm: no hinge, no shadow, and an initial for the cloth.
    fn is_small(self) -> bool {
        matches!(self, Self::Xs | Self::Sm)
    }

    /// `r-cover-s` for xs and sm, `r-cover` from md: 2 and 3 design px, so
    /// they follow the rem with the cover.
    pub(crate) fn radius(self) -> gpui_kit::Rems {
        if self.is_small() {
            rems(2. / 16.)
        } else {
            rems(3. / 16.)
        }
    }

    /// The cloth cover's title size in px. It stands in for an image, so it
    /// is the one place off the type scale.
    fn cloth_type(self) -> f32 {
        match self {
            Self::Xs => 10.,
            Self::Sm => 16.,
            Self::Md => 9.,
            Self::Lg => 11.,
            Self::Xl => 18.,
            Self::Xxl => 22.,
        }
    }

    /// Whether the cloth cover names the author (lg and up).
    fn cloth_has_author(self) -> bool {
        matches!(self, Self::Lg | Self::Xl | Self::Xxl)
    }
}

/// Where a cover's image is.
#[derive(Clone)]
pub enum CoverArt {
    /// Still loading: the skeleton.
    Loading,
    /// The image.
    Image(Arc<RenderImage>),
    /// No image, or it couldn't be loaded: the cloth cover.
    None,
}

impl CoverArt {
    fn loaded(image: Option<Arc<RenderImage>>) -> Self {
        image.map_or(Self::None, Self::Image)
    }

    /// The image, once it has loaded.
    pub fn image(&self) -> Option<Arc<RenderImage>> {
        match self {
            Self::Image(image) => Some(image.clone()),
            _ => None,
        }
    }
}

/// The detail cover for `url` (the book pane's), through the same pipeline as
/// the other covers, scaled to `app_state::COVER_HEIGHT`. The caller releases
/// it with `app_state::release_cover_image`.
pub fn detail_art(url: &str, window: &mut Window, cx: &mut App) -> CoverArt {
    window
        .use_asset::<CoverImage>(&url.to_owned(), cx)
        .map_or(CoverArt::Loading, CoverArt::loaded)
}

/// The thumbnail for `url` at `size`'s [`CoverSize::image_height`].
fn thumbnail_art(url: &str, size: CoverSize, window: &mut Window, cx: &mut App) -> CoverArt {
    let url = url.to_owned();
    let image = match size.image_height() {
        THUMBNAIL_HEIGHT => window.use_asset::<CoverThumbnail>(&url, cx),
        _ => window.use_asset::<CoverMedium>(&url, cx),
    };
    image.map_or(CoverArt::Loading, CoverArt::loaded)
}

enum Source {
    /// A thumbnail URL, loaded when the cover renders; none for no image.
    Thumbnail(Option<SharedString>),
    /// An image the caller loaded.
    Art(CoverArt),
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A book's cover. See the module doc.
#[derive(IntoElement)]
pub struct Cover {
    size: CoverSize,
    title: SharedString,
    author: Option<SharedString>,
    source: Source,
    shadow: bool,
    click: Option<(ElementId, ClickHandler)>,
}

impl Cover {
    /// The cover of the book titled `title`, at `size`. With no image given
    /// it is the cloth cover.
    pub fn new(size: CoverSize, title: impl Into<SharedString>) -> Self {
        Self {
            size,
            title: title.into(),
            author: None,
            source: Source::Thumbnail(None),
            shadow: false,
            click: None,
        }
    }

    /// The author, named at the foot of a cloth cover from lg up.
    pub fn author(mut self, author: Option<impl Into<SharedString>>) -> Self {
        self.author = author.map(Into::into);
        self
    }

    /// Loads the image at `url`, decoded on the cover-decoder thread at the
    /// size's [`CoverSize::image_height`]. `None` is a book without an image.
    /// md and lg images stay loaded for the run unless released
    /// (`app_state::release_cover_medium`), as similar books are.
    pub fn thumbnail(mut self, url: Option<impl Into<SharedString>>) -> Self {
        self.source = Source::Thumbnail(url.map(Into::into));
        self
    }

    /// An image the caller loaded: the book pane's detail cover.
    pub fn art(mut self, art: CoverArt) -> Self {
        self.source = Source::Art(art);
        self
    }

    /// `shadow-cover`, where the cover leads its row or block (Home's
    /// reading rows, the book header, Up next, similar books). Only from md
    /// up; xs and sm never take it.
    pub fn shadow(mut self, shadow: bool) -> Self {
        self.shadow = shadow;
        self
    }

    /// A click on the cover runs `handler`. `id` must be unique among the
    /// cover's siblings.
    pub fn on_click(
        mut self,
        id: impl Into<ElementId>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some((id.into(), Rc::new(handler)));
        self
    }

    /// A click on the cover opens `page`, usually the book's.
    pub fn opens(self, id: impl Into<ElementId>, page: Page) -> Self {
        self.on_click(id, move |_, _, cx| navigation::open(page.clone(), cx))
    }
}

impl RenderOnce for Cover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let size = self.size;
        let radius = size.radius();
        let art = match self.source {
            Source::Thumbnail(Some(url)) => thumbnail_art(&url, size, window, cx),
            Source::Thumbnail(None) => CoverArt::None,
            Source::Art(art) => art,
        };
        let loading = matches!(art, CoverArt::Loading);

        let cover = div()
            .relative()
            .flex_none()
            .w(size.width())
            .h(size.height())
            .rounded(radius)
            .when(self.shadow && !size.is_small(), |this| {
                this.shadow(colors.shadow_cover())
            })
            .map(|this| match art {
                CoverArt::Loading => this.bg(colors.skeleton),
                CoverArt::Image(image) => this.bg(colors.skeleton).child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .rounded(radius)
                        .object_fit(ObjectFit::Cover),
                ),
                CoverArt::None => this.bg(colors.cloth_for(&self.title)).child(cloth(
                    size,
                    &self.title,
                    self.author.as_ref(),
                    window.rem_size(),
                    cx,
                )),
            })
            // The hinge: a strip on the spine side, from cover-hinge to clear.
            .when(!size.is_small() && !loading, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(rems(size.width_px() * 0.06 / 16.))
                        .rounded_l(radius)
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(colors.cover_hinge, 0.),
                            linear_color_stop(colors.cover_hinge.opacity(0.), 1.),
                        )),
                )
            })
            // The edge, over the image.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(radius)
                    .border_1()
                    .border_color(colors.cover_edge),
            );

        match self.click {
            Some((id, handler)) => cover
                .id(id)
                .on_click(move |event, window, cx| handler(event, window, cx))
                .into_any_element(),
            None => cover.into_any_element(),
        }
    }
}

/// The letter a small cloth cover shows: the title's first, skipping a
/// leading "The " or "Untitled " (Hardcover's placeholder titles for
/// unannounced books, "Untitled Empyrean #4").
pub fn initial(title: &str) -> Option<char> {
    let title = title
        .strip_prefix("The ")
        .or_else(|| title.strip_prefix("Untitled "))
        .unwrap_or(title);
    title.chars().next()
}

/// The cloth cover's content: an initial (xs, sm), or the stamped frame, the
/// title, the rule and, from lg, the author. Its measures are design px of
/// the cover's own (`dp`), so it scales with the cover's rem-based box; the
/// frame and the rules stay one device-independent pixel, as hairlines.
fn cloth(
    size: CoverSize,
    title: &str,
    author: Option<&SharedString>,
    rem: Pixels,
    cx: &App,
) -> AnyElement {
    let ink = colors(cx).cloth_ink;
    let dp = |design: f32| rem * (design / 16.);
    let w = size.width_px();
    let h = w * 1.5;
    let type_px = size.cloth_type();
    let base = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .text_center()
        .overflow_hidden()
        .type_family(Family::SerifText)
        .text_color(ink)
        .text_size(dp(type_px));

    if size.is_small() {
        return base
            .justify_center()
            .gap(dp(3.))
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(dp(type_px))
            .children(initial(title).map(|c| SharedString::from(c.to_string())))
            .when(size == CoverSize::Sm, |this| {
                this.child(div().flex_none().w(dp(8.)).h(px(1.)).bg(ink.opacity(0.6)))
            })
            .into_any_element();
    }

    // Padding 16% on top, 13% on the sides and bottom; the content is 74% of
    // the width.
    let content = w * 0.74;
    let rtl = is_rtl(title);
    // Vazirmatn at the same size; Arabic script needs a 1.5 line height.
    let title_px = type_px;
    let title_line = if rtl { type_px * 1.5 } else { type_px * 1.2 };
    base.pt(dp(w * 0.16))
        .px(dp(w * 0.13))
        .pb(dp(w * 0.13))
        // The blind-stamped frame, inset 5%.
        .child(
            div()
                .absolute()
                .top(dp(h * 0.05))
                .bottom(dp(h * 0.05))
                .left(dp(w * 0.05))
                .right(dp(w * 0.05))
                .rounded(px(1.))
                .border_1()
                .border_color(ink.opacity(0.28)),
        )
        .child(
            div()
                .w_full()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(dp(title_px))
                .line_height(dp(title_line))
                .child(
                    WrappingText::new(title.to_owned())
                        .centered()
                        .width(dp(content))
                        .line_clamp(CLOTH_TITLE_LINES)
                        .fit_words(CLOTH_MIN_SCALE),
                ),
        )
        .child(
            div()
                .flex_none()
                .mt(dp(content * 0.10))
                .w(dp(content * 0.24))
                .h(px(1.))
                .bg(ink.opacity(0.6)),
        )
        .when_some(
            author.filter(|_| size.cloth_has_author()).cloned(),
            |this, author| {
                let author_px = type_px * 0.8;
                this.child(
                    div()
                        .mt_auto()
                        .w_full()
                        .text_color(ink.opacity(0.85))
                        .text_size(dp(author_px))
                        .line_height(dp(author_px * 1.2))
                        .child(
                            WrappingText::new(author)
                                .centered()
                                .width(dp(content))
                                .line_clamp(CLOTH_AUTHOR_LINES)
                                .fit_words(CLOTH_MIN_SCALE),
                        ),
                )
            },
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initial_skips_the_and_untitled() {
        assert_eq!(initial("Dune"), Some('D'));
        assert_eq!(initial("The Way of Kings"), Some('W'));
        assert_eq!(initial("Untitled Empyrean #4"), Some('E'));
        // Only one leading word, as the mockup does.
        assert_eq!(initial("The Untitled"), Some('U'));
        // Not inside a word.
        assert_eq!(initial("Theory"), Some('T'));
        assert_eq!(initial("ملت عشق"), Some('م'));
        assert_eq!(initial(""), None);
        assert_eq!(initial("The "), None);
    }

    #[test]
    fn covers_are_two_by_three() {
        for size in [
            CoverSize::Xs,
            CoverSize::Sm,
            CoverSize::Md,
            CoverSize::Lg,
            CoverSize::Xl,
            CoverSize::Xxl,
        ] {
            assert_eq!(size.height().0, size.width().0 * 1.5);
        }
        assert_eq!(CoverSize::Sm.width_px(), 32.);
        assert_eq!(CoverSize::Xl.height().0 * 16., 192.);
    }

    #[test]
    fn thumbnails_are_sharp_at_two_and_a_half_times_up_to_lg() {
        for size in [CoverSize::Xs, CoverSize::Sm, CoverSize::Md, CoverSize::Lg] {
            let shown = size.height().0 * 16.;
            assert!(
                size.image_height() as f32 >= shown * 2.4,
                "{size:?}: {} for {shown}",
                size.image_height(),
            );
        }
        // The row thumbnail stays small: rows are many and kept for the run.
        assert_eq!(CoverSize::Sm.image_height(), 144);
        assert_eq!(CoverSize::Md.image_height(), 288);
    }

    #[test]
    fn small_covers_have_tighter_corners_and_no_lift() {
        assert_eq!(CoverSize::Xs.radius(), rems(0.125));
        assert_eq!(CoverSize::Sm.radius(), rems(0.125));
        assert_eq!(CoverSize::Md.radius(), rems(0.1875));
        assert!(CoverSize::Sm.is_small() && !CoverSize::Md.is_small());
        assert!(!CoverSize::Md.cloth_has_author() && CoverSize::Lg.cloth_has_author());
    }
}
