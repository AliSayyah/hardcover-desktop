//! Chips (`hardcover-design`, Chips): neutral pills, 20 high, on the `chip`
//! ink overlay, with `fg-muted` text at 12/500 and an optional 12 px icon.
//! The overlay is translucent on purpose: a chip stays visible on a hovered
//! or selected row.
//!
//! - `StatusChip`: a book's library status at the end of a row, with the
//!   shelf's sidebar icon, so statuses differ by shape as well as by name.
//!   `StatusIconChip` is its compact form, for a row too narrow for the
//!   name: the icon alone on the pill, the name in its tooltip.
//! - `TagChip`: one catalogue tag (a genre, a mood, a content warning) on the
//!   book page. Content warnings carry the warning icon in `warning`, the
//!   only colour a chip ever has.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::{
    App, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::ReadingStatus;

use crate::design::{TypeRoles as _, colors, size};
use crate::format::status_label;

/// The pill every chip is drawn on.
fn pill(cx: &App) -> Div {
    let colors = colors(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(size::CHIP)
        .px_2()
        .gap_1()
        .rounded_full()
        .bg(colors.chip)
        .caption(cx)
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors.fg_muted)
}

/// The icon of a status, as on the sidebar's shelves.
pub fn status_icon(status: ReadingStatus) -> IconName {
    match status {
        ReadingStatus::CurrentlyReading => IconName::BookOpen,
        ReadingStatus::WantToRead => IconName::Inbox,
        ReadingStatus::Read => IconName::CircleCheck,
        ReadingStatus::Paused => IconName::Pause,
        ReadingStatus::DidNotFinish => IconName::Ban,
        ReadingStatus::Ignored => IconName::EyeOff,
    }
}

/// A book's library status: its shelf icon and name.
#[derive(IntoElement)]
pub struct StatusChip(pub ReadingStatus);

impl RenderOnce for StatusChip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        pill(cx)
            .child(Icon::new(status_icon(self.0)).xsmall())
            .child(status_label(self.0))
    }
}

/// A book's library status where a row has no room for its name: the
/// shelf icon alone on a 20 × 20 pill, the name in its tooltip and
/// accessible label.
#[derive(IntoElement)]
pub struct StatusIconChip(pub ReadingStatus);

impl RenderOnce for StatusIconChip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let label: SharedString = status_label(self.0).into();
        pill(cx)
            .id("status-icon-chip")
            .px_1()
            .aria_label(label.clone())
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .child(Icon::new(status_icon(self.0)).xsmall())
    }
}

/// One catalogue tag. A long one is cut short with an ellipsis.
#[derive(IntoElement)]
pub struct TagChip {
    text: SharedString,
    warning: bool,
}

impl TagChip {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            warning: false,
        }
    }

    /// Marks a content warning with the warning icon.
    pub fn warning(mut self, warning: bool) -> Self {
        self.warning = warning;
        self
    }
}

impl RenderOnce for TagChip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        pill(cx)
            .flex_shrink(1.)
            .max_w_full()
            .min_w_0()
            .when(self.warning, |this| {
                this.child(
                    Icon::new(IconName::TriangleAlert)
                        .xsmall()
                        .flex_shrink_0()
                        .text_color(colors(cx).warning),
                )
            })
            .child(div().min_w_0().truncate().child(self.text))
    }
}
