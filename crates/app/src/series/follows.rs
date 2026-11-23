//! The series the reader follows on Hardcover, as an entity: what the
//! Series page shows, and the only series Home's Up next and Coming soon
//! look at. A [`Resource`] kept in the library cache (`resource::saved`),
//! so the last launch's follows show at once, offline too, and fetched
//! again when stale; reading needs `read:social`, following and
//! unfollowing `write:social`.
//!
//! Until the follows are known ([`FollowsModel::ids`] is `None`) nothing
//! acts on them: "not loaded" is never "follows nothing", so the series and
//! upcoming books saved from the last launch stay.
//!
//! It also runs "Add N books to Want to read…" ([`FollowsModel::add_books`]):
//! one library write per book, in order, remembered per series so its
//! commands show that it is busy.

use std::collections::{HashMap, HashSet};

use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{
    App, AppContext as _, AsyncWindowContext, Context, Entity, Global, SharedString, Task,
    WeakEntity, Window,
};
use hardcover_core::auth::scopes::{READ_SOCIAL, WRITE_SOCIAL};
use hardcover_core::{BookId, Date, FollowedSeries, LibraryEntry, ReadingStatus, SeriesId};

use crate::dates::local_day;
use crate::format::{count_text, needs_permission};
use crate::io::Priority;
use crate::library::model::{
    AccountChanged, LibraryModel, LibraryWrite, STILL_SAVING, WriteOutcome,
};
use crate::notify::{notify_failure, notify_outcome};
use crate::resource::{Binding, Demand, Landing, LoadStatus, Resource, saved};
use crate::session::SessionScope;
use crate::ui::kit::open_alert_dialog;

/// The follows' row in the library cache.
const SAVED_AS: &str = "follows";

pub struct FollowsModel {
    session: SessionScope,
    library: Entity<LibraryModel>,
    follows: Resource<(), Vec<FollowedSeries>>,
    /// Bumped whenever the followed set changes.
    revision: u64,
    /// Series being followed or unfollowed.
    writing: HashSet<SeriesId>,
    /// Series whose books are being added to Want to read.
    adding: HashSet<SeriesId>,
    _hydrating: Task<()>,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

struct FollowsGlobal(WeakEntity<FollowsModel>);

impl Global for FollowsGlobal {}

impl FollowsModel {
    /// Creates the model for a new workspace, replacing any earlier one,
    /// and shows the follows of the last launch while they load. The caller
    /// owns it.
    pub fn init(
        session: SessionScope,
        library: &Entity<LibraryModel>,
        cx: &mut App,
    ) -> Entity<Self> {
        let model = cx.new(|cx| {
            let follows = Resource::new(session.clone(), READ_SOCIAL, Priority::Background)
                .persist(saved::saver(&session, SAVED_AS, saved::single));
            let hydrating =
                saved::hydrate(&session, SAVED_AS, saved::parse_single, Self::bind(), cx);
            let subscriptions = vec![cx.subscribe(library, |this, _, _: &AccountChanged, cx| {
                this._hydrating = Task::ready(());
                this.follows.forget_from_disk();
                this.revision += 1;
                cx.notify();
            })];
            Self {
                session,
                library: library.clone(),
                follows,
                revision: 0,
                writing: HashSet::new(),
                adding: HashSet::new(),
                _hydrating: hydrating,
                _subscriptions: subscriptions,
            }
        });
        cx.set_global(FollowsGlobal(model.downgrade()));
        model
    }

    /// Forgets the workspace's model, on signing out.
    pub fn release(cx: &mut App) {
        if cx.has_global::<FollowsGlobal>() {
            cx.remove_global::<FollowsGlobal>();
        }
    }

    /// The workspace's model, once signed in.
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<FollowsGlobal>()?.0.upgrade()
    }

    fn bind() -> Binding<Self, (), Vec<FollowedSeries>> {
        Binding {
            get: |m| &mut m.follows,
            landed: |this, _, landing, cx| {
                if landing == Landing::Changed {
                    this.revision += 1;
                }
                cx.notify();
            },
        }
    }

    // ----- reads -----

