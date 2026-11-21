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
//! - [`SmallControl`]: the system's small button, select and segmented bar (24 high,
//!   padding 8, 12 px text, 12 px icon, gap 4). The kit's `Size::Small` sets
//!   14 px text and a 14 px icon; use `.small_control()` (or
//!   `.small_icon_button()` for an icon alone) wherever a small control
//!   goes, never `.small()`. A small segmented bar's segments are
//!   [`small_tab`]s.
//! - [`Alert`], built by [`open_alert_dialog`]: the app's own footer, whose
//!   buttons take focus by handle and show the ring only after keyboard
//!   input ([`KeyboardRing`]).
//! - [`close_own_dialog`]: a dialog that closes itself after an awaited
//!   save (the list form) closes only while it is the one in front; the
//!   kit's `close_dialog` closes the newest, whichever that is.
//! - [`OnTextSpine`]: a quiet button (ghost, or a danger label) that starts a
//!   line puts its label, not its padding, on the content's leading edge.
//! - [`danger_menu_item`]: a destructive menu command ("Delete read…") in
//!   `danger`, its icon too; the kit's menu items take no colour.
//! - [`gutter_list`] and [`gutter_scrollbar`]: a scrollbar that stays in the
//!   8 inset beside a list's rows (the shelf and search lists, the sidebar).
//!   The kit's thumb is 6 wide and 4 in from the edge, 8 wide on hover, so
//!   it reaches 10 to 12 in and covers the trailing edge of a selected row;
//!   this one is 4 wide and 1 in.
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
//! - **The segmented tab bar's indicator** is always paper (`background`)
//!   and casts the kit's small raised shadow, a shadow on a control: both
//!   are hard-coded in the bar.
//! - **A small segment** pads its label 10, not the small control's 8: the
//!   padding sits on an inner box the tab's style doesn't reach.
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
//! - **A notification's action** ("Undo") is built with the kit's own
//!   `Size::Small` after the app's style, so its label is 14 px, not 12.
//! - **The kit's focus ring** is 3 px, not the system's 2: its width is a
//!   private constant. The app's own rows, text buttons and components draw
//!   2 (`ui::focus`).
//! - **Px measures** don't follow the interface scale (`ui_scale`): chart
//!   labels are a fixed 10 px, and a segmented `TabBar`'s height and padding
//!   and a dialog's margins are px.
//! - **The kit's buttons and switches show their ring whenever they hold
//!   focus**, not only after keyboard input: they check only that they are
//!   focused. A press never focuses them, so the ring shows where focus is
//!   handed to one after a pointer action: a menu closing onto its trigger,
//!   a dialog closing onto the button that opened it, Back returning to the
//!   control that opened the page. A button that takes its focus from a
//!   handle the app keeps follows the rule ([`KeyboardRing`]: the alert
//!   footers, and the book view's buttons focus is handed to: the status,
//!   "more" and read menus' triggers, the panel's button, "Add to library",
//!   "Add a past read…" and the Notes and review openers); a switch can't
//!   be given one.
//! - **Empty-state icons** are Lucide's fixed SVGs, drawn at stroke 2, not
//!   the system's 1.5.

use gpui_kit::base::FocusableExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::{ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{Dialog, DialogContent, DialogFooter};
use gpui_kit::component::list::{List, ListDelegate, ListState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarHandle, ScrollbarThumbStyle};
use gpui_kit::component::select::{Select, SelectDelegate, SelectItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Icon, IconName, Sizable as _, WindowExt as _};
use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    App, ClickEvent, Div, ElementId, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, NoAction, ParentElement as _, Pixels, SharedString, Styled, Window, div,
    prelude::FluentBuilder as _, px, rems,
};

use super::focus::focus_visible;
use crate::bidi::WrappingText;
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

impl SmallControl for TabBar {
    /// The kit's small segmented bar: 24 high. Its tabs set their own text
    /// size, so give it [`small_tab`]s, whose labels are 12 px.
    fn small_control(self) -> Self {
        self.small()
    }
}

