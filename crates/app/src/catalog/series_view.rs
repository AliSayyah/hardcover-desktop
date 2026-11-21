//! A series page: its books in reading order with the reader's progress.
//!
//! The header has the series' name as the page title, "by" its author (a text
//! button to their page), the facts and how many of the books the reader has
//! read, the description in `prose`, and "Open on Hardcover". The books
//! follow as catalogue rows.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::ScrollHandle;
use gpui_kit::component::button::Button;
use gpui_kit::component::list::ListState;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{SeriesDetails, SeriesId};

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, focus_list_if_fallback_is_focused, follow_list,
    load_failed, new_list, page_body, page_header_scrolling, page_list, read_summary, replace_rows,
    reset_scroll, row_details,
};
use super::navigation::Page;
use crate::app_state::Services;
use crate::bidi::WrappingText;
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::series_position_text;
use crate::freshness::Fetched;
use crate::library::model::LibraryModel;
use crate::ui::kit::SmallControl as _;
use crate::ui::{ExternalLink, Skeleton, TextButton};

enum Loaded {
    /// Kept for the run, fetched again when stale ([`crate::freshness`]).
    Ready(Fetched<Arc<SeriesDetails>>),
    Failed(SharedString),
}

pub struct SeriesView {
    model: Entity<LibraryModel>,
    current: Option<SeriesId>,
    /// Pages fetched this session, so Back costs no request.
    pages: HashMap<SeriesId, Loaded>,
    loading: Option<SeriesId>,
    task: Option<Task<()>>,
    /// Holds focus while the page loads; see [`Self::focus`].
    focus_when_ready: Option<FocusHandle>,
    list: Entity<ListState<CatalogBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// The series whose books the list holds.
    listed: Option<SeriesId>,
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
        let subscriptions = follow_list(&list, &model, &pane, window, cx);
        Self {
            model,
            current: None,
            pages: HashMap::new(),
            loading: None,
            task: None,
            focus_when_ready: None,
            list,
            pane,
            listed: None,
            header_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, id: SeriesId, window: &mut Window, cx: &mut Context<Self>) {
        if self.current != Some(id) {
            reset_scroll(&self.header_scroll);
        }
        self.current = Some(id);
        self.load(window, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list, which renders only once the page has loaded.
    /// Until then `fallback` holds focus, so Escape and Back keep working, and
    /// the list takes over when the page arrives if `fallback` still has it.
    pub fn focus(&mut self, fallback: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_ready() {
            self.focus_when_ready = None;
            crate::ui::enter_list(&self.list, window, cx);
        } else {
            self.focus_when_ready = Some(fallback.clone());
            window.focus(fallback, cx);
        }
    }

    fn is_ready(&self) -> bool {
        self.current
            .is_some_and(|id| matches!(self.pages.get(&id), Some(Loaded::Ready(_))))
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        let revalidating = match self.pages.get(&id) {
            Some(Loaded::Ready(fetched)) if !fetched.is_stale(cx) => return,
            Some(Loaded::Ready(_)) => true,
            Some(Loaded::Failed(_)) | None => false,
        };
        if self.loading == Some(id) {
            return;
        }
        // A stale page stays on screen while it is fetched again.
        if !revalidating {
            self.pages.remove(&id);
        }
        self.loading = Some(id);
        let client = Services::global(cx).client().clone();
        // Replacing the task drops the fetch for a page no longer shown.
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { client.series(id) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.loading == Some(id) {
                    this.loading = None;
                }
                let result = result.map(Arc::new);
                let changed = match (this.pages.get_mut(&id), result) {
                    // A refetch: a failure keeps the page shown, and only
                    // records a refused scope.
                    (Some(Loaded::Ready(fetched)), result) => {
                        if let Err(e) = &result {
                            Services::error_message(e, Some(READ_CATALOG), cx);
                        }
                        fetched.refetched(result, cx)
                    }
                    (_, Ok(page)) => {
                        this.pages.insert(id, Loaded::Ready(Fetched::new(page, cx)));
                        false
                    }
                    (_, Err(e)) => {
                        let message = Services::error_message(&e, Some(READ_CATALOG), cx);
                        this.pages.insert(id, Loaded::Failed(message.into()));
                        false
                    }
                };
                if changed
                    && this.listed == Some(id)
                    && let Some(rows) = this.rows(id)
                {
                    replace_rows(&this.list, rows, window, cx);
                }
                this.fill_list(window, cx);
                if this.is_ready() {
                    focus_list_if_fallback_is_focused(
                        &this.list,
                        this.focus_when_ready.take(),
                        window,
                        cx,
                    );
                }
                cx.notify();
            });
        }));
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = None;
        self.load(window, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    fn fill_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        if self.listed == Some(id) {
            return;
        }
        let Some(rows) = self.rows(id) else {
            return;
        };
        self.listed = Some(id);
        self.list.update(cx, |list, cx| {
            list.delegate_mut().set_rows(rows);
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
    }

    /// The list's rows for a loaded page.
    fn rows(&self, id: SeriesId) -> Option<Vec<CatalogRow>> {
        let Some(Loaded::Ready(series)) = self.pages.get(&id) else {
            return None;
        };
        let rows = series
            .value()
            .books
            .iter()
            .map(|b| CatalogRow {
                details: row_details(&[b.position.map_or_default(series_position_text)], &b.book),
                book: b.book.clone(),
            })
            .collect();
        Some(rows)
    }

    /// Fetches the page again if it is stale ([`crate::freshness`]), and
    /// the book in the pane beside the list.
    /// A failed page is tried again only with `shown_again`.
    pub fn revalidate(&mut self, shown_again: bool, window: &mut Window, cx: &mut Context<Self>) {
        let failed = self
            .current
            .is_some_and(|id| matches!(self.pages.get(&id), Some(Loaded::Failed(_))));
        if shown_again || !failed {
            self.load(window, cx);
        }
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
        v_flex()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .page_title()
                            .child(WrappingText::new(series.name.clone())),
                    )
                    .when_some(series.author.clone(), |this, author| {
                        this.child(
                            h_flex()
                                .gap_1()
                                .body()
                                .child(div().text_color(muted).child("by"))
                                .child(
                                    TextButton::new("series-author", author.name.clone())
                                        .opens(Page::Author(author.id)),
                                ),
                        )
                    })
                    .when(!facts.is_empty(), |this| {
                        this.child(
                            div()
                                .body()
                                .tabular()
                                .text_color(muted)
                                .child(facts.join(" · ")),
                        )
                    })
                    .when_some(summary, |this, summary| {
                        this.child(div().body().tabular().child(summary))
                    }),
            )
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
    pub fn name(&self, id: SeriesId) -> Option<&str> {
        match self.pages.get(&id)? {
            Loaded::Ready(details) => Some(&details.value().name),
            Loaded::Failed(_) => None,
        }
    }
}

