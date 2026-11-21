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
//! - `TagRow`: chips wrapping across the width, 6 apart both ways (`s-6`):
//!   the book header's genres and each group of the Tags block.
//! - `TagGroup`: one group of the Tags block, its label (`label` role) 6
//!   above its `TagRow`; `tag_groups` stacks them 16 apart.

pub mod genre;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, canvas, div, prelude::FluentBuilder as _, px,
};
use hardcover_core::ReadingStatus;

use crate::design::{TypeRoles as _, colors, size};
use crate::format::status_label;

/// The pill every chip is drawn on.
fn pill(cx: &App) -> Div {
    let colors = colors(cx);
    div()
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
}

impl TagChip {
    /// A mood or another tag: no icon.
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            kind: TagKind::Plain,
        }
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
        let id = ElementId::Name(format!("tag-{}", self.text).into());
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
                        window.refresh();
                    }
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .inset_0();
        let full = self.text.clone();
        pill(cx)
            .id(id)
            .flex_shrink(1.)
            .max_w_full()
            .min_w_0()
            .when(*cut.read(cx), |this| {
                let this = this
                    .aria_label(full.clone())
                    .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx));
                #[cfg(test)]
                let this = this.debug_selector(|| "tag-cut".into());
                this
            })
            .children(icon)
            .child(
                div()
                    .relative()
                    .min_w_0()
                    .truncate()
                    .child(self.text)
                    .child(measure),
            )
    }
}

/// Tags wrapping across the width, 6 apart both ways (`s-6`, the gap of a
/// wrapped chip group).
#[derive(IntoElement)]
pub struct TagRow(Vec<TagChip>);

impl TagRow {
    pub fn new(chips: impl IntoIterator<Item = TagChip>) -> Self {
        Self(chips.into_iter().collect())
    }
}

impl RenderOnce for TagRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap_1p5()
            .children(self.0)
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
    pub fn new(label: impl Into<SharedString>, chips: impl IntoIterator<Item = TagChip>) -> Self {
        Self {
            label: label.into(),
            row: TagRow::new(chips),
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
