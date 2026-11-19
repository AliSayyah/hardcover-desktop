//! The About dialog: what the app is, its version, and where to find it.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::link::Link;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, FontWeight, Image, ImageFormat, ParentElement as _, Styled as _, Window, div, img, rems,
};
use hardcover_core::update::REPOSITORY;

/// The app icon, as installed with the desktop entry.
static ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../assets/icon.svg").to_vec(),
    ))
});

pub fn open_about(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, window, cx| {
        let muted = cx.theme().muted_foreground;
        let repository = format!("https://github.com/{REPOSITORY}");
        dialog.w(rems(24.).to_pixels(window.rem_size())).child(
            v_flex()
                .items_center()
                .text_center()
                .gap_4()
                .pb_2()
                .child(img(ICON.clone()).size_16())
                .child(
                    v_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Hardcover Desktop"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                        ),
                )
                .child(div().text_sm().child(
                    "An unofficial Linux desktop client for Hardcover, the social reading tracker.",
                ))
                .child(
                    h_flex()
                        .gap_4()
                        .text_sm()
                        .child(
                            Link::new("about-hardcover")
                                .href("https://hardcover.app")
                                .child("hardcover.app"),
                        )
                        .child(
                            Link::new("about-source")
                                .href(repository.clone())
                                .child("Source code"),
                        )
                        .child(
                            Link::new("about-issues")
                                .href(format!("{repository}/issues"))
                                .child("Report a problem"),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("MIT licence. Not affiliated with or endorsed by Hardcover."),
                ),
        )
    });
}
