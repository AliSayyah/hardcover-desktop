//! Where the kit's components miss the system and the theme can't reach
//! (`hardcover-design`, Kit limits). Each fix is a
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
//! - [`SmallControl`]: the system's small button and select (24 high,
//!   padding 8, 12 px text, 12 px icon, gap 4). The kit's `Size::Small` sets
//!   14 px text and a 14 px icon; use `.small_control()` (or
//!   `.small_icon_button()` for an icon alone) wherever a small control
//!   goes, never `.small()`.
//! - [`OnTextSpine`]: a quiet button (ghost, or a danger label) that starts a
//!   line puts its label, not its padding, on the content's leading edge.
//!
//! What can't be fixed without patching the kit (the same list as the
//! system's "Kit limits"):
//! - **Menus** (`PopupMenu`: dropdown and context menus) and the **Select
//!   dropdown** take no style from outside: they round at 6, not 8, and keep
//!   the kit's shadow, not `shadow-popover`.
//! - **Menu items** are 26 high (one every 28 with the gap), not 32, and
//!   their icons are 12, not 16: the menu sets both itself. An element item
//!   only gets a minimum height, and rebuilding every item as one would lose
//!   the kit's icon slot, check marks and key hints, and mix two heights.
//! - **Menu hover in dark mode** stays at 10%: the kit reads one colour
//!   (`accent`) for menu items and ghost buttons, and halves it for ghost
//!   buttons in dark mode. The theme picks 10% so ghost buttons get the
//!   system's 5%.
//! - **A disabled switch** fades its track only, not its thumb.
//! - **Radios** draw their outline in `border-strong` and halve it when
//!   disabled; a style reaches the radio's row, not its circle. An
//!   unchecked disabled radio is faint, most in dark mode, so the app avoids
//!   one where it can (Settings shows a theme fixed by `HARDCOVER_THEME` as
//!   a value); a form's radios are disabled only while it saves.
//! - **`DatePicker`** paints its field on an inner element the style doesn't
//!   reach, so it stays paper.
//! - **A dialog's shadow** is set after its style is applied, so it keeps
//!   the kit's.

use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::select::{Select, SelectDelegate, SelectItem};
use gpui_kit::component::{Icon, IconName, Sizable as _, WindowExt as _};
use gpui_kit::{
    App, Div, InteractiveElement as _, IntoElement, KeyBinding, NoAction, ParentElement as _,
    Styled, Window, div, rems,
};

use crate::design::{TypeRole, colors, size};

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

/// The system's small control: 24 high, padding 8, 12 px text, a 12 px icon,
/// 4 between icon and label (`hardcover-design`, Buttons and links → Sizes;
/// Inputs). The kit's `Size::Small` is 24 high but sets 14 px text and a
/// 14 px icon, so a small control would read as loud as a default one.
///
/// ```ignore
/// Button::new("update").label("Update progress…").small_control()
/// Button::new("read-menu").ghost().icon(IconName::Ellipsis).small_icon_button()
/// Select::new(&self.privacy).small_control().w(size::SELECT_SMALL)
/// ```
///
/// Call it instead of `.small()`, and don't size the control again after it.
pub trait SmallControl: Sized {
    fn small_control(self) -> Self;
}

impl SmallControl for Button {
    /// The kit's extra-small button has the system's small text, icon and
    /// gap; its height and padding go back to the small control's.
    fn small_control(self) -> Self {
        self.xsmall().h(size::CONTROL_SMALL).px_2()
    }
}

impl<D> SmallControl for Select<D>
where
    D: SelectDelegate + 'static,
    <D::Item as SelectItem>::Value: PartialEq + Clone,
{
    /// The kit's small select (24 high, padding 8, its menu's rows at the
    /// kit's small size) with 12 px text and a 12 px chevron.
    fn small_control(self) -> Self {
        let (text, line) = TypeRole::Caption.scale();
        self.small()
            .icon(Icon::new(IconName::ChevronDown))
            .text_size(rems(text / 16.))
            .line_height(rems(line / 16.))
    }
}

/// A small button that shows only an icon: 24 square, the icon 12.
pub trait SmallIconButton: Sized {
    fn small_icon_button(self) -> Self;
}

impl SmallIconButton for Button {
    fn small_icon_button(self) -> Self {
        self.xsmall().size(size::CONTROL_SMALL)
    }
}

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

impl OnTextSpine for Button {}

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
