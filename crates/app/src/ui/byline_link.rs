//! Byline link (`hardcover-design`, Buttons and links): a person's name that
//! opens their Hardcover profile in the browser. Ink, 14/500, underlined on
//! hover, with the pointing hand. It repeats on every review, so it stays
//! quiet: no accent, no external-link icon. A right-to-left name goes without
//! the underline, which GPUI on Linux draws beside it.
//!
//! Only for names that open the browser. An in-app destination is a
//! [`super::TextButton`]; "Open on Hardcover" is an [`super::ExternalLink`].
//!
//! ```ignore
//! BylineLink::new(("reviewer", id), name, profile_url)
//! ```

use gpui_kit::base::Button as BaseButton;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::focus::{FocusTreatment as _, control_radius, focus_visible, keyed_focus};
use super::reveal::probe;
use crate::bidi::is_rtl;
use crate::design::TypeRoles as _;

/// A name that opens a profile in the browser. See the module doc.
#[derive(IntoElement)]
pub struct BylineLink {
    id: ElementId,
    name: SharedString,
    url: SharedString,
}

impl BylineLink {
    /// `name` linking to `url`. `id` must be unique among its siblings.
    pub fn new(
        id: impl Into<ElementId>,
        name: impl Into<SharedString>,
        url: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            url: url.into(),
        }
    }
}

impl RenderOnce for BylineLink {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = keyed_focus(self.id.clone(), window, cx);
        let focused = focus_visible(&focus, window);
        let radius = control_radius(cx);
        let underline = !is_rtl(&self.name);
        let url = self.url;
        probe(
            div().flex().min_w_0().child(
                BaseButton::new(self.id)
                    .role(Role::Link)
                    .track_focus(&focus)
                    .accessibility_label(self.name.clone())
                    .relative()
                    .min_w_0()
                    .justify_start()
                    .rounded(radius)
                    .cursor_pointer()
                    .text_color(cx.theme().foreground)
                    .body_strong()
                    .when(underline, |this| this.hover(|style| style.underline()))
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .when(focused, |this| this.focus_ring_around(radius, cx))
                    .child(div().min_w_0().truncate().child(self.name)),
            ),
        )
    }
}
