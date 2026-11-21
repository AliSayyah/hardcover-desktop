//! Library writes several views make, through [`LibraryModel::write`].

use std::collections::HashMap;

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{App, Context, Entity, Subscription, Window};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{BookId, EditionId, Rating, ReadingStatus, UserBookId};

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

/// Ratings asked for and not yet saved, per entry, for a view with rating
/// controls (the book header, Home's "Rate what you finished").
///
/// One rating write runs per entry at a time. A rating asked for while
/// another write on the entry runs is held, and the latest one held is sent
/// when that write ends, so no change the reader made is dropped. Until a
/// rating is saved (or fails) it is what the view shows ([`Ratings::shown`]).
pub(crate) struct Ratings {
    library: Entity<LibraryModel>,
    /// What to show while a rating is held or saving.
    shown: HashMap<UserBookId, Option<Rating>>,
    /// Held while another write on the entry runs.
    held: HashMap<UserBookId, Option<Rating>>,
    _library: Subscription,
}

impl Ratings {
    pub(crate) fn new(
        library: Entity<LibraryModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observed = cx.observe_in(&library, window, |this, _, window, cx| {
            this.library_changed(window, cx);
        });
        Self {
            library,
            shown: HashMap::new(),
            held: HashMap::new(),
            _library: observed,
        }
    }

    /// The rating to show for `id` while one is held or saving, if any.
    pub(crate) fn shown(&self, id: UserBookId) -> Option<Option<Rating>> {
        self.shown.get(&id).copied()
    }

    /// Saves `rating` for `id`, now or once the write running on it ends.
    pub(crate) fn rate(
        &mut self,
        id: UserBookId,
        rating: Option<Rating>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shown.insert(id, rating);
        self.held.insert(id, rating);
        self.send_held(window, cx);
        cx.notify();
    }

    /// Sends what is held for entries with no write running, and forgets
    /// what was shown for those whose rating write ended.
    fn library_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send_held(window, cx);
        let model = self.library.read(cx);
        let before = self.shown.len();
        let held = &self.held;
        self.shown.retain(|id, _| {
            held.contains_key(id) || model.pending_write(*id) == Some(WriteKind::Rating)
        });
        if self.shown.len() != before {
            cx.notify();
        }
    }

    fn send_held(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.library.read(cx);
        let ready: Vec<(UserBookId, Option<Rating>)> = self
            .held
            .iter()
            .filter(|(id, _)| model.pending_write(**id).is_none())
            .map(|(id, rating)| (*id, *rating))
            .collect();
        for (id, rating) in ready {
            self.held.remove(&id);
            let model = self.library.read(cx);
            let Some(entry) = model.entry(id) else {
                self.shown.remove(&id);
                continue;
            };
            if entry.rating == rating {
                self.shown.remove(&id);
                continue;
            }
            // Sending the status keeps Hardcover from marking the book Read.
            let keep = entry.status;
            let write = LibraryWrite::entry(
                id,
                WriteKind::Rating,
                "Couldn't save the rating",
                move |client| client.set_rating(id, rating, keep),
            );
            // The write's result reaches this through the model's notify.
            drop(self.library.update(cx, |m, cx| m.write(write, window, cx)));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui_kit::{AppContext as _, TestAppContext};
    use hardcover_core::store::LibraryCache;

    use super::*;
    use crate::session::SessionScope;

    /// D18: ratings asked for while another write on the entry runs are
    /// held, not dropped, and only the last is sent, once that write ends.
    #[gpui_kit::test]
    fn ratings_asked_for_during_a_write_are_held_and_the_last_sent(cx: &mut TestAppContext) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        cx.update(|cx| {
            gpui_kit::init(cx);
            session.install(cx);
        });
        let model = cx.new(|_| LibraryModel::new(session));
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let (id, status) = model.read_with(cx, |m, _| {
            let e = &m.entries()[0];
            (e.user_book_id, e.status)
        });
        let cx = cx.add_empty_window();
        let ratings = cx.update(|window, cx| cx.new(|cx| Ratings::new(model.clone(), window, cx)));
        // Count the rating writes as they start.
        let started = Rc::new(Cell::new(0));
        let _counting = cx.update(|_, cx| {
            let started = started.clone();
            let was = Rc::new(Cell::new(false));
            cx.observe(&model, move |model, cx| {
                let now = model.read(cx).pending_write(id) == Some(WriteKind::Rating);
                if now && !was.get() {
                    started.set(started.get() + 1);
                }
                was.set(now);
            })
        });

        cx.update(|window, cx| {
            // Another write on the entry runs: the same status, sent again.
            set_status(&model, id, status, window, cx);
            ratings.update(cx, |r, cx| {
                r.rate(id, Rating::new(3.0), window, cx);
                r.rate(id, Rating::new(4.5), window, cx);
            });
        });
        assert_eq!(started.get(), 0, "held while the status saves");
        let shown = ratings.read_with(cx, |r, _| r.shown(id));
        assert_eq!(shown, Some(Rating::new(4.5)), "the last one shows");

        cx.run_until_parked();
        assert_eq!(started.get(), 1, "one rating write");
        let rating = model.read_with(cx, |m, _| m.entry(id).and_then(|e| e.rating));
        assert_eq!(rating, Rating::new(4.5));
        assert_eq!(ratings.read_with(cx, |r, _| r.shown(id)), None);
    }
}
