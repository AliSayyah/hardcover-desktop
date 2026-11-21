//! A half-star rating control.
//!
//! gpui-kit's `Rating` stores whole stars (`usize`), but Hardcover ratings go
//! in half steps, so showing a 3.5 there would round it and the next click
//! would overwrite it. This control keeps the `Rating` value exact: each star
//! has a leading half (n − 0.5) and a trailing half (n), clicking the current
//! value clears it, and the keyboard steps by half a star.
//!
//! Controlled: the owner passes the value and receives requested changes.
//!
//! The stars are 20 px (16 in a row, [`StarRating::small`]), 2 apart, in
//! `gold` (the user's own rating is the only gold in the app) over
//! `star-empty`. The pointer's preview draws the stars a click would set as
//! gold outlines over the empty star, so it never reads as a saved rating.
//! Keyboard focus shows the system's ring (`ui::focus`). An unrated control
//! reads "Rate" beside it; that word is the owner's to place.
//!
//! The pointer's targets are ten equal slices across the whole control,
//! padding included: each half star is the control's full height, and the
//! gaps between stars belong to the halves beside them, so no click lands
//! on nothing.

use std::rc::Rc;

use gpui_kit::component::h_flex;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, ElementId, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _, Window, actions, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::Rating;

use crate::design::colors;
use crate::ui::focus::{FocusTreatment as _, control_radius, focus_visible};
use crate::ui::{StarInk, StarSize, render_star};

/// Key context of the control; [`key_bindings`] are its keys.
pub const KEY_CONTEXT: &str = "StarRating";

actions!(star_rating, [IncreaseRating, DecreaseRating, ClearRating]);

/// The control's keys: Right and Up add half a star, Left and Down take one
/// off, Backspace and Delete clear. `main` binds them.
pub fn key_bindings() -> [gpui_kit::KeyBinding; 6] {
    use gpui_kit::KeyBinding;
    let context = Some(KEY_CONTEXT);
    [
        KeyBinding::new("right", IncreaseRating, context),
        KeyBinding::new("up", IncreaseRating, context),
        KeyBinding::new("left", DecreaseRating, context),
        KeyBinding::new("down", DecreaseRating, context),
        KeyBinding::new("backspace", ClearRating, context),
        KeyBinding::new("delete", ClearRating, context),
    ]
}

const STARS: u8 = 5;

/// How much of one star is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StarFill {
    Empty,
    Half,
    Full,
}

/// The fill of star `star` (1-based) for `value` stars.
pub(crate) fn star_fill(value: f32, star: u8) -> StarFill {
    let star = f32::from(star);
    if value >= star {
        StarFill::Full
    } else if value >= star - 0.5 {
        StarFill::Half
    } else {
        StarFill::Empty
    }
}

/// The value a click on `target` requests: clicking the current value clears.
pub(crate) fn clicked_value(current: Option<Rating>, target: f32) -> Option<Rating> {
    Rating::new(target).filter(|r| Some(*r) != current)
}

/// The value after one keyboard step of half a star (`up` or down). Stepping
/// down from half a star clears; stepping up from none gives half a star.
pub(crate) fn stepped_value(current: Option<Rating>, up: bool) -> Option<Rating> {
    let v = current.map_or(0.0, Rating::value);
    let next = if up {
        (v + 0.5).min(Rating::MAX)
    } else {
        v - 0.5
    };
    Rating::new(next)
}

