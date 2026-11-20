//! External link (`hardcover-design`, Buttons and links → Link): something
//! that opens the browser, such as "Open on Hardcover". Accent, underlined,
//! with a 12 px external-link icon after the words, and the pointing hand.
//!
//! Unlike the kit's `Link`, it is a real control: Tab reaches it, Enter and
//! Space open it, and it shows the focus ring around its words.
//!
//! Only for things outside the app. In-app navigation is a
//! [`super::TextButton`]; a person's name that opens their profile is a
//! [`super::BylineLink`].
//!
//! ```ignore
//! ExternalLink::new("open-on-hardcover", "Open on Hardcover", url)
//! ```

use gpui_kit::base::Button as BaseButton;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::{
    App, ElementId, IntoElement, ParentElement as _, RenderOnce, Role, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};

use super::focus::{FocusTreatment as _, control_radius, focus_visible, keyed_focus};
use super::reveal::probe;
use crate::design::{TypeRoles as _, colors};

/// A link that opens the browser. See the module doc.
#[derive(IntoElement)]
pub struct ExternalLink {
    id: ElementId,
    label: SharedString,
    url: SharedString,
}

impl ExternalLink {
    /// `label` opening `url`. `id` must be unique among its siblings.
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        url: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            url: url.into(),
        }
    }
}

impl RenderOnce for ExternalLink {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = keyed_focus(self.id.clone(), window, cx);
        let focused = focus_visible(&focus, window);
        let radius = control_radius(cx);
        let accent = colors(cx).accent;
        let url = self.url;
        probe(
            div().flex().min_w_0().child(
                BaseButton::new(self.id)
                    .role(Role::Link)
                    .track_focus(&focus)
                    .accessibility_label(self.label.clone())
                    .relative()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .rounded(radius)
                    .cursor_pointer()
                    .body()
                    .text_color(accent)
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .when(focused, |this| this.focus_ring_around(radius, cx))
                    .child(div().min_w_0().truncate().underline().child(self.label))
                    .child(
                        Icon::new(IconName::ExternalLink)
                            .size_3()
                            .text_color(accent),
                    ),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{has_focus, host, tab};
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn tab_reaches_it(cx: &mut TestAppContext) {
        let cx = host(cx, |_, _| {
            ExternalLink::new("link", "Open on Hardcover", "https://hardcover.app")
                .into_any_element()
        });
        tab(cx);
        assert!(has_focus(cx));
    }
}