/// One segment of a small segmented `TabBar` (`.small_control()`): its label
/// at 12/16, as the system's small controls are. The kit's small tab sets
/// 14 px text on the tab itself, after any style the app gives it, so the
/// label is a child that sets its own size.
///
/// ```ignore
/// TabBar::new("search-kind").segmented().small_control().child(small_tab("Books"))
/// ```
pub fn small_tab(label: impl Into<SharedString>) -> Tab {
    let label = label.into();
    let (text, line) = TypeRole::Caption.scale();
    Tab::new().aria_label(label.clone()).child(
        div()
            .text_size(rems(text / 16.))
            .line_height(rems(line / 16.))
            .child(label),
    )
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

/// A destructive menu command ("Remove from library…", "Delete read…"): its
/// label and Delete icon in `danger`, as every destructive command is. The
/// kit's menu items take no colour, so it is an element item. Disabled, it
/// takes the kit's muted colour instead.
pub fn danger_menu_item(label: impl Into<SharedString>, enabled: bool, cx: &App) -> PopupMenuItem {
    let label = label.into();
    let danger = enabled.then(|| colors(cx).danger);
    PopupMenuItem::element(move |_, _| {
        div()
            .when_some(danger, gpui_kit::Styled::text_color)
            .child(label.clone())
    })
    .icon(Icon::new(IconName::Delete).when_some(danger, gpui_kit::Styled::text_color))
    .disabled(!enabled)
}

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

/// Whether the dialog holding `content` (a handle its content tracks on its
/// root) is the one in front: focus is in the content, on the dialog around
/// it, or nowhere. A dialog opened over it takes focus into itself.
///
/// The kit closes only the newest dialog (`close_dialog`), so a dialog that
/// closes itself after an awaited save checks this first, and waits for
/// focus to come back otherwise ([`close_own_dialog`]).
pub fn dialog_in_front(content: &FocusHandle, window: &Window, cx: &mut App) -> bool {
    window.focused(cx).is_none()
        || content.contains_focused(window, cx)
        || content.within_focused(window, cx)
}

/// Closes the dialog holding `content` if it is in front
/// ([`dialog_in_front`]) and returns `true`; otherwise leaves every dialog
/// open and returns `false`, and the caller closes it once focus comes back
/// into it (`cx.on_focus_in(content, ..)`).
pub fn close_own_dialog(content: &FocusHandle, window: &mut Window, cx: &mut App) -> bool {
    let in_front = dialog_in_front(content, window, cx);
    if in_front {
        window.close_dialog(cx);
    }
    in_front
}

/// A button that takes its focus from `handle`, a handle the app keeps, so
/// the app can focus it, and that shows the kit's ring only after keyboard
/// input, like the app's own controls (`ui::focus`). The kit's buttons
/// otherwise show it whenever they hold focus (Kit limits).
pub trait KeyboardRing: Sized {
    fn keyboard_ring(self, handle: &FocusHandle, window: &Window) -> Self;
}

impl KeyboardRing for Button {
    fn keyboard_ring(self, handle: &FocusHandle, window: &Window) -> Self {
        self.track_focus(handle)
            .focus_ring(focus_visible(handle, window))
    }
}

type AlertContent = Rc<dyn Fn(DialogContent, &mut Window, &mut App) -> DialogContent>;
type AlertOk = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool>;

/// A confirmation's parts, as [`open_alert_dialog`] builds it: the kit's
/// `AlertDialog` names, with the app's own footer, whose buttons focus by
/// handle. The title and description go through `WrappingText`, since they
/// often hold a book's or a list's name, which may be right to left.
pub struct Alert {
    title: Option<SharedString>,
    description: Option<SharedString>,
    content: Option<AlertContent>,
    cancel: Option<SharedString>,
    ok_text: SharedString,
    ok_variant: ButtonVariant,
    on_ok: Option<AlertOk>,
}

impl Alert {
    fn new() -> Self {
        Self {
            title: None,
            description: None,
            content: None,
            cancel: None,
            ok_text: "OK".into(),
            ok_variant: ButtonVariant::Primary,
            on_ok: None,
        }
    }

    /// The decision: `Delete “Book club picks”?`.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Only new information: the scope, the consequence, the way back.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// More under the description (release notes, a link).
    pub fn content(
        mut self,
        builder: impl Fn(DialogContent, &mut Window, &mut App) -> DialogContent + 'static,
    ) -> Self {
        self.content = Some(Rc::new(builder));
        self
    }

    /// Adds Cancel before the result.
    pub fn confirm(mut self) -> Self {
        self.cancel.get_or_insert_with(|| "Cancel".into());
        self
    }

    /// Cancel's label ("Later"); also adds it.
    pub fn cancel_text(mut self, text: impl Into<SharedString>) -> Self {
        self.cancel = Some(text.into());
        self
    }

    /// The result's label: "Delete", "Mark as read".
    pub fn ok_text(mut self, text: impl Into<SharedString>) -> Self {
        self.ok_text = text.into();
        self
    }

    /// `Danger` for a destructive result; primary otherwise.
    pub fn ok_variant(mut self, variant: ButtonVariant) -> Self {
        self.ok_variant = variant;
        self
    }

    /// Runs the result; the alert closes when it returns `true`.
    pub fn on_ok(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_ok = Some(Rc::new(handler));
        self
    }
}

/// Which of an alert's buttons holds focus when it opens.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AlertFocus {
    /// Cancel when there is one, else the result.
    Cancel,
    /// The result.
    Result,
}