type ChangeHandler = Rc<dyn Fn(&Option<Rating>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct StarRating {
    id: SharedString,
    value: Option<Rating>,
    disabled: bool,
    small: bool,
    tooltip: Option<SharedString>,
    on_change: Option<ChangeHandler>,
}

impl StarRating {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            value: None,
            disabled: false,
            small: false,
            tooltip: None,
            on_change: None,
        }
    }

    pub fn value(mut self, value: Option<Rating>) -> Self {
        self.value = value;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// A tooltip, shown even while disabled: say why it is
    /// (`format::needs_permission`).
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// The 16 px stars of a row (Home's "Rate what you finished"), instead
    /// of the book page's 20.
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    /// Called with the requested value; the owner decides whether to apply it.
    pub fn on_change(
        mut self,
        handler: impl Fn(&Option<Rating>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    fn child_id(&self, part: &str) -> ElementId {
        ElementId::from(SharedString::from(format!("{}-{part}", self.id)))
    }
}

impl RenderOnce for StarRating {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus: FocusHandle = window
            .use_keyed_state(self.child_id("focus"), cx, |_, cx| {
                cx.focus_handle().tab_stop(true)
            })
            .read(cx)
            .clone();
        let hover = window.use_keyed_state(self.child_id("hover"), cx, |_, _| None::<f32>);
        let hovered = if self.disabled { None } else { *hover.read(cx) };
        let shown = hovered.unwrap_or_else(|| self.value.map_or(0.0, Rating::value));
        let focused = focus_visible(&focus, window);
        let radius = control_radius(cx);

        let value = self.value;
        let disabled = self.disabled;
        let size = if self.small {
            StarSize::Medium
        } else {
            StarSize::Large
        };
        let on_change = self.on_change.clone();
        let request = move |next: Option<Rating>, window: &mut Window, cx: &mut App| {
            if let Some(handler) = &on_change
                && next != value
            {
                handler(&next, window, cx);
            }
        };

        // A hover previews the value it would set: outlines, not solid stars.
        let gold = colors(cx).gold;
        let filled_color = if hovered.is_some() {
            StarInk::Outline(gold)
        } else {
            StarInk::Solid(gold)
        };
        let empty_color = colors(cx).star_empty;
        let label = match value {
            Some(r) => format!("Rating: {} of 5 stars", r.value()),
            None => "Rating: not rated".to_owned(),
        };

        h_flex()
            .id(ElementId::from(self.id.clone()))
            .relative()
            .gap_0p5()
            .p_0p5()
            .rounded(radius)
            .aria_label(label)
            .when(!disabled, |this| {
                let request = Rc::new(request);
                this.track_focus(&focus)
                    .key_context(KEY_CONTEXT)
                    .on_action({
                        let request = request.clone();
                        move |_: &IncreaseRating, window, cx| {
                            request(stepped_value(value, true), window, cx);
                        }
                    })
                    .on_action({
                        let request = request.clone();
                        move |_: &DecreaseRating, window, cx| {
                            request(stepped_value(value, false), window, cx);
                        }
                    })
                    .on_action({
                        let request = request.clone();
                        move |_: &ClearRating, window, cx| request(None, window, cx)
                    })
                    .on_hover({
                        let hover = hover.clone();
                        move |hovered, _, cx| {
                            if !*hovered {
                                hover.update(cx, |h, cx| {
                                    *h = None;
                                    cx.notify();
                                });
                            }
                        }
                    })
                    .children(
                        (1..=STARS).map(|n| render_star(n, shown, size, filled_color, empty_color)),
                    )
                    .child(
                        h_flex()
                            .absolute()
                            .inset_0()
                            .children((1..=STARS * 2).map(|slice| {
                                half_star_target(
                                    f32::from(slice) / 2.,
                                    value,
                                    hover.clone(),
                                    focus.clone(),
                                    request.clone(),
                                )
                            })),
                    )
            })
            .when(disabled, |this| {
                this.opacity(0.5).children(
                    (1..=STARS).map(|n| render_star(n, shown, size, filled_color, empty_color)),
                )
            })
            .when_some(self.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .when(focused, |this| this.focus_ring(radius, cx))
    }
}

/// One of the ten slices across the control: a click sets `target` stars.
fn half_star_target(
    target: f32,
    value: Option<Rating>,
    hover: gpui_kit::Entity<Option<f32>>,
    focus: FocusHandle,
    request: Rc<impl Fn(Option<Rating>, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .id(ElementId::from(SharedString::from(format!(
            "half-{target}"
        ))))
        .flex_1()
        .h_full()
        .on_mouse_move(move |_, _, cx| {
            hover.update(cx, |h, cx| {
                if *h != Some(target) {
                    *h = Some(target);
                    cx.notify();
                }
            });
        })
        .on_click(move |_, window, cx| {
            focus.focus(window, cx);
            request(clicked_value(value, target), window, cx);
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(v: f32) -> Option<Rating> {
        Rating::new(v)
    }

    #[test]
    fn fills() {
        assert_eq!(star_fill(3.5, 3), StarFill::Full);
        assert_eq!(star_fill(3.5, 4), StarFill::Half);
        assert_eq!(star_fill(3.5, 5), StarFill::Empty);
        assert_eq!(star_fill(0.0, 1), StarFill::Empty);
        assert_eq!(star_fill(0.5, 1), StarFill::Half);
        assert_eq!(star_fill(5.0, 5), StarFill::Full);
    }

    #[test]
    fn clicks_set_half_steps_and_clicking_the_value_clears() {
        assert_eq!(clicked_value(None, 3.5), r(3.5));
        assert_eq!(clicked_value(r(2.0), 4.0), r(4.0));
        assert_eq!(clicked_value(r(4.0), 4.0), None);
        assert_eq!(clicked_value(r(4.5), 4.0), r(4.0));
    }

    #[test]
    fn keyboard_steps() {
        assert_eq!(stepped_value(None, true), r(0.5));
        assert_eq!(stepped_value(r(3.5), true), r(4.0));
        assert_eq!(stepped_value(r(5.0), true), r(5.0));
        assert_eq!(stepped_value(r(3.0), false), r(2.5));
        assert_eq!(stepped_value(r(0.5), false), None);
        assert_eq!(stepped_value(None, false), None);
    }

    /// The arrows step the rating by half a star; Delete clears it.
    #[gpui_kit::test]
    fn the_keys_step_by_half_a_star_and_delete_clears(cx: &mut gpui_kit::TestAppContext) {
        use crate::ui::test_support::{host, press, redraw, tab};
        use gpui_kit::IntoElement as _;
        use std::cell::Cell;

        let value = Rc::new(Cell::new(r(3.0)));
        let cx = host(cx, {
            let value = value.clone();
            move |_, _| {
                let set = value.clone();
                StarRating::new("stars")
                    .value(value.get())
                    .on_change(move |rating, _, _| set.set(*rating))
                    .into_any_element()
            }
        });
        cx.update(|_, cx| cx.bind_keys(key_bindings()));
        tab(cx);
        let mut key = |name: &str| {
            press(cx, name);
            redraw(cx);
            value.get()
        };
        assert_eq!(key("right"), r(3.5));
        assert_eq!(key("up"), r(4.0));
        assert_eq!(key("left"), r(3.5));
        assert_eq!(key("down"), r(3.0));
        assert_eq!(key("delete"), None);
        assert_eq!(key("up"), r(0.5));
        assert_eq!(key("backspace"), None);
    }
}
