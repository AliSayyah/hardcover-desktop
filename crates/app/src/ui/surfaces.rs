//! Panels, rails and blocks (`hardcover-design`, Panels, rails and hairlines;
//! Shelf → Book pane): how a pane or page is divided. Structure comes from
//! spacing, hairlines and type; boxes are rare.
//!
//! - [`panel`]: the one inset panel a pane may have (progress, Not started).
//! - [`Rail`] and [`RailSection`]: the 272 column beside a page's main column
//!   (Home, the full book page), its sections separated by hairlines.
//! - [`Block`] and [`blocks`]: a pane's blocks ("About this book", "Details"),
//!   each a heading with an optional small action, 24 apart.
//!
//! Never put a card inside a card, and never box a rail's sections.

use gpui_kit::component::h_flex;
use gpui_kit::{
    AnyElement, App, Div, IntoElement, ParentElement, RenderOnce, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};

use super::focus::box_radius;
use super::reveal::probe;
use crate::design::{TypeRoles as _, colors, size};

/// A panel: `surface` fill, `r-box` corners, 16 inset, no border or shadow.
/// A pane has at most one: the progress block for a book you have started,
/// or the Not started block for a book on Want to read.
///
/// ```ignore
/// panel(cx).flex().flex_col().gap_3().child(..)
/// ```
pub fn panel(cx: &App) -> Div {
    div().bg(colors(cx).surface).rounded(box_radius(cx)).p_4()
}

/// The rail: a column 272 wide with a hairline on its leading edge and a
/// 24 inset, holding [`RailSection`]s separated by hairlines (24 above and
/// below). It starts at the top, beside the page's header.
///
/// ```ignore
/// Rail::new()
///     .section(RailSection::new("On Hardcover").child(..))
///     .section(RailSection::new("Reads").action(add_read).child(..))
/// ```
#[derive(IntoElement, Default)]
pub struct Rail {
    sections: Vec<AnyElement>,
}

impl Rail {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next section, under a hairline.
    pub fn section(mut self, section: impl IntoElement) -> Self {
        self.sections.push(section.into_any_element());
        self
    }

    /// Sections that apply, in order; a missing one leaves no gap.
    pub fn sections(mut self, sections: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.sections
            .extend(sections.into_iter().map(IntoElement::into_any_element));
        self
    }
}

impl RenderOnce for Rail {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let border = colors(cx).border;
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(size::RAIL)
            .pl_6()
            .border_l_1()
            .border_color(border)
            .children(
                self.sections
                    .into_iter()
                    .enumerate()
                    .map(move |(ix, section)| {
                        probe(
                            div()
                                .when(ix > 0, |this| {
                                    this.mt_6().pt_6().border_t_1().border_color(border)
                                })
                                .child(section),
                        )
                    }),
            )
    }
}

/// A section's title row: the title, with an optional small action on the
/// trailing edge. With an action it is a small control high, so the
/// content under it starts at the same place either way.
fn title_row(title: Div, action: Option<AnyElement>) -> Div {
    h_flex()
        .w_full()
        .min_h(size::CONTROL_SMALL)
        .gap_2()
        .child(title.flex_1().min_w_0())
        .children(action.map(|action| probe(div().flex_none().child(action))))
}

/// One section of a [`Rail`]: a `label` title, an optional small trailing
/// action ("Add a past read…", a small ghost button), and its content 8
/// below.
#[derive(IntoElement)]
pub struct RailSection {
    title: SharedString,
    action: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl RailSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            action: None,
            children: Vec::new(),
        }
    }

    /// A small action on the title's trailing edge.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl ParentElement for RailSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for RailSection {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(title_row(div().label(cx).child(self.title), self.action))
            .children(self.children.into_iter().map(probe))
    }
}

/// A block of a pane or page: a `heading` ("About this book", "Details"),
/// an optional small action on its trailing edge ("Edit"), and its content 8
/// below. Stack blocks with [`blocks`].
///
/// ```ignore
/// Block::new("Notes")
///     .action(Button::new("edit-notes").ghost().small().label("Edit"))
///     .child(notes)
/// ```
#[derive(IntoElement)]
pub struct Block {
    heading: SharedString,
    action: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl Block {
    pub fn new(heading: impl Into<SharedString>) -> Self {
        Self {
            heading: heading.into(),
            action: None,
            children: Vec::new(),
        }
    }

    /// A small action, or a caption such as "12 reviews", on the heading's
    /// trailing edge.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl ParentElement for Block {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Block {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .child(title_row(div().heading().child(self.heading), self.action))
            .children(self.children.into_iter().map(probe))
    }
}

/// Blocks stacked 24 apart: the column a pane's blocks go in.
pub fn blocks() -> Div {
    div().flex().flex_col().w_full().gap_6()
}
