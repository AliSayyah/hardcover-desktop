//! The full "Coming soon" list (`hardcover-design`, Screens → Coming soon), a
//! catalogue page opened from Home: every book still to come in the reader's
//! series, under a `label` per month for the coming 12 months, then per year.
//! The workspace draws the object bar with Back above it.
//!
//! - **Content:** inset 32, filling the page: the page title, a muted line
//!   with the count, then the groups (24 above each label, 4 below), each a
//!   sequence of upcoming rows.
//! - **Loading:** skeleton rows where the rows will be.
//! - **Failed:** one muted sentence with "Try again".
//! - **Empty:** the empty state.

use crate::ui::reveal::RevealScroll as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{IconName, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, div,
};

use super::model::{UpcomingModel, UpcomingStatus};
use super::row::UpcomingRow;
use super::schedule::{grouped, shown};
use crate::cover::CoverSize;
use crate::design::{TypeRoles as _, colors};
use crate::format::count_text;
use crate::library::model::LibraryModel;
use crate::ui::kit::SmallControl as _;
use crate::ui::{Lane, PageHeader, Sequence, Skeleton, empty_state};

/// Skeleton rows while the list loads.
const SKELETON_ROWS: usize = 4;

pub struct UpcomingView {
    upcoming: Entity<UpcomingModel>,
    library: Entity<LibraryModel>,
    _subscriptions: Vec<Subscription>,
}

impl UpcomingView {
    pub fn new(
        upcoming: Entity<UpcomingModel>,
        library: Entity<LibraryModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&upcoming, |_, _, cx| cx.notify()),
            cx.observe(&library, |_, _, cx| cx.notify()),
        ];
        Self {
            upcoming,
            library,
            _subscriptions: subscriptions,
        }
    }

    /// "Couldn't load …" and "Try again", in one muted line.
    fn failed(message: SharedString, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_wrap()
            .gap_2()
            .child(div().body().text_color(colors(cx).fg_muted).child(message))
            .child(
                Button::new("upcoming-retry")
                    .small_control()
                    .label("Try again")
                    .on_click(
                        cx.listener(|this, _, _, cx| this.upcoming.update(cx, UpcomingModel::load)),
                    ),
            )
    }
}

/// The muted line under the title.
pub fn count_line(books: usize) -> String {
    match books {
        1 => "1 book still to come in the series on your shelves".to_owned(),
        n => format!(
            "{} books still to come in the series on your shelves",
            count_text(n)
        ),
    }
}

/// An upcoming row while the list loads: its cover, two lines and the lanes.
fn skeleton_row() -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .child(Skeleton::cover(CoverSize::Sm))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_2()
                .child(Skeleton::line().w_3_5())
                .child(Skeleton::line().w_2_5()),
        )
        .child(Lane::action())
        .into_any_element()
}

/// The page's column: inset 32, filling the page; the rows' date and action
/// lanes keep the trailing edge, lined up down the list.
fn column() -> gpui_kit::Div {
    div().flex().flex_col().w_full().p_8()
}

fn group_label(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div().label(cx).pt_6().pb_1().child(label.into())
}

impl Render for UpcomingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today = crate::dates::today();
        let upcoming = self.upcoming.read(cx);
        let status = upcoming.status().clone();
        let library = self.library.read(cx);
        let books = shown(
            upcoming.books(),
            |id| library.entry_for_book(id).map(|e| e.status),
            today,
        );
        let root = div().id("upcoming-page").size_full();

        if books.is_empty() {
            let body: AnyElement = match status {
                UpcomingStatus::Idle | UpcomingStatus::Loading => column()
                    .child(PageHeader::new("Coming soon"))
                    .child(div().pt_6().child(
                        Sequence::new().children((0..SKELETON_ROWS).map(|_| skeleton_row())),
                    ))
                    .into_any_element(),
                UpcomingStatus::Failed(message) => column()
                    .gap_2()
                    .child(PageHeader::new("Coming soon"))
                    .child(Self::failed(message, cx))
                    .into_any_element(),
                UpcomingStatus::Loaded => empty_state(
                    IconName::Calendar,
                    "Nothing coming soon".into(),
                    "Hardcover lists no unreleased books in the series on your shelves.".into(),
                )
                .into_any_element(),
            };
            return root.child(body).into_any_element();
        }

        let count = count_line(books.len());
        let groups = grouped(&books, today)
            .into_iter()
            .map(|(group, books)| {
                div()
                    .flex()
                    .flex_col()
                    .child(group_label(group.label(today), cx))
                    .child(Sequence::new().children(books.iter().map(|book| {
                        UpcomingRow::new("upcoming", book, today, self.library.clone())
                    })))
            })
            .collect::<Vec<_>>();
        let failed = match status {
            UpcomingStatus::Failed(message) => Some(div().pt_2().child(Self::failed(message, cx))),
            _ => None,
        };

        root.scroll_y_revealing_focus()
            .child(
                column()
                    .child(PageHeader::new("Coming soon").line(count))
                    .children(failed)
                    .children(groups),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_line_counts_books() {
        assert_eq!(
            count_line(1),
            "1 book still to come in the series on your shelves"
        );
        assert_eq!(
            count_line(1200),
            "1,200 books still to come in the series on your shelves"
        );
    }
}
