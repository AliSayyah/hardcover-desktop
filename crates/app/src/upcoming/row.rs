//! One upcoming book: cover, title, its series, the release date, and its
//! Want to Read status or the button that puts it there. Shared by the Home
//! card and the full list, so the two read alike.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Entity, FontWeight, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, div, img, prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{BookSummary, Date, ReadingStatus, UpcomingBook};

use super::schedule::{release_text, series_text, soon_text};
use crate::app_state::{Services, cover_thumbnail};
use crate::catalog::navigation::{self, Page};
use crate::chip::StatusChip;
use crate::format::needs_permission;
use crate::library::model::LibraryModel;

/// The width of the trailing lane, so dates line up whether a row has the
/// chip or the button.
const ACTION_WIDTH: f32 = 8.5;

#[derive(IntoElement)]
pub struct UpcomingRow {
    /// Keeps ids apart where both lists could be built in one frame.
    prefix: &'static str,
    book: UpcomingBook,
    today: Date,
    library: Entity<LibraryModel>,
    /// Draws the rule above it.
    divided: bool,
}

impl UpcomingRow {
    pub fn new(
        prefix: &'static str,
        book: &UpcomingBook,
        today: Date,
        library: Entity<LibraryModel>,
    ) -> Self {
        Self {
            prefix,
            book: book.clone(),
            today,
            library,
            divided: false,
        }
    }

    pub fn divided(mut self, divided: bool) -> Self {
        self.divided = divided;
        self
    }
}

impl RenderOnce for UpcomingRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.book.book.id;
        let prefix = self.prefix;
        let muted = cx.theme().muted_foreground;
        let page = Page::Book(self.book.book.clone());
        let image = self
            .book
            .book
            .cover
            .as_ref()
            .and_then(|c| cover_thumbnail(&c.url, window, cx));
        let library = self.library.read(cx);
        let status = library.entry_for_book(id).map(|e| e.status);
        let adding = library.is_adding(id);
        let can_write = Services::global(cx).has_scope(WRITE_LIBRARY);
        let soon = soon_text(self.book.release_date, self.today);
        let series = series_text(&self.book);

        let action = match status {
            Some(status) => div().child(StatusChip(status)).into_any_element(),
            None => {
                let library = self.library.clone();
                let book = self.book.book.clone();
                Button::new(SharedString::from(format!("{prefix}-add-{id}")))
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .label("Want to read")
                    .loading(adding)
                    .disabled(adding || !can_write)
                    .tooltip(if can_write {
                        "Add to your Want to Read shelf".to_owned()
                    } else {
                        needs_permission(WRITE_LIBRARY)
                    })
                    .on_click(move |_, window, cx| {
                        add_to_want_to_read(library.clone(), book.clone(), window, cx)
                    })
                    .into_any_element()
            }
        };

        h_flex()
            .id(SharedString::from(format!("{prefix}-row-{id}")))
            .gap_3()
            .py_1p5()
            .when(self.divided, |this| {
                this.border_t_1().border_color(cx.theme().border)
            })
            .child(
                div()
                    .id(SharedString::from(format!("{prefix}-cover-{id}")))
                    .flex_shrink_0()
                    .w(rems(1.5))
                    .h(rems(2.25))
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .bg(cx.theme().secondary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click({
                        let page = page.clone();
                        move |_, _, cx| navigation::open(page.clone(), cx)
                    })
                    .map(|this| match image {
                        Some(image) => {
                            this.child(img(image).size_full().object_fit(ObjectFit::Cover))
                        }
                        None => {
                            this.child(Icon::new(IconName::BookOpen).xsmall().text_color(muted))
                        }
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(title_button(prefix, &self.book.book, page))
                    .when(!series.is_empty(), |this| {
                        this.child(div().text_sm().text_color(muted).truncate().child(series))
                    }),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .items_end()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(release_text(self.book.release_date, self.today)),
                    )
                    .when_some(soon, |this, soon| {
                        this.child(div().text_xs().text_color(muted).child(soon))
                    }),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .w(rems(ACTION_WIDTH))
                    .justify_end()
                    .child(action),
            )
    }
}

/// The title as a quiet button that opens the book's page: reachable by
/// keyboard, unlike the cover.
fn title_button(prefix: &str, book: &BookSummary, page: Page) -> impl IntoElement {
    // A row, so the button keeps its content's width instead of stretching
    // (and centring the title); it may still shrink, and the title truncates.
    h_flex().min_w_0().child(
        Button::new(SharedString::from(format!("{prefix}-title-{}", book.id)))
            .text()
            .min_w_0()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(book.title.clone()),
            )
            .on_click(move |_, _, cx| navigation::open(page.clone(), cx)),
    )
}

/// Adds `book` to Want to Read in the background, as the book page does; the
/// library applies the entry the write returns. A failure is shown as a
/// notification.
pub fn add_to_want_to_read(
    library: Entity<LibraryModel>,
    book: BookSummary,
    window: &mut Window,
    cx: &mut App,
) {
    if !library.update(cx, |m, cx| m.begin_add(book.id, cx)) {
        return;
    }
    let client = Services::global(cx).client().clone();
    let book_id = book.id;
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move {
                    client.add_to_library(book_id, ReadingStatus::WantToRead)
                })
                .await;
            library.update(cx, |m, cx| {
                m.end_add(book_id, cx);
                if let Ok(entry) = &result {
                    m.apply_written(entry.clone(), cx);
                }
            });
            if let Err(error) = result {
                let _ = cx.update(|window, cx| {
                    let message = Services::error_message(&error, Some(WRITE_LIBRARY), cx);
                    window.push_notification(
                        Notification::error(message).title("Couldn't add the book"),
                        cx,
                    );
                });
            }
        })
        .detach();
}
