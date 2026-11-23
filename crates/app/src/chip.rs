//! Chips (`hardcover-design`, Chips): neutral pills, 20 high, on the `chip`
//! ink overlay, with `fg-muted` text at 12/500 and an optional 12 px icon.
//! The overlay is translucent on purpose: a chip stays visible on a hovered
//! or selected row.
//!
//! - `StatusChip`: a book's library status at the end of a row, with the
//!   shelf's sidebar icon, so statuses differ by shape as well as by name.
//!   `StatusIconChip` is its compact form, for a row too narrow for the
//!   name: the icon alone on the pill, the name in its tooltip.
//! - `TagChip`: one catalogue tag (a genre, a mood, a content warning) on the
//!   book page. A genre carries an icon chosen from its name
//!   ([`genre::genre_icon`]) in `accent`, a content warning the warning icon
//!   in `warning`: the only two colours a chip ever has. Moods and other
//!   tags have no icon. A tag too long for its row is cut with an ellipsis
//!   and gets a tooltip with its whole text.
//! - A **page chip** (`TagChip::opens`, or `TagChip::genre_page` for a genre
//!   with a tag page) is the same pill, pressed: it opens a page. The
//!   `hover` fill over its own and the pointing hand on hover, the focus
//!   ring outside the pill and the 1 px accent edge inside it; nothing else
//!   changes, so it still reads as a tag.
//!   It is a `ui::reveal` probe, so a scrolling page keeps it in view, and
//!   `TagChip::key` names it when its text may repeat in its row.
//! - `TagRow`: chips wrapping across the width, 6 apart both ways (`s-6`):
//!   the book header's genres and each group of the Tags block. A row of
//!   page chips is one Tab stop, the arrows moving along it
//!   (`TagRow::keys`, `ui::chip_keys`).
//! - `TagGroup`: one group of the Tags block, its label (`label` role) 6
//!   above its `TagRow`; `tag_groups` stacks them 16 apart.

mod filter;
pub mod genre;

pub use filter::{FilterChip, filter_chip_width, small_menu_button_width};

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, canvas, div, prelude::FluentBuilder as _, px,
};
use hardcover_core::ReadingStatus;
use hardcover_core::tags::browse_genre;

use crate::catalog::navigation::{self, Page};
use crate::catalog::tag::Tag;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::status_label;
use crate::ui::chip_keys::ChipKeys;
use crate::ui::focus::{FocusTreatment as _, focus_visible, keyed_focus};
use crate::ui::reveal::probe;

/// The pill every chip is drawn on.
fn pill(cx: &App) -> Div {
    pill_style(div(), cx)
}

/// The pill's look, on any element (a page chip's button).
fn pill_style<E: gpui_kit::Styled>(element: E, cx: &App) -> E {
    let colors = colors(cx);
    element
        .flex()
        .flex_none()
        .items_center()
        .h(size::CHIP)
        .px_2()
        .gap_1()
        .rounded_full()
        .bg(colors.chip)
        .caption(cx)
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors.fg_muted)
}

/// The icon of a status, as on the sidebar's shelves.
pub fn status_icon(status: ReadingStatus) -> IconName {
    match status {
        ReadingStatus::CurrentlyReading => IconName::BookOpen,
        ReadingStatus::WantToRead => IconName::Inbox,
        ReadingStatus::Read => IconName::CircleCheck,
        ReadingStatus::Paused => IconName::Pause,
        ReadingStatus::DidNotFinish => IconName::Ban,
        ReadingStatus::Ignored => IconName::EyeOff,
    }
}

/// A book's library status: its shelf icon and name.
#[derive(IntoElement)]
pub struct StatusChip(pub ReadingStatus);

impl RenderOnce for StatusChip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        pill(cx)
            .child(Icon::new(status_icon(self.0)).xsmall())
            .child(status_label(self.0))
    }
}

/// A book's library status where a row has no room for its name: the
/// shelf icon alone on a 20 × 20 pill, the name in its tooltip and
/// accessible label.
#[derive(IntoElement)]
pub struct StatusIconChip(pub ReadingStatus);