    /// The followed series, once known (this run or the last); `None`
    /// until then.
    pub fn followed(&self) -> Option<&[FollowedSeries]> {
        self.follows.value(&()).map(|f| f.as_slice())
    }

    /// The followed series' ids, sorted, once known.
    pub fn ids(&self) -> Option<Vec<SeriesId>> {
        let mut ids: Vec<SeriesId> = self.followed()?.iter().map(|f| f.id).collect();
        ids.sort_unstable();
        ids.dedup();
        Some(ids)
    }

    /// The followed ids as a set; empty until known.
    pub fn id_set(&self) -> HashSet<SeriesId> {
        self.followed()
            .map(|f| f.iter().map(|s| s.id).collect())
            .unwrap_or_default()
    }

    pub fn is_followed(&self, id: SeriesId) -> bool {
        self.followed()
            .is_some_and(|f| f.iter().any(|s| s.id == id))
    }

    /// The day each series was followed, here.
    pub fn followed_on(&self) -> HashMap<SeriesId, Date> {
        self.followed()
            .into_iter()
            .flatten()
            .filter_map(|s| Some((s.id, local_day(s.followed_at?))))
            .collect()
    }

    pub fn status(&self) -> LoadStatus {
        self.follows.status(&())
    }

    /// Bumped whenever the followed set changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// A follow or unfollow of `id` is being written.
    pub fn is_writing(&self, id: SeriesId) -> bool {
        self.writing.contains(&id)
    }

    /// Books of `id` are being added to Want to read.
    pub fn is_adding(&self, id: SeriesId) -> bool {
        self.adding.contains(&id)
    }

    // ----- loading -----

    /// Asks for the follows where they are stale or failed (launch,
    /// Refresh, a new grant, "Try again").
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Shown, cx);
    }

    /// Asks for the follows as `demand` calls for.
    pub fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if self
            .follows
            .request((), demand, |c| c.followed_series(), Self::bind(), cx)
        {
            cx.notify();
        }
    }

    // ----- writes -----

    /// Follows series `id`. `None` when dropped because the session ended,
    /// else the result or the message to show.
    pub fn follow(
        &mut self,
        id: SeriesId,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        let Some(refused) = self.start_write(id, cx) else {
            let call = self.session.interactive(move |c| c.follow_series(id));
            return self.finish_write(
                id,
                call,
                |follows, followed| {
                    follows.retain(|s| s.id != followed.id);
                    follows.push(followed);
                },
                cx,
            );
        };
        refused
    }

    /// Unfollows series `id` (every id its follows are stored under). `None`
    /// when dropped because the session ended, else the result or the
    /// message to show.
    pub fn unfollow(
        &mut self,
        id: SeriesId,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        if let Some(refused) = self.start_write(id, cx) {
            return refused;
        }
        let stored: Vec<SeriesId> = self
            .followed()
            .and_then(|f| f.iter().find(|s| s.id == id))
            .map_or_else(|| vec![id], |s| s.followed_as.clone());
        let call = self.session.interactive(move |c| {
            c.unfollow_series(&stored)?;
            Ok(FollowedSeries::new(id, None))
        });
        self.finish_write(
            id,
            call,
            |follows, gone| follows.retain(|s| s.id != gone.id),
            cx,
        )
    }

    /// Why a follow write of `id` can't start, as the task to return; `None`
    /// when it may, with `id` marked as writing.
    fn start_write(
        &mut self,
        id: SeriesId,
        cx: &mut Context<Self>,
    ) -> Option<Task<Option<Result<(), SharedString>>>> {
        if !self.session.is_alive() {
            return Some(Task::ready(None));
        }
        if !self.session.has_scope(WRITE_SOCIAL) {
            return Some(Task::ready(Some(
                Err(needs_permission(WRITE_SOCIAL).into()),
            )));
        }
        if !self.writing.insert(id) {
            return Some(Task::ready(Some(Err(STILL_SAVING.into()))));
        }
        cx.notify();
        None
    }

    /// Applies a follow write's result to the follows: in place when they
    /// are known (a fetch in flight is older, and dropped), else by
    /// fetching them.
    fn finish_write(
        &mut self,
        id: SeriesId,
        call: impl Future<Output = Option<hardcover_core::Result<FollowedSeries>>> + 'static,
        apply: fn(&mut Vec<FollowedSeries>, FollowedSeries),
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.writing.remove(&id);
                cx.notify();
                Some(match result? {
                    Ok(written) => {
                        if this.followed().is_some() {
                            this.follows.update(&(), |f| apply(f, written));
                            this.revision += 1;
                        } else {
                            this.follows.note_write(&());
                            this.request(Demand::Now, cx);
                        }
                        Ok(())
                    }
                    Err(e) => Err(this
                        .session
                        .error_message(&e, Some(WRITE_SOCIAL), cx)
                        .into()),
                })
            })
            .ok()
            .flatten()
        })
    }

    /// "Add N books to Want to read…": adds `books` of series `id`, one at
    /// a time and in order. A book that joined the library meanwhile is
    /// left alone (adding it again would overwrite its shelf); the writes
    /// stop when the session ends. Success shows on the rows themselves; a
    /// failure is one notification, saying how many were added.
    pub fn add_books(
        &mut self,
        id: SeriesId,
        books: Vec<BookId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if books.is_empty() || !self.session.is_alive() || !self.adding.insert(id) {
            return;
        }
        cx.notify();
        let library = self.library.clone();
        cx.spawn_in(window, async move |this, cx| {
            let total = books.len();
            let mut added = 0_usize;
            let mut failure: Option<SharedString> = None;
            for book in books {
                match add_one(&library, book, cx).await {
                    WriteOutcome::Saved(_) => added += 1,
                    // Already there (it joined meanwhile): nothing to do.
                    WriteOutcome::InLibrary => {}
                    WriteOutcome::Busy => {
                        failure.get_or_insert_with(|| STILL_SAVING.into());
                    }
                    WriteOutcome::Failed { message, .. } => {
                        failure = Some(message);
                        break;
                    }
                    WriteOutcome::Ended => return,
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.adding.remove(&id);
                cx.notify();
                if let Some(message) = failure {
                    let title = format!(
                        "Added {} of {} books to Want to read",
                        count_text(added),
                        count_text(total)
                    );
                    window.push_notification(
                        Notification::error(message).title(SharedString::from(title)),
                        cx,
                    );
                }
            });
        })
        .detach();
    }
}

