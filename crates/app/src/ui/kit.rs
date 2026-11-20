//! Where the kit's components miss the system and the theme can't reach
//! (`hardcover-design`, Migration status → States and the kit). Each fix is a
//! style the kit lets the app refine; use them wherever the kit component
//! appears.
//!
//! - [`RaisedField`]: an `Input` or `Select` on `raised` (the kit paints
//!   inputs paper in light mode and a tint of the outline in dark).
//! - [`open_dialog`] and [`open_alert_dialog`]: dialogs on `raised` (the kit
//!   paints them paper). A dialog stays open on Enter unless it says
//!   otherwise (`on_ok`): Enter in a field is the field's (a form submits
//!   from `InputEvent::PressEnter`), so a failed save keeps the dialog, its
//!   input and its error. A confirmation is 384 wide and opens with focus on
//!   Cancel, so Enter never runs a destructive command by accident.
//! - [`bind_keys`]: Enter on a focused button in a dialog presses that
//!   button, as Space does, instead of the kit's dialog-wide Confirm; so
//!   does Enter on a row's own button in a list ([`row_action`]), instead of
//!   the list's Confirm, which opens the row.
//! - [`OnTextSpine`]: a quiet button (ghost, or a danger label) that starts a
//!   line puts its label, not its padding, on the content's leading edge.
//! - [`PopoverSurface`]: the kit's `Popover` at `r-box` with
//!   `shadow-popover`.
//!
//! What can't be fixed without patching the kit:
//! - **Menus** (`PopupMenu`: dropdown and context menus) and the **Select
//!   dropdown** take no style from outside: they round at 6, not 8, and keep
//!   the kit's shadow.
//! - **Menu hover in dark mode** stays at 10%: the kit reads one colour
//!   (`accent`) for menu items and ghost buttons, and halves it for ghost
//!   buttons in dark mode. The theme picks 10% so ghost buttons get the
//!   system's 5%.
//! - **`DatePicker`** paints its field on an inner element the style doesn't
//!   reach, so it stays paper.
//! - **A dialog's shadow** is set after its style is applied, so it keeps
//!   the kit's.

use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _};
use gpui_kit::{
    App, Div, InteractiveElement as _, IntoElement, KeyBinding, NoAction, ParentElement as _,
    Styled, Window, div,
};

use crate::design::{colors, size};

/// Enter in a dialog, outside a text field, goes to the focused control
/// (a button presses, as with Space) rather than to the kit's dialog-wide
/// `Confirm`, which closes the dialog whatever has focus. Inside a field,
/// the field and `Confirm` keep Enter. Bind after `gpui_kit::init`: the
/// later binding wins at the same depth.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", NoAction, Some("Dialog && !Input")),
        KeyBinding::new("enter", NoAction, Some(ROW_ACTION)),
    ]);
}

const ROW_ACTION: &str = "RowAction";

/// A control inside a list row (the kit's `ListItem` suffix, such as a
/// row's remove button): Enter on it presses it, as Space does, rather
/// than confirming the row.
///
/// ```ignore
/// ListItem::new(id).suffix(move |_, _| row_action(Button::new("remove").icon(IconName::Close)))
/// ```
pub fn row_action(control: impl IntoElement) -> Div {
    div().key_context(ROW_ACTION).child(control)
}

/// A field on `raised`: `Input::new(..)` and `Select::new(..)`.
///
/// ```ignore
/// Input::new(&self.filter).raised_field(false, cx)
/// Select::new(&self.status).disabled(busy).raised_field(busy, cx)
/// ```
pub trait RaisedField: Styled + Sized {
    /// `raised` fill; a `disabled` field is the same at 50% opacity, the
    /// system's one disabled treatment (the fill replaces the kit's own
    /// disabled tint).
    fn raised_field(self, disabled: bool, cx: &App) -> Self {
        let raised = self.bg(colors(cx).raised);
        if disabled {
            raised.opacity(0.5)
        } else {
            raised
        }
    }
}

impl<T: Styled + Sized> RaisedField for T {}

/// A quiet button whose label starts a line: pulled back by the kit's own
/// label inset, so the label sits on the text spine like the content above
/// it (`hardcover-design`, Alignment: peers attach to the same spine).
///
/// ```ignore
/// Button::new("delete-goal").danger().label("Delete goal…").on_text_spine(false)
/// ```
pub trait OnTextSpine: Styled + Sized {
    /// `small` for a small button (the kit insets its label 8, else 10).
    fn on_text_spine(self, small: bool) -> Self {
        self.ml(-if small { KIT_INSET_SMALL } else { KIT_INSET })
    }
}

impl OnTextSpine for gpui_kit::component::button::Button {}

/// The kit's label inset in a button: `px_2p5`, and `px_2` when small.
const KIT_INSET: gpui_kit::Rems = gpui_kit::rems(0.625);
const KIT_INSET_SMALL: gpui_kit::Rems = gpui_kit::rems(0.5);

/// Opens a dialog on `raised`. Use instead of `window.open_dialog`.
///
/// Enter doesn't close it: a form submits from its field
/// (`InputEvent::PressEnter`) and closes itself once the save succeeds, and a
/// focused button takes Enter as a press ([`bind_keys`]). A dialog that
/// should act on Enter wherever focus is sets its own `on_ok`.
pub fn open_dialog(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let raised = colors(cx).raised;
        build(dialog.on_ok(|_, _, _| false), window, cx).bg(raised)
    });
}

