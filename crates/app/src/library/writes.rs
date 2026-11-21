//! Library writes several views make, through [`LibraryModel::write`].

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{App, Entity, Window};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{BookId, EditionId, ReadingStatus, UserBookId};

use super::model::{LibraryModel, LibraryWrite, WriteKind, WriteOutcome};
use crate::session;

/// Adds a book with `status`.
pub(crate) fn add_to_library(
    library: &Entity<LibraryModel>,
    book: BookId,
    status: ReadingStatus,
    window: &mut Window,
    cx: &mut App,
) {
    let write = LibraryWrite::add(book, "Couldn't add the book", move |client| {
        client.add_to_library(book, status)
    });
    drop(library.update(cx, |model, cx| model.write(write, window, cx)));
}

/// Changes an entry's status.
pub(crate) fn set_status(
    library: &Entity<LibraryModel>,
    id: UserBookId,
    status: ReadingStatus,
    window: &mut Window,
    cx: &mut App,
) {
    let write = LibraryWrite::entry(
        id,
        WriteKind::Status,
        "Couldn't change the status",
        move |client| client.set_status(id, status),
    );
    drop(library.update(cx, |model, cx| model.write(write, window, cx)));
}

/// Moves an entry to `edition`, its open read with it
/// ([`hardcover_core::Client::set_edition`]). Says so when the read stays on
/// the old edition.
pub(crate) fn set_edition(
    library: &Entity<LibraryModel>,
    id: UserBookId,
    edition: EditionId,
    window: &mut Window,
    cx: &mut App,
) {
    let write = LibraryWrite::entry(
        id,
        WriteKind::Edition,
        "Couldn't change the edition",
        move |client| client.set_edition(id, edition),
    );
    let outcome = library.update(cx, |model, cx| model.write(write, window, cx));
    window
        .spawn(cx, async move |cx| {
            if let WriteOutcome::Saved(change) = outcome.await
                && let Some(error) = change.read_left_behind
            {
                let _ = cx.update(|window, cx| {
                    let message = session::error_message(&error, Some(WRITE_LIBRARY), cx);
                    window.push_notification(
                        Notification::warning(format!(
                            "Your current read still counts progress against the old edition. {message}"
                        ))
                        .title("Edition changed, progress not moved"),
                        cx,
                    );
                });
            }
        })
        .detach();
}
