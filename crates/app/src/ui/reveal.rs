//! Keeps keyboard focus in view (`hardcover-design`, States → Focus-visible:
//! "never clipped by a scrolling parent"). GPUI doesn't scroll a scrolling
//! `div` to the element that takes focus, so Tab could move focus below the
//! fold with nothing on screen changing.
//!
//! A scrolling region opts in with one call on its scroll container:
//! - [`RevealScroll::scroll_y_revealing_focus`] instead of the kit's
//!   `.overflow_y_scrollbar()`;
//! - [`RevealScroll::reveals_focus`] on a container that already scrolls with
//!   its own [`ScrollHandle`] (the sidebar).
//!
//! When focus moves inside the region by keyboard, the region scrolls just
//! enough to show the focused element with its ring, once: the reader's own
//! scrolling afterwards is left alone, and content loading under a focus
//! that hasn't moved never scrolls the region.
//!
//! **What is shown.** GPUI tells a region whether focus is inside it, not
//! where. The bounds come from [`FocusProbe`]s: the shared components wrap
//! themselves in one, so the region shows the innermost probe around the
//! focused element.
//! - The components that track their own focus ([`super::TextButton`],
//!   [`super::ExternalLink`], [`super::BylineLink`], [`super::NavRow`],
//!   [`super::GoalRow`], [`super::AccountRow`]) are shown exactly.
//! - A kit control inside a row-scale part ([`super::Sequence`]'s rows,
//!   [`super::PropertyList`]'s rows, [`super::Lane`], [`super::Block`]'s and
//!   [`super::SectionHeader`]'s actions, [`super::panel`]) is shown with that
//!   part's box.
//! - A kit control outside every probe isn't moved to. Wrap it, or the part
//!   it sits in, in [`probe`].
//! - A box taller than the region is aligned by its top.
//!
//! ```ignore
//! // A destination's root: was `root.overflow_y_scrollbar()`.
//! root.scroll_y_revealing_focus().child(page)
//! // A scroller with its own handle.
//! div().id("sidebar").overflow_y_scroll().track_scroll(&scroll).reveals_focus(&scroll)
//! // A kit control outside the shared parts.
//! probe(Button::new("more").label("Show more reviews"))
//! ```

use std::cell::RefCell;
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, FocusHandle, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ParentElement, Pixels, RenderOnce, ScrollHandle,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, point, px,
};

use crate::design::size;

thread_local! {
    /// The regions being prepainted, innermost last, each with the bounds
    /// its content reported for the focused element.
    static REGIONS: RefCell<Vec<Option<Bounds<Pixels>>>> = const { RefCell::new(Vec::new()) };
}

/// Reports `bounds` as the focused element's to the innermost region being
/// prepainted, unless a probe nearer the focus already did.
fn report(bounds: Bounds<Pixels>) {
    REGIONS.with(|regions| {
        if let Some(slot @ None) = regions.borrow_mut().last_mut() {
            *slot = Some(bounds);
        }
    });
}

// ----- the region -----

/// The opt-in on a scroll container (an element with an id). See the module
/// doc.
pub trait RevealScroll: StatefulInteractiveElement + ScrollableElement + Sized + 'static {
    /// A vertical scroller with the kit's scrollbar, in place of
    /// `.overflow_y_scrollbar()`, that keeps the focused element in view.
    #[track_caller]
    fn scroll_y_revealing_focus(self) -> RevealingScroller<Self> {
        let id = Element::id(&self).unwrap_or_else(|| ElementId::CodeLocation(*Location::caller()));
        RevealingScroller { id, element: self }
    }

    /// This container, which scrolls with `scroll` (`.track_scroll(&scroll)`),
    /// keeps the focused element in view.
    #[track_caller]
    fn reveals_focus(self, scroll: &ScrollHandle) -> RevealFocus {
        let id = Element::id(&self).unwrap_or_else(|| ElementId::CodeLocation(*Location::caller()));
        RevealFocus {
            id,
            scroll: scroll.clone(),
            child: self.into_any_element(),
        }
    }
}

impl<E: StatefulInteractiveElement + ScrollableElement + Sized + 'static> RevealScroll for E {}

