//! Stars (`hardcover-design`, Stars): a rating shown, not set. Five of the
//! app's star (`crate::star`) in `fg-muted`, unfilled ones in `star-empty`,
//! a half star split cleanly down the middle. Hardcover's figures (the
//! average on the book page's rail) and another reader's rating on a review.
//!
//! Never gold: gold is only the user's own rating, which is the rating
//! control (`book::star_rating::StarRating`), not this.
//!
//! - [`Stars::new`]: 16 px, beside a figure ("4.63").
//! - [`Stars::small`]: 12 px, inline (a review's byline).
//!
//! ```ignore
//! Stars::new(4.63).accessibility_label("rail-average", "4.63 average")
//! Stars::new(rating.value()).small()
//! ```

use gpui_kit::component::h_flex;
use gpui_kit::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, relative,
};

use crate::book::star_rating::{StarFill, star_fill};
use crate::design::colors;

/// Five read-only stars. See the module doc.
#[derive(IntoElement)]
pub struct Stars {
    value: f32,
    small: bool,
    label: Option<(ElementId, SharedString)>,
}

impl Stars {
    /// `value` stars of five, in half steps (anything else rounds down to
    /// the half below).
    pub fn new(value: f32) -> Self {
        Self {
            value,
            small: false,
            label: None,
        }
    }

    /// The 12 px stars that sit inline.
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    /// The figure's accessible name, such as "Rated 4.5 of 5", on an
    /// element `id` (unique among its siblings).
    pub fn accessibility_label(
        mut self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
    ) -> Self {
        self.label = Some((id.into(), label.into()));
        self
    }
}

impl RenderOnce for Stars {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let filled = colors(cx).fg_muted;
        let empty = colors(cx).star_empty;
        let value = self.value;
        let small = self.small;
        let star = move || {
            let icon = crate::star::icon().flex_shrink_0();
            if small { icon.size_3() } else { icon.size_4() }
        };
        let stars = h_flex()
            .flex_none()
            .gap_0p5()
            .children((1..=5u8).map(move |position| {
                div()
                    .relative()
                    .flex_shrink_0()
                    .map(|this| if small { this.size_3() } else { this.size_4() })
                    .map(|this| match star_fill(value, position) {
                        StarFill::Full => this.child(star().text_color(filled)),
                        StarFill::Empty => this.child(star().text_color(empty)),
                        // The empty star with its leading half filled.
                        StarFill::Half => this.child(star().text_color(empty)).child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .h_full()
                                .w(relative(0.5))
                                .overflow_hidden()
                                .child(star().text_color(filled)),
                        ),
                    })
            }));
        match self.label {
            Some((id, label)) => stars.id(id).aria_label(label).into_any_element(),
            None => stars.into_any_element(),
        }
    }
}