/// Adds `book` to Want to read through the library's write, which refuses
/// a book already in the library; the caller reports failures.
async fn add_one(
    library: &Entity<LibraryModel>,
    book: BookId,
    cx: &mut AsyncWindowContext,
) -> WriteOutcome<LibraryEntry> {
    let write = LibraryWrite::add(book, "Couldn't add the book", move |client| {
        client.add_to_library(book, ReadingStatus::WantToRead)
    })
    .reported_by_caller();
    let Ok(outcome) =
        cx.update(|window, cx| library.update(cx, |m, cx| m.write(write, window, cx)))
    else {
        return WriteOutcome::Ended;
    };
    outcome.await
}

// ----- the commands the views share -----

/// The notification id of "You no longer follow …": one at a time.
struct UnfollowedNote;

/// "Follow" on a series page or in Add series: a failure is a notification.
pub fn follow(id: SeriesId, window: &mut Window, cx: &mut App) {
    let Some(model) = FollowsModel::global(cx) else {
        return;
    };
    let task = model.update(cx, |m, cx| m.follow(id, cx));
    notify_failure(task, "Couldn't follow the series", window, cx);
}

/// "Unfollow series": the series leaves the Series page and Home, so a
/// notification says so with Undo, which follows it again.
pub fn unfollow(id: SeriesId, name: &str, window: &mut Window, cx: &mut App) {
    let Some(model) = FollowsModel::global(cx) else {
        return;
    };
    let message = unfollowed_text(name);
    let task = model.update(cx, |m, cx| m.unfollow(id, cx));
    notify_outcome(
        task,
        "Couldn't unfollow the series",
        move |(), _, _| {
            let note = Notification::new()
                .id::<UnfollowedNote>()
                .message(message)
                .action(move |_, _, cx| {
                    let note = cx.entity().downgrade();
                    Button::new("undo-unfollow-series").label("Undo").on_click(
                        move |_, window, cx| {
                            follow(id, window, cx);
                            let _ = note.update(cx, |note, cx| note.dismiss(window, cx));
                        },
                    )
                })
                .autohide(true);
            Some(note)
        },
        window,
        cx,
    );
}

