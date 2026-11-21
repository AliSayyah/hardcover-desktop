//! Discover: the books the most readers finished on Hardcover in a period,
//! each with its shelf. Each period is fetched once per session (two
//! requests); Refresh fetches the shown one again.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListState;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{Disableable as _, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookSummary, TrendingPeriod};

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, follow_list, new_list, page_body, page_header, page_list,
    page_loading, replace_rows, row_details,
};
use crate::format::authors_text;
use crate::freshness::Fetched;
use crate::library::model::LibraryModel;
use crate::session::{self, SessionScope};
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::small_tab;
use crate::ui::{LoadFailed, PageHeader};

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
        TrendingPeriod::Week => {
            "The books the most readers finished on Hardcover in the last week."
        }
        TrendingPeriod::Month => {
            "The books the most readers finished on Hardcover in the last month."
        }
        TrendingPeriod::ThreeMonths => {
            "The books the most readers finished on Hardcover in the last three months."
        }
        TrendingPeriod::Year => {
            "The books the most readers finished on Hardcover in the last year."
        }
        TrendingPeriod::AllTime => "The books the most readers have finished on Hardcover.",
    }
}

enum Loaded {
    /// Kept for the run, fetched again when stale ([`crate::freshness`]).
    Ready(Fetched<Arc<Vec<BookSummary>>>),
    Failed(SharedString),
}

pub struct DiscoverView {
    period: TrendingPeriod,
    /// Periods fetched this session.
    periods: HashMap<TrendingPeriod, Loaded>,
    loading: Option<TrendingPeriod>,
    /// The fetch running was asked for with the Refresh button, which shows
    /// it; a stale period fetched again on its own does so quietly.
    refreshing: bool,
    task: Option<Task<()>>,
    list: Entity<ListState<CatalogBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
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
    pub fn new(model: &Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = ListPane::new(model.clone(), window, cx);
        let list = new_list(
            model.clone(),
            "Hardcover lists no books for this period.",
            &pane,
            window,
            cx,
        );
        let subscriptions = follow_list(&list, model, &pane, window, cx);
        Self {
            period: TrendingPeriod::Month,
            periods: HashMap::new(),
            loading: None,
            refreshing: false,
            task: None,
            list,
            pane,
            listed: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Loads the shown period if this session has not yet, or again if it
    /// is stale.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load(false, window, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list once it has books, or the view until then.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_ready() {
            crate::ui::enter_list(&self.list, window, cx);
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

    /// Fetches the shown period's books if there are none, or they are
    /// stale, or `again` (the Refresh button). Books already shown stay
    /// while they are fetched again.
    fn load(&mut self, again: bool, window: &mut Window, cx: &mut Context<Self>) {
        let period = self.period;
        let (ready, stale) = match self.periods.get(&period) {
            Some(Loaded::Ready(fetched)) => (true, fetched.is_stale(cx)),
            Some(Loaded::Failed(_)) | None => (false, false),
        };
        if (ready && !again && !stale) || self.loading == Some(period) {
            return;
        }
        if !ready {
            self.periods.remove(&period);
        }
        self.loading = Some(period);
        self.refreshing = again;
        let Some(session) = SessionScope::current(cx) else {
            return;
        };
        let books = session.interactive(move |client| client.trending_books(period));
        // Replacing the task drops the fetch for a period no longer shown.
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let Some(result) = books.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.loading == Some(period) {
                    this.loading = None;
                }
                let asked = std::mem::take(&mut this.refreshing);
                let result = result.map(Arc::new);
                match (this.periods.get_mut(&period), result) {
                    // A failed refresh keeps the books already shown; only
                    // the Refresh button says so.
                    (Some(Loaded::Ready(fetched)), Err(e)) => {
                        fetched.refetched(Err(()), cx);
                        // Records a refused scope even when nothing is said.
                        let message = session::error_message(&e, Some(READ_CATALOG), cx);
                        if asked {
                            window.push_notification(
                                Notification::error(message).title("Couldn't refresh Discover"),
                                cx,
                            );
                        }
                    }
                    // Newer books replace the rows, keeping the selected one.
                    (Some(Loaded::Ready(fetched)), Ok(books)) => {
                        if fetched.refetched(Ok::<_, ()>(books), cx)
                            && this.listed == Some(period)
                            && let Some(rows) = this.rows(period)
                        {
                            replace_rows(&this.list, rows, window, cx);
                        }
                    }
                    (_, Ok(books)) => {
                        this.periods
                            .insert(period, Loaded::Ready(Fetched::new(books, cx)));
                    }
                    (_, Err(e)) => {
                        this.periods.insert(
                            period,
                            Loaded::Failed(
                                session::error_message(&e, Some(READ_CATALOG), cx).into(),
                            ),
                        );
                    }
                }
                let had_focus = this.focus_handle.is_focused(window);
                this.fill_list(window, cx);
                if had_focus && this.is_ready() {
                    crate::ui::enter_list(&this.list, window, cx);
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
        let Some(rows) = self.rows(period) else {
            return;
        };
        self.listed = Some(period);
        self.list.update(cx, |list, cx| {
            list.delegate_mut().set_rows(rows);
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
    }

    /// The list's rows for a loaded period.
    fn rows(&self, period: TrendingPeriod) -> Option<Vec<CatalogRow>> {
        let Some(Loaded::Ready(books)) = self.periods.get(&period) else {
            return None;
        };
        let rows = books
            .value()
            .iter()
            .map(|book| CatalogRow {
                details: row_details(&[authors_text(&book.authors)], book),
                book: book.clone(),
            })
            .collect();
        Some(rows)
    }

    /// Fetches the shown period again if it is stale ([`crate::freshness`]),
    /// and the book in the pane beside the list.
    /// A failed period is tried again only with `shown_again`.
    pub fn revalidate(&mut self, shown_again: bool, window: &mut Window, cx: &mut Context<Self>) {
        let failed = matches!(self.periods.get(&self.period), Some(Loaded::Failed(_)));
        if shown_again || !failed {
            self.load(false, window, cx);
        }
        self.pane.revalidate(shown_again, cx);
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let refreshing = self.refreshing && self.loading == Some(self.period) && self.is_ready();
        let selected = PERIODS.iter().position(|p| *p == self.period).unwrap_or(0);
        v_flex()
            .gap_4()
            .child(PageHeader::new("Discover").line(period_description(self.period)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        TabBar::new("trending-period")
                            .segmented()
                            .small_control()
                            .selected_index(selected)
                            .children(PERIODS.map(|p| small_tab(period_label(p))))
                            .on_click(move |ix, window, cx| {
                                let period = PERIODS[*ix];
                                let _ =
                                    view.update(cx, |this, cx| this.set_period(period, window, cx));
                            }),
                    )
                    .child(
                        Button::new("refresh-trending")
                            .ghost()
                            .small_control()
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
            Some(Loaded::Ready(_)) => page_list(&self.list).into_any_element(),
            Some(Loaded::Failed(message)) => page_header()
                .pt_0()
                .child(
                    LoadFailed::new("retry-trending", message.clone())
                        .on_retry(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                )
                .into_any_element(),
            None => page_loading().into_any_element(),
        }
    }
}

impl Render for DiscoverView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let column = v_flex()
            .id("discover")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .child(page_header().child(self.render_header(cx)))
            .child(page_body().child(self.render_body(cx)));
        self.pane
            .layout(&self.list, CatalogBooks::book_at, column, window, cx)
    }
}
