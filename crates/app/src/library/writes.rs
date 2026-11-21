//! Library writes several views make, through [`LibraryModel::write`].

use std::collections::HashMap;

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{App, Context, Entity, Subscription, Window};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{BookId, EditionId, LibraryEntry, Rating, ReadingStatus, UserBookId};

use super::model::{LibraryModel, LibraryWrite, WriteKind, WriteOutcome};
use crate::format::{progress_summary, short_position};
use crate::session;
use crate::ui::kit::open_alert_dialog;

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

/// What moving an entry to another status does to its reads, and so
/// whether to ask first (the rules Home follows: "Start reading" goes at
/// once, "Mark as read…" asks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatusChange {
    /// Starts or finishes nothing: made at once.
    Now,
    /// Starts a new read (to Currently reading from anything but Want to
    /// read) or finishes one (to Read): asked first, saying what happens.
    Confirm {
        title: String,
        description: String,
        ok: &'static str,
    },
}

/// The change from `entry`'s status to `to`, or `None` if it has it.
///
/// - **To Currently reading** starts a new read on Hardcover, whatever the
///   entry had. From Want to read that is the point ("Start reading"), so it
///   goes at once; from anything else the reader may expect the read they
///   had to carry on, so it asks, naming where an unfinished read stays.
/// - **To Read** finishes the open read today, or marks the book read today
///   when none is open: it asks.
/// - **Anything else** (Want to read, Paused, Did not finish, Ignored)
///   leaves the reads as they are: at once.
pub(crate) fn status_change(entry: &LibraryEntry, to: ReadingStatus) -> Option<StatusChange> {
    if entry.status == to {
        return None;
    }
    let title = &entry.book.title;
    Some(match to {
        ReadingStatus::CurrentlyReading if entry.status == ReadingStatus::WantToRead => {
            StatusChange::Now
        }
        ReadingStatus::CurrentlyReading => {
            let kept = match entry.open_read() {
                Some(_) => {
                    let which = if entry.status == ReadingStatus::Paused {
                        "paused"
                    } else {
                        "unfinished"
                    };
                    match short_position(&progress_summary(entry)) {
                        Some(at) => format!(" Your {which} read stays at {at}."),
                        None => format!(" Your {which} read stays as it is."),
                    }
                }
                None if !entry.reads.is_empty() => " Your earlier reads stay.".to_owned(),
                None => String::new(),
            };
            StatusChange::Confirm {
                title: format!("Start a new read of “{title}”?"),
                description: format!("This starts a new read, from today.{kept}"),
                ok: "Start reading",
            }
        }
        ReadingStatus::Read => StatusChange::Confirm {
            title: format!("Mark “{title}” as read?"),
            description: if entry.open_read().is_some() {
                "It moves to your Read shelf, and this read ends today.".to_owned()
            } else {
                "It moves to your Read shelf, finished today.".to_owned()
            },
            ok: "Mark as read",
        },
        _ => StatusChange::Now,
    })
}

/// Moves an entry to `to`: at once, or after asking when that starts or
/// finishes a read ([`status_change`]). Focus goes back to whatever had it
/// when this was called, so a command from a menu should first focus the
/// menu's trigger.
pub(crate) fn change_status(
    library: &Entity<LibraryModel>,
    id: UserBookId,
    to: ReadingStatus,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(change) = library.read(cx).entry(id).and_then(|e| status_change(e, to)) else {
        return;
    };
    match change {
        StatusChange::Now => set_status(library, id, to, window, cx),
        StatusChange::Confirm {
            title,
            description,
            ok,
        } => {
            let library = library.clone();
            open_alert_dialog(window, cx, move |alert, _, _| {
                let library = library.clone();
                alert
                    .title(title.clone())
                    .description(description.clone())
                    .confirm()
                    .ok_text(ok)
                    .on_ok(move |_, window, cx| {
                        set_status(&library, id, to, window, cx);
                        true
                    })
            });
        }
    }
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
    use hardcover_core::{BookSummary, ReadId, ReadSession, UserId};

    fn entry(status: ReadingStatus) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(1), "Dune");
        book.pages = Some(400);
        LibraryEntry::new(UserBookId::new(1), UserId::new(1), book, status)
    }

    fn read(id: i64, page: Option<u32>, finished: bool) -> ReadSession {
        let mut read = ReadSession::new(ReadId::new(id));
        read.progress_pages = page;
        if finished {
            read.finished_at = hardcover_core::Date::from_ymd_opt(2026, 1, 2);
        }
        read
    }

    fn description(change: Option<StatusChange>) -> String {
        match change {
            Some(StatusChange::Confirm { description, .. }) => description,
            other => panic!("expected a confirmation, got {other:?}"),
        }
    }

    /// D3: a change that starts or finishes a read asks first and says what
    /// happens; Home's rules decide which.
    #[test]
    fn changes_that_start_or_finish_a_read_ask_first() {
        use ReadingStatus::*;
        // Starting a book on Want to read is the point: at once.
        assert_eq!(
            status_change(&entry(WantToRead), CurrentlyReading),
            Some(StatusChange::Now)
        );
        // The same status: nothing to do.
        assert_eq!(status_change(&entry(Paused), Paused), None);
        // Neither starts nor finishes a read.
        for (from, to) in [
            (CurrentlyReading, Paused),
            (CurrentlyReading, DidNotFinish),
            (Read, WantToRead),
            (Paused, Ignored),
        ] {
            assert_eq!(status_change(&entry(from), to), Some(StatusChange::Now));
        }

        // Back to reading from Paused: a new read, the paused one stays.
        let mut paused = entry(Paused);
        paused.reads.push(read(1, Some(200), false));
        assert_eq!(
            description(status_change(&paused, CurrentlyReading)),
            "This starts a new read, from today. Your paused read stays at page 200."
        );
        let mut reread = entry(Read);
        reread.reads.push(read(1, None, true));
        assert_eq!(
            description(status_change(&reread, CurrentlyReading)),
            "This starts a new read, from today. Your earlier reads stay."
        );

        // Finishing: the open read ends today.
        let mut reading = entry(CurrentlyReading);
        reading.reads.push(read(1, Some(120), false));
        let Some(StatusChange::Confirm {
            title,
            description: finishing,
            ok,
        }) = status_change(&reading, Read)
        else {
            panic!("finishing asks first");
        };
        assert_eq!(title, "Mark “Dune” as read?");
        assert_eq!(
            finishing,
            "It moves to your Read shelf, and this read ends today."
        );
        assert_eq!(ok, "Mark as read");
        assert_eq!(
            description(status_change(&entry(WantToRead), Read)),
            "It moves to your Read shelf, finished today."
        );
    }

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
