//! The focus treatment (`hardcover-design`, States → Focus-visible): a 2 px
//! `ring` outside, plus a 1 px accent edge.
//! - Controls take both ([`FocusTreatment::focus_ring`]).
//! - Rows take the ring and an inset edge ([`FocusTreatment::focus_row`]).
//!   A list clips each row, so the row's ring is drawn over the list, still
//!   clipped where the list cuts the row off.
//! - Bare text with no box of its own (a text button, a byline link) takes
//!   both outside the text ([`FocusTreatment::focus_ring_around`]), so the
//!   edge never touches a glyph.
//!
//! The kit's own controls draw this themselves from the theme, with a 3 px
//! ring (its width is private to the kit). Use these for
//! the app's components that track their own focus. Keeping the focused
//! element in view is [`super::reveal`]'s, except in a virtual list, whose
//! rows use [`reveal_when`].
//!
//! A component that tracks its own focus takes its handle from
//! [`keyed_focus`], which also lets a page give focus back to it: Back
//! returns to the control that opened the page ([`remember_focus`],
//! [`return_focus`]), though the screen under the page was rebuilt.
//!
//! ```ignore
//! let focus = keyed_focus(id.clone(), window, cx);
//! div()
//!     .track_focus(&focus)
//!     .rounded(cx.theme().radius)
//!     .when(focus_visible(&focus, window), |this| this.focus_ring(cx.theme().radius, cx))
//! ```

use std::panic::Location;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, App, AvailableSpace, Bounds, BoxShadow, ContentMask, Element, ElementId,
    FocusHandle, Global, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId,
    ParentElement, Pixels, ScrollHandle, StatefulInteractiveElement, Style, Styled, Window, canvas,
    div, point, px, relative, size as gsize,
};

use crate::design::{colors, size};

/// Whether `handle` shows its focus: it is focused and the last input came
/// from the keyboard, so a click doesn't leave a ring behind.
pub fn focus_visible(handle: &FocusHandle, window: &Window) -> bool {
    handle.is_focused(window) && window.last_input_was_keyboard()
}

