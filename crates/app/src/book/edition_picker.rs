//! The edition picker: a book's editions, fetched a page at a time when the
//! picker first opens, kept for the rest of the run.
//!
//! One picker exists per book, so a response can never land on another book.
//! Choosing an edition emits [`EditionChosen`]; the detail view makes the write.

use crate::ui::Skeleton;
use crate::ui::reveal::RevealScroll as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::{Disableable as _, Icon, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Task, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookId, EditionId, EditionSummary};

use crate::app_state::Services;
use crate::bidi::WrappingText;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::edition_text;
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::open_dialog;

/// The user picked `0`.
pub struct EditionChosen(pub EditionSummary);

pub struct EditionPicker {
    book: BookId,
    book_title: SharedString,
    editions: Vec<EditionSummary>,
    /// The entry's edition, marked in the list.
    current: Option<EditionId>,
    /// More pages may follow the ones fetched.
    has_more: bool,
    /// Where the next page starts: the rows Hardcover sent so far, which
    /// the de-duplicated `editions` may be fewer than.
    next_offset: u32,
    loading: bool,
    error: Option<SharedString>,
    fetch: Option<Task<()>>,
}

impl EventEmitter<EditionChosen> for EditionPicker {}

impl EditionPicker {
    pub fn new(book: BookId, book_title: SharedString, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            book,
            book_title,
            editions: Vec::new(),
            current: None,
            has_more: true,
            next_offset: 0,
            loading: false,
            error: None,
            fetch: None,
        };
        this.fetch_more(cx);
        this
    }

    pub fn set_current(&mut self, current: Option<EditionId>, cx: &mut Context<Self>) {
        if self.current != current {
            self.current = current;
            cx.notify();
        }
    }

    /// Fetches the next page, or retries after a failure.
    fn fetch_more(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.has_more {
            return;
        }
        let offset = self.next_offset;
        self.loading = true;
        self.error = None;
        cx.notify();
        let client = Services::global(cx).client().clone();
        let book = self.book;
        self.fetch = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.book_editions(book, offset) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(page) => {
                        this.has_more = page.has_more;
                        this.next_offset = page.next_offset;
                        // Offset paging can repeat a row if the order shifted between pages.
                        for edition in page.items {
                            if !this.editions.iter().any(|e| e.id == edition.id) {
                                this.editions.push(edition);
                            }
                        }
                    }
                    Err(e) => {
                        this.error =
                            Some(Services::error_message(&e, Some(READ_CATALOG), cx).into());
                    }
                }
                cx.notify();
            });
        }));
    }

    /// Opens `view` in a dialog.
    pub fn open(view: Entity<Self>, window: &mut Window, cx: &mut App) {
        open_dialog(window, cx, move |dialog: Dialog, window, _| {
            dialog
                .title("Choose an edition")
                .w(size::DIALOG_FORM.to_pixels(window.rem_size()))
                .child(view.clone())
        });
    }

    fn render_row(&self, edition: &EditionSummary, cx: &mut Context<Self>) -> AnyElement {
        let muted = colors(cx).fg_muted;
        let current = self.current == Some(edition.id);
        let headline = edition_text(edition).unwrap_or_else(|| "Edition".to_owned());
        let mut details: Vec<String> = Vec::new();
        if let Some(title) = edition.title.as_ref().filter(|t| **t != *self.book_title) {
            details.push(title.clone());
        }
        if let Some(language) = &edition.language {
            details.push(language.clone());
        }
        if let Some(isbn) = &edition.isbn_13 {
            details.push(format!("ISBN {isbn}"));
        }
        let chosen = edition.clone();
        Button::new(ElementId::Name(format!("edition-{}", edition.id).into()))
            .ghost()
            .w_full()
            .h_auto()
            .py_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .items_start()
                            .gap_0p5()
                            .child(
                                div()
                                    .map(|this| {
                                        if current {
                                            this.body_strong()
                                        } else {
                                            this.body()
                                        }
                                    })
                                    .child(WrappingText::new(headline)),
                            )
                            .when(!details.is_empty(), |this| {
                                this.child(
                                    div()
                                        .caption(cx)
                                        .child(WrappingText::new(details.join(" · "))),
                                )
                            }),
                    )
                    .when(current, |this| {
                        this.child(Icon::new(IconName::Check).size_4().text_color(muted))
                    }),
            )
            .on_click(cx.listener(move |_, _, window, cx| {
                cx.emit(EditionChosen(chosen.clone()));
                window.close_dialog(cx);
            }))
            .into_any_element()
    }
}

impl Render for EditionPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        let rows: Vec<AnyElement> = self
            .editions
            .iter()
            .map(|e| self.render_row(e, cx))
            .collect();
        let empty = rows.is_empty() && !self.loading && self.error.is_none();
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .id("edition-list")
                    .max_h(size::DIALOG_LIST_MAX)
                    .scroll_y_revealing_focus()
                    .gap_0p5()
                    .children(rows)
                    .when(self.loading && self.editions.is_empty(), |this| {
                        this.children((0..4).map(|_| {
                            v_flex()
                                .gap_1()
                                .py_2()
                                .child(Skeleton::line().h_4().w_2_3())
                                .child(Skeleton::line().h_3().w_1_3())
                        }))
                    }),
            )
            .when(empty, |this| {
                this.child(
                    div()
                        .body()
                        .text_color(muted)
                        .child("Hardcover lists no editions for this book."),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    h_flex()
                        .gap_2()
                        .body()
                        .child(div().text_color(muted).child(error))
                        .child(
                            Button::new("retry-editions")
                                .small_control()
                                .label("Try again")
                                .on_click(cx.listener(|this, _, _, cx| this.fetch_more(cx))),
                        ),
                )
            })
            .when(
                self.has_more && self.error.is_none() && !self.editions.is_empty(),
                |this| {
                    this.child(
                        div().child(
                            Button::new("more-editions")
                                .small_control()
                                .label("Show more")
                                .loading(self.loading)
                                .disabled(self.loading)
                                .on_click(cx.listener(|this, _, _, cx| this.fetch_more(cx))),
                        ),
                    )
                },
            )
    }
}
