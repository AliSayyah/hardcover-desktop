//! The sidebar's keyboard model (`hardcover-design`, Sidebar → Keyboard):
//! its destinations are one stop in the Tab order, however many lists the
//! reader has, and the arrows move between them, as in a list.
//!
//! - Tab into the sidebar lands on the row it was left on (the first row
//!   until then); Tab and Shift+Tab from any row leave it.
//! - Up and Down move to the row above and below, stopping at the ends;
//!   Home and End go to the first and the last. Enter and Space press the
//!   row, as before.
//! - Every row stays a real button and a tab stop to GPUI, so the walk uses
//!   GPUI's own order and a row needs nothing of its own: a `NavRow`, a
//!   `GoalRow` or anything else focusable inside counts.
//! - Ctrl's shortcut hints are drawn by the rows and don't depend on focus.
//!
//! ```ignore
//! let keys = cx.new(|cx| SidebarKeys::new(window, cx));
//! keys.read(cx).wrap(div().id("sidebar-destinations")).child(rows)
//! ```

use gpui_kit::{
    App, Context, FocusHandle, InteractiveElement as _, KeyBinding, Stateful, Subscription,
    WeakFocusHandle, Window, actions,
};

actions!(
    sidebar,
    [
        LeaveForward,
        LeaveBack,
        NextRow,
        PreviousRow,
        FirstRow,
        LastRow
    ]
);

/// The key context of the sidebar's destinations.
pub const CONTEXT: &str = "SidebarDestinations";

/// The sidebar's keys. Its bindings are deeper than the window's own Tab,
/// so they win while focus is in the sidebar.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", LeaveForward, Some(CONTEXT)),
        KeyBinding::new("shift-tab", LeaveBack, Some(CONTEXT)),
        KeyBinding::new("down", NextRow, Some(CONTEXT)),
        KeyBinding::new("up", PreviousRow, Some(CONTEXT)),
        KeyBinding::new("home", FirstRow, Some(CONTEXT)),
        KeyBinding::new("end", LastRow, Some(CONTEXT)),
    ]);
}

/// The sidebar destinations' focus, and the row they were left on.
pub struct SidebarKeys {
    focus: FocusHandle,
    /// The row focus last left the sidebar from, where Tab comes back to.
    last: Option<WeakFocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl SidebarKeys {
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

    /// The keyboard brought focus in: back to the row it left from, else to
    /// the first row.
    fn entered(&self, window: &mut Window, cx: &mut App) {
        let focus = self.focus.clone();
        let last = self
            .last
            .as_ref()
            .and_then(WeakFocusHandle::upgrade)
            .filter(|row| focus.contains(row, window) && *row != focus);
        // After the frame that moved focus in, whose tab order this walks.
        window.defer(cx, move |window, cx| {
            if !focus.contains_focused(window, cx) {
                return;
            }
            match last {
                Some(row) if !row.is_focused(window) => window.focus(&row, cx),
                Some(_) => {}
                None => to_end(&focus, Window::focus_prev, Window::focus_next, window, cx),
            }
        });
    }

    /// `element` as the sidebar's destinations: its rows are one tab stop,
    /// with the arrows inside.
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
            .on_action(move |_: &NextRow, window, cx| {
                step(&f3, Window::focus_next, window, cx);
            })
            .on_action(move |_: &PreviousRow, window, cx| {
                step(&f4, Window::focus_prev, window, cx);
            })
            .on_action(move |_: &FirstRow, window, cx| {
                to_end(&f5, Window::focus_prev, Window::focus_next, window, cx);
            })
            .on_action(move |_: &LastRow, window, cx| {
                to_end(&focus, Window::focus_next, Window::focus_prev, window, cx);
            })
    }
}

type Advance = fn(&mut Window, &mut App);

/// How many stops a walk takes at most: more than the sidebar can hold, so
/// a tab order that cycles short of leaving it can't hang the app.
const WALK_LIMIT: usize = 500;

/// Moves focus out of `sidebar`, past its other rows.
fn leave(sidebar: &FocusHandle, advance: Advance, window: &mut Window, cx: &mut App) {
    for _ in 0..WALK_LIMIT {
        advance(window, cx);
        if !sidebar.contains_focused(window, cx) {
            return;
        }
    }
}

/// Moves focus one row along, staying put at the end.
fn step(sidebar: &FocusHandle, advance: Advance, window: &mut Window, cx: &mut App) {
    let Some(from) = window.focused(cx) else {
        return;
    };
    advance(window, cx);
    if !sidebar.contains_focused(window, cx) {
        window.focus(&from, cx);
    }
}

