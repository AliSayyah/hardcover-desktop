//! The keys of a row of pressed chips (`hardcover-design`, Chips → Keys):
//! the row is one stop in the Tab order, as the sidebar's destinations are,
//! and the arrows move along it.
//!
//! - Tab into the row lands on the chip it was left from (the first until
//!   then); Tab and Shift+Tab from any chip leave it.
//! - Left and Right move to the chip before and after, stopping at the
//!   ends; Home and End go to the first and the last. Enter and Space press
//!   the chip.
//! - A chip that isn't pressed (a content warning, a genre with no page)
//!   isn't focusable, so the walk passes it by.
//!
//! The walk is the sidebar's (`sidebar_keys`), over GPUI's own tab order.
//!
//! ```ignore
//! let keys = cx.new(|cx| ChipKeys::new(window, cx));
//! TagRow::new(chips).keys("header-genres", &keys)
//! ```

use gpui_kit::{
    App, Context, FocusHandle, InteractiveElement as _, KeyBinding, Stateful, Subscription,
    WeakFocusHandle, Window, actions,
};

use crate::sidebar_keys::{leave, step, to_end};

actions!(
    chip_row,
    [
        LeaveForward,
        LeaveBack,
        NextChip,
        PreviousChip,
        FirstChip,
        LastChip
    ]
);

/// The key context of a row of pressed chips.
pub const CONTEXT: &str = "ChipRow";

/// The row's keys. Deeper than the window's own Tab, so they win while
/// focus is in the row.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", LeaveForward, Some(CONTEXT)),
        KeyBinding::new("shift-tab", LeaveBack, Some(CONTEXT)),
        KeyBinding::new("right", NextChip, Some(CONTEXT)),
        KeyBinding::new("left", PreviousChip, Some(CONTEXT)),
        KeyBinding::new("home", FirstChip, Some(CONTEXT)),
        KeyBinding::new("end", LastChip, Some(CONTEXT)),
    ]);
}

/// A chip row's focus, and the chip it was left on.
pub struct ChipKeys {
    focus: FocusHandle,
    /// The chip focus last left the row from, where Tab comes back to.
    last: Option<WeakFocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl ChipKeys {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus_out(&focus, window, |this, event, _, _| {
                this.last = Some(event.blurred);
            }),
            cx.on_focus_in(&focus, window, |this, window, cx| {
                // A click goes where it was aimed.
                if window.last_input_was_keyboard() {
                    this.entered(window, cx);
                }
            }),
        ];
        Self {
            focus,
            last: None,
            _subscriptions: subscriptions,
        }
    }

    /// The keyboard brought focus in: back to the chip it left from, else
    /// to the first chip.
    fn entered(&self, window: &mut Window, cx: &mut App) {
        let focus = self.focus.clone();
        let last = self
            .last
            .as_ref()
            .and_then(WeakFocusHandle::upgrade)
            .filter(|chip| focus.contains(chip, window) && *chip != focus);
        // After the frame that moved focus in, whose tab order this walks.
        window.defer(cx, move |window, cx| {
            if !focus.contains_focused(window, cx) {
                return;
            }
            match last {
                Some(chip) if !chip.is_focused(window) => window.focus(&chip, cx),
                Some(_) => {}
                None => to_end(&focus, Window::focus_prev, Window::focus_next, window, cx),
            }
        });
    }

    /// `element` as the row: its pressed chips are one tab stop, with the
    /// arrows inside.
    pub fn wrap(&self, element: Stateful<gpui_kit::Div>) -> Stateful<gpui_kit::Div> {
        let focus = self.focus.clone();
        let (f1, f2, f3, f4, f5) = (
            focus.clone(),
            focus.clone(),
            focus.clone(),
            focus.clone(),
            focus.clone(),
        );
        element
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(move |_: &LeaveForward, window, cx| {
                leave(&f1, Window::focus_next, window, cx);
            })
            .on_action(move |_: &LeaveBack, window, cx| {
                leave(&f2, Window::focus_prev, window, cx);
            })
            .on_action(move |_: &NextChip, window, cx| {
                step(&f3, Window::focus_next, window, cx);
            })
            .on_action(move |_: &PreviousChip, window, cx| {
                step(&f4, Window::focus_prev, window, cx);
            })
            .on_action(move |_: &FirstChip, window, cx| {
                to_end(&f5, Window::focus_prev, Window::focus_next, window, cx);
            })
            .on_action(move |_: &LastChip, window, cx| {
                to_end(&focus, Window::focus_next, Window::focus_prev, window, cx);
            })
    }
}