/// Opens an alert dialog (a confirmation) on `raised`, 384 wide. Use instead
/// of `window.open_alert_dialog`.
///
/// It opens with focus on Cancel when it has one (`.confirm()`), so a
/// keyboard user sees what Enter does and a destructive confirmation never
/// runs on a stray Enter; else on its one button. A confirmation whose
/// result is safe opens on that result instead:
/// [`open_alert_dialog_on_result`]. Focus goes to the button itself, by
/// its handle, whatever the content holds before it (a link, say).
pub fn open_alert_dialog(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(Alert, &mut Window, &mut App) -> Alert + 'static,
) {
    open_alert(window, cx, build, AlertFocus::Cancel);
}

/// [`open_alert_dialog`] for a confirmation that isn't destructive ("Install"
/// an update): focus opens on its result.
pub fn open_alert_dialog_on_result(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(Alert, &mut Window, &mut App) -> Alert + 'static,
) {
    open_alert(window, cx, build, AlertFocus::Result);
}

fn open_alert(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(Alert, &mut Window, &mut App) -> Alert + 'static,
    focus: AlertFocus,
) {
    let cancel_focus = cx.focus_handle();
    let ok_focus = cx.focus_handle();
    // The kit focuses the dialog's shell as it opens; initial focus is set
    // on the first build of this opening only, so a later rebuild never
    // takes it from where the reader moved it.
    let first_build = Rc::new(Cell::new(true));
    window.open_alert_dialog(cx, move |dialog, window, cx| {
        let alert = build(Alert::new(), window, cx);
        let target = match (focus, alert.cancel.is_some()) {
            (AlertFocus::Cancel, true) => &cancel_focus,
            _ => &ok_focus,
        };
        if first_build.replace(false) {
            target.focus(window, cx);
        }
        let cancel = alert.cancel.map(|label| {
            Button::new("alert-cancel")
                .label(label)
                .keyboard_ring(&cancel_focus, window)
                .on_click(|_, window, cx| window.close_dialog(cx))
        });
        let on_ok = alert.on_ok;
        let ok = Button::new("alert-ok")
            .label(alert.ok_text)
            .with_variant(alert.ok_variant)
            .keyboard_ring(&ok_focus, window)
            .on_click(move |event, window, cx| {
                if on_ok.as_ref().is_none_or(|on_ok| on_ok(event, window, cx)) {
                    window.close_dialog(cx);
                }
            });
        let raised = colors(cx).raised;
        let width = size::DIALOG_CONFIRM.to_pixels(window.rem_size());
        dialog
            .width(width)
            .when_some(alert.title, |this, title| {
                this.title(WrappingText::new(title))
            })
            .when_some(alert.description, |this, text| {
                this.description(WrappingText::new(text))
            })
            .when_some(alert.content, |this, content| {
                this.content(move |c, window, cx| content(c, window, cx))
            })
            .footer(DialogFooter::new().children(cancel).child(ok))
            .bg(raised)
    });
}

