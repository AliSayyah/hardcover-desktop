//! A series page: its books in reading order with the reader's progress.
//!
//! The header has the series' name as the page title, "by" its author (a text
//! button to their page), the facts and how many of the books the reader has
//! read, the description in `prose`, and "Open on Hardcover". The books
//! follow as catalogue rows.

use std::sync::Arc;

use gpui_kit::ScrollHandle;
use gpui_kit::component::list::ListState;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::{SeriesDetails, SeriesId};

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, PageRows, demand, focus_list_if_fallback_is_focused,
    follow_list, new_list, page_body, page_header_scrolling, page_list, read_summary, reset_scroll,
    row_details,
};
use super::navigation::Page;
use crate::bidi::WrappingText;
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::series_position_text;
use crate::library::model::LibraryModel;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::{ExternalLink, LoadFailed, PageHeader, Skeleton, TextButton};

pub struct SeriesView {
    model: Entity<LibraryModel>,
    /// The session's catalogue, which holds the pages; `None` once signed
    /// out.
    catalog: Option<Catalog>,
    current: Option<SeriesId>,
    /// Holds focus while the page loads; see [`Self::focus`].
    focus_when_ready: Option<FocusHandle>,
    list: Entity<ListState<CatalogBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// The series whose books the list holds, and each page's selection.
    rows: PageRows<SeriesId, SeriesDetails>,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl SeriesView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = ListPane::new(model.clone(), window, cx);
        let list = new_list(
            model.clone(),
            "Hardcover lists no books in this series.",
            &pane,
            window,
            cx,
        );
        let mut subscriptions = follow_list(&list, &model, &pane, window, cx);
        let catalog = Catalog::global(cx);
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe_in(
                &catalog.series,
                window,
                |this, _, SharedChanged(id): &SharedChanged<SeriesId>, window, cx| {
                    if this.current == Some(*id) {
                        this.page_changed(window, cx);
                    }
                },
            ));
        }
        Self {
            model,
            catalog,
            current: None,
            focus_when_ready: None,
            list,
            pane,
            rows: PageRows::default(),
            header_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, id: SeriesId, window: &mut Window, cx: &mut Context<Self>) {
        if self.current != Some(id) {
            reset_scroll(&self.header_scroll);
        }
        self.current = Some(id);
        self.request(Demand::Shown, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list, which renders only once the page has loaded.
    /// Until then `fallback` holds focus, so Escape and Back keep working, and
    /// the list takes over when the page arrives if `fallback` still has it.
    pub fn focus(&mut self, fallback: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        if self.page(cx).is_some() {
            self.focus_when_ready = None;
            crate::ui::enter_list(&self.list, window, cx);
        } else {
            self.focus_when_ready = Some(fallback.clone());
            window.focus(fallback, cx);
        }
    }

    /// The current page, once fetched.
    fn page(&self, cx: &App) -> Option<Arc<SeriesDetails>> {
        let id = self.current?;
        self.catalog
            .as_ref()?
            .series
            .read(cx)
            .resource()
            .value(&id)
            .cloned()
    }

    fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if let (Some(id), Some(catalog)) = (self.current, &self.catalog) {
            catalog.request_series(id, demand, cx);
        }
    }

    /// The page shown changed in the catalogue: it loaded, failed, or was
    /// fetched again.
    fn page_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_list(window, cx);
        if self.page(cx).is_some() {
            focus_list_if_fallback_is_focused(&self.list, self.focus_when_ready.take(), window, cx);
        }
        cx.notify();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Now, cx);
        cx.notify();
    }

    fn fill_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        let page = self.page(cx);
        self.rows
            .show(id, page.as_ref(), Self::rows, false, &self.list, window, cx);
    }

    /// The list's rows for a loaded page.
    fn rows(series: &SeriesDetails) -> Vec<CatalogRow> {
        series
            .books
            .iter()
            .map(|b| CatalogRow {
                details: row_details(&[b.position.map_or_default(series_position_text)], &b.book),
                book: b.book.clone(),
            })
            .collect()
    }

    /// Asks for the page again if it is stale (`resource`; a failed one only
    /// when `shown_again`), and for the book in the pane beside the list.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.request(demand(shown_again), cx);
        self.pane.revalidate(shown_again, cx);
    }

    fn render_header(&self, series: &SeriesDetails, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        // The main (whole-numbered) books; a series without numbers counts all.
        let main: Vec<_> = series.books.iter().filter(|b| b.is_main()).collect();
        let counted: Vec<_> = if main.is_empty() {
            series.books.iter().collect()
        } else {
            main
        };
        let mut facts: Vec<String> = Vec::new();
        match counted.len() {
            0 => {}
            1 => facts.push("1 book".into()),
            n => facts.push(format!("{n} books")),
        }
        let extra = series.books.len() - counted.len();
        if extra > 0 {
            facts.push(format!("{extra} more in the series"));
        }
        match series.is_completed {
            Some(true) => facts.push("Complete".into()),
            Some(false) => facts.push("Ongoing".into()),
            None => {}
        }
        let summary = read_summary(
            counted
                .iter()
                .map(|b| model.entry_for_book(b.book.id).map(|e| e.status)),
        );
        let muted = colors(cx).fg_muted;
        let mut header = PageHeader::new(series.name.clone()).wraps();
        if let Some(author) = series.author.clone() {
            header = header.under(
                h_flex()
                    .gap_1()
                    .body()
                    .child(div().text_color(muted).child("by"))
                    .child(
                        TextButton::new("series-author", author.name.clone())
                            .opens(Page::Author(author.id)),
                    ),
            );
        }
        if !facts.is_empty() {
            header = header.facts(facts.join(" · "));
        }
        if let Some(summary) = summary {
            header = header.under(div().body().tabular().child(summary));
        }
        v_flex()
            .gap_4()
            .child(header)
            .when_some(series.description.clone(), |this, text| {
                this.child(
                    div()
                        .max_w(size::PROSE)
                        .prose()
                        .child(WrappingText::new(text).line_clamp(3)),
                )
            })
            .when_some(series.web_url(), |this, url| {
                this.child(ExternalLink::new(
                    "series-on-hardcover",
                    "Open on Hardcover",
                    url,
                ))
            })
            .into_any_element()
    }
}

