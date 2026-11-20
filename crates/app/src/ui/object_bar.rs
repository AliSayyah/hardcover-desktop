//! Object bar (`hardcover-design`, Shelf → Book pane and Full book page): the
//! 56 high bar above an object, holding its tools on the trailing edge and,
//! on a page over a destination, Back on the leading edge.
//!
//! - **Book pane:** no hairline, no Back. Inset 16.
//! - **Page over a destination** (a book, series, author, Coming soon): a
//!   hairline under it and Back, a ghost button with arrow-left and the name
//!   of where it goes ("Discover", "Currently reading"). Inset 32 on the
//!   leading edge (Back's own padding hangs into it, so its arrow sits on the
//!   page's 32 spine) and 24 on the trailing edge.
//! - The object's tools sit on the trailing edge, 4 apart: the book's "Lists"
//!   menu button and its "more" icon button. They live here, never over the
//!   title.
//!
//! The view that shows the object supplies its tools through
//! [`ObjectTools`]; whoever draws the bar asks it with [`tools_of`]:
//!
//! ```ignore
//! ObjectBar::new()
//!     .hairline(true)
//!     .back("Discover", cx.listener(|this, _, window, cx| this.go_back(window, cx)))
//!     .tools(tools_of(&self.book_page, window, cx))
//! ```

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{IconName, h_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Entity, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, rems,
};

use crate::GoBack;
use crate::design::{colors, size};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A view whose object has tools for the object bar, such as the book page.
pub trait ObjectTools {
    /// The tools, in order, for the trailing edge. Empty for none.
    fn object_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement>
    where
        Self: Sized;
}

/// The tools `view` hands to the bar.
pub fn tools_of<V: ObjectTools + 'static>(
    view: &Entity<V>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    view.update(cx, |view, cx| view.object_tools(window, cx))
}

/// The bar above an object. See the module doc.
#[derive(IntoElement, Default)]
pub struct ObjectBar {
    back: Option<(SharedString, ClickHandler)>,
    tools: Vec<AnyElement>,
    hairline: bool,
}

impl ObjectBar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Back, reading `label` (where it goes), running `on_click`. Its tooltip
    /// names the `GoBack` shortcut.
    pub fn back(
        mut self,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.back = Some((label.into(), Rc::new(on_click)));
        self
    }

    /// The object's tools, for the trailing edge.
    pub fn tools(mut self, tools: impl IntoIterator<Item = AnyElement>) -> Self {
        self.tools.extend(tools);
        self
    }

    /// One tool, after any already given.
    pub fn tool(mut self, tool: impl IntoElement) -> Self {
        self.tools.push(tool.into_any_element());
        self
    }

    /// The hairline under the bar, and the page's insets: for a page over a
    /// destination. Without it the bar has the pane's insets.
    pub fn hairline(mut self, hairline: bool) -> Self {
        self.hairline = hairline;
        self
    }
}

impl RenderOnce for ObjectBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .flex_none()
            .w_full()
            .h(size::OBJECT_BAR)
            .gap_1()
            .map(|this| {
                if self.hairline {
                    this.pl_8()
                        .pr_6()
                        .border_b_1()
                        .border_color(colors(cx).border)
                } else {
                    this.px_4()
                }
            })
            .when_some(self.back, |this, (label, on_click)| {
                this.child(
                    Button::new("object-bar-back")
                        .ghost()
                        .icon(IconName::ArrowLeft)
                        .label(label)
                        // Its padding hangs into the inset, so the arrow sits
                        // on the content's spine.
                        .ml(rems(-0.75))
                        .max_w(rems(20.))
                        .tooltip_with_action("Back", &GoBack, Some("Workspace"))
                        .on_click(move |event, window, cx| on_click(event, window, cx)),
                )
            })
            .child(div().flex_1())
            .children(self.tools)
    }
}

/// Where Back goes, for its label: the page under the current one, or the
/// destination under every page. A page whose name isn't known here (a
/// series, an author) gives plain "Back".
pub fn back_label(previous: Option<BackTo<'_>>, destination: &str) -> SharedString {
    match previous {
        None => SharedString::from(destination.to_owned()),
        Some(BackTo::Named(name)) => SharedString::from(name.to_owned()),
        Some(BackTo::Unnamed) => SharedString::new_static("Back"),
    }
}

/// The page Back returns to, when it isn't the destination.
#[derive(Clone, Copy, Debug)]
pub enum BackTo<'a> {
    Named(&'a str),
    Unnamed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{counter, has_focus, host, press, tab};
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn back_is_a_button_that_runs_its_handler(cx: &mut TestAppContext) {
        let (backs, on_back) = counter();
        let cx = host(cx, move |_, _| {
            ObjectBar::new()
                .hairline(true)
                .back("Home", on_back.clone())
                .into_any_element()
        });
        tab(cx);
        press(cx, "enter");
        assert_eq!(backs.get(), 1);
    }

    #[gpui_kit::test]
    fn a_bar_without_back_or_tools_takes_no_focus(cx: &mut TestAppContext) {
        let cx = host(cx, |_, _| ObjectBar::new().into_any_element());
        tab(cx);
        assert!(!has_focus(cx));
    }

    #[test]
    fn back_names_where_it_goes() {
        assert_eq!(back_label(None, "Discover").as_ref(), "Discover");
        assert_eq!(
            back_label(Some(BackTo::Named("Coming soon")), "Home").as_ref(),
            "Coming soon"
        );
        assert_eq!(
            back_label(Some(BackTo::Unnamed), "Currently reading").as_ref(),
            "Back"
        );
    }
}