/// The thumb of a [`gutter_scrollbar`]: 4 wide, 1 in from the edge, so it
/// spans 1 to 5 in. A row inset 8 keeps clear of it, and so does its focus
/// ring, drawn 3 outside the row.
const GUTTER_THUMB_WIDTH: Pixels = px(4.);
const GUTTER_THUMB_INSET: Pixels = px(1.);

/// The kit's vertical scrollbar over a scroller whose rows sit 8 in from its
/// trailing edge (the sidebar), kept in that inset: the thumb is 4 wide and
/// 1 in, at rest, hovered and dragged, so it never covers a row's selection
/// fill or its focus ring. Its colours stay the theme's. Add it as the scroller's last child,
/// in place of `.vertical_scrollbar(&handle)`; `id` keeps its own state (two
/// scrollbars drawn from this one place would share it otherwise).
pub fn gutter_scrollbar<H: ScrollbarHandle + Clone>(
    id: impl Into<ElementId>,
    handle: &H,
) -> impl IntoElement {
    let thumb =
        |style: ScrollbarThumbStyle| style.width(GUTTER_THUMB_WIDTH).inset(GUTTER_THUMB_INSET);
    div().absolute().inset_0().child(
        Scrollbar::vertical(handle)
            .id(id)
            .viewport_from_layout()
            .styles(|styles| styles.thumb(thumb).thumb_hover(thumb).thumb_active(thumb)),
    )
}