/// "You no longer follow “Mistborn”. It's left Home too."
pub fn unfollowed_text(name: &str) -> String {
    format!("You no longer follow “{name}”. It's left Home too.")
}

/// "Add N books to Want to read…": asks first, naming the books, then adds
/// them ([`FollowsModel::add_books`]). `titles` are the books' titles in
/// order; `later` says whether books not out yet are left out.
pub fn confirm_add_books(
    id: SeriesId,
    name: &str,
    books: Vec<(BookId, String)>,
    later: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(model) = FollowsModel::global(cx) else {
        return;
    };
    if books.is_empty() {
        return;
    }
    let n = books.len();
    let heading: SharedString = add_books_heading(n).into();
    let description: SharedString = add_books_text(name, &books, later).into();
    let ok: SharedString = if n == 1 {
        "Add 1 book".into()
    } else {
        format!("Add {} books", count_text(n)).into()
    };
    let ids: Vec<BookId> = books.into_iter().map(|(id, _)| id).collect();
    open_alert_dialog(window, cx, move |alert, _, _| {
        let (model, ids) = (model.clone(), ids.clone());
        alert
            .title(heading.clone())
            .description(description.clone())
            .confirm()
            .ok_text(ok.clone())
            // Nothing is destroyed: the result is primary, Cancel has the focus.
            .ok_variant(ButtonVariant::Primary)
            .on_ok(move |_, window, cx| {
                let ids = ids.clone();
                model.update(cx, |m, cx| m.add_books(id, ids, window, cx));
                true
            })
    });
}

/// "Add 5 books to Want to read?"
fn add_books_heading(n: usize) -> String {
    match n {
        1 => "Add 1 book to Want to read?".to_owned(),
        n => format!("Add {} books to Want to read?", count_text(n)),
    }
}

/// Most books a confirmation names; the rest are counted.
const NAMED_BOOKS: usize = 5;

/// "From “Dune”: Children of Dune, God Emperor of Dune and 3 more. Books not
/// out yet are left out."
fn add_books_text(name: &str, books: &[(BookId, String)], later: bool) -> String {
    let titles: Vec<&str> = books.iter().map(|(_, t)| t.as_str()).collect();
    let list = match titles.as_slice() {
        [one] => (*one).to_owned(),
        all if all.len() <= NAMED_BOOKS => {
            let (last, rest) = all.split_last().unwrap_or((&"", &[]));
            format!("{} and {last}", rest.join(", "))
        }
        all => format!(
            "{} and {} more",
            all[..NAMED_BOOKS - 1].join(", "),
            count_text(all.len() - (NAMED_BOOKS - 1))
        ),
    };
    let later = if later {
        " Books not out yet are left out."
    } else {
        ""
    };
    format!("From “{name}”: {list}.{later}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn books(n: i64) -> Vec<(BookId, String)> {
        (1..=n)
            .map(|i| (BookId::new(i), format!("Book {i}")))
            .collect()
    }

    #[test]
    fn the_confirmation_names_the_books_and_counts_the_rest() {
        assert_eq!(add_books_heading(1), "Add 1 book to Want to read?");
        assert_eq!(add_books_heading(7), "Add 7 books to Want to read?");
        assert_eq!(
            add_books_text("Dune", &books(1), false),
            "From “Dune”: Book 1."
        );
        assert_eq!(
            add_books_text("Dune", &books(3), true),
            "From “Dune”: Book 1, Book 2 and Book 3. Books not out yet are left out."
        );
        assert_eq!(
            add_books_text("Goosebumps", &books(108), false),
            "From “Goosebumps”: Book 1, Book 2, Book 3, Book 4 and 104 more."
        );
        assert_eq!(
            unfollowed_text("Mistborn"),
            "You no longer follow “Mistborn”. It's left Home too."
        );
    }
}
