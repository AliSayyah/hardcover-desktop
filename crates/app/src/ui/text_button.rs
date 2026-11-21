//! Text button (`hardcover-design`, Buttons and links): ink text that gains
//! its underline on hover, with the default cursor. In-app navigation inside
//! content: a book title, an author, a series. A cover is never the only way
//! to a book; its title is a text button too.
//!
//! It is a real button: Tab reaches it, Enter and Space press it, and it
//! shows the focus ring. Never use it for something that opens the browser
//! (that is a link, or a [`super::BylineLink`]).
//!
//! ```ignore
//! TextButton::book(("home-title", id), &entry.book).title_18()
//! TextButton::new("series-link", series.name.clone()).opens(Page::Series(series.id))
//! ```
//!
//! Its type is set like any element's (`.title_18()`, `.body_strong()`); it
//! is `body-strong` until told otherwise. The label is one truncated line,
//! or wraps with [`TextButton::wrap`], at most a few lines with
//! [`TextButton::line_clamp`] (similar books' titles).

use std::rc::Rc;

use gpui_kit::base::Button as BaseButton;
use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _,
};

use super::focus::{FocusTreatment as _, control_radius, focus_visible, keyed_focus};
use super::reveal::probe;
use hardcover_core::BookSummary;

use crate::bidi::{WrappingText, is_rtl};
use crate::catalog::navigation::{self, Page};
use crate::design::{TypeRoles as _, colors};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A quiet in-app navigation button. See the module doc.
#[derive(IntoElement)]
pub struct TextButton {
    id: ElementId,
    label: SharedString,
    style: StyleRefinement,
    wrap: bool,
    line_clamp: Option<usize>,
    width: Option<Pixels>,
    on_click: Option<ClickHandler>,
}

impl TextButton {
    /// A text button reading `label`. `id` must be unique among its siblings.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            style: StyleRefinement::default(),
            wrap: false,
            line_clamp: None,
            width: None,
            on_click: None,
        }
    }

    /// A book's title that opens the book's page.
    pub fn book(id: impl Into<ElementId>, book: &BookSummary) -> Self {
        Self::new(id, book.title.clone()).opens(Page::Book(book.clone()))
    }

    /// Runs `handler` on a click, Enter or Space.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Opens `page` (a book, a series, an author) when pressed.
    pub fn opens(self, page: Page) -> Self {
        self.on_click(move |_, _, cx| navigation::open(page.clone(), cx))
    }

    /// Lets the label wrap onto more lines instead of truncating it (Up next,
    /// a long title in a header). Right-to-left labels wrap through
    /// `WrappingText`.
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self
    }

    /// Wraps the label onto at most `lines` lines (similar books' titles).
    /// The clamp is the label's own, so the focus ring outside it is never
    /// clipped.
    pub fn line_clamp(mut self, lines: usize) -> Self {
        self.wrap = true;
        self.line_clamp = Some(lines);
        self
    }
}

impl TextButton {
    /// Breaks the wrapped label's lines itself for a box this wide (a cover
    /// shelf's titles, as wide as their covers): between words, never with a
    /// space or a comma at the start of a line. See `WrappingText::width`.
    pub fn wrap_width(mut self, width: Pixels) -> Self {
        self.wrap = true;
        self.width = Some(width);
        self
    }
}

impl Styled for TextButton {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for TextButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = keyed_focus(self.id.clone(), window, cx);
        let focused = focus_visible(&focus, window);
        let radius = control_radius(cx);
        // GPUI on Linux draws a right-to-left line's underline beside it, so
        // such a label goes without (`hardcover-design`, Byline link).
        let underline = !is_rtl(&self.label);
        let wrap = self.wrap;
        let line_clamp = self.line_clamp;
        let width = self.width;
        let label = self.label.clone();
        let on_click = self.on_click;

        // A row, so the button keeps its content's width instead of
        // stretching; it may still shrink, and its label truncates.
        probe(
            div().flex().min_w_0().max_w_full().child(
                BaseButton::new(self.id)
                    .track_focus(&focus)
                    .accessibility_label(label.clone())
                    .relative()
                    .min_w_0()
                    .max_w_full()
                    .justify_start()
                    .text_left()
                    .rounded(radius)
                    .text_color(colors(cx).fg)
                    .body_strong()
                    .when(underline, |this| this.hover(gpui_kit::Styled::underline))
                    .when_some(on_click, |this, handler| {
                        this.on_click(move |event, window, cx| handler(event, window, cx))
                    })
                    .refine_style(&self.style)
                    .when(focused, |this| this.focus_ring_around(radius, cx))
                    .child(if wrap {
                        div()
                            .min_w_0()
                            .child(
                                WrappingText::new(label)
                                    .when_some(line_clamp, WrappingText::line_clamp)
                                    .when_some(width, WrappingText::width),
                            )
                            .into_any_element()
                    } else if !underline {
                        // GPUI lays a line out as if it read left to right,
                        // so a Persian title's brackets and punctuation
                        // land on the wrong side unless the bidirectional
                        // layout places them: one line, cut at its end.
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .child(WrappingText::new(label).line_clamp(1))
                            .into_any_element()
                    } else {
                        div().min_w_0().truncate().child(label).into_any_element()
                    }),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{counter, host, press, tab};
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn tab_reaches_it_and_enter_and_space_press_it(cx: &mut TestAppContext) {
        let (presses, on_press) = counter();
        let cx = host(cx, move |_, _| {
            TextButton::new("title", "Project Hail Mary")
                .on_click(on_press.clone())
                .into_any_element()
        });
        // Nothing is focused yet, so a key reaches no button.
        press(cx, "enter");
        assert_eq!(presses.get(), 0);

        tab(cx);
        press(cx, "enter");
        press(cx, "space");
        assert_eq!(presses.get(), 2);
    }
}
