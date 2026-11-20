//! A series page: its books in reading order with the reader's progress.
//!
//! The header has the series' name as the page title, "by" its author (a text
//! button to their page), the facts and how many of the books the reader has
//! read, the description in `prose`, and "Open on Hardcover". The books
//! follow as catalogue rows.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::component::list::ListState;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{SeriesDetails, SeriesId};

use super::book_list::{
    CatalogBooks, CatalogRow, follow_list, load_failed, new_list, page_body, page_header_scrolling,
    page_list, read_summary, row_details,
};
use super::navigation::Page;
use crate::app_state::Services;
use crate::bidi::WrappingText;
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::series_position_text;
use crate::library::model::LibraryModel;
use crate::ui::{ExternalLink, Skeleton, TextButton};

enum Loaded {
    Ready(Arc<SeriesDetails>),
    Failed(SharedString),
}

pub struct SeriesView {
    model: Entity<LibraryModel>,
    current: Option<SeriesId>,
    /// Pages fetched this session, so Back costs no request.
    pages: HashMap<SeriesId, Loaded>,
    loading: Option<SeriesId>,
    _task: Option<Task<()>>,
    /// Holds focus while the page loads; see [`Self::focus`].
    focus_when_ready: Option<FocusHandle>,
    list: Entity<ListState<CatalogBooks>>,
    /// The series whose books the list holds.
    listed: Option<SeriesId>,
    _subscriptions: Vec<Subscription>,
}

impl SeriesView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list = new_list(
            model.clone(),
            "Hardcover lists no books in this series.",
            window,
            cx,
        );
        let subscriptions = follow_list(&list, &model, window, cx);
        Self {
            model,
            current: None,
            pages: HashMap::new(),
            loading: None,
            _task: None,
            focus_when_ready: None,
            list,
            listed: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, id: SeriesId, window: &mut Window, cx: &mut Context<Self>) {
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
            self.list.update(cx, |list, cx| list.focus(window, cx));
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
        if matches!(self.pages.get(&id), Some(Loaded::Ready(_))) || self.loading == Some(id) {
            return;
        }
        self.pages.remove(&id);
        self.loading = Some(id);
        let client = Services::global(cx).client().clone();
        // Replacing the task drops the fetch for a page no longer shown.
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { client.series(id) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.loading == Some(id) {
                    this.loading = None;
                }
                let page = match result {
                    Ok(series) => Loaded::Ready(Arc::new(series)),
                    Err(e) => {
                        Loaded::Failed(Services::error_message(&e, Some(READ_CATALOG), cx).into())
                    }
                };
                this.pages.insert(id, page);
                this.fill_list(window, cx);
                if this.is_ready()
                    && let Some(fallback) = this.focus_when_ready.take()
                    && fallback.is_focused(window)
                {
                    this.list.update(cx, |list, cx| list.focus(window, cx));
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
        let Some(Loaded::Ready(series)) = self.pages.get(&id) else {
            return;
        };
        let rows: Vec<CatalogRow> = series
            .books
            .iter()
            .map(|b| CatalogRow {
                details: row_details(
                    &[b.position.map(series_position_text).unwrap_or_default()],
                    &b.book,
                ),
                book: b.book.clone(),
            })
            .collect();
        self.listed = Some(id);
        self.list.update(cx, |list, cx| {
            list.delegate_mut().set_rows(rows);
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
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
            Loaded::Ready(details) => Some(&details.name),
            Loaded::Failed(_) => None,
        }
    }
}

impl Render for SeriesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current.and_then(|id| self.pages.get(&id));
        let header = match current {
            Some(Loaded::Ready(series)) => self.render_header(&series.clone(), cx),
            Some(Loaded::Failed(message)) => load_failed(
                message.clone(),
                Button::new("retry-series")
                    .small()
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
        v_flex()
            .size_full()
            .min_w_0()
            .child(page_header_scrolling("series-header", header))
            .when(ready, |this| {
                this.child(page_body().child(page_list(&self.list)))
            })
    }
}
