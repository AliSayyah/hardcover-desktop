//! The sidebar's own parts (`hardcover-design`, Sidebar), besides its
//! destinations ([`super::NavRow`]). All sit on the sidebar's three spines:
//! icons at x 16, labels at x 40, counts on the trailing lane.
//!
//! - [`brand_row`]: the 40-high brand row at the top, the book-open icon in
//!   `accent` and "Hardcover" at 16/600.
//! - [`sidebar_label`]: a section's `label` ("Library", "Lists"), 16 above
//!   and 4 below.
//! - [`GoalRow`]: the 40-high reading goal row: the period on the label spine,
//!   "9 of 12 books" on the count lane, and a 2 px meter with its pace tick
//!   under them, from the label spine to the trailing edge. It opens the goal.
//! - [`AccountRow`]: the account at the foot: a 32 avatar, the username in
//!   `body-strong`, the sync line in caption, and a chevrons-up-down icon. It
//!   is the trigger of the account menu (`.dropdown_menu(..)`), and stays
//!   pressed while the menu is open. Why a sync failed is the sync line's
//!   tooltip, the row's accessible description, and a caption under the
//!   sync line while the row has keyboard focus.
//!
//! ```ignore
//! v_flex()
//!     .child(brand_row(cx))
//!     .child(sidebar_label("Reading goal", cx))
//!     .child(GoalRow::new(("goal", id), "2026", "9 of 12 books", 75.).pace(Some(88.)))
//!     .child(AccountRow::new("account", Avatar::new("demo_reader"), "demo_reader", "Updated just now")
//!         .dropdown_menu(|menu, _, _| menu))
//! ```

use std::rc::Rc;

use gpui_kit::base::Button as BaseButton;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Selectable};
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, FontWeight, InteractiveElement,
    Interactivity, IntoElement, ParentElement as _, RenderOnce, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _,
};

use super::Meter;
use super::focus::{FocusTreatment as _, control_radius, focus_visible, keyed_focus};
use super::reveal::probe;
use crate::bidi::WrappingText;
use crate::design::type_roles::Family;
use crate::design::{TypeRole, TypeRoles as _, colors, size};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The brand at the top of the sidebar.
pub fn brand_row(cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(size::ROW_TALL)
        .px_2()
        .type_role(Family::Sans, FontWeight::SEMIBOLD, (16., 24.))
        .child(
            Icon::new(IconName::BookOpen)
                .size_4()
                .text_color(colors(cx).accent),
        )
        .child("Hardcover")
}

/// A section's label in the sidebar: "Library", "Reading goal", "Lists".
pub fn sidebar_label(title: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_none()
        .pt_4()
        .pb_1()
        .px_2()
        .label(cx)
        .truncate()
        .child(title.into())
}

/// The sidebar's reading goal row. See the module doc.
#[derive(IntoElement)]
pub struct GoalRow {
    id: ElementId,
    label: SharedString,
    count: SharedString,
    percent: f32,
    pace: Option<f32>,
    accessibility_label: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl GoalRow {
    /// `label` sits on the label spine ("2026"), `count` on the count lane
    /// ("9 of 12 books"); `percent` fills the meter.
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        count: impl Into<SharedString>,
        percent: f32,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count: count.into(),
            percent,
            pace: None,
            accessibility_label: None,
            on_click: None,
        }
    }

    /// The pace tick, at `percent` of the way. `None` draws none.
    pub fn pace(mut self, percent: Option<f32>) -> Self {
        self.pace = percent;
        self
    }

    /// What a screen reader says: the label, the count and the pace.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for GoalRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let focus = keyed_focus(self.id.clone(), window, cx);
        let focused = focus_visible(&focus, window);
        let radius = control_radius(cx);
        let on_click = self.on_click;
        let label = self
            .accessibility_label
            .unwrap_or_else(|| format!("{}, {}", self.label, self.count).into());
        let meter_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "meter".into());
        probe(
            BaseButton::new(self.id)
                .track_focus(&focus)
                .accessibility_label(label)
                .relative()
                .flex_col()
                .items_stretch()
                .justify_center()
                .gap_1()
                .w_full()
                .h(size::ROW_TALL)
                .px_2()
                .rounded(radius)
                .text_color(colors.fg)
                .body()
                .hover(|style| style.bg(colors.hover))
                .active(|style| style.bg(colors.selected))
                .when_some(on_click, |this, handler| {
                    this.on_click(move |event, window, cx| handler(event, window, cx))
                })
                .when(focused, |this| this.focus_ring(radius, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(crate::icons::target())
                                .size_4()
                                .flex_none()
                                .text_color(colors.fg_muted),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(self.label))
                        .child(
                            div()
                                .flex_none()
                                .pl_2()
                                .caption(cx)
                                .baseline_on(TypeRole::Caption, TypeRole::Body)
                                .tabular()
                                .child(self.count),
                        ),
                )
                // From the label spine: past the 16 icon and its 8 gap.
                .child(
                    div().pl_6().child(
                        Meter::new(meter_id, self.percent, "Progress toward the goal")
                            .thin()
                            .pace(self.pace),
                    ),
                ),
        )
    }
}

