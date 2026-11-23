//! The filter chip (`hardcover-design`, Chips → Filter chip): a genre or a
//! mood in a shelf's filter bar, pressed to narrow the shelf.
//!
//! - The pill of every chip (20 high, `chip` fill, `fg-muted` at 12/500),
//!   then the count, tabular at weight 400: "Fantasy 5". A genre carries its
//!   icon in `accent` (`genre::genre_icon`), a mood none.
//! - **Hover:** the `hover` fill over its own; **pressed** the `selected`.
//! - **Selected:** the `selected` fill over its own, the text in `fg`, and
//!   a 12 px check in `fg` in the icon's place (before a mood's name), so
//!   the state never rests on the fill alone.
//! - **Disabled** (a press would leave no books; never a selected chip):
//!   50% opacity, no response, out of the Tab order, so a row's arrows skip
//!   it.
//! - **Focus:** the ring outside the pill and the 1 px accent edge inside it,
//!   after keyboard input.
//! - A real button: Enter and Space press it. To a screen reader it is a
//!   toggle named with its count ("Fantasy, 5 books").
//! - A name too long for its row is cut with an ellipsis.
//!
//! [`filter_chip_width`] is its width as drawn, for fitting a row.
//!
//! ```ignore
//! FilterChip::genre(("genre", key), "Fantasy", 5)
//!     .selected(true)
//!     .on_click(move |_, window, cx| press(window, cx))
//! ```

use std::rc::Rc;

use gpui_kit::base::Button as BaseButton;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::{
    App, ClickEvent, ElementId, Font, FontFeatures, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, TextRun, Toggled, Window, div, font,
    prelude::FluentBuilder as _, px,
};

use super::genre::genre_icon;
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::ui::focus::{FocusTreatment as _, focus_visible, keyed_focus};
use crate::ui::reveal::probe;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A genre or mood a shelf can be filtered by. See the module doc.
#[derive(IntoElement)]
pub struct FilterChip {
    id: ElementId,
    label: SharedString,
    count: usize,
    genre: bool,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl FilterChip {
    /// A genre: its icon (from the name) in `accent`.
    pub fn genre(id: impl Into<ElementId>, label: impl Into<SharedString>, count: usize) -> Self {
        Self {
            genre: true,
            ..Self::mood(id, label, count)
        }
    }

    /// A mood: no icon.
    pub fn mood(id: impl Into<ElementId>, label: impl Into<SharedString>, count: usize) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count,
            genre: false,
            selected: false,
            on_click: None,
        }
    }

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

    /// A press would leave no books, and it isn't selected.
    fn disabled(&self) -> bool {
        self.count == 0 && !self.selected
    }
}

/// "5 books", for a chip's accessible name.
fn books(count: usize) -> String {
    match count {
        1 => "1 book".to_owned(),
        n => format!("{} books", crate::format::count_text(n)),
    }
}

impl RenderOnce for FilterChip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let disabled = self.disabled();
        let selected = self.selected;
        let focus = keyed_focus(self.id.clone(), window, cx);
        let focused = !disabled && focus_visible(&focus, window);
        let radius = size::CHIP.to_pixels(window.rem_size()) / 2.;
        let lead = if selected {
            Some(Icon::new(IconName::Check).xsmall().text_color(colors.fg))
        } else if self.genre {
            genre_icon(&self.label).map(|icon| Icon::new(icon).xsmall().text_color(colors.accent))
        } else {
            None
        };
        let name: SharedString = format!("{}, {}", self.label, books(self.count)).into();
        let on_click = self.on_click.filter(|_| !disabled);
        let chip = BaseButton::new(self.id)
            .track_focus(&focus)
            .disabled(disabled)
            .selected(selected)
            .accessibility_label(name)
            .aria_toggled(if selected {
                Toggled::True
            } else {
                Toggled::False
            })
            .relative()
            .flex()
            .flex_none()
            .flex_shrink(1.)
            .min_w_0()
            .items_center()
            .h(size::CHIP)
            .px_2()
            .gap_1()
            .rounded_full()
            .bg(colors.chip)
            .caption(cx)
            .font_weight(FontWeight::MEDIUM)
            .text_color(if selected { colors.fg } else { colors.fg_muted })
            .when(disabled, |this| this.opacity(0.5))
            // The state's fill over the chip's own: hover, pressed, selected.
            .child(
                div()
                    .id("fill")
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .when(selected, |this| this.bg(colors.selected))
                    .when(!selected && !disabled, |this| {
                        this.hover(|s| s.bg(colors.hover))
                            .active(|s| s.bg(colors.selected))
                    }),
            )
            .when_some(on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when(focused, |this| this.focus_ring(radius, cx))
            .children(lead.map(|icon| icon.flex_shrink_0()))
            .child(div().min_w_0().truncate().child(self.label))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::NORMAL)
                    .tabular()
                    .child(crate::format::count_text(self.count)),
            );
        probe(div().flex().min_w_0().child(chip))
    }
}

/// The width of a [`FilterChip`] as drawn, in px at the window's rem: its
/// padding, the icon or check when it has one, the name and the count.
pub fn filter_chip_width(label: &str, count: usize, lead_icon: bool, window: &Window) -> Pixels {
    let rem = window.rem_size();
    let at = |design: f32| rem * (design / 16.);
    let name = text_width(label, FontWeight::MEDIUM, false, window);
    let count = text_width(
        &crate::format::count_text(count),
        FontWeight::NORMAL,
        true,
        window,
    );
    let icon = if lead_icon { at(12. + 4.) } else { px(0.) };
    // Padding 8 on each side, 4 between the name and the count, and a
    // pixel to spare for rounding.
    at(8. + 8. + 4.) + icon + name + count + px(1.)
}

/// The width of a small ghost menu button labelled `label` with its caret
/// ("More"), in px at the window's rem.
pub fn small_menu_button_width(label: &str, window: &Window) -> Pixels {
    let rem = window.rem_size();
    let at = |design: f32| rem * (design / 16.);
    at(8. + 4. + 12. + 8.) + text_width(label, FontWeight::MEDIUM, false, window) + px(1.)
}

/// How wide `text` is set at the caption size in Inter.
fn text_width(text: &str, weight: FontWeight, tabular: bool, window: &Window) -> Pixels {
    let rem = window.rem_size();
    let (size_px, _) = TypeRole::Caption.scale();
    let font = Font {
        weight,
        features: if tabular {
            FontFeatures(std::sync::Arc::new(vec![
                ("tnum".into(), 1),
                ("lnum".into(), 1),
            ]))
        } else {
            FontFeatures::default()
        },
        ..font(TypeRole::Caption.family().name())
    };
    let run = TextRun {
        len: text.len(),
        font,
        color: gpui_kit::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(
            SharedString::from(text.to_owned()),
            rem * (size_px / 16.),
            &[run],
            None,
        )
        .width
}
