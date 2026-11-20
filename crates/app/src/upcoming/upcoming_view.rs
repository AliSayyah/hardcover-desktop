//! The full "Coming soon" list, a catalogue page opened from Home: every book
//! still to come in the reader's series, under a heading per month for the
//! coming 12 months, then per year.

use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, rems,
};

use super::model::{UpcomingModel, UpcomingStatus};
use super::row::UpcomingRow;
use super::schedule::{grouped, shown};
use crate::library::library_view::empty_state;
use crate::library::model::LibraryModel;

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
}

impl Render for UpcomingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today = chrono::Local::now().date_naive();
        let upcoming = self.upcoming.read(cx);
        let status = upcoming.status().clone();
        let library = self.library.read(cx);
        let books = shown(
            upcoming.books(),
            |id| library.entry_for_book(id).map(|e| e.status),
            today,
        );
        let muted = cx.theme().muted_foreground;
        let root = div().id("upcoming-page").size_full();

        if books.is_empty() {
            let body: AnyElement = match status {
                UpcomingStatus::Idle | UpcomingStatus::Loading => h_flex()
                    .size_full()
                    .justify_center()
                    .child(Spinner::new())
                    .into_any_element(),
                UpcomingStatus::Failed(message) => v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_sm()
                    .child(div().text_color(muted).child(message))
                    .child(
                        Button::new("upcoming-retry")
                            .small()
                            .label("Try again")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.upcoming.update(cx, |u, cx| u.load(cx))
                            })),
                    )
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

        let count = match books.len() {
            1 => "1 book still to come in the series on your shelves".to_owned(),
            n => format!("{n} books still to come in the series on your shelves"),
        };
        let failed = match &status {
            UpcomingStatus::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let groups = grouped(&books, today).into_iter().map(|(group, books)| {
            let rows = books.iter().enumerate().map(|(ix, book)| {
                UpcomingRow::new("upcoming", book, today, self.library.clone()).divided(ix > 0)
            });
            v_flex()
                .id(SharedString::from(format!(
                    "upcoming-group-{}",
                    group.key()
                )))
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .child(group.label(today)),
                )
                .child(v_flex().children(rows))
        });

        root.overflow_y_scrollbar()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(48.))
                    .px_6()
                    .pt_6()
                    .pb_5()
                    .gap_5()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Coming soon"),
                            )
                            .child(div().text_sm().text_color(muted).child(count))
                            .when_some(failed, |this, message| {
                                this.child(div().text_sm().text_color(muted).child(message))
                            }),
                    )
                    .children(groups),
            )
            .into_any_element()
    }
}