/// The account at the foot of the sidebar, and the account menu's trigger.
/// See the module doc.
#[derive(IntoElement)]
pub struct AccountRow {
    base: Stateful<Div>,
    id: ElementId,
    avatar: AnyElement,
    name: SharedString,
    status: AnyElement,
    status_tooltip: Option<SharedString>,
    focus: Option<FocusHandle>,
    open: bool,
}

impl AccountRow {
    /// `avatar` is a [`super::Avatar`]; `status` is the sync line ("Updated
    /// just now"), which may lead with a 12 px icon.
    pub fn new(
        id: impl Into<ElementId>,
        avatar: impl IntoElement,
        name: impl Into<SharedString>,
        status: impl IntoElement,
    ) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            id,
            avatar: avatar.into_any_element(),
            name: name.into(),
            status: status.into_any_element(),
            status_tooltip: None,
            focus: None,
            open: false,
        }
    }

    /// The row's focus, kept by its view, so the menu and the dialogs it
    /// opens can give focus back to the row (`PopupMenu::action_context`).
    /// Without one the row keeps its own.
    pub fn focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }

    /// More about the sync line, such as why it failed: its tooltip, the
    /// row's accessible description, and a caption under the sync line
    /// while the row has keyboard focus.
    pub fn status_tooltip(mut self, tooltip: Option<SharedString>) -> Self {
        self.status_tooltip = tooltip;
        self
    }
}

impl Selectable for AccountRow {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl Styled for AccountRow {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for AccountRow {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

/// The menu opens from Enter or Space on the focused row, or a click.
impl DropdownMenu for AccountRow {}

impl RenderOnce for AccountRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let focus = match self.focus {
            Some(focus) => focus,
            None => keyed_focus(self.id.clone(), window, cx),
        };
        let focused = focus_visible(&focus, window);
        let radius = control_radius(cx);
        let status_id =
            ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "status".into());
        let reason = self.status_tooltip.clone().filter(|_| focused);
        self.base
            .role(Role::Button)
            .aria_label(self.name.clone())
            .when_some(self.status_tooltip.clone(), |this, tip| {
                this.aria_description(tip)
            })
            .track_focus(&focus.clone().tab_stop(true))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .w_full()
            .p_2()
            .rounded(radius)
            .text_color(colors.fg)
            .when(self.open, |this| this.bg(colors.selected))
            .when(!self.open, |this| {
                this.hover(|style| style.bg(colors.hover))
                    .active(|style| style.bg(colors.selected))
            })
            .when(focused, |this| this.focus_ring(radius, cx))
            .child(self.avatar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().body_strong().truncate().child(self.name))
                    .child(
                        div()
                            .id(status_id)
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w_0()
                            .caption(cx)
                            .child(self.status)
                            .when_some(self.status_tooltip.clone(), |this, tip| {
                                this.tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                            }),
                    )
                    // The reason, for the keyboard, under the sync line.
                    .when_some(reason, |this, reason| {
                        this.child(div().caption(cx).child(WrappingText::new(reason)))
                    }),
            )
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size_4()
                    .flex_none()
                    .text_color(colors.fg_muted),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::test_support::{host, press, tab};
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn the_goal_row_opens_from_the_keyboard(cx: &mut TestAppContext) {
        let opened = Rc::new(Cell::new(0));
        let cx = host(cx, {
            let opened = opened.clone();
            move |_, _| {
                let opened = opened.clone();
                GoalRow::new("goal", "2026", "9 of 12 books", 75.)
                    .pace(Some(88.))
                    .on_click(move |_, _, _| opened.set(opened.get() + 1))
                    .into_any_element()
            }
        });
        tab(cx);
        press(cx, "enter");
        assert_eq!(opened.get(), 1);
    }
}