/// See [`RevealScroll::scroll_y_revealing_focus`].
#[derive(IntoElement)]
pub struct RevealingScroller<E: RevealScroll> {
    id: ElementId,
    element: E,
}

impl<E: RevealScroll> RenderOnce for RevealingScroller<E> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::NamedChild(Arc::new(self.id), "reveal-scroll".into());
        let scroll = window
            .use_keyed_state(id.clone(), cx, |_, _| ScrollHandle::default())
            .read(cx)
            .clone();
        RevealFocus {
            id,
            scroll: scroll.clone(),
            child: self
                .element
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .vertical_scrollbar(&scroll)
                .into_any_element(),
        }
    }
}

// A drop-in for the kit's scroller: children and style go to the element.
impl<E: RevealScroll> ParentElement for RevealingScroller<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}

impl<E: RevealScroll> Styled for RevealingScroller<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

/// See [`RevealScroll::reveals_focus`].
#[derive(IntoElement)]
pub struct RevealFocus {
    id: ElementId,
    scroll: ScrollHandle,
    child: AnyElement,
}

impl RenderOnce for RevealFocus {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = ElementId::NamedChild(Arc::new(self.id), "reveal-focus".into());
        let seen = window
            .use_keyed_state(key, cx, |_, _| Rc::new(RefCell::new(None)))
            .read(cx)
            .clone();
        Region {
            scroll: self.scroll,
            seen,
            child: Some(self.child),
        }
    }
}

/// The region's element: it lays out as its child, and after the child's
/// prepaint scrolls to what its probes reported.
struct Region {
    scroll: ScrollHandle,
    /// The focus this region last acted on, so it acts once per move.
    seen: Rc<RefCell<Option<FocusHandle>>>,
    child: Option<AnyElement>,
}

