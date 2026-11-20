//! Where the kit's components miss the system and the theme can't reach
//! (`hardcover-design`, Migration status → States and the kit). Each fix is a
//! style the kit lets the app refine; use them wherever the kit component
//! appears.
//!
//! - [`RaisedField`]: an `Input` or `Select` on `raised` (the kit paints
//!   inputs paper in light mode and a tint of the outline in dark).
//! - [`open_dialog`] and [`open_alert_dialog`]: dialogs on `raised` (the kit
//!   paints them paper).
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
use gpui_kit::{App, Styled, Window};

use crate::design::colors;

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

/// Opens a dialog on `raised`. Use instead of `window.open_dialog`.
pub fn open_dialog(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let raised = colors(cx).raised;
        build(dialog, window, cx).bg(raised)
    });
}

/// Opens an alert dialog (a confirmation) on `raised`. Use instead of
/// `window.open_alert_dialog`.
pub fn open_alert_dialog(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(AlertDialog, &mut Window, &mut App) -> AlertDialog + 'static,
) {
    window.open_alert_dialog(cx, move |alert, window, cx| {
        let raised = colors(cx).raised;
        build(alert, window, cx).bg(raised)
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
