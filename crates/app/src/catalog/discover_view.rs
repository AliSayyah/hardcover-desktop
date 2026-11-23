//! Discover: "For you" and "Top picks" (`vibes::shelves`, with
//! `read:vibes`), then the books the most readers finished on Hardcover in
//! a period, each with its shelf. Each period is kept in the session's
//! catalogue (`resource::Catalog`) and fetched again when stale; the
//! Refresh button fetches the shown one now. In a short window the title and
//! the vibes' rows scroll away on their own; Trending's heading, its period
//! switch and the list stay.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListState;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{Disableable as _, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, Styled as _,
    Subscription, Window, div,
};
use hardcover_core::{BookSummary, TrendingPeriod};

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, PageRows, demand, follow_list, new_list, page_body,
    page_header, page_header_scrolling, page_list, page_loading, row_details,
};
use crate::design::{TypeRoles as _, colors};
use crate::format::authors_text;
use crate::library::model::LibraryModel;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::small_tab;
use crate::ui::{LoadFailed, PageHeader, SectionHeader};
use crate::vibes::shelves::VibeShelves;

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

pub struct DiscoverView {
    period: TrendingPeriod,
    /// The session's catalogue, which holds the periods; `None` once signed
    /// out.
    catalog: Option<Catalog>,
    /// The period being fetched again with the Refresh button, which says
    /// so if that fails; a stale period fetched again on its own is quiet.
    refreshing: Option<TrendingPeriod>,
    list: Entity<ListState<CatalogBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// The period whose books the list holds, and each period's selection.
    rows: PageRows<TrendingPeriod, Vec<BookSummary>>,
    /// "For you" and "Top picks", above the trending books.
    vibes: Entity<VibeShelves>,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
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
        let mut subscriptions = follow_list(&list, model, &pane, window, cx);
        let catalog = Catalog::global(cx);
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe_in(
                &catalog.trending,
                window,
                |this, _, SharedChanged(period): &SharedChanged<TrendingPeriod>, window, cx| {
                    if this.period == *period {
                        this.period_changed(window, cx);
                    }
                },
            ));
        }
        Self {
            period: TrendingPeriod::Month,
            catalog,
            refreshing: None,
            list,
            pane,
            rows: PageRows::default(),
            vibes: cx.new(VibeShelves::new),
            header_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Asks for the shown period if this session has not yet, or again if
    /// it is stale or failed.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request(Demand::Shown, cx);
        self.vibes.update(cx, |v, cx| v.request(Demand::Shown, cx));
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list once it has books, or the view until then.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.books(cx).is_some() {
            crate::ui::enter_list(&self.list, window, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }

    /// The shown period's books, once fetched.
    fn books(&self, cx: &App) -> Option<Arc<Vec<BookSummary>>> {
        self.catalog
            .as_ref()?
            .trending
            .read(cx)
            .resource()
            .value(&self.period)
            .cloned()
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.catalog
            .as_ref()
            .is_some_and(|c| c.trending.read(cx).resource().is_loading(&self.period))
    }

    fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if let Some(catalog) = &self.catalog {
            catalog.request_trending(self.period, demand, cx);
        }
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

    /// The Refresh button, or "Try again": fetches the period now.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refreshing = Some(self.period);
        self.request(Demand::Now, cx);
        cx.notify();
    }

    /// The shown period changed in the catalogue: it loaded, failed, or was
    /// fetched again. A failed Refresh of books already shown says so.
    fn period_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.refreshing == Some(self.period) && !self.is_loading(cx) {
            self.refreshing = None;
            let failed = self.catalog.as_ref().and_then(|c| {
                let store = c.trending.read(cx).resource();
                store
                    .value(&self.period)
                    .and_then(|_| store.failure(&self.period).cloned())
            });
            if let Some(message) = failed {
                window.push_notification(
                    Notification::error(message).title("Couldn't refresh Discover"),
                    cx,
                );
            }
        }
        let had_focus = self.focus_handle.is_focused(window);
        self.fill_list(window, cx);
        if had_focus && self.books(cx).is_some() {
            crate::ui::enter_list(&self.list, window, cx);
        }
        cx.notify();
    }

    fn fill_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let books = self.books(cx);
        self.rows.show(
            self.period,
            books.as_ref(),
            |books| Self::rows(books),
            false,
            &self.list,
            window,
            cx,
        );
    }

    /// The list's rows for a period's books.
    fn rows(books: &[BookSummary]) -> Vec<CatalogRow> {
        books
            .iter()
            .map(|book| CatalogRow {
                details: row_details(&[authors_text(&book.authors)], book),
                book: book.clone(),
            })
            .collect()
    }

    /// Asks for the shown period again if it is stale (`resource`; a failed
    /// one only when `shown_again`), and for the book in the pane beside
    /// the list.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.request(demand(shown_again), cx);
        self.vibes
            .update(cx, |v, cx| v.request(demand(shown_again), cx));
        self.pane.revalidate(shown_again, cx);
    }

    /// The title and the vibes' rows: they scroll away in a short window.
    fn render_header(&self) -> impl IntoElement {
        // The rows keep their own space above and below them, so Trending
        // sits where the first section would without them.
        v_flex()
            .child(PageHeader::new("Discover"))
            .child(self.vibes.clone())
    }

    /// Trending's heading, its period switch and Refresh, between the
    /// header and the list: they stay when a short window scrolls the
    /// header away.
    fn render_trending_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let loading = self.is_loading(cx);
        let refreshing =
            self.refreshing == Some(self.period) && loading && self.books(cx).is_some();
        let selected = PERIODS.iter().position(|p| *p == self.period).unwrap_or(0);
        // 24 under the title, the header's 16 and 8 more.
        page_header().pt_2().child(
            v_flex()
                .gap_4()
                .child(
                    v_flex()
                        .gap_1()
                        .child(SectionHeader::new("Trending"))
                        .child(
                            div()
                                .body()
                                .text_color(colors(cx).fg_muted)
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
                                .small_control()
                                .selected_index(selected)
                                .children(PERIODS.map(|p| small_tab(period_label(p))))
                                .on_click(move |ix, window, cx| {
                                    let period = PERIODS[*ix];
                                    let _ = view
                                        .update(cx, |this, cx| this.set_period(period, window, cx));
                                }),
                        )
                        .child(
                            Button::new("refresh-trending")
                                .ghost()
                                .small_control()
                                .icon(IconName::RefreshCw)
                                .label("Refresh")
                                .loading(refreshing)
                                .disabled(loading)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                ),
        )
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let error = self.catalog.as_ref().and_then(|c| {
            let store = c.trending.read(cx).resource();
            store
                .error(&self.period)
                .filter(|_| !store.is_loading(&self.period))
                .cloned()
        });
        match (self.books(cx), error) {
            (Some(_), _) => page_list(&self.list).into_any_element(),
            (None, Some(message)) => page_header()
                .pt_0()
                .child(
                    LoadFailed::new("retry-trending", message)
                        .on_retry(cx.listener(|this, _, _, cx| this.refresh(cx))),
                )
                .into_any_element(),
            (None, None) => page_loading().into_any_element(),
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
            .child(page_header_scrolling(
                "discover-header",
                &self.header_scroll,
                self.render_header(),
            ))
            .child(self.render_trending_bar(cx))
            .child(page_body().child(self.render_body(cx)));
        self.pane
            .layout(&self.list, CatalogBooks::book_at, column, window, cx)
    }
}
