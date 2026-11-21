//! The edition picker: a book's editions, fetched a page at a time when the
//! picker opens, from the session's catalogue (`resource::Catalog`), so
//! every view's picker for the book shares them.
//!
//! One picker exists per book and view; it keeps only which edition is the
//! entry's. Choosing an edition emits [`EditionChosen`]; the view that opened
//! it makes the write.

use crate::ui::{LoadFailed, Skeleton};
use crate::ui::reveal::RevealScroll as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::{Disableable as _, Icon, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::{BookId, EditionId, EditionSummary};

use crate::bidi::WrappingText;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::edition_text;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::open_dialog;

/// The user picked `0`.
pub struct EditionChosen(pub EditionSummary);

pub struct EditionPicker {
    book: BookId,
    book_title: SharedString,
    catalog: Option<Catalog>,
    /// The entry's edition, marked in the list.
    current: Option<EditionId>,
    _subscription: Option<Subscription>,
}

impl EventEmitter<EditionChosen> for EditionPicker {}

impl EditionPicker {
    pub fn new(book: BookId, book_title: SharedString, cx: &mut Context<Self>) -> Self {
        let catalog = Catalog::global(cx);
        let subscription = catalog.as_ref().map(|catalog| {
            cx.subscribe(
                &catalog.editions,
                move |_, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if *id == book {
                        cx.notify();
                    }
                },
            )
        });
        if let Some(catalog) = &catalog {
            catalog.request_editions(book, Demand::Shown, cx);
        }
        Self {
            book,
            book_title,
            catalog,
            current: None,
            _subscription: subscription,
        }
    }

    pub fn set_current(&mut self, current: Option<EditionId>, cx: &mut Context<Self>) {
        if self.current != current {
            self.current = current;
            cx.notify();
        }
    }

    /// Fetches the next page, or the first again after it failed.
    fn fetch_more(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        let loaded = catalog
            .editions
            .read(cx)
            .resource()
            .value(&self.book)
            .is_some();
        if loaded {
            catalog.more_editions(self.book, cx);
        } else {
            catalog.request_editions(self.book, Demand::Now, cx);
        }
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
        let store = self
            .catalog
            .as_ref()
            .map(|c| c.editions.read(cx).resource());
        let paged = store.and_then(|s| s.value(&self.book)).cloned();
        let loading = store.is_some_and(|s| s.is_loading(&self.book));
        let error = store.and_then(|s| {
            s.error(&self.book)
                .or_else(|| s.more_error(&self.book))
                .cloned()
        });
        let has_more = paged.as_ref().is_some_and(|p| p.has_more);
        let editions: &[EditionSummary] = paged.as_deref().map_or(&[], |p| p.items.as_slice());
        let rows: Vec<AnyElement> = editions.iter().map(|e| self.render_row(e, cx)).collect();
        let empty = rows.is_empty() && !loading && error.is_none();
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .id("edition-list")
                    .max_h(size::DIALOG_LIST_MAX)
                    .scroll_y_revealing_focus()
                    .gap_0p5()
                    .children(rows)
                    .when(loading && editions.is_empty(), |this| {
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
            .when_some(error.clone(), |this, error| {
                this.child(
                    LoadFailed::new("retry-editions", error)
                        .on_retry(cx.listener(|this, _, _, cx| this.fetch_more(cx))),
                )
            })
            .when(
                has_more && error.is_none() && !editions.is_empty(),
                |this| {
                    this.child(
                        div().child(
                            Button::new("more-editions")
                                .small_control()
                                .label("Show more")
                                .loading(loading)
                                .disabled(loading)
                                .on_click(cx.listener(|this, _, _, cx| this.fetch_more(cx))),
                        ),
                    )
                },
            )
    }
}
