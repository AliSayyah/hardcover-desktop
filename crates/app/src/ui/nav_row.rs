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
use gpui_kit::component::{Icon, IconName};
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};

use super::focus::{FocusTreatment as _, control_radius, focus_visible, keyed_focus};
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
        let on_click = self.on_click;
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
            .when(!selected, |this| {
                this.hover(|style| style.bg(colors.hover))
                    .active(|style| style.bg(colors.selected))
            })
            .when_some(on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when(focused, |this| this.focus_ring(radius, cx))
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
