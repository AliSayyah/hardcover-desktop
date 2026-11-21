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