impl RenderOnce for StatusIconChip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let label: SharedString = status_label(self.0).into();
        pill(cx)
            .id("status-icon-chip")
            .role(Role::Image)
            .px_1()
            .aria_label(label.clone())
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .child(Icon::new(status_icon(self.0)).xsmall())
    }
}

/// What a tag is, which decides its icon.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Plain,
    Genre,
    Warning,
}

/// One catalogue tag. A long one is cut short with an ellipsis, and then
/// shows its whole text in a tooltip (and to a screen reader).
#[derive(IntoElement)]
pub struct TagChip {
    text: SharedString,
    kind: TagKind,
    /// A page chip's page.
    page: Option<Page>,
    /// The chip's element id, when its text may not be unique in its row.
    key: Option<ElementId>,
}

impl TagChip {
    /// A mood or another tag: no icon.
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            kind: TagKind::Plain,
            page: None,
            key: None,
        }
    }

    /// A genre that opens its tag page when Hardcover's Browse list has it
    /// (`tags::browse_genre`); any other genre stays a plain chip.
    pub fn genre_page(text: impl Into<SharedString>) -> Self {
        let chip = Self::genre(text);
        match browse_genre(&chip.text) {
            Some(genre) => chip.opens(Page::Tag {
                tag: Tag::Genre(genre),
                within: None,
            }),
            None => chip,
        }
    }

    /// Names the chip `key` rather than after its text: for chips whose
    /// text may repeat in a row (Discover's featured vibes, by name).
    pub fn key(mut self, key: impl Into<ElementId>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A page chip: pressing it (a click, Enter, Space) opens `page`.
    pub fn opens(mut self, page: Page) -> Self {
        self.page = Some(page);
        self
    }

    /// A genre: the icon its name maps to, in `accent`, or none when the
    /// name matches no genre.
    pub fn genre(text: impl Into<SharedString>) -> Self {
        Self {
            kind: TagKind::Genre,
            ..Self::new(text)
        }
    }

    /// Marks a content warning with the warning icon.
    pub fn warning(mut self, warning: bool) -> Self {
        if warning {
            self.kind = TagKind::Warning;
        } else if self.kind == TagKind::Warning {
            self.kind = TagKind::Plain;
        }
        self
    }
}

impl RenderOnce for TagChip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        // On the chip's leading (left) side, whatever the tag's direction:
        // the interface is never mirrored (`hardcover-design`,
        // Right-to-left text).
        let icon: Option<AnyElement> = match self.kind {
            TagKind::Plain => None,
            TagKind::Genre => genre::genre_icon(&self.text).map(|icon| {
                Icon::new(icon)
                    .xsmall()
                    .flex_shrink_0()
                    .text_color(colors.accent)
                    .into_any_element()
            }),
            TagKind::Warning => Some(
                Icon::new(IconName::TriangleAlert)
                    .xsmall()
                    .flex_shrink_0()
                    .text_color(colors.warning)
                    .into_any_element(),
            ),
        };
        // Whether the last frame cut the text short, measured as it was laid
        // out: only a cut tag needs the tooltip with its whole text.
        let id = self
            .key
            .clone()
            .unwrap_or_else(|| ElementId::Name(format!("tag-{}", self.text).into()));
        let cut = window.use_keyed_state(id.clone(), cx, |_, _| false);
        let text = self.text.clone();
        let measure = canvas(
            {
                let cut = cut.clone();
                move |bounds, window, cx| {
                    let style = window.text_style();
                    let font_size = style.font_size.to_pixels(window.rem_size());
                    let run = style.to_run(text.len());
                    let width = window
                        .text_system()
                        .shape_line(text.clone(), font_size, &[run], None)
                        .width;
                    let is_cut = width > bounds.size.width + px(0.5);
                    if *cut.read(cx) != is_cut {
                        cut.update(cx, |c, _| *c = is_cut);
                        crate::ui::refresh_after_draw(window);
                    }
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .inset_0();
        let full = self.text.clone();
        let is_cut = *cut.read(cx);
        let label = div()
            .relative()
            .min_w_0()
            .truncate()
            .child(self.text)
            .child(measure);
        let Some(page) = self.page else {
            return pill(cx)
                .id(id)
                .flex_shrink(1.)
                .max_w_full()
                .min_w_0()
                .when(is_cut, |this| {
                    let this = this
                        .aria_label(full.clone())
                        .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx));
                    #[cfg(test)]
                    let this = this.debug_selector(|| "tag-cut".into());
                    this
                })
                .children(icon)
                .child(label)
                .into_any_element();
        };
        // A page chip: the same pill, pressed. A probe, so a scrolling
        // page keeps it in view when it takes focus (`ui::reveal`).
        let focus = keyed_focus(id.clone(), window, cx);
        let focused = focus_visible(&focus, window);
        // The hover overlay over the chip's own: both are ink, so the two
        // translucent fills make one.
        let hovered = colors
            .fg
            .opacity(1. - (1. - colors.chip.a) * (1. - colors.hover.a));
        let chip = pill_style(gpui_kit::base::Button::new(id), cx)
            .track_focus(&focus)
            .accessibility_label(full.clone())
            .relative()
            .flex_shrink(1.)
            .max_w_full()
            .min_w_0()
            .cursor_pointer()
            .hover(move |style| style.bg(hovered))
            .on_click(move |_, _, cx| navigation::open(page.clone(), cx))
            .when(is_cut, |this| {
                this.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
            })
            .when(focused, |this| {
                this.focus_ring(size::CHIP.to_pixels(window.rem_size()), cx)
            })
            .children(icon)
            .child(label);
        probe(chip).into_any_element()
    }
}