impl SeriesView {
    /// The name of a page already loaded this session, for Back's label.
    pub fn name<'a>(&self, id: SeriesId, cx: &'a App) -> Option<&'a str> {
        let series = self
            .catalog
            .as_ref()?
            .series
            .read(cx)
            .resource()
            .value(&id)?;
        Some(&series.name)
    }
}

impl Render for SeriesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.page(cx);
        let error = match (self.current, &self.catalog) {
            (Some(id), Some(catalog)) => {
                let store = catalog.series.read(cx).resource();
                store.error(&id).filter(|_| !store.is_loading(&id)).cloned()
            }
            _ => None,
        };
        let header = match (&page, error) {
            (Some(series), _) => self.render_header(series, cx),
            (None, Some(message)) => LoadFailed::new("retry-series", message)
                .on_retry(cx.listener(|this, _, _, cx| this.retry(cx)))
                .into_any_element(),
            (None, None) => v_flex()
                .gap_2()
                .child(Skeleton::line_in(TypeRole::PageTitle).w_64())
                .child(Skeleton::line().w_48())
                .child(Skeleton::line().w_32())
                .into_any_element(),
        };
        let ready = page.is_some();
        let column = v_flex()
            .size_full()
            .min_w_0()
            .child(page_header_scrolling(
                "series-header",
                &self.header_scroll,
                header,
            ))
            .when(ready, |this| {
                this.child(page_body().child(page_list(&self.list)))
            });
        self.pane
            .layout(&self.list, CatalogBooks::book_at, column, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui_kit::component::IndexPath;
    use gpui_kit::component::list::ListDelegate as _;
    use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
    use hardcover_core::BookId;
    use hardcover_core::store::LibraryCache;

    use super::*;
    use crate::session::SessionScope;

    /// Two demo series with at least three books each.
    fn demo_series() -> (SeriesId, SeriesId) {
        let client = hardcover_core::demo::client();
        let me = client.me().unwrap();
        let entries = client.library_with_progress(me.id, |_| {}).unwrap().entries;
        let mut ids: Vec<SeriesId> = entries
            .iter()
            .flat_map(|e| e.series.iter().filter_map(|s| s.id))
            .filter(|id| client.series(*id).is_ok_and(|s| s.books.len() >= 3))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert!(ids.len() >= 2, "two demo series with three books");
        (ids[0], ids[1])
    }

    fn settle(cx: &mut VisualTestContext) {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    fn selected_book(
        view: &Entity<SeriesView>,
        cx: &mut VisualTestContext,
    ) -> Option<(usize, BookId)> {
        view.read_with(cx, |view, cx| {
            let list = view.list.read(cx);
            let row = list.selected_index()?.row;
            Some((row, list.delegate().book_at(row)?.id))
        })
    }

    fn rows(view: &Entity<SeriesView>, cx: &mut VisualTestContext) -> usize {
        view.read_with(cx, |view, cx| {
            view.list.read(cx).delegate().items_count(0, cx)
        })
    }

    fn series_view(cx: &mut TestAppContext) -> (Entity<SeriesView>, &mut VisualTestContext) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        cx.update(|cx| {
            gpui_kit::init(cx);
            session.install(cx);
            Catalog::init(&session, cx);
        });
        let model = cx.new(|_| LibraryModel::new(session));
        cx.add_window_view(|window, cx| SeriesView::new(model, window, cx))
    }

    fn select(view: &Entity<SeriesView>, row: usize, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.list.update(cx, |list, cx| {
                    list.set_selected_index(Some(IndexPath::new(row)), window, cx);
                });
            });
        });
    }

    #[gpui_kit::test]
    fn a_page_fetched_again_keeps_the_selected_book(cx: &mut TestAppContext) {
        let (id, _) = demo_series();
        let (view, cx) = series_view(cx);
        cx.update(|window, cx| view.update(cx, |view, cx| view.show(id, window, cx)));
        settle(cx);

        // As if the page had been fetched with its books the other way
        // round, the reader having selected the first row.
        let catalog = view.read_with(cx, |view, _| view.catalog.clone().unwrap());
        let count = catalog.series.update(cx, |store, cx| {
            let mut older = (**store.resource().value(&id).unwrap()).clone();
            older.books.reverse();
            let count = older.books.len();
            store.set(id, older, cx);
            count
        });
        cx.run_until_parked();
        select(&view, 0, cx);
        let (_, book) = selected_book(&view, cx).unwrap();

        cx.update(|_, cx| {
            crate::resource::refresh_all(cx);
            view.update(cx, |view, cx| view.revalidate(false, cx));
        });
        settle(cx);
        assert_eq!(
            selected_book(&view, cx),
            Some((count - 1, book)),
            "the same book, now in the last row"
        );
    }

    /// D15: another page doesn't show the rows (or the pane's book) of the
    /// page left while it loads, and Back finds the book selected there.
    #[gpui_kit::test]
    fn another_page_starts_empty_and_back_finds_the_selection(cx: &mut TestAppContext) {
        let (first, second) = demo_series();
        let (view, cx) = series_view(cx);
        let show = |id, cx: &mut VisualTestContext| {
            cx.update(|window, cx| view.update(cx, |view, cx| view.show(id, window, cx)));
        };
        show(first, cx);
        settle(cx);
        select(&view, 1, cx);
        let chosen = selected_book(&view, cx).unwrap();

        show(second, cx);
        assert_eq!(rows(&view, cx), 0, "nothing of the page left");
        assert_eq!(selected_book(&view, cx), None);
        settle(cx);
        assert!(rows(&view, cx) >= 3);
        assert_eq!(selected_book(&view, cx), None, "a new page selects nothing");

        show(first, cx);
        assert_eq!(
            selected_book(&view, cx),
            Some(chosen),
            "Back finds it again"
        );
    }
}
