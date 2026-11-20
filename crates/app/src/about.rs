//! The About dialog: what the app is, its version, and where to find it.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::link::Link;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{App, Image, ImageFormat, ParentElement as _, Styled as _, Window, div, img};
use hardcover_core::update::REPOSITORY;

use crate::design::{TypeRoles as _, size};
use crate::ui::kit::open_dialog;

/// The app icon, as installed with the desktop entry.
static ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../assets/icon.svg").to_vec(),
    ))
});

pub fn open_about(window: &mut Window, cx: &mut App) {
    open_dialog(window, cx, |dialog, window, cx| {
        let repository = format!("https://github.com/{REPOSITORY}");
        // Dialogs don't inherit the shell's family, so the roles set it.
        dialog
            .w(size::DIALOG_CONFIRM.to_pixels(window.rem_size()))
            .child(
            v_flex()
                .items_center()
                .text_center()
                .gap_4()
                .pb_2()
                .body()
                .child(img(ICON.clone()).size_16())
                .child(
                    v_flex()
                        .items_center()
                        .child(div().section().child("Hardcover Desktop"))
                        .child(
                            div()
                                .caption(cx)
                                .tabular()
                                .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                        ),
                )
                .child(div().child(
                    "An unofficial Linux desktop client for Hardcover, the social reading tracker.",
                ))
                .child(
                    h_flex()
                        .gap_4()
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
                        .caption(cx)
                        .child("MIT licence. Not affiliated with or endorsed by Hardcover."),
                ),
        )
    });
}
