//! External link (`hardcover-design`, Buttons and links → Link): the one
//! kind of link, for things that open the browser ("Open on Hardcover").
//! Accent and underlined (the kit's `Link`), with a 12 px external-link icon
//! after the words, 4 apart. Never for in-app navigation: that is a
//! [`super::TextButton`].
//!
//! ```ignore
//! ExternalLink::new("series-on-hardcover", "Open on Hardcover", url)
//! ```

use gpui_kit::component::link::Link;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::{
    App, ElementId, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window,
};

use crate::design::TypeRoles as _;

/// A link that opens `url` in the browser. See the module doc.
#[derive(IntoElement)]
pub struct ExternalLink {
    id: ElementId,
    label: SharedString,
    url: SharedString,
}

impl ExternalLink {
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
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Link::new(self.id)
            .href(self.url)
            .body()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(self.label)
            .child(Icon::new(IconName::ExternalLink).xsmall())
    }
}
