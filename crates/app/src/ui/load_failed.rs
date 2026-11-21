//! A load that failed (`hardcover-design`, States → Error): what happened,
//! next to where the content would be, and "Try again".
//!
//! - **Inline** (the default): one muted sentence with a small "Try again"
//!   beside it, 8 apart, wrapping under it in a narrow column. For a part
//!   of a page that couldn't load, or a reload that failed over content
//!   still shown: the series tracker, Coming soon, a catalogue page, a
//!   list's books.
//! - **Filling its region** ([`LoadFailed::region`]): an empty state's
//!   layout (`super::EmptyState`) with the alert icon, a title naming what
//!   failed, the sentence, and a default "Try again" under it. For a screen
//!   with nothing else to show: Home, Stats, Series and the shelves when the
//!   library has never loaded.
//!
//! The sentence comes from `session::error_message`, which already says how
//! to recover where Try again isn't enough.
//!
//! ```ignore
//! LoadFailed::new("series-retry", message).on_retry(cx.listener(|this, _, _, cx| this.retry(cx)))
//! LoadFailed::new("library-retry", message)
//!     .region("Couldn't load your library")
//!     .on_retry(move |_, _, cx| model.update(cx, LibraryModel::sync))
//! ```

use std::rc::Rc;

use gpui_kit::component::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::{
    App, ClickEvent, ElementId, IntoElement, ParentElement as _, RenderOnce, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::EmptyState;
use super::kit::SmallControl as _;
use super::reveal::probe;
use crate::design::{TypeRoles as _, colors};

type RetryHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A failed load and its "Try again". See the module doc.
#[derive(IntoElement)]
pub struct LoadFailed {
    id: ElementId,
    message: SharedString,
    title: Option<SharedString>,
    on_retry: Option<RetryHandler>,
}

impl LoadFailed {
    /// `id` names the "Try again" button; `message` says what happened.
    pub fn new(id: impl Into<ElementId>, message: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            message: message.into(),
            title: None,
            on_retry: None,
        }
    }

    /// Fills the region it is in, as an empty state titled `title` ("Couldn't
    /// load your library").
    pub fn region(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// What "Try again" does. Without it, only the sentence shows.
    pub fn on_retry(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_retry = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for LoadFailed {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let retry = |small: bool| {
            self.on_retry.clone().map(|handler| {
                Button::new(self.id.clone())
                    .label("Try again")
                    .when(small, |b| b.small_control())
                    .on_click(move |event, window, cx| handler(event, window, cx))
            })
        };
        if let Some(title) = self.title.clone() {
            let state =
                EmptyState::new(IconName::CircleAlert, title).description(self.message.clone());
            return match retry(false) {
                Some(button) => state.action(button).into_any_element(),
                None => state.into_any_element(),
            };
        }
        probe(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .body()
                        .text_color(colors(cx).fg_muted)
                        .child(self.message.clone()),
                )
                .children(retry(true)),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{counter, host, press, tab};
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn try_again_is_a_tab_stop_that_retries(cx: &mut TestAppContext) {
        for region in [false, true] {
            let (tried, retry) = counter();
            let cx = host(cx, move |_, _| {
                let failed = LoadFailed::new("retry", "No connection.").on_retry(retry.clone());
                if region {
                    failed
                        .region("Couldn't load your library")
                        .into_any_element()
                } else {
                    failed.into_any_element()
                }
            });
            tab(cx);
            press(cx, "enter");
            assert_eq!(tried.get(), 1, "region: {region}");
        }
    }
}
