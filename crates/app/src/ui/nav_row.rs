//! Nav row (`hardcover-design`, Sidebar): one destination in the sidebar.
//!
//! - 32 high, padding 8, `r-control`, on the three sidebar spines: the 16 px
//!   icon, the label, and the count lane (caption, tabular, `fg-muted`).
//! - Hover: the system's 5% `hover` fill (the kit's sidebar menu hovers at
//!   8%, which is why the sidebar needs this row).
//! - Selected: the `selected` fill, the label at weight 500, the icon in
//!   `accent`, the count in `fg`. Pressed takes the `selected` fill too.
//! - A real button: Tab reaches it, Enter and Space press it, and it shows
//!   the focus ring.
//! - While Ctrl is held, the destination's shortcut ([`NavRow::hint`],
//!   usually a `Kbd`) takes the place of its count.
//! - A command row in the sidebar ("New list…", "Set a reading goal…") is
//!   [`NavRow::muted`]: its label in `fg-muted`, so it reads apart from the
//!   destinations around it.
//! - Disabled ([`NavRow::disabled`]): 50% opacity, no hover or press, out of
//!   the Tab order, with the reason in its tooltip ([`NavRow::tooltip`],
//!   usually `format::needs_permission`).
//! - In a scroller, [`NavRow::reveal_in`] scrolls the row into view when it
//!   gains focus.
//!
//! ```ignore
//! NavRow::new("nav-home", IconName::LayoutDashboard, "Home")
//!     .selected(destination == Destination::Home)
//!     .count(model.count(status).to_string())
//!     .hint(key_hints.then(|| Kbd::global_binding_for_action(&ShowHome, window)).flatten())
//!     .on_click(move |_, window, cx| show(Destination::Home, window, cx))
//! ```

use std::rc::Rc;

use gpui_kit::base::Button as BaseButton;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::focus::{
    FocusTreatment as _, control_radius, focus_visible, keyed_focus, reveal_on_focus,
};
use crate::design::{TypeRoles as _, colors, size};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A sidebar destination. See the module doc.
#[derive(IntoElement)]
pub struct NavRow {
    id: ElementId,
    icon: IconName,
    label: SharedString,
    count: Option<SharedString>,
    hint: Option<AnyElement>,
    selected: bool,
    muted: bool,
    disabled: bool,
    tooltip: Option<SharedString>,
    reveal_in: Option<ScrollHandle>,
    on_click: Option<ClickHandler>,
}

impl NavRow {
    pub fn new(id: impl Into<ElementId>, icon: IconName, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            count: None,
            hint: None,
            selected: false,
            muted: false,
            disabled: false,
            tooltip: None,
            reveal_in: None,
            on_click: None,
        }
    }

    /// The figure on the count lane, such as a shelf's size.
    pub fn count(mut self, count: impl Into<SharedString>) -> Self {
        self.count = Some(count.into());
        self
    }

    /// The shortcut shown instead of the count while Ctrl is held. `None`
    /// keeps the count.
    pub fn hint(mut self, hint: Option<impl IntoElement>) -> Self {
        self.hint = hint.map(IntoElement::into_any_element);
        self
    }

    /// The current destination.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A command among destinations ("New list…"): the label in `fg-muted`.
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// Unavailable: 50% opacity, no response. Say why with [`NavRow::tooltip`].
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// A tooltip, such as why the row is disabled.
    pub fn tooltip(mut self, tooltip: Option<impl Into<SharedString>>) -> Self {
        self.tooltip = tooltip.map(Into::into);
        self
    }

    /// The scroller the row sits in: it scrolls to show the row when the row
    /// gains focus ([`super::focus::reveal_on_focus`]).
    pub fn reveal_in(mut self, scroll: &ScrollHandle) -> Self {
        self.reveal_in = Some(scroll.clone());
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

impl RenderOnce for NavRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let focus = keyed_focus(self.id.clone(), window, cx);
        let focused = focus_visible(&focus, window);
        let radius = control_radius(cx);
        let selected = self.selected;
        let disabled = self.disabled;
        let on_click = self.on_click;
        let tooltip = self.tooltip;
        let reveal = self
            .reveal_in
            .map(|scroll| reveal_on_focus(self.id.clone(), &focus, &scroll, window, cx));
        let trailing: Option<AnyElement> = match (self.hint, self.count) {
            (Some(hint), _) => Some(hint),
            (None, Some(count)) => Some(
                div()
                    .caption(cx)
                    .tabular()
                    .when(selected, |this| this.text_color(colors.fg))
                    .child(count)
                    .into_any_element(),
            ),
            (None, None) => None,
        };

        BaseButton::new(self.id)
            .track_focus(&focus)
            .selected(selected)
            .disabled(disabled)
            .accessibility_label(self.label.clone())
            .relative()
            .w_full()
            .h(size::CONTROL)
            .px_2()
            .gap_2()
            .justify_start()
            .rounded(radius)
            .text_color(colors.fg)
            .body()
            .when(selected, |this| {
                this.bg(colors.selected).font_weight(FontWeight::MEDIUM)
            })
            .when(self.muted, |this| this.text_color(colors.fg_muted))
            .when(disabled, |this| this.opacity(0.5))
            .when(!selected && !disabled, |this| {
                this.hover(|style| style.bg(colors.hover))
                    .active(|style| style.bg(colors.selected))
            })
            .when_some(tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .when_some(on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when(focused, |this| this.focus_ring(radius, cx))
            .children(reveal)
            .child(
                Icon::new(self.icon)
                    .size_4()
                    .flex_none()
                    .text_color(if selected {
                        colors.accent
                    } else {
                        colors.fg_muted
                    }),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.label))
            .children(trailing.map(|t| div().flex_none().pl_2().child(t)))
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
    fn each_row_reports_its_own_press(cx: &mut TestAppContext) {
        let chosen = Rc::new(Cell::new(None));
        let cx = host(cx, {
            let chosen = chosen.clone();
            move |_, _| {
                div()
                    .children((0..2usize).map(|ix| {
                        let chosen = chosen.clone();
                        NavRow::new(("nav", ix), IconName::BookOpen, "Shelf")
                            .count("3")
                            .selected(ix == 0)
                            .on_click(move |_, _, _| chosen.set(Some(ix)))
                    }))
                    .into_any_element()
            }
        });
        tab(cx);
        tab(cx);
        press(cx, "enter");
        assert_eq!(chosen.get(), Some(1));
    }

    #[gpui_kit::test]
    fn a_disabled_row_is_skipped_and_reports_no_press(cx: &mut TestAppContext) {
        let chosen = Rc::new(Cell::new(None));
        let cx = host(cx, {
            let chosen = chosen.clone();
            move |_, _| {
                div()
                    .children((0..2usize).map(|ix| {
                        let chosen = chosen.clone();
                        NavRow::new(("nav", ix), IconName::Plus, "New list…")
                            .disabled(ix == 0)
                            .tooltip(Some("No permission"))
                            .on_click(move |_, _, _| chosen.set(Some(ix)))
                    }))
                    .into_any_element()
            }
        });
        // The first Tab skips the disabled row and lands on the second.
        tab(cx);
        press(cx, "enter");
        assert_eq!(chosen.get(), Some(1));
    }
}
