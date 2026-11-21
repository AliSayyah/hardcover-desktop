//! Shared library write operations used by rows and the book page.

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{App, AppContext as _, Entity, Window};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{BookId, EditionId, ReadingStatus, UserBookId};

use super::model::LibraryModel;
use crate::app_state::Services;
use crate::format::error_message;

/// Adds a book with `status` in the background and applies the returned entry.
pub(crate) fn add_to_library(
    library: Entity<LibraryModel>,
    book: BookId,
    status: ReadingStatus,
    window: &mut Window,
    cx: &mut App,
) {
    if !library.update(cx, |model, cx| model.begin_add(book, cx)) {
        return;
    }
    let client = Services::global(cx).client().clone();
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move { client.add_to_library(book, status) })
                .await;
            library.update(cx, |model, cx| {
                model.end_add(book, cx);
                if let Ok(entry) = &result {
                    model.apply_written(entry.clone(), cx);
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

/// Changes an entry's status in the background and applies the returned entry.
pub(crate) fn set_status(
    library: Entity<LibraryModel>,
    id: UserBookId,
    status: ReadingStatus,
    window: &mut Window,
    cx: &mut App,
) {
    if !library.update(cx, |model, cx| {
        model.begin_write(id, super::model::WriteKind::Status, cx)
    }) {
        return;
    }
    let client = Services::global(cx).client().clone();
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move { client.set_status(id, status) })
                .await;
            library.update(cx, |model, cx| {
                model.end_write(id, cx);
                if let Ok(entry) = &result {
                    model.apply_written(entry.clone(), cx);
                }
            });
            if let Err(error) = result {
                let _ = cx.update(|window, cx| {
                    let message = Services::error_message(&error, Some(WRITE_LIBRARY), cx);
                    window.push_notification(
                        Notification::error(message).title("Couldn't change the status"),
                        cx,
                    );
                });
            }
        })
        .detach();
}

/// Moves an entry to `edition` in the background, its open read with it
/// ([`hardcover_core::Client::set_edition`]), and applies the returned entry.
/// Says so when the read stays on the old edition.
pub(crate) fn set_edition(
    library: Entity<LibraryModel>,
    id: UserBookId,
    edition: EditionId,
    window: &mut Window,
    cx: &mut App,
) {
    if !library.update(cx, |model, cx| {
        model.begin_write(id, super::model::WriteKind::Edition, cx)
    }) {
        return;
    }
    let client = Services::global(cx).client().clone();
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move { client.set_edition(id, edition) })
                .await;
            library.update(cx, |model, cx| {
                model.end_write(id, cx);
                if let Ok(change) = &result {
                    model.apply_written(change.entry.clone(), cx);
                }
            });
            let _ = cx.update(|window, cx| match result {
                Ok(change) => {
                    if let Some(error) = change.read_left_behind {
                        window.push_notification(
                            Notification::warning(format!(
                                "Your current read still counts progress against the old edition. {}",
                                error_message(&error)
                            ))
                            .title("Edition changed, progress not moved"),
                            cx,
                        );
                    }
                }
                Err(error) => {
                    let message = Services::error_message(&error, Some(WRITE_LIBRARY), cx);
                    window.push_notification(
                        Notification::error(message).title("Couldn't change the edition"),
                        cx,
                    );
                }
            });
        })
        .detach();
}
