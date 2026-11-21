use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    App, Asset, IntoElement as _, ParentElement as _, RenderImage, Styled as _, TestAppContext,
    div, img, px,
};

use super::usage::Budget;
use super::{CoverService, CoverThumbnail, Usage, release, usage};
use crate::cover::{Cover, CoverSize};
use crate::ui::test_support::{host, redraw};

fn url(i: usize) -> String {
    format!("https://assets.hardcover.app/edition/{i}/cover.png")
}

// GPUI's test scheduler refuses wake-ups from other threads, so these tests
// keep the decoder out: the wiring with no cache (a load ends at once, with
// no image), the textures with a test asset.

#[gpui_kit::test]
fn thumbnails_drawn_past_the_budget_are_released(cx: &mut TestAppContext) {
    let budget = Budget { keep: 2, slack: 1 };
    cx.update(|cx| {
        cx.set_global(CoverService::with_cache(
            None,
            Usage::new(budget, usage::MEDIUM_BUDGET),
        ));
    });
    let shown = Rc::new(Cell::new(0));
    let cx = host(cx, {
        let shown = shown.clone();
        move |_, _| {
            Cover::new(CoverSize::Sm, "A synthetic book")
                .thumbnail(Some(url(shown.get())))
                .into_any_element()
        }
    });
    // One cover at a time, as rows scroll by.
    for i in 0..4 {
        shown.set(i);
        redraw(cx);
        cx.run_until_parked();
    }
    // The fourth went past keep + slack: the two drawn longest ago went.
    for i in 0..4 {
        let cached = cx.update(|_, cx| cx.has_asset::<CoverThumbnail>(&url(i)));
        assert_eq!(cached, i >= 2, "cover {i}");
    }
    // Drawn again, a released cover loads again.
    shown.set(0);
    redraw(cx);
    assert!(cx.update(|_, cx| cx.has_asset::<CoverThumbnail>(&url(0))));
}

/// A view that shows one cover and sits still, embedded cached.
struct Still;

impl gpui_kit::Render for Still {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        div()
            .size_full()
            .child(Cover::new(CoverSize::Sm, "A still book").thumbnail(Some(url(100))))
    }
}

/// Covers scrolling by beside a still, cached view.
struct Scrolling {
    still: gpui_kit::Entity<Still>,
    shown: Rc<Cell<usize>>,
}

impl gpui_kit::Render for Scrolling {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        div()
            .size_full()
            .child(
                div().size(px(100.)).child(
                    self.still
                        .clone()
                        .cached(gpui_kit::StyleRefinement::default().size_full()),
                ),
            )
            .child(
                Cover::new(CoverSize::Sm, "A synthetic book")
                    .thumbnail(Some(url(self.shown.get()))),
            )
    }
}

/// A cover a cached view shows without drawing it again is not lost when
/// others scroll past the budget: the release draws every view again, and
/// the still view's cover loads again.
#[gpui_kit::test]
fn a_still_cached_view_keeps_its_cover(cx: &mut TestAppContext) {
    use gpui_kit::AppContext as _;

    let budget = Budget { keep: 2, slack: 1 };
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(CoverService::with_cache(
            None,
            Usage::new(budget, usage::MEDIUM_BUDGET),
        ));
    });
    let shown = Rc::new(Cell::new(0));
    let (root, cx) = cx.add_window_view({
        let shown = shown.clone();
        move |_, cx| Scrolling {
            still: cx.new(|_| Still),
            shown,
        }
    });
    redraw(cx);
    cx.run_until_parked();
    for i in 0..6 {
        shown.set(i);
        root.update(cx, |_, cx| cx.notify());
        redraw(cx);
        cx.run_until_parked();
        redraw(cx);
    }
    assert!(
        cx.update(|_, cx| cx.has_asset::<CoverThumbnail>(&url(100))),
        "the still view's cover is loaded"
    );
}

/// A cover that loads at once.
enum TestCover {}

impl Asset for TestCover {
    type Source = String;
    type Output = Option<Arc<RenderImage>>;

    fn load(_: String, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        let pixels = image::RgbaImage::from_pixel(4, 6, image::Rgba([90, 40, 20, 255]));
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]));
        async move { Some(image) }
    }
}

#[gpui_kit::test]
fn a_released_cover_takes_its_texture_with_it(cx: &mut TestAppContext) {
    let cx = host(cx, |window, cx| {
        let image = window.use_asset::<TestCover>(&url(1), cx).flatten();
        div()
            .size(px(40.))
            .children(image.map(|image| img(image).size_full()))
            .into_any_element()
    });
    cx.run_until_parked();
    redraw(cx);
    let image = cx
        .update(|_, cx| cx.fetch_asset::<TestCover>(&url(1)))
        .flatten()
        .unwrap();
    assert!(cx.update(|window, _| window.has_image_atlas_entry(&image)));

    // As `release_unused` runs: deferred, outside any window's update.
    cx.update(|_, cx| cx.defer(|cx| release::<TestCover>(&url(1), None, cx)));
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| {
        cx.has_asset::<TestCover>(&url(1)) || window.has_image_atlas_entry(&image)
    }));
}
