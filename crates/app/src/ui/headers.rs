//! Headers (`hardcover-design`, Type and Screens): the title of a
//! destination or page, Home's greeting, and the title of a section on a
//! page.
//!
//! - [`PageHeader`]: a destination's or page's `page-title` ("Currently
//!   reading"), with its count beside it in `fg-muted`, the page's tools on
//!   its trailing edge, and lines under it, 4 apart: muted ones ("5 books
//!   still to come in the series on your shelves"), facts, or anything else
//!   (a byline). A catalogue name wraps ([`PageHeader::wraps`]); a
//!   destination's title is cut.
//! - [`Greeting`]: Home's header: the date in `fg-muted` over the greeting
//!   in `display-30`. [`greeting`] and [`date_line`] give the words.
//! - [`SectionHeader`]: a `section` title on Home or a page ("Continue
//!   reading", "Coming soon") with a small ghost action on its trailing edge
//!   ([`see_all`]).

use chrono::{Datelike as _, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::h_flex;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement as _, Rems, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, rems,
};

use super::kit::SmallControl as _;
use super::reveal::probe;
use crate::bidi::WrappingText;
use crate::design::{TypeRole, TypeRoles as _, colors, size};

/// A destination's title. See the module doc.
///
/// ```ignore
/// PageHeader::new("Currently reading").count(3)
/// PageHeader::new("Want to read").count_of(2, 8) // filtered
/// PageHeader::new("Coming soon").line("5 books still to come in the series on your shelves")
/// // A catalogue page: a name that may wrap, its tools, its facts.
/// PageHeader::new(list.name.clone())
///     .wraps()
///     .actions(tools)
///     .facts("12 books · Ranked")
///     .under(byline)
/// ```
#[derive(IntoElement)]
pub struct PageHeader {
    title: SharedString,
    count: Option<SharedString>,
    wraps: bool,
    actions: Option<AnyElement>,
    lines: Vec<AnyElement>,
}

/// Between the title and each line under it (4).
const LINE_GAP: Rems = rems(0.25);

impl PageHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            count: None,
            wraps: false,
            actions: None,
            lines: Vec::new(),
        }
    }

    /// How many things the destination holds, beside the title.
    pub fn count(mut self, count: impl crate::format::Count) -> Self {
        self.count = Some(crate::format::count_text(count).into());
        self
    }

    /// How many of them a filter shows, beside the title: "2 of 8".
    pub fn count_of(
        mut self,
        shown: impl crate::format::Count,
        total: impl crate::format::Count,
    ) -> Self {
        self.count = Some(
            format!(
                "{} of {}",
                crate::format::count_text(shown),
                crate::format::count_text(total)
            )
            .into(),
        );
        self
    }

    /// Lets the title wrap ([`WrappingText`]) instead of being cut: a
    /// catalogue name (a series, a list), which may be long or Persian.
    pub fn wraps(mut self) -> Self {
        self.wraps = true;
        self
    }

    /// The page's tools on the title's trailing edge, at its top.
    pub fn actions(mut self, actions: impl IntoElement) -> Self {
        self.actions = Some(actions.into_any_element());
        self
    }

    /// A muted line under the title, at most the reading measure
    /// (`size::PROSE`) wide. Lines stack in the order they are added.
    pub fn line(self, line: impl Into<SharedString>) -> Self {
        let line = div().max_w(size::PROSE).body().child(line.into());
        self.muted(line)
    }

    /// A muted line of facts under the title, in tabular figures ("12
    /// books · 3 more in the series · Ongoing").
    pub fn facts(self, facts: impl Into<SharedString>) -> Self {
        let line = div().body().tabular().child(facts.into());
        self.muted(line)
    }

    /// Anything else under the title: a byline, a summary in `fg`.
    pub fn under(mut self, line: impl IntoElement) -> Self {
        self.lines.push(line.into_any_element());
        self
    }

    fn muted(mut self, line: gpui_kit::Div) -> Self {
        // The colour is set when drawn, from the theme.
        self.lines.push(MutedLine(line).into_any_element());
        self
    }

    /// How tall a header is whose title and `lines` lines of `body` text
    /// each take one line, which the layout tests check against. No header
    /// sits inside a virtual list any more (the series tracker's stays above
    /// it), so nothing else needs it.
    #[cfg(test)]
    pub fn height(lines: usize, rem: gpui_kit::Pixels) -> gpui_kit::Pixels {
        #[expect(clippy::cast_precision_loss, reason = "a handful of lines")]
        let lines = lines as f32;
        (TypeRole::PageTitle.line_height() + (LINE_GAP + TypeRole::Body.line_height()) * lines)
            .to_pixels(rem)
    }
}

/// A line of [`PageHeader`] in `fg-muted`.
#[derive(IntoElement)]
struct MutedLine(gpui_kit::Div);

impl RenderOnce for MutedLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.0.text_color(colors(cx).fg_muted)
    }
}

impl RenderOnce for PageHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        let title = if self.wraps {
            div()
                .page_title()
                .min_w_0()
                .child(WrappingText::new(self.title))
        } else {
            div().page_title().min_w_0().truncate().child(self.title)
        };
        let title = h_flex()
            .gap_2()
            .min_w_0()
            .child(title)
            .when_some(self.count, |this, count| {
                this.child(
                    div()
                        .flex_none()
                        .body()
                        .baseline_on(TypeRole::Body, TypeRole::PageTitle)
                        .tabular()
                        .text_color(muted)
                        .child(count),
                )
            });
        let title = match self.actions {
            Some(actions) => h_flex()
                .items_start()
                .justify_between()
                .gap_4()
                .min_w_0()
                .child(title)
                .child(div().flex_none().child(actions))
                .into_any_element(),
            None => title.into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(LINE_GAP)
            .min_w_0()
            .child(title)
            .children(self.lines)
    }
}

