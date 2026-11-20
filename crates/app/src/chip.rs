//! Chips: small rounded pills on the muted surface.
//!
//! - `StatusChip`: a book's library status at the end of a row, with the
//!   shelf's sidebar icon, so statuses differ by shape as well as by name.
//! - `TagChip`: one catalogue tag (a genre, a mood, a content warning) on the
//!   book page. Content warnings carry a warning icon, the only colour here.
//!
//! Both stay neutral: colour is spent only where it means something.

use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::{
    App, FontWeight, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::ReadingStatus;

use crate::format::status_label;

/// The height of a tag chip. The book page's tag group labels use it as their
/// line height, so a label sits on the line of the first row of chips.
pub const TAG_HEIGHT: f32 = 1.5;

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
        let theme = cx.theme();
        Tag::custom(theme.muted, theme.muted_foreground, theme.muted)
            .rounded_full()
            .h(rems(1.375))
            .py_0()
            .pl_1p5()
            .pr_2()
            .gap_1()
            .font_weight(FontWeight::MEDIUM)
            .child(Icon::new(status_icon(self.0)).xsmall())
            .child(status_label(self.0))
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
        let theme = cx.theme();
        Tag::custom(theme.muted, theme.foreground, theme.muted)
            .rounded_full()
            .h(rems(TAG_HEIGHT))
            .max_w_full()
            .min_w_0()
            .py_0()
            .px_2p5()
            .gap_1()
            .when(self.warning, |this| {
                this.pl_2().child(
                    Icon::new(IconName::TriangleAlert)
                        .xsmall()
                        .flex_shrink_0()
                        .text_color(theme.warning),
                )
            })
            .child(div().min_w_0().truncate().child(self.text))
    }
}