impl Render for SeriesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current.and_then(|id| self.pages.get(&id));
        let header = match current {
            Some(Loaded::Ready(series)) => self.render_header(&series.value().clone(), cx),
            Some(Loaded::Failed(message)) => load_failed(
                message.clone(),
                Button::new("retry-series")
                    .small_control()
                    .label("Try again")
                    .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
                cx,
            )
            .into_any_element(),
            None => v_flex()
                .gap_2()
                .child(Skeleton::line_in(TypeRole::PageTitle).w_64())
                .child(Skeleton::line().w_48())
                .child(Skeleton::line().w_32())
                .into_any_element(),
        };
        let ready = matches!(current, Some(Loaded::Ready(_)));
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
    use gpui_kit::{TestAppContext, VisualTestContext};
    use hardcover_core::BookId;
    use hardcover_core::auth::GrantedScopes;
    use hardcover_core::store::LibraryCache;

    use super::*;

    /// A demo series with at least three books.
    fn demo_series() -> SeriesId {
        let client = hardcover_core::demo::client();
        let me = client.me().unwrap();
        let entries = client.library_with_progress(me.id, |_| {}).unwrap().entries;
        entries
            .iter()
            .flat_map(|e| e.series.iter().filter_map(|s| s.id))
            .find(|id| client.series(*id).is_ok_and(|s| s.books.len() >= 3))
            .expect("a demo series with three books")
    }

    fn settle(cx: &mut VisualTestContext) {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    fn selected_book(view: &Entity<SeriesView>, cx: &mut VisualTestContext) -> (usize, BookId) {
        view.read_with(cx, |view, cx| {
            let list = view.list.read(cx);
            let row = list.selected_index().unwrap().row;
            (row, list.delegate().book_at(row).unwrap().id)
        })
    }

    #[gpui_kit::test]
    fn a_page_fetched_again_keeps_the_selected_book(cx: &mut TestAppContext) {
        let id = demo_series();
        cx.update(|cx| {
            gpui_kit::init(cx);
            let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
            let client = hardcover_core::demo::client();
            cx.set_global(Services::new(client, None, cache, GrantedScopes::all()));
        });
        let model = cx.new(|_| LibraryModel::new());
        let (view, cx) = cx.add_window_view(|window, cx| SeriesView::new(model, window, cx));
        cx.update(|window, cx| view.update(cx, |view, cx| view.show(id, window, cx)));
        settle(cx);

        // As if the page had been fetched with its books the other way
        // round, the reader having selected the first row.
        let count = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let Some(Loaded::Ready(fetched)) = view.pages.get(&id) else {
                    panic!("the page loaded");
                };
                let mut older = (**fetched.value()).clone();
                older.books.reverse();
                let count = older.books.len();
                view.pages
                    .insert(id, Loaded::Ready(Fetched::new(Arc::new(older), cx)));
                view.listed = None;
                view.fill_list(window, cx);
                view.list.update(cx, |list, cx| {
                    list.set_selected_index(Some(IndexPath::new(0)), window, cx);
                });
                count
            })
        });
        let (_, book) = selected_book(&view, cx);

        cx.update(|window, cx| {
            crate::freshness::refresh_all(cx);
            view.update(cx, |view, cx| view.revalidate(false, window, cx));
        });
        settle(cx);
        assert_eq!(
            selected_book(&view, cx),
            (count - 1, book),
            "the same book, now in the last row"
        );
    }
}