/// Tags wrapping across the width, 6 apart both ways (`s-6`, the gap of a
/// wrapped chip group).
#[derive(IntoElement)]
pub struct TagRow {
    chips: Vec<TagChip>,
    keys: Option<(ElementId, Entity<ChipKeys>)>,
}

impl TagRow {
    pub fn new(chips: impl IntoIterator<Item = TagChip>) -> Self {
        Self {
            chips: chips.into_iter().collect(),
            keys: None,
        }
    }

    /// A row of page chips: one Tab stop, the arrows moving along it.
    /// `id` must be unique among the row's siblings.
    pub fn keys(mut self, id: impl Into<ElementId>, keys: &Entity<ChipKeys>) -> Self {
        self.keys = Some((id.into(), keys.clone()));
        self
    }
}

impl RenderOnce for TagRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let row = |row: Div| row.w_full().min_w_0().flex_wrap().gap_1p5();
        match self.keys {
            Some((id, keys)) => keys
                .read(cx)
                .wrap(row(h_flex()).id(id))
                .children(self.chips)
                .into_any_element(),
            None => row(h_flex()).children(self.chips).into_any_element(),
        }
    }
}

/// One group of the Tags block ("Genres", "Moods", "Content warnings",
/// "Other tags"): its label in the `label` role, 6 above its chips, which
/// take the block's whole width.
#[derive(IntoElement)]
pub struct TagGroup {
    label: SharedString,
    row: TagRow,
}

impl TagGroup {
    /// The gallery's: the book view's groups are built with [`Self::row`].
    #[cfg_attr(not(feature = "demo"), allow(dead_code))]
    pub fn new(label: impl Into<SharedString>, chips: impl IntoIterator<Item = TagChip>) -> Self {
        Self::row(label, TagRow::new(chips))
    }

    /// A group of `row` (a row of page chips with its keys).
    pub fn row(label: impl Into<SharedString>, row: TagRow) -> Self {
        Self {
            label: label.into(),
            row,
        }
    }
}

impl RenderOnce for TagGroup {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_1p5()
            .child(div().label(cx).child(self.label))
            .child(self.row)
    }
}

/// The Tags block's groups, 16 apart (`s-16`, groups in a section).
pub fn tag_groups(groups: impl IntoIterator<Item = TagGroup>) -> Div {
    v_flex().w_full().gap_4().children(groups)
}

#[cfg(test)]
mod tests {
    use gpui_kit::{IntoElement as _, ParentElement as _, Styled as _, TestAppContext, div, rems};

    use super::TagChip;
    use crate::ui::test_support::{host, redraw};

