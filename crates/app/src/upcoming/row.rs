//! The upcoming row (`hardcover-design`, Rows → Upcoming row), shared by
//! Home's "Coming soon" and the full list, so the two read alike. A sequence
//! row: put it in a `ui::Sequence`, which draws the hairlines and the 12
//! above and below.
//!
//! - an sm cover (usually a cloth cover: unreleased books rarely have one);
//! - the title, a text button in `body-strong`, and the series with its
//!   position, muted;
//! - the date lane, right-aligned and tabular: the date in `body-strong`,
//!   with a caption under it when it is within two weeks ("In 6 days");
//! - the action lane, 136 wide: a small ghost "Want to read", or the
//!   `StatusChip` once the book is on a shelf. The lane is kept either way, so
//!   the dates line up.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, IconName, h_flex};
use gpui_kit::{
    App, Entity, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{BookId, Date, ReadingStatus, UpcomingBook};

use super::schedule::{release_text, series_text, soon_text};
use crate::catalog::navigation::Page;
use crate::chip::StatusChip;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors};
use crate::format::needs_permission;
use crate::library::model::LibraryModel;
use crate::library::writes;
use crate::session;
use crate::ui::kit::SmallControl as _;
use crate::ui::{Lane, TextButton};

#[derive(IntoElement)]
pub struct UpcomingRow {
    /// Keeps ids apart where both lists could be built in one frame.
    prefix: &'static str,
    book: UpcomingBook,
    today: Date,
    library: Entity<LibraryModel>,
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
        }
    }
}

impl RenderOnce for UpcomingRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.book.book.id;
        let prefix = self.prefix;
        let muted = colors(cx).fg_muted;
        let page = Page::Book(self.book.book.clone());
        let library = self.library.read(cx);
        let status = library.entry_for_book(id).map(|e| e.status);
        let adding = library.is_adding(id);
        let can_write = session::has_scope(cx, WRITE_LIBRARY);
        let soon = soon_text(self.book.release_date, self.today);
        let series = series_text(&self.book);

        let action = if let Some(status) = status {
            StatusChip(status).into_any_element()
        } else {
            let library = self.library.clone();
            let book = id;
            Button::new(SharedString::from(format!("{prefix}-add-{id}")))
                .ghost()
                .small_control()
                .icon(IconName::Plus)
                .label("Want to read")
                .loading(adding)
                .disabled(adding || !can_write)
                .when(!can_write, |this| {
                    this.tooltip(needs_permission(WRITE_LIBRARY))
                })
                .on_click(move |_, window, cx| {
                    add_to_want_to_read(library.clone(), book, window, cx);
                })
                .into_any_element()
        };

        h_flex()
            .w_full()
            .gap_3()
            .child(
                Cover::new(CoverSize::Sm, self.book.book.title.clone())
                    .thumbnail(self.book.book.cover.as_ref().map(|c| c.url.clone()))
                    .opens(SharedString::from(format!("{prefix}-cover-{id}")), page),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(TextButton::book(
                        SharedString::from(format!("{prefix}-title-{id}")),
                        &self.book.book,
                    ))
                    .when(!series.is_empty(), |this| {
                        this.child(div().body().text_color(muted).truncate().child(series))
                    }),
            )
            .child(
                Lane::fit()
                    .stacked()
                    .child(
                        div()
                            .body_strong()
                            .tabular()
                            .child(release_text(self.book.release_date, self.today)),
                    )
                    .when_some(soon, |this, soon| {
                        this.child(div().caption(cx).tabular().child(soon))
                    }),
            )
            .child(Lane::action().child(action))
    }
}

/// Adds `book` to Want to Read in the background, as the book page does; the
/// library applies the entry the write returns. A failure is shown as a
/// notification.
pub fn add_to_want_to_read(
    library: Entity<LibraryModel>,
    book: BookId,
    window: &mut Window,
    cx: &mut App,
) {
    writes::add_to_library(&library, book, ReadingStatus::WantToRead, window, cx);
}
