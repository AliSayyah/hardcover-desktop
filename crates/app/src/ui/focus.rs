//! The focus treatment (`hardcover-design`, States → Focus-visible): a 3 px
//! `ring` outside, plus a 1 px accent edge. Controls take both
//! ([`FocusTreatment::focus_ring`]); rows take the edge alone
//! ([`FocusTreatment::focus_edge`]), because they sit in lists that clip
//! whatever is drawn outside them. Bare text with no box of its own (a text
//! button, a byline link) takes both outside the text
//! ([`FocusTreatment::focus_ring_around`]), so the edge never touches a
//! glyph.
//!
//! The kit's own controls draw this themselves from the theme. Use these for
//! the app's components that track their own focus.
//!
//! A focused element must also be seen: GPUI doesn't scroll a plain
//! scrolling `div` to the element that takes focus. A component inside one
//! adds [`reveal_on_focus`], which scrolls just enough to show it, with its
//! ring, each time it gains focus.
//!
//! ```ignore
//! let focus = keyed_focus(id.clone(), window, cx);
//! div()
//!     .track_focus(&focus)
//!     .rounded(cx.theme().radius)
//!     .when(focus_visible(&focus, window), |this| this.focus_ring(cx.theme().radius, cx))
//! ```

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, App, BoxShadow, ElementId, FocusHandle, Hsla, IntoElement as _, ParentElement,
    Pixels, ScrollHandle, Styled, Window, canvas, div, point, px,
};

use crate::design::{colors, size};

/// Whether `handle` shows its focus: it is focused and the last input came
/// from the keyboard, so a click doesn't leave a ring behind.
pub fn focus_visible(handle: &FocusHandle, window: &Window) -> bool {
    handle.is_focused(window) && window.last_input_was_keyboard()
}

/// A focus handle kept for the element `id` across frames, for a component
/// that is rebuilt every render (`RenderOnce`).
pub fn keyed_focus(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(id.into(), cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone()
}

/// The 1 px accent edge, drawn inside the element's bounds.
fn edge(cx: &App) -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), colors(cx).accent)
        .blur_radius(px(0.))
        .spread_radius(size::FOCUS_EDGE)
        .inset()
}

/// The focus treatment, on any styled parent.
pub trait FocusTreatment: Styled + ParentElement + Sized {
    /// A control's focus: the 3 px ring outside and the 1 px accent edge.
    /// `radius` is the element's own corner radius, so the ring follows it.
    /// The element must be positioned (`relative`, the default for the
    /// app's components), and nothing around it may clip.
    ///
    /// Replaces the element's box shadow.
    fn focus_ring(self, radius: Pixels, cx: &App) -> Self {
        let ring = size::FOCUS_RING;
        // A band drawn only outside the element, so a translucent fill
        // (a hovered ghost control) doesn't show a shadow through it.
        self.shadow(vec![edge(cx)]).child(
            div()
                .absolute()
                .top(-ring)
                .left(-ring)
                .right(-ring)
                .bottom(-ring)
                .rounded(radius + ring)
                .border(ring)
                .border_color(colors(cx).ring),
        )
    }

    /// The focus of bare text (a text button, a byline link): the 1 px
    /// accent edge just outside the text, standing in for the border it
    /// doesn't have, and the 3 px ring outside that. The same conditions as
    /// [`FocusTreatment::focus_ring`]; the element's box shadow is kept.
    fn focus_ring_around(self, radius: Pixels, cx: &App) -> Self {
        let edge = size::FOCUS_EDGE;
        let ring = size::FOCUS_RING;
        let band = |inset: Pixels, width: Pixels, color: Hsla| {
            div()
                .absolute()
                .top(-inset)
                .left(-inset)
                .right(-inset)
                .bottom(-inset)
                .rounded(radius + inset)
                .border(width)
                .border_color(color)
        };
        self.child(band(edge, edge, colors(cx).accent)).child(band(
            edge + ring,
            ring,
            colors(cx).ring,
        ))
    }

    /// A row's focus: the 1 px accent edge alone, inside the row. Replaces
    /// the element's box shadow.
    fn focus_edge(self, cx: &App) -> Self {
        self.shadow(vec![edge(cx)])
    }
}

impl<T: Styled + ParentElement + Sized> FocusTreatment for T {}

/// The corner radius of controls (`r-control`).
pub fn control_radius(cx: &App) -> Pixels {
    cx.theme().radius
}

/// The corner radius of rows, panels, popovers and dialogs (`r-box`).
pub fn box_radius(cx: &App) -> Pixels {
    cx.theme().radius_lg
}

/// An invisible child that keeps its parent in view inside the scroller
/// `scroll`: when `focus` is gained, the scroller moves just enough to show
/// the parent and its focus ring, and then leaves the scroll to the reader
/// until focus leaves and comes back. The parent must be positioned
/// (`relative`); `id` is the parent's id.
pub fn reveal_on_focus(
    id: impl Into<ElementId>,
    focus: &FocusHandle,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id = ElementId::NamedChild(std::sync::Arc::new(id.into()), "reveal".into());
    let revealed = window.use_keyed_state(id, cx, |_, _| false);
    let focused = focus.is_focused(window);
    if !focused {
        revealed.update(cx, |r, _| *r = false);
    }
    let scroll = scroll.clone();
    let margin = size::FOCUS_RING + px(1.);
    canvas(
        move |bounds, window, cx| {
            if !focused || *revealed.read(cx) {
                return;
            }
            revealed.update(cx, |r, _| *r = true);
            let viewport = scroll.bounds();
            let offset = scroll.offset();
            let shift = if bounds.top() - margin < viewport.top() {
                viewport.top() - (bounds.top() - margin)
            } else if bounds.bottom() + margin > viewport.bottom() {
                viewport.bottom() - (bounds.bottom() + margin)
            } else {
                return;
            };
            scroll.set_offset(point(offset.x, offset.y + shift));
            window.refresh();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}