/// Opens an alert dialog (a confirmation) on `raised`, 384 wide. Use instead
/// of `window.open_alert_dialog`.
///
/// It opens with focus on its first button: Cancel when it has one
/// (`.confirm()`), so a keyboard user sees what Enter does and a
/// destructive confirmation never runs on a stray Enter. A confirmation
/// whose result is safe opens on that result instead:
/// [`open_alert_dialog_on_result`].
pub fn open_alert_dialog(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(AlertDialog, &mut Window, &mut App) -> AlertDialog + 'static,
) {
    open_alert(window, cx, build, 1);
}

/// [`open_alert_dialog`] for a confirmation that isn't destructive ("Install"
/// an update): focus opens on its result, the button after Cancel. Its
/// content must hold no control of its own before the buttons.
pub fn open_alert_dialog_on_result(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(AlertDialog, &mut Window, &mut App) -> AlertDialog + 'static,
) {
    open_alert(window, cx, build, 2);
}

/// Opens the alert, and once it is drawn moves focus from the dialog, which
/// holds it at first, `steps` controls on.
fn open_alert(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(AlertDialog, &mut Window, &mut App) -> AlertDialog + 'static,
    steps: usize,
) {
    window.open_alert_dialog(cx, move |alert, window, cx| {
        let raised = colors(cx).raised;
        let width = size::DIALOG_CONFIRM.to_pixels(window.rem_size());
        build(alert.width(width), window, cx).bg(raised)
    });
    let dialog = window.focused(cx);
    window.on_next_frame(move |window, cx| {
        // Unless focus moved meanwhile, or the alert is gone.
        if window.has_active_dialog(cx) && window.focused(cx) == dialog {
            for _ in 0..steps {
                window.focus_next(cx);
            }
        }
    });
}

/// The kit's `Popover` as the system's popover: `raised`, `r-box`,
/// `shadow-popover`. Its padding stays the kit's.
pub trait PopoverSurface: Styled + Sized {
    fn popover_surface(self, cx: &App) -> Self {
        self.bg(colors(cx).raised)
            .rounded(cx.theme().radius_lg)
            .shadow(colors(cx).shadow_popover())
    }
}

impl<T: Styled + Sized> PopoverSurface for T {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::component::button::Button;
    use gpui_kit::component::input::{Input, InputEvent, InputState};
    use gpui_kit::{
        AppContext as _, Context, IntoElement, Render, TestAppContext, VisualTestContext, div,
    };

    use super::*;
    use crate::ui::test_support::{press, redraw};

    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn open(cx: &mut TestAppContext) -> &mut VisualTestContext {
        cx.update(|cx| {
            gpui_kit::init(cx);
            bind_keys(cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| Empty);
            gpui_kit::base::Root::new(view, window, cx)
        });
        cx
    }

    fn dialog_open(cx: &mut VisualTestContext) -> bool {
        cx.update(|window, cx| window.has_active_dialog(cx))
    }

    #[gpui_kit::test]
    fn enter_in_a_dialog_goes_to_what_has_focus(cx: &mut TestAppContext) {
        let cx = open(cx);
        let submitted = Rc::new(Cell::new(0));
        let pressed = Rc::new(Cell::new(0));
        let input = cx.update(|window, cx| cx.new(|cx| InputState::new(window, cx)));
        cx.update(|window, cx| {
            let submitted = submitted.clone();
            window
                .subscribe(&input, cx, move |_, event: &InputEvent, _, _| {
                    if let InputEvent::PressEnter { .. } = event {
                        submitted.set(submitted.get() + 1);
                    }
                })
                .detach();
        });
        cx.update(|window, cx| {
            let input = input.clone();
            let pressed = pressed.clone();
            open_dialog(window, cx, move |dialog, _, _| {
                let pressed = pressed.clone();
                dialog.child(Input::new(&input)).child(
                    Button::new("press")
                        .label("Press")
                        .on_click(move |_, _, _| pressed.set(pressed.get() + 1)),
                )
            });
        });
        redraw(cx);
        cx.update(|window, cx| input.update(cx, |s, cx| s.focus(window, cx)));
        redraw(cx);

        // In the field: the field's, and the dialog stays with its input.
        press(cx, "enter");
        redraw(cx);
        assert_eq!(submitted.get(), 1);
        assert!(dialog_open(cx));

        // On a button: a press, as with Space, and nothing else.
        cx.update(|window, cx| window.focus_next(cx));
        redraw(cx);
        press(cx, "enter");
        redraw(cx);
        assert_eq!(pressed.get(), 1);
        assert!(dialog_open(cx));
    }

    #[gpui_kit::test]
    fn enter_on_a_confirmations_buttons_presses_them(cx: &mut TestAppContext) {
        let cx = open(cx);
        let confirmed = Rc::new(Cell::new(0));
        // Opens the alert with focus `steps` controls on from the dialog: 1
        // is Cancel, where it opens (moved there on the next frame, which a
        // test window doesn't run), 2 the action.
        let show = |steps: usize, cx: &mut VisualTestContext| {
            let confirmed = confirmed.clone();
            cx.update(|window, cx| {
                open_alert_dialog(window, cx, move |alert, _, _| {
                    let confirmed = confirmed.clone();
                    alert
                        .title("Delete?")
                        .confirm()
                        .ok_text("Delete")
                        .on_ok(move |_, _, _| {
                            confirmed.set(confirmed.get() + 1);
                            true
                        })
                });
            });
            redraw(cx);
            for _ in 0..steps {
                cx.update(|window, cx| window.focus_next(cx));
            }
            redraw(cx);
        };
        let settle = |cx: &mut VisualTestContext| {
            cx.run_until_parked();
            redraw(cx);
        };

        show(1, cx);
        press(cx, "enter");
        settle(cx);
        assert_eq!(confirmed.get(), 0);
        assert!(!dialog_open(cx));

        show(2, cx);
        press(cx, "enter");
        settle(cx);
        assert_eq!(confirmed.get(), 1);
    }
}