/// A list whose rows are inset 8 (`p_2`), filling its parent, with a
/// [`gutter_scrollbar`] in place of the kit's own.
pub fn gutter_list<D: ListDelegate>(
    id: impl Into<ElementId>,
    list: &Entity<ListState<D>>,
    cx: &App,
) -> Div {
    div()
        .relative()
        .size_full()
        .child(List::new(list).size_full().p_2().scrollbar_visible(false))
        .child(gutter_scrollbar(id, list.read(cx).scroll_handle()))
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
    use crate::ui::ExternalLink;
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
        cx.update(gpui_kit::component::WindowExt::has_active_dialog)
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
        cx.update(gpui_kit::Window::focus_next);
        redraw(cx);
        press(cx, "enter");
        redraw(cx);
        assert_eq!(pressed.get(), 1);
        assert!(dialog_open(cx));
    }

    gpui_kit::actions!(kit_test, [ConfirmRow]);

    /// Enter on a row's own button presses it; without `row_action` the
    /// list around it would take Enter as confirming the row.
    #[gpui_kit::test]
    fn enter_on_a_rows_own_button_presses_it(cx: &mut TestAppContext) {
        use crate::ui::test_support::{counter, host, tab};
        use gpui_kit::{InteractiveElement as _, KeyBinding};

        let run = |cx: &mut TestAppContext, wrapped: bool| {
            let (pressed, on_click) = counter();
            let confirmed = Rc::new(Cell::new(0));
            let cx = host(cx, {
                let confirmed = confirmed.clone();
                move |_, _| {
                    let confirmed = confirmed.clone();
                    let button = Button::new("remove")
                        .label("Remove")
                        .on_click(on_click.clone());
                    div()
                        .key_context("TestList")
                        .on_action(move |_: &ConfirmRow, _, _| {
                            confirmed.set(confirmed.get() + 1);
                        })
                        .map(|this| {
                            if wrapped {
                                this.child(row_action(button))
                            } else {
                                this.child(button)
                            }
                        })
                        .into_any_element()
                }
            });
            cx.update(|_, cx| {
                cx.bind_keys([KeyBinding::new("enter", ConfirmRow, Some("TestList"))]);
            });
            tab(cx);
            press(cx, "enter");
            redraw(cx);
            (pressed.get(), confirmed.get())
        };
        assert_eq!(run(cx, false), (0, 1), "the list takes Enter");
        assert_eq!(run(cx, true), (1, 0), "the row's button takes Enter");
    }

    /// Opens an alert whose result counts in `confirmed`, settles it and
    /// presses Enter on whatever it opened on.
    fn confirm_with_enter(
        cx: &mut VisualTestContext,
        on_result: bool,
        build: impl Fn(Alert) -> Alert + 'static,
    ) -> usize {
        let confirmed = Rc::new(Cell::new(0));
        cx.update(|window, cx| {
            let confirmed = confirmed.clone();
            let build = move |alert: Alert, _: &mut Window, _: &mut App| {
                let confirmed = confirmed.clone();
                build(alert).on_ok(move |_, _, _| {
                    confirmed.set(confirmed.get() + 1);
                    true
                })
            };
            if on_result {
                open_alert_dialog_on_result(window, cx, build);
            } else {
                open_alert_dialog(window, cx, build);
            }
        });
        redraw(cx);
        redraw(cx);
        press(cx, "enter");
        cx.run_until_parked();
        redraw(cx);
        confirmed.get()
    }

    #[gpui_kit::test]
    fn a_confirmation_opens_on_cancel(cx: &mut TestAppContext) {
        let cx = open(cx);
        let delete = |alert: Alert| {
            alert
                .title("Delete “Book club picks”?")
                .confirm()
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
        };
        assert_eq!(confirm_with_enter(cx, false, delete), 0);
        assert!(!dialog_open(cx), "Enter on Cancel closes it");
    }

    #[gpui_kit::test]
    fn a_safe_confirmation_opens_on_its_result_past_a_link(cx: &mut TestAppContext) {
        let cx = open(cx);
        let install = |alert: Alert| {
            alert
                .title("Hardcover Desktop 9.9.9 is out")
                .content(|content, _, _| {
                    content.child(ExternalLink::new(
                        "page",
                        "Open the release page",
                        "https://x",
                    ))
                })
                .confirm()
                .cancel_text("Later")
                .ok_text("Install")
        };
        assert_eq!(confirm_with_enter(cx, true, install), 1);
    }

    /// C3: a dialog that hands over to a confirmation (Settings → "Sign
    /// out…", a goal → "Delete goal…") closes itself first; Cancel on the
    /// confirmation gives focus back to what opened the dialog.
    #[gpui_kit::test]
    fn cancel_on_a_confirmation_that_replaced_a_dialog_returns_to_its_opener(
        cx: &mut TestAppContext,
    ) {
        let cx = open(cx);
        cx.update(|window, _| window.activate_window());
        let opener = cx.update(|_, cx| cx.focus_handle());
        cx.update(|window, cx| {
            window.focus(&opener, cx);
            open_dialog(window, cx, |dialog, _, _| {
                dialog.child(Button::new("sign-out").label("Sign out…"))
            });
        });
        redraw(cx);
        cx.update(|window, cx| {
            // As Settings' "Sign out…" does.
            window.close_all_dialogs(cx);
            open_alert_dialog(window, cx, |alert, _, _| {
                alert
                    .title("Sign out of Hardcover?")
                    .confirm()
                    .ok_text("Sign out")
                    .ok_variant(ButtonVariant::Danger)
            });
        });
        redraw(cx);
        redraw(cx);
        press(cx, "enter");
        cx.run_until_parked();
        redraw(cx);
        assert!(!dialog_open(cx), "Cancel closed it");
        assert!(cx.update(|window, _| opener.is_focused(window)));
    }

    /// The update alert with nothing to install has a link before its one
    /// button: counting Tab steps from the dialog landed on the link.
    #[gpui_kit::test]
    fn an_alert_with_one_button_opens_on_it_past_a_link(cx: &mut TestAppContext) {
        let cx = open(cx);
        let nothing_to_install = |alert: Alert| {
            alert
                .title("Hardcover Desktop 9.9.9 is out")
                .content(|content, _, _| {
                    content.child(ExternalLink::new(
                        "page",
                        "Open the release page",
                        "https://x",
                    ))
                })
                .ok_text("Close")
        };
        assert_eq!(confirm_with_enter(cx, false, nothing_to_install), 1);
        assert!(!dialog_open(cx));
    }
}