/// Moves focus to the row at one end: out past it with `out`, then one
/// stop back in with `back`.
fn to_end(sidebar: &FocusHandle, out: Advance, back: Advance, window: &mut Window, cx: &mut App) {
    leave(sidebar, out, window, cx);
    back(window, cx);
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::IconName;
    use gpui_kit::component::button::Button;
    use gpui_kit::{
        AppContext as _, Entity, IntoElement as _, ParentElement as _, TestAppContext,
        VisualTestContext, div,
    };

    use super::*;
    use crate::ui::NavRow;
    use crate::ui::test_support::{host, press, redraw, shift_tab, tab};

    const ROWS: usize = 5;

    /// A button, the sidebar's rows, and a button after them, as the window
    /// has the sidebar between the brand and the page.
    fn sidebar(cx: &mut TestAppContext) -> (&mut VisualTestContext, Entity<SidebarKeys>) {
        cx.update(bind_keys);
        let keys: std::rc::Rc<std::cell::RefCell<Option<Entity<SidebarKeys>>>> = Default::default();
        let slot = keys.clone();
        let cx = host(cx, move |window, cx| {
            let keys = slot
                .borrow_mut()
                .get_or_insert_with(|| cx.new(|cx| SidebarKeys::new(window, cx)))
                .clone();
            div()
                .child(Button::new("before").label("Before"))
                .child(keys.read(cx).wrap(div().id("sidebar")).children(
                    (0..ROWS).map(|ix| NavRow::new(("row", ix), IconName::BookOpen, "List")),
                ))
                .child(Button::new("after").label("After"))
                .into_any_element()
        });
        // Focus events reach only an active window.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let keys = keys.borrow().clone().expect("drawn");
        (cx, keys)
    }

    /// Which row has focus: its place among the window's stops, the first
    /// button being 0.
    fn focused_stop(cx: &mut VisualTestContext) -> usize {
        cx.update(|window, cx| {
            let target = window.focused(cx).expect("something has focus");
            // From the first stop of the window to the focused one.
            window.blur(cx);
            window.focus_next(cx);
            let mut stop = 0;
            while window.focused(cx).as_ref() != Some(&target) {
                window.focus_next(cx);
                stop += 1;
                assert!(stop < 20, "the focused handle is a stop");
            }
            stop
        })
    }

    fn settle(cx: &mut VisualTestContext) {
        redraw(cx);
        cx.run_until_parked();
        redraw(cx);
    }

    #[gpui_kit::test]
    fn the_rows_are_one_tab_stop_with_arrows_inside(cx: &mut TestAppContext) {
        let (cx, _keys) = sidebar(cx);
        tab(cx);
        assert_eq!(focused_stop(cx), 0, "the button before");
        tab(cx);
        assert_eq!(focused_stop(cx), 1, "the first row");

        press(cx, "down");
        press(cx, "down");
        redraw(cx);
        assert_eq!(focused_stop(cx), 3, "two rows down");
        press(cx, "end");
        redraw(cx);
        assert_eq!(focused_stop(cx), ROWS, "the last row");
        press(cx, "down");
        redraw(cx);
        assert_eq!(focused_stop(cx), ROWS, "Down stops at the last row");
        press(cx, "home");
        redraw(cx);
        assert_eq!(focused_stop(cx), 1, "the first row");
        press(cx, "up");
        redraw(cx);
        assert_eq!(focused_stop(cx), 1, "Up stops at the first row");

        press(cx, "down");
        press(cx, "tab");
        settle(cx);
        assert_eq!(focused_stop(cx), ROWS + 1, "Tab leaves past the other rows");

        // Shift+Tab comes back to the row it left from, not the last row.
        shift_tab(cx);
        settle(cx);
        assert_eq!(focused_stop(cx), 2, "back on the row it left");
        press(cx, "shift-tab");
        settle(cx);
        assert_eq!(focused_stop(cx), 0, "Shift+Tab leaves before the rows");
    }

    #[gpui_kit::test]
    fn shift_tab_into_the_rows_lands_on_the_first_until_one_was_left(cx: &mut TestAppContext) {
        let (cx, _keys) = sidebar(cx);
        press(cx, "a");
        shift_tab(cx);
        assert_eq!(focused_stop(cx), ROWS + 1, "the button after");
        shift_tab(cx);
        settle(cx);
        assert_eq!(focused_stop(cx), 1, "the first row, not the last");
    }
}
