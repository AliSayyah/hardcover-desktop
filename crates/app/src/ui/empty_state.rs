//! Empty state (`hardcover-design`, Empty states): centred in the region it
//! explains, never an alert.
//!
//! - a 24 px icon in `fg-faint`;
//! - a `body-strong` title;
//! - one `fg-muted` sentence, at most 288 wide;
//! - optionally one button, 12 below.
//!
//! Use the app's own words: "No matches" / "No books in Did not finish match
//! “dune”.", "Select a book to see its details".
//!
//! ```ignore
//! EmptyState::new(IconName::Search, "No matches")
//!     .description(format!("No books in {shelf} match “{query}”."))
//!     .action(Button::new("retry").label("Try again"))
//! ```
//!
//! The system asks for a 1.5 stroke on the icon; the kit's Lucide icons are
//! fixed SVGs drawn at 2, so they keep it.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};

use crate::design::{TypeRoles as _, colors, size};

/// A region's empty state. See the module doc.
#[derive(IntoElement)]
pub struct EmptyState {
    icon: IconName,
    title: SharedString,
    description: Option<SharedString>,
    action: Option<AnyElement>,
}

impl EmptyState {
    pub fn new(icon: IconName, title: impl Into<SharedString>) -> Self {
        Self {
            icon,
            title: title.into(),
            description: None,
            action: None,
        }
    }

    /// The one sentence under the title. An empty one is left out.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        let description = description.into();
        self.description = (!description.is_empty()).then_some(description);
        self
    }

    /// One button under the sentence, such as "Try again".
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .text_center()
            .gap_1()
            .p_8()
            .child(
                Icon::new(self.icon)
                    .size_6()
                    .mb_2()
                    .text_color(colors.fg_faint),
            )
            .child(div().body_strong().text_color(colors.fg).child(self.title))
            .when_some(self.description, |this, description| {
                this.child(
                    div()
                        .body()
                        .max_w(size::EMPTY_TEXT)
                        .text_color(colors.fg_muted)
                        .child(description),
                )
            })
            .when_some(self.action, |this, action| {
                this.child(div().mt_3().child(action))
            })
    }
}

/// An empty state with a title and one sentence.
pub fn empty_state(icon: IconName, title: SharedString, description: SharedString) -> EmptyState {
    EmptyState::new(icon, title).description(description)
}

/// [`empty_state`] with a button below, such as "Try again".
pub fn empty_state_with_action(
    icon: IconName,
    title: SharedString,
    description: SharedString,
    action: impl IntoElement,
) -> EmptyState {
    empty_state(icon, title, description).action(action)
}