/// Home's header: the date over the greeting. See the module doc.
///
/// ```ignore
/// let now = chrono::Local::now();
/// Greeting::new(date_line(now.date_naive()), greeting(now.hour()))
/// ```
#[derive(IntoElement)]
pub struct Greeting {
    date: SharedString,
    greeting: SharedString,
}

impl Greeting {
    pub fn new(date: impl Into<SharedString>, greeting: impl Into<SharedString>) -> Self {
        Self {
            date: date.into(),
            greeting: greeting.into(),
        }
    }
}

impl RenderOnce for Greeting {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .body()
                    .text_color(colors(cx).fg_muted)
                    .child(self.date),
            )
            .child(div().display_30().child(self.greeting))
    }
}

/// The greeting for the local hour: "Good morning" until noon, "Good
/// afternoon" until six, then "Good evening".
pub fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// The greeting for a local time.
pub fn greeting_at(time: &impl chrono::Timelike) -> &'static str {
    greeting(time.hour())
}

/// Home's date line: "Friday, November 20".
pub fn date_line(date: NaiveDate) -> String {
    format!(
        "{}, {} {}",
        date.format("%A"),
        date.format("%B"),
        date.day()
    )
}

/// A section's title on Home or a page, with an optional small action on
/// its trailing edge. See the module doc.
///
/// ```ignore
/// SectionHeader::new("Coming soon").action(see_all("home-see-all", Some(5), on_click))
/// ```
#[derive(IntoElement)]
pub struct SectionHeader {
    title: SharedString,
    action: Option<AnyElement>,
}

impl SectionHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            action: None,
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for SectionHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .min_h(size::CONTROL_SMALL)
            .gap_2()
            .child(
                div()
                    .section()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(self.title),
            )
            .children(
                self.action
                    .map(|action| probe(div().flex_none().child(action))),
            )
    }
}

/// The small ghost "See all 5" (or "See all" without a count) that opens a
/// section's full list.
pub fn see_all(
    id: impl Into<ElementId>,
    count: Option<u32>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .ghost()
        .small_control()
        .label(see_all_label(count))
        .on_click(on_click)
}

fn see_all_label(count: Option<u32>) -> String {
    match count {
        Some(n) => format!("See all {}", crate::format::count_text(n)),
        None => "See all".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::host;
    use gpui_kit::{InteractiveElement as _, TestAppContext, px};

    const LONG: &str = "The Exceedingly Long and Winding Chronicle of the Northern Reaches";

    /// A header drawn in a box `width` wide.
    fn measure(
        cx: &mut TestAppContext,
        width: f32,
        build: impl Fn() -> PageHeader + 'static,
    ) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        let cx = host(cx, move |_, _| {
            div()
                .w(px(width))
                .child(div().debug_selector(|| "header".into()).child(build()))
                .into_any_element()
        });
        cx.debug_bounds("header").expect("the header is drawn")
    }

    /// A catalogue name wraps onto more lines instead of being cut.
    #[gpui_kit::test]
    fn a_wrapping_title_takes_more_lines(cx: &mut TestAppContext) {
        let rem = px(16.);
        let cut = measure(cx, 320., || PageHeader::new(LONG));
        assert_eq!(cut.size.height, PageHeader::height(0, rem));
        let wrapped = measure(cx, 320., || PageHeader::new(LONG).wraps());
        assert!(
            wrapped.size.height >= PageHeader::height(0, rem) * 2.,
            "{wrapped:?}"
        );
    }

    /// Lines stack 4 apart under the title, and the height a virtual list
    /// sizes the header from is the one it draws.
    #[gpui_kit::test]
    fn lines_stack_under_the_title(cx: &mut TestAppContext) {
        let header = measure(cx, 800., || {
            PageHeader::new("Series")
                .count(4_u32)
                .line("The series you've started.")
                .facts("4 books · Ongoing")
        });
        assert_eq!(header.size.height, PageHeader::height(2, px(16.)));
    }

    /// The page's tools sit on the title's trailing edge, at its top.
    #[gpui_kit::test]
    fn actions_sit_at_the_trailing_edge(cx: &mut TestAppContext) {
        let cx = host(cx, |_, _| {
            div()
                .w(px(600.))
                .child(
                    PageHeader::new(LONG).wraps().actions(
                        div()
                            .w(px(80.))
                            .h(px(24.))
                            .debug_selector(|| "action".into()),
                    ),
                )
                .into_any_element()
        });
        let action = cx.debug_bounds("action").expect("the actions are drawn");
        assert_eq!(action.right(), px(600.));
        assert_eq!(action.top(), px(0.));
    }

    #[test]
    fn the_greeting_follows_the_hour() {
        assert_eq!(greeting(5), "Good morning");
        assert_eq!(greeting(11), "Good morning");
        assert_eq!(greeting(12), "Good afternoon");
        assert_eq!(greeting(17), "Good afternoon");
        assert_eq!(greeting(18), "Good evening");
        assert_eq!(greeting(0), "Good evening");
        assert_eq!(greeting(4), "Good evening");
    }

    #[test]
    fn the_date_line_names_the_day() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert_eq!(date_line(date), "Friday, November 20");
        let date = NaiveDate::from_ymd_opt(2027, 1, 5).unwrap();
        assert_eq!(date_line(date), "Tuesday, January 5");
    }

    #[test]
    fn see_all_counts_with_separators() {
        assert_eq!(see_all_label(Some(5)), "See all 5");
        assert_eq!(see_all_label(Some(1200)), "See all 1,200");
        assert_eq!(see_all_label(None), "See all");
    }
}