/// A focus handle kept for the element `id` across frames, for a component
/// that is rebuilt every render (`RenderOnce`).
///
/// The handle lives as long as the element is drawn. A screen that comes
/// back after a page (Back) is drawn afresh with a new handle; the element
/// takes focus again if it had it when the page opened ([`return_focus`]).
pub fn keyed_focus(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> FocusHandle {
    let id = id.into();
    let handle = window
        .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone();
    window.with_global_id(id, |global, window| {
        let memory = cx.default_global::<FocusMemory>();
        if handle.is_focused(window) {
            memory.focused = Some((global.clone(), handle.clone()));
        }
        if memory.returning.as_ref() == Some(global) {
            memory.returning = None;
            let handle = handle.clone();
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
    });
    handle
}

/// Which keyed element has focus, and which should take it back.
#[derive(Default)]
struct FocusMemory {
    /// The last keyed element drawn with focus, and that focus.
    focused: Option<(GlobalElementId, FocusHandle)>,
    /// The element to focus the next time it is drawn.
    returning: Option<GlobalElementId>,
}

impl Global for FocusMemory {}

/// Where focus was, to give it back later with [`return_focus`].
#[derive(Clone)]
pub enum FocusReturn {
    /// A component with a [`keyed_focus`], found again by its place in the
    /// tree when it is next drawn.
    Keyed(GlobalElementId),
    /// Any other focused element (a list), whose handle outlives the frame.
    Handle(FocusHandle),
}

/// Where focus is now, to return to it later. `None` when nothing is
/// focused.
pub fn remember_focus(window: &Window, cx: &App) -> Option<FocusReturn> {
    let focused = window.focused(cx)?;
    let keyed = cx
        .try_global::<FocusMemory>()
        .and_then(|memory| memory.focused.clone())
        .filter(|(_, handle)| *handle == focused);
    Some(match keyed {
        Some((global, _)) => FocusReturn::Keyed(global),
        None => FocusReturn::Handle(focused),
    })
}

/// Gives focus back to where [`remember_focus`] found it. Focus the
/// fallback first: if the element isn't drawn again, focus stays there.
pub fn return_focus(to: FocusReturn, window: &mut Window, cx: &mut App) {
    match to {
        FocusReturn::Keyed(global) => {
            cx.default_global::<FocusMemory>().returning = Some(global);
            // Only for the frame that draws the screen again.
            window.on_next_frame(|window, _| {
                window.on_next_frame(|_, cx| cx.default_global::<FocusMemory>().returning = None);
            });
        }
        // A handle no longer drawn loses focus on the next frame, and the
        // window's focus-lost handling takes over.
        FocusReturn::Handle(handle) => window.focus(&handle, cx),
    }
}

/// Marks a drawn ring for the UI tests (`cx.debug_bounds("focus-ring")`).
fn marked(ring: gpui_kit::Div) -> gpui_kit::Div {
    #[cfg(test)]
    let ring = gpui_kit::InteractiveElement::debug_selector(ring, || "focus-ring".into());
    ring
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
    /// A control's focus: the 2 px ring outside and the 1 px accent edge.
    /// `radius` is the element's own corner radius, so the ring follows it.
    /// The element must be positioned (`relative`, the default for the
    /// app's components), and nothing around it may clip.
    ///
    /// Replaces the element's box shadow.
    fn focus_ring(self, radius: Pixels, cx: &App) -> Self {
        let ring = size::FOCUS_RING;
        // A band drawn only outside the element, so a translucent fill
        // (a hovered ghost control) doesn't show a shadow through it.
        self.shadow(vec![edge(cx)]).child(marked(
            div()
                .absolute()
                .top(-ring)
                .left(-ring)
                .right(-ring)
                .bottom(-ring)
                .rounded(radius + ring)
                .border(ring)
                .border_color(colors(cx).ring),
        ))
    }

    /// The focus of bare text (a text button, a byline link): the 1 px
    /// accent edge just outside the text, standing in for the border it
    /// doesn't have, and the 2 px ring outside that. The same conditions as
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
        self.child(band(edge, edge, colors(cx).accent))
            .child(marked(band(edge + ring, ring, colors(cx).ring)))
    }

    /// A row's focus: the 1 px accent edge inside the row and the 2 px ring
    /// outside it, drawn over the list that clips the row. Replaces the
    /// element's box shadow. The ring follows the row's own box, measured
    /// as it is laid out, so a row that puts its children in an inner box
    /// (the kit's `ListItem`, beside its suffix) still gets it whole.
    fn focus_row(self, radius: Pixels, cx: &App) -> Self
    where
        Self: StatefulInteractiveElement,
    {
        let row = ScrollHandle::new();
        self.shadow(vec![edge(cx)])
            .track_scroll(&row)
            .child(RowRing {
                row,
                radius,
                color: colors(cx).ring,
            })
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

/// Keeps a region that holds several controls, such as a row in a virtual
/// list, in view inside the scroller `scroll` (`ui::reveal` can't see into a
/// virtual list): when `active` becomes true (focus moved into the
/// region), the scroller moves just enough to show the parent with `margin`
/// to spare above and below. In a virtual list a margin keeps a sliver of the
/// neighbouring rows on screen, so they are drawn and Tab and Shift+Tab can
/// reach them.
pub fn reveal_when(
    id: impl Into<ElementId>,
    active: bool,
    scroll: &ScrollHandle,
    margin: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id = ElementId::NamedChild(std::sync::Arc::new(id.into()), "reveal".into());
    let revealed = window.use_keyed_state(id, cx, |_, _| false);
    if !active {
        revealed.update(cx, |r, _| *r = false);
    }
    let scroll = scroll.clone();
    canvas(
        move |bounds, window, cx| {
            if !active || *revealed.read(cx) {
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
        |_, (), _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

/// A row's ring, drawn after the frame over whatever is around the row, so
/// the list doesn't clip it; an edge where the list cuts the row off still
/// clips it there.
struct RowRing {
    /// Tracks the row, which records its bounds before its children are
    /// prepainted.
    row: ScrollHandle,
    radius: Pixels,
    color: Hsla,
}

impl IntoElement for RowRing {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RowRing {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // Over the whole row.
        let mut style = Style {
            position: gpui_kit::Position::Absolute,
            size: gsize(relative(1.).into(), relative(1.).into()),
            ..Style::default()
        };
        style.inset.top = px(0.).into();
        style.inset.left = px(0.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let ring = size::FOCUS_RING;
        let row = self.row.bounds();
        let bounds = if row.size.width > px(0.) { row } else { bounds };
        let mask = ring_mask(bounds, window.content_mask().bounds, ring);
        let mut band = div()
            .w(bounds.size.width + ring * 2.)
            .h(bounds.size.height + ring * 2.)
            .rounded(self.radius + ring)
            .border(ring)
            .border_color(self.color)
            .into_any_element();
        // A deferred element is drawn as laid out: on its own, at its size.
        band.layout_as_root(AvailableSpace::min_size(), window, cx);
        window.defer_draw(
            band,
            bounds.origin - point(ring, ring),
            0,
            Some(ContentMask { bounds: mask }),
        );
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (): &mut (),
        (): &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
}

/// Where a row's ring may draw: `ring` beyond each edge of the `row` that
/// the current `clip` doesn't cut, and the clip's own edge where it does.
fn ring_mask(row: Bounds<Pixels>, clip: Bounds<Pixels>, ring: Pixels) -> Bounds<Pixels> {
    let slack = px(0.5);
    let left = if clip.left() <= row.left() + slack {
        row.left() - ring
    } else {
        clip.left()
    };
    let top = if clip.top() <= row.top() + slack {
        row.top() - ring
    } else {
        clip.top()
    };
    let right = if clip.right() >= row.right() - slack {
        row.right() + ring
    } else {
        clip.right()
    };
    let bottom = if clip.bottom() >= row.bottom() - slack {
        row.bottom() + ring
    } else {
        clip.bottom()
    };
    Bounds::from_corners(point(left, top), point(right, bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), gsize(px(w), px(h)))
    }

    #[test]
    fn a_rows_ring_is_clipped_only_where_the_list_cuts_the_row() {
        let ring = px(3.);
        // The row's own clip, as a list draws it: the whole ring shows.
        assert_eq!(
            ring_mask(b(10., 10., 100., 40.), b(10., 10., 100., 42.), ring),
            b(7., 7., 106., 46.)
        );
        // Scrolled half under the list's top: cut there.
        assert_eq!(
            ring_mask(b(10., -10., 100., 40.), b(10., 0., 100., 32.), ring),
            b(7., 0., 106., 33.)
        );
    }

    #[gpui_kit::test]
    fn a_focused_list_row_draws_its_ring(cx: &mut gpui_kit::TestAppContext) {
        use crate::ui::ListRow as _;
        use crate::ui::test_support::{host, redraw};
        use gpui_kit::ParentElement as _;
        use gpui_kit::component::list::ListItem;

        // The ring is drawn after the row, over the list's clip: it must be
        // laid out on its own, or drawing it panics.
        let cx = host(cx, |_, cx| {
            div()
                .overflow_hidden()
                .child(ListItem::new("row").list_row(true, cx).child("Dune"))
                .into_any_element()
        });
        redraw(cx);
    }

    /// The ring shows only after keyboard input: focus that arrives
    /// without a key (handed over after a click) shows none, a key shows
    /// it, and moving the pointer hides it again.
    #[gpui_kit::test]
    fn the_ring_shows_only_after_keyboard_input(cx: &mut gpui_kit::TestAppContext) {
        use crate::ui::NavRow;
        use crate::ui::test_support::{host, press, redraw, tab};
        use gpui_kit::component::IconName;
        use gpui_kit::{IntoElement as _, Modifiers, VisualTestContext};

        let ring = |cx: &mut VisualTestContext| cx.debug_bounds("focus-ring").is_some();
        let cx = host(cx, |_, _| {
            NavRow::new("nav-home", IconName::LayoutDashboard, "Home").into_any_element()
        });
        tab(cx);
        assert!(crate::ui::test_support::has_focus(cx));
        assert!(!ring(cx), "focus handed over without a key");

        press(cx, "a");
        redraw(cx);
        assert!(ring(cx), "after a key");

        cx.simulate_mouse_move(point(px(400.), px(400.)), None, Modifiers::none());
        redraw(cx);
        assert!(!ring(cx), "after the pointer moved");
    }

    /// Back gives focus to the control that opened the page, though the
    /// page's cover dropped the control's state while it was hidden.
    #[gpui_kit::test]
    fn back_returns_focus_to_what_opened_the_page(cx: &mut gpui_kit::TestAppContext) {
        use crate::ui::NavRow;
        use crate::ui::test_support::{has_focus, host, redraw, tab};
        use gpui_kit::component::IconName;
        use gpui_kit::component::button::Button;
        use gpui_kit::{IntoElement as _, ParentElement as _};
        use std::cell::Cell;
        use std::rc::Rc;

        let page = Rc::new(Cell::new(false));
        let cx = host(cx, {
            let page = page.clone();
            move |_, _| {
                if page.get() {
                    div().child(Button::new("on-the-page").label("On the page"))
                } else {
                    div().child(NavRow::new("opener", IconName::BookOpen, "Opener"))
                }
                .into_any_element()
            }
        });
        let back = |cx: &mut gpui_kit::VisualTestContext, returning: bool| {
            tab(cx);
            let from = cx.update(|window, cx| remember_focus(window, cx));
            assert!(
                matches!(from, Some(FocusReturn::Keyed(_))),
                "a keyed control"
            );
            page.set(true);
            redraw(cx);
            tab(cx);
            page.set(false);
            if returning {
                let from = from.expect("focus was remembered");
                cx.update(|window, cx| return_focus(from, window, cx));
            }
            redraw(cx);
            cx.run_until_parked();
            redraw(cx);
            has_focus(cx)
        };
        assert!(!back(cx, false), "without it, focus is lost with the page");
        assert!(back(cx, true), "with it, the opener has focus again");
    }
}