impl IntoElement for Region {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Region {
    type RequestLayoutState = AnyElement;
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
    ) -> (LayoutId, AnyElement) {
        let mut child = self.child.take().expect("a region is laid out once");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        REGIONS.with(|regions| regions.borrow_mut().push(None));
        let inside = child.prepaint(window, cx);
        let reported = REGIONS.with(|regions| regions.borrow_mut().pop().flatten());

        let focused = window.focused(cx);
        if inside.is_some() {
            // An enclosing region shows this one's part of the focus.
            report(reported.map_or(bounds, |r| r.intersect(&self.scroll.bounds())));
        }
        if *self.seen.borrow() == focused {
            return;
        }
        *self.seen.borrow_mut() = focused;
        let (Some(_), Some(target)) = (inside, reported) else {
            return;
        };
        if !window.last_input_was_keyboard() {
            return;
        }
        if let Some(offset) = revealing_offset(
            target,
            self.scroll.bounds(),
            self.scroll.offset(),
            self.scroll.max_offset(),
        ) {
            self.scroll.set_offset(offset);
            window.refresh();
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

/// The scroll offset that shows `target`, ring included, inside `viewport`,
/// moving as little as possible; `None` when it is already in view.
///
/// A target taller than the viewport is a part with no probe nearer the
/// focus (a whole column), so where in it the focus is isn't known: while
/// it shows at all nothing moves, and from outside only its near edge comes
/// in (its top from below, its bottom from above).
fn revealing_offset(
    target: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    offset: gpui_kit::Point<Pixels>,
    max: gpui_kit::Point<Pixels>,
) -> Option<gpui_kit::Point<Pixels>> {
    let margin = size::FOCUS_RING + px(1.);
    let top = target.top() - margin;
    let bottom = target.bottom() + margin;
    let shift = if bottom - top > viewport.size.height {
        if top >= viewport.bottom() {
            viewport.top() - top
        } else if bottom <= viewport.top() {
            viewport.bottom() - bottom
        } else {
            return None;
        }
    } else if top < viewport.top() {
        viewport.top() - top
    } else if bottom > viewport.bottom() {
        viewport.bottom() - bottom
    } else {
        return None;
    };
    // Offsets run from 0 (the top) down to -max.
    let y = (offset.y + shift).clamp(-max.y, px(0.));
    (y != offset.y).then(|| point(offset.x, y))
}

// ----- the probe -----

/// Wraps `child`, a part that may hold focus, so a region shows this part's
/// box when focus is inside it. Lays out as `child`; adds nothing.
pub fn probe(child: impl IntoElement) -> FocusProbe {
    FocusProbe {
        child: Some(child.into_any_element()),
    }
}

/// See [`probe`].
pub struct FocusProbe {
    child: Option<AnyElement>,
}

impl IntoElement for FocusProbe {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FocusProbe {
    type RequestLayoutState = AnyElement;
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
    ) -> (LayoutId, AnyElement) {
        let mut child = self.child.take().expect("a probe is laid out once");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        if child.prepaint(window, cx).is_some() {
            report(bounds);
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::button::Button;
    use gpui_kit::{
        InteractiveElement as _, TestAppContext, VisualTestContext, div, size as gsize,
    };

    use super::*;
    use crate::ui::Sequence;
    use crate::ui::test_support::{host, press, redraw, tab};

    fn b(y: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(y)), gsize(px(10.), px(h)))
    }

    #[test]
    fn the_offset_moves_just_enough() {
        let viewport = b(0., 100.);
        let max = point(px(0.), px(500.));
        let at = |y: f32| point(px(0.), px(y));
        // Already in view.
        assert_eq!(revealing_offset(b(20., 20.), viewport, at(0.), max), None);
        // Below: its bottom, ring and all, on the viewport's bottom.
        assert_eq!(
            revealing_offset(b(150., 20.), viewport, at(0.), max),
            Some(at(-74.))
        );
        // Above: its top on the viewport's top.
        assert_eq!(
            revealing_offset(b(-50., 20.), viewport, at(-200.), max),
            Some(at(-146.))
        );
        // Taller than the viewport: nothing moves while any of it shows...
        assert_eq!(
            revealing_offset(b(-200., 400.), viewport, at(-300.), max),
            None
        );
        assert_eq!(revealing_offset(b(50., 300.), viewport, at(0.), max), None);
        // ...and from outside, its near edge comes in.
        assert_eq!(
            revealing_offset(b(150., 300.), viewport, at(0.), max),
            Some(at(-146.))
        );
        assert_eq!(
            revealing_offset(b(-500., 300.), viewport, at(-600.), max),
            Some(at(-304.))
        );
        // Never past the ends.
        assert_eq!(
            revealing_offset(b(900., 20.), viewport, at(0.), max),
            Some(at(-500.))
        );
    }

    /// A region 120 high, with a kit button in a sequence's row 400 down.
    fn region() -> impl IntoElement {
        div()
            .debug_selector(|| "region".into())
            .h(px(120.))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("region")
                    .size_full()
                    .flex()
                    .flex_col()
                    .scroll_y_revealing_focus()
                    .child(div().flex_none().h(px(400.)))
                    .child(
                        Sequence::new().child(
                            div()
                                .debug_selector(|| "row".into())
                                .child(Button::new("below").label("Below the fold")),
                        ),
                    ),
            )
    }

    /// The bounds of the element marked `selector`, as last drawn.
    fn bounds_of(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
        cx.debug_bounds(selector).expect("the element was drawn")
    }

    #[gpui_kit::test]
    fn tabbing_to_a_kit_button_below_the_fold_scrolls_to_it(cx: &mut TestAppContext) {
        let cx = host(cx, |_, _| region().into_any_element());
        let region = bounds_of(cx, "region");
        assert!(bounds_of(cx, "row").top() > region.bottom());

        // Any key: the focus that follows came from the keyboard.
        press(cx, "f13");
        tab(cx);
        redraw(cx);
        let row = bounds_of(cx, "row");
        assert!(
            row.top() >= region.top() && row.bottom() <= region.bottom(),
            "the row {row:?} is inside the region {region:?}"
        );
    }

    #[gpui_kit::test]
    fn a_pointer_focus_doesnt_scroll(cx: &mut TestAppContext) {
        let cx = host(cx, |_, _| region().into_any_element());
        let before = bounds_of(cx, "row");
        tab(cx);
        redraw(cx);
        assert_eq!(bounds_of(cx, "row"), before);
    }
}
