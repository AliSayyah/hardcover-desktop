//! Trending (`hardcover-design`, Discover → Trending): the books the most
//! readers finished on Hardcover in a period, over Discover, opened from
//! "Trending this month"'s "See all" (`Page::Trending`).
//!
//! - **Header:** the page title "Trending", a muted line saying what the
//!   period's list is, then the period segments (Week, Month, 3 months,
//!   Year, All time) and Refresh. The period is kept with the page in the
//!   page stack (`navigation::replace`), so Back finds it.
//! - **Rows:** catalogue rows with each book's StatusChip, in the shared
//!   frame of `book_list`: the book pane beside the list in a wide window.
//! - Each period is kept in the session's catalogue (`resource::Catalog`)
//!   and fetched again when stale; Refresh fetches the shown one now, and
//!   says so if that fails.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListState;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{Disableable as _, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, ScrollHandle, Styled as _, Subscription, Window,
};
use hardcover_core::{TrendingBook, TrendingPeriod};

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, PageRows, demand, focus_list_if_fallback_is_focused,
    follow_list, new_list, page_body, page_header_scrolling, page_list, page_loading, row_details,
};
use super::navigation::{self, Page};
use crate::format::authors_text;
use crate::library::model::LibraryModel;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::small_tab;
use crate::ui::{LoadFailed, PageHeader};

/// The periods offered, in order.
pub const PERIODS: [TrendingPeriod; 5] = [
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

pub struct TrendingView {
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
    rows: PageRows<TrendingPeriod, Vec<TrendingBook>>,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
    /// Holds focus while the books load; see [`Self::focus`].
    focus_when_ready: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl TrendingView {
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
            header_scroll: ScrollHandle::new(),
            focus_when_ready: None,
            _subscriptions: subscriptions,
        }
    }

    /// Shows `period`'s books: what the catalogue has at once, asking for
    /// them if this session has not yet, or again if stale or failed.
    pub fn show(&mut self, period: TrendingPeriod, window: &mut Window, cx: &mut Context<Self>) {
        self.period = period;
        self.request(Demand::Shown, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list, which renders only once the books are in.
    /// Until then `fallback` holds focus, so Escape and Back keep working,
    /// and the list takes over when the books arrive if `fallback` still
    /// has it.
    pub fn focus(&mut self, fallback: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        if self.books(cx).is_some() {
            self.focus_when_ready = None;
            crate::ui::enter_list(&self.list, window, cx);
        } else {
            self.focus_when_ready = Some(fallback.clone());
            window.focus(fallback, cx);
        }
    }

    /// The shown period's books, once fetched.
    fn books(&self, cx: &App) -> Option<Arc<Vec<TrendingBook>>> {
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

    /// A period segment chose `period`: the page changes in place, and the
    /// page stack keeps the choice for Back.
    fn set_period(&mut self, period: TrendingPeriod, window: &mut Window, cx: &mut Context<Self>) {
        if self.period == period {
            return;
        }
        let list_focused = self.list.read(cx).focus_handle(cx).is_focused(window);
        self.show(period, window, cx);
        navigation::replace(Page::Trending(period), cx);
        if list_focused && self.books(cx).is_some() {
            crate::ui::enter_list(&self.list, window, cx);
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
                    Notification::error(message).title("Couldn't refresh Trending"),
                    cx,
                );
            }
        }
        self.fill_list(window, cx);
        if self.books(cx).is_some() {
            focus_list_if_fallback_is_focused(&self.list, self.focus_when_ready.take(), window, cx);
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
    fn rows(books: &[TrendingBook]) -> Vec<CatalogRow> {
        books
            .iter()
            .map(|t| CatalogRow {
                details: row_details(&[authors_text(&t.book.authors)], &t.book),
                book: t.book.clone(),
            })
            .collect()
    }

    /// Asks for the shown period again if it is stale (`resource`; a failed
    /// one only when `shown_again`), and for the book in the pane beside
    /// the list.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.request(demand(shown_again), cx);
        self.pane.revalidate(shown_again, cx);
    }

    /// The title, the period's line, the period segments and Refresh.
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let loading = self.is_loading(cx);
        let refreshing =
            self.refreshing == Some(self.period) && loading && self.books(cx).is_some();
        let selected = PERIODS.iter().position(|p| *p == self.period).unwrap_or(0);
        v_flex()
            .gap_4()
            .child(PageHeader::new("Trending").line(period_description(self.period)))
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
                            .disabled(loading)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
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
            (None, Some(message)) => LoadFailed::new("retry-trending", message)
                .region("Couldn't load Trending")
                .on_retry(cx.listener(|this, _, _, cx| this.refresh(cx)))
                .into_any_element(),
            (None, None) => page_loading().into_any_element(),
        }
    }
}

impl Render for TrendingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let column = v_flex()
            .id("trending-page")
            .size_full()
            .min_w_0()
            .child(page_header_scrolling(
                "trending-header",
                &self.header_scroll,
                self.render_header(cx),
            ))
            .child(page_body().child(self.render_body(cx)));
        self.pane
            .layout(&self.list, CatalogBooks::book_at, column, window, cx)
    }
}
