//! Discover: the books the most readers finished on Hardcover in a period,
//! each with its shelf. Each period is fetched once per session (two
//! requests); Refresh fetches the shown one again.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::{List, ListState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Task, Window, div, rems,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookSummary, TrendingPeriod};

use super::book_list::{CatalogBooks, CatalogRow, follow_list, new_list, row_details};
use crate::app_state::Services;
use crate::format::authors_text;
use crate::library::model::LibraryModel;

/// The periods offered, in order.
const PERIODS: [TrendingPeriod; 5] = [
    TrendingPeriod::Week,
    TrendingPeriod::Month,
    TrendingPeriod::ThreeMonths,
    TrendingPeriod::Year,
    TrendingPeriod::AllTime,
];

fn period_label(period: TrendingPeriod) -> &'static str {
    match period {
        TrendingPeriod::Week => "Week",
        TrendingPeriod::Month => "Month",
        TrendingPeriod::ThreeMonths => "3 months",
        TrendingPeriod::Year => "Year",
        TrendingPeriod::AllTime => "All time",
    }
}

/// The line under the title for a period.
fn period_description(period: TrendingPeriod) -> &'static str {
    match period {
        TrendingPeriod::Week => "The books the most readers finished on Hardcover this week.",
        TrendingPeriod::Month => "The books the most readers finished on Hardcover this month.",
        TrendingPeriod::ThreeMonths => {
            "The books the most readers finished on Hardcover in the last three months."
        }
        TrendingPeriod::Year => "The books the most readers finished on Hardcover this year.",
        TrendingPeriod::AllTime => "The books the most readers have finished on Hardcover.",
    }
}

enum Loaded {
    Ready(Arc<Vec<BookSummary>>),
    Failed(SharedString),
}

pub struct DiscoverView {
    period: TrendingPeriod,
    /// Periods fetched this session.
    periods: HashMap<TrendingPeriod, Loaded>,
    loading: Option<TrendingPeriod>,
    _task: Option<Task<()>>,
    list: Entity<ListState<CatalogBooks>>,
    /// The period whose books the list holds.
    listed: Option<TrendingPeriod>,
    /// Holds focus while the books load, so shortcuts keep working.
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for DiscoverView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DiscoverView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list = new_list(
            model.clone(),
            "Hardcover lists no books for this period.",
            window,
            cx,
        );
        let subscriptions = follow_list(&list, &model, cx);
        Self {
            period: TrendingPeriod::Month,
            periods: HashMap::new(),
            loading: None,
            _task: None,
            list,
            listed: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Loads the shown period if this session has not yet.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load(false, window, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list once it has books, or the view until then.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_ready() {
            self.list.update(cx, |list, cx| list.focus(window, cx));
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn is_ready(&self) -> bool {
        matches!(self.periods.get(&self.period), Some(Loaded::Ready(_)))
    }

    fn set_period(&mut self, period: TrendingPeriod, window: &mut Window, cx: &mut Context<Self>) {
        if self.period == period {
            return;
        }
        let list_focused = self.list.read(cx).focus_handle(cx).is_focused(window);
        self.period = period;
        self.show(window, cx);
        if list_focused {
            self.focus(window, cx);
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load(true, window, cx);
        cx.notify();
    }

    fn load(&mut self, again: bool, window: &mut Window, cx: &mut Context<Self>) {
        let period = self.period;
        let ready = matches!(self.periods.get(&period), Some(Loaded::Ready(_)));
        if (ready && !again) || self.loading == Some(period) {
            return;
        }
        if !ready {
            self.periods.remove(&period);
        }
        self.loading = Some(period);
        let client = Services::global(cx).client().clone();
        // Replacing the task drops the fetch for a period no longer shown.
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.trending_books(period) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.loading == Some(period) {
                    this.loading = None;
                }
                match result {
                    Ok(books) => {
                        this.periods.insert(period, Loaded::Ready(Arc::new(books)));
                        // Refill even if this period was listed: these are newer.
                        if this.listed == Some(period) {
                            this.listed = None;
                        }
                    }
                    // A failed refresh keeps the books already shown.
                    Err(e) if this.periods.contains_key(&period) => window.push_notification(
                        Notification::error(Services::error_message(&e, Some(READ_CATALOG), cx))
                            .title("Couldn't refresh Discover"),
                        cx,
                    ),
                    Err(e) => {
                        this.periods.insert(
                            period,
                            Loaded::Failed(
                                Services::error_message(&e, Some(READ_CATALOG), cx).into(),
                            ),
                        );
                    }
                }
                let had_focus = this.focus_handle.is_focused(window);
                this.fill_list(window, cx);
                if had_focus && this.is_ready() {
                    this.list.update(cx, |list, cx| list.focus(window, cx));
                }
                cx.notify();
            });
        }));
    }

    fn fill_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let period = self.period;
        if self.listed == Some(period) {
            return;
        }
        let Some(Loaded::Ready(books)) = self.periods.get(&period) else {
            return;
        };
        let rows: Vec<CatalogRow> = books
            .iter()
            .map(|book| CatalogRow {
                details: row_details(&[authors_text(&book.authors)], book),
                book: book.clone(),
            })
            .collect();
        self.listed = Some(period);
        self.list.update(cx, |list, cx| {
            list.delegate_mut().set_rows(rows);
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let refreshing = self.loading == Some(self.period) && self.is_ready();
        let selected = PERIODS.iter().position(|p| *p == self.period).unwrap_or(0);
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Discover"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(period_description(self.period)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        TabBar::new("trending-period")
                            .segmented()
                            .small()
                            .selected_index(selected)
                            .children(PERIODS.map(|p| Tab::new().label(period_label(p))))
                            .on_click(move |ix, window, cx| {
                                let period = PERIODS[*ix];
                                let _ =
                                    view.update(cx, |this, cx| this.set_period(period, window, cx));
                            }),
                    )
                    .child(
                        Button::new("refresh-trending")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label("Refresh")
                            .loading(refreshing)
                            .disabled(self.loading == Some(self.period))
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    ),
            )
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.periods.get(&self.period) {
            Some(Loaded::Ready(_)) => List::new(&self.list).size_full().p_1().into_any_element(),
            Some(Loaded::Failed(message)) => h_flex()
                .px_3()
                .gap_2()
                .text_sm()
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(message.clone()),
                )
                .child(
                    Button::new("retry-trending")
                        .small()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                )
                .into_any_element(),
            None => v_flex()
                .px_3()
                .pt_2()
                .gap_3()
                .children((0..6).map(|i| {
                    h_flex()
                        .id(("trending-skeleton", i as u64))
                        .gap_3()
                        .child(Skeleton::new().w_8().h_12())
                        .child(
                            v_flex()
                                .gap_1()
                                .child(Skeleton::new().h_4().w_48())
                                .child(Skeleton::new().h_3().w_32()),
                        )
                }))
                .into_any_element(),
        }
    }
}

impl Render for DiscoverView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("discover")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .child(
                div()
                    .w_full()
                    .max_w(rems(48.))
                    .px_6()
                    .pt_6()
                    .pb_3()
                    .child(self.render_header(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .max_w(rems(48.))
                    .px_3()
                    .child(self.render_body(cx)),
            )
    }
}