    /// Only a tag cut short gets the tooltip: measured as it is laid out,
    /// so it shows on the next frame.
    #[gpui_kit::test]
    fn a_cut_tag_gets_its_whole_text_in_a_tooltip(cx: &mut TestAppContext) {
        let long = "Found family and the long road home through the marshes";
        let cx = host(cx, move |_, _| {
            div()
                .flex()
                .flex_col()
                .child(div().w(rems(6.)).child(TagChip::new(long)))
                .into_any_element()
        });
        redraw(cx);
        assert!(cx.debug_bounds("tag-cut").is_some(), "a cut tag");
    }

    /// A row of genre page chips is one Tab stop: the arrows move along it
    /// (past a genre with no page), Enter opens the genre's page, Tab
    /// leaves and Shift+Tab comes back to the chip it left.
    #[gpui_kit::test]
    fn a_row_of_page_chips_is_one_tab_stop(cx: &mut TestAppContext) {
        use std::cell::RefCell;
        use std::rc::Rc;

        use gpui_kit::component::button::Button;
        use gpui_kit::{AppContext as _, Entity};
        use hardcover_core::tags::browse_genre;

        use crate::catalog::navigation::{NavigationEvent, Navigator, Page};
        use crate::catalog::tag::Tag;
        use crate::ui::chip_keys::ChipKeys;
        use crate::ui::test_support::{counter, press, shift_tab, tab};

        let opened: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        cx.update(|cx| {
            let navigator = Navigator::init(cx);
            let opened = opened.clone();
            cx.subscribe(&navigator, move |_, event: &NavigationEvent, _| {
                if let NavigationEvent::Open(Page::Tag {
                    tag: Tag::Genre(genre),
                    ..
                }) = event
                {
                    opened.borrow_mut().push(genre.name);
                }
            })
            .detach();
        });
        let (after, on_after) = counter();
        let slot: Rc<RefCell<Option<Entity<ChipKeys>>>> = Rc::default();
        let cx = host(cx, move |window, cx| {
            let keys = slot
                .borrow_mut()
                .get_or_insert_with(|| cx.new(|cx| ChipKeys::new(window, cx)))
                .clone();
            div()
                .child(Button::new("before").label("Before"))
                .child(
                    super::TagRow::new(
                        ["Fantasy", "General", "Sci-fi", "Mystery"].map(TagChip::genre_page),
                    )
                    .keys("genres", &keys),
                )
                .child(
                    Button::new("after")
                        .label("After")
                        .on_click(on_after.clone()),
                )
                .into_any_element()
        });
        cx.update(|window, _| window.activate_window());
        let settle = |cx: &mut gpui_kit::VisualTestContext| {
            redraw(cx);
            cx.run_until_parked();
            redraw(cx);
        };
        let open = |cx: &mut gpui_kit::VisualTestContext| {
            press(cx, "enter");
            settle(cx);
        };
        assert!(browse_genre("General").is_none(), "a genre with no page");

        tab(cx);
        tab(cx);
        settle(cx);
        open(cx);
        press(cx, "right");
        settle(cx);
        open(cx);
        press(cx, "end");
        settle(cx);
        open(cx);
        press(cx, "right");
        settle(cx);
        open(cx);
        assert_eq!(
            *opened.borrow(),
            vec!["Fantasy", "Science Fiction", "Mystery", "Mystery"],
            "General is passed by, and Right stops at the end"
        );

        press(cx, "left");
        press(cx, "tab");
        settle(cx);
        open(cx);
        assert_eq!(after.get(), 1, "Tab left the row");
        shift_tab(cx);
        settle(cx);
        open(cx);
        assert_eq!(
            opened.borrow().last(),
            Some(&"Science Fiction"),
            "back on the chip it left"
        );
    }

    #[gpui_kit::test]
    fn a_tag_that_fits_gets_no_tooltip(cx: &mut TestAppContext) {
        let cx = host(cx, |_, _| {
            div()
                .w(rems(30.))
                .child(TagChip::new("Hopeful"))
                .into_any_element()
        });
        redraw(cx);
        assert!(cx.debug_bounds("tag-cut").is_none());
    }
}
