//! Headers (`hardcover-design`, Type and Screens): the title of a
//! destination or page, Home's greeting, and the title of a section on a
//! page.
//!
//! - [`PageHeader`]: a destination's `page-title` ("Currently reading"), with
//!   its count beside it in `fg-muted` and an optional muted line under it
//!   ("5 books still to come in the series on your shelves").
//! - [`Greeting`]: Home's header: the date in `fg-muted` over the greeting
//!   in `display-30`. [`greeting`] and [`date_line`] give the words.
//! - [`SectionHeader`]: a `section` title on Home or a page ("Continue
//!   reading", "Coming soon") with a small ghost action on its trailing edge
//!   ([`see_all`]).

use chrono::{Datelike as _, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::h_flex;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::kit::SmallControl as _;
use super::reveal::probe;
use crate::design::{TypeRole, TypeRoles as _, colors, size};

/// A destination's title. See the module doc.
///
/// ```ignore
/// PageHeader::new("Currently reading").count(3)
/// PageHeader::new("Coming soon").line("5 books still to come in the series on your shelves")
/// ```
#[derive(IntoElement)]
pub struct PageHeader {
    title: SharedString,
    count: Option<SharedString>,
    line: Option<SharedString>,
}

impl PageHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            count: None,
            line: None,
        }
    }

    /// How many things the destination holds, beside the title.
    pub fn count(mut self, count: impl std::fmt::Display) -> Self {
        self.count = Some(crate::format::count_text(count).into());
        self
    }

    /// One muted line under the title.
    pub fn line(mut self, line: impl Into<SharedString>) -> Self {
        self.line = Some(line.into());
        self
    }
}

impl RenderOnce for PageHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .child(
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .child(div().page_title().min_w_0().truncate().child(self.title))
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
                    }),
            )
            .when_some(self.line, |this, line| {
                this.child(div().body().text_color(muted).child(line))
            })
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
