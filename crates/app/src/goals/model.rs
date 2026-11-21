//! The user's reading goals as an entity: loaded once signed in, reloaded
//! shortly after the library changes (Hardcover recalculates progress from
//! the user's reading), and the writes that change them.
//!
//! The goals are a [`Resource`] kept in the library cache between launches
//! (`resource::saved`): the last good ones show at once, offline too.
//!
//! Everything here needs the goal scopes; without `read:goals` the model
//! stays empty and the views hide the goal section.

use std::time::Duration;

use gpui_kit::{Context, Entity, SharedString, Subscription, Task};
use hardcover_core::auth::scopes::{READ_GOALS, WRITE_GOALS};
use hardcover_core::{Date, Error, Goal, GoalId, LibraryEntry, NewGoal};

use crate::io::Priority;
use crate::library::model::{AccountChanged, LibraryModel};
use crate::resource::follow::Followed;
use crate::resource::{
    Binding, Debounce, Demand, Landing, LibraryFollow, LoadStatus, Resource, saved,
};
use crate::session::SessionScope;

/// How long after a library change to reload: Hardcover recalculates goal
/// progress after a write, and several writes in a row need one reload.
const RELOAD_DELAY: Duration = Duration::from_secs(3);

/// The goals' row in the library cache.
const SAVED_AS: &str = "goals";

pub struct GoalsModel {
    session: SessionScope,
    goals: Resource<(), Vec<Goal>>,
    /// A write is in flight; the controls that write are disabled meanwhile.
    writing: bool,
    /// The library revision the goals were last loaded after.
    library: LibraryFollow<u64>,
    reload: Debounce,
    /// Reading the goals saved at the last launch.
    _hydrating: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl GoalsModel {
    /// The goals, loaded at once.
    pub fn new(
        session: SessionScope,
        library: &Entity<LibraryModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(library, |this, library, cx| {
                let library = library.read(cx);
                // The library loading is not news: the goals load at start.
                // A sync that changed it is.
                if this.library.check(library, LibraryModel::revision) == Some(Followed::Changed)
                    && this.session.allows(READ_GOALS)
                {
                    this.reload.schedule(RELOAD_DELAY, Self::reload, cx);
                }
            }),
            cx.subscribe(library, |this, _, _: &AccountChanged, cx| {
                this._hydrating = Task::ready(());
                this.goals.forget_from_disk();
                cx.notify();
            }),
        ];
        let goals = Resource::new(session.clone(), READ_GOALS, Priority::Background)
            .persist(saved::saver(&session, SAVED_AS, saved::single));
        let hydrating = saved::hydrate(&session, SAVED_AS, saved::parse_single, Self::bind(), cx);
        let mut model = Self {
            session,
            goals,
            writing: false,
            library: LibraryFollow::default(),
            reload: Debounce::default(),
            _hydrating: hydrating,
            _subscriptions: subscriptions,
        };
        model.load(cx);
        model
    }

    fn bind() -> Binding<Self, (), Vec<Goal>> {
        Binding {
            get: |m| &mut m.goals,
            landed: |this, _, landing, cx| {
                // A write came after the load started, and may carry a
                // library change it hadn't seen: load again shortly.
                if landing == Landing::Dropped {
                    this.reload.schedule(RELOAD_DELAY, Self::reload, cx);
                }
                cx.notify();
            },
        }
    }

    // ----- reads -----

    pub fn goals(&self) -> &[Goal] {
        self.goals.value(&()).map_or(&[], |g| g.as_slice())
    }

    pub fn goal(&self, id: GoalId) -> Option<&Goal> {
        self.goals().iter().find(|g| g.id == id)
    }

    /// Goals whose dates include `today`, the latest end date first.
    pub fn current(&self, today: Date) -> impl Iterator<Item = &Goal> {
        self.goals().iter().filter(move |g| g.is_current(today))
    }

    pub fn status(&self) -> LoadStatus {
        self.goals.status(&())
    }

    /// The goals have loaded (or failed) at least once, or the last launch's
    /// are shown. Reloads keep the old goals on screen, so the sidebar
    /// doesn't flicker while they run.
    pub fn has_loaded(&self) -> bool {
        self.goals.has_settled(&())
    }

    pub fn is_writing(&self) -> bool {
        self.writing
    }

    /// The library's entries with a read finished within the goal's dates,
    /// each with its latest such date, the most recent first. A book read
    /// twice in the period shows once, and the library knows nothing of a
    /// goal's conditions, so this is an approximation of what Hardcover
    /// counted, not its count.
    pub fn finished_in<'a>(
        goal: &Goal,
        entries: &'a [LibraryEntry],
    ) -> Vec<(&'a LibraryEntry, Date)> {
        let mut finished: Vec<(&LibraryEntry, Date)> = entries
            .iter()
            .filter_map(|e| {
                e.reads
                    .iter()
                    .filter_map(|r| r.finished_at)
                    .filter(|f| goal.is_current(*f))
                    .max()
                    .map(|f| (e, f))
            })
            .collect();
        finished.sort_by_key(|(_, f)| std::cmp::Reverse(*f));
        finished
    }

    // ----- loading -----

    /// Loads the goals where they are stale or failed (at start, Refresh, a
    /// new grant), keeping the ones shown until the new ones arrive.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Shown, cx);
    }

    /// "Try again" after a failed load.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Now, cx);
    }

    /// After a library change or a write: Hardcover has recalculated the
    /// progress, however fresh the goals shown.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Now, cx);
    }

    fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if self
            .goals
            .request((), demand, |client| client.goals(), Self::bind(), cx)
        {
            cx.notify();
        }
    }

    // ----- writes -----

    /// Runs one goal write and applies its result. Resolves to `None` if the
    /// write was dropped because the session ended (nothing to show), else
    /// to `Err` with the message to show; the goals are unchanged then.
    fn write<T: Send + 'static>(
        &mut self,
        run: impl FnOnce(&hardcover_core::Client) -> Result<T, Error> + Send + 'static,
        apply: impl FnOnce(&mut Vec<Goal>, T) + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        if !self.session.is_alive() {
            return Task::ready(None);
        }
        if self.writing {
            return Task::ready(Some(Err(
                "Another change is still being saved. Try again in a moment.".into(),
            )));
        }
        if !self.session.has_scope(WRITE_GOALS) {
            return Task::ready(Some(Err(
                crate::format::needs_permission(WRITE_GOALS).into()
            )));
        }
        self.writing = true;
        // A load already on its way is older than the write.
        self.goals.note_write(&());
        cx.notify();
        let call = self.session.interactive(run);
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.writing = false;
                cx.notify();
                // Dropped: the session ended (or the call panicked).
                Some(match result? {
                    Ok(value) => {
                        // A load that started before the write is dropped
                        // when it lands (and loads again then).
                        let mut goals = this.goals().to_vec();
                        apply(&mut goals, value);
                        sort(&mut goals);
                        this.goals.set((), goals, cx);
                        Ok(())
                    }
                    Err(e) => Err(this.session.error_message(&e, Some(WRITE_GOALS), cx).into()),
                })
            })
            .ok()
            .flatten()
        })
    }

    pub fn create(
        &mut self,
        goal: NewGoal,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        self.write(
            move |client| client.create_goal(&goal),
            |goals, created| goals.push(created),
            cx,
        )
    }

    /// Changes goal `id` with `edit`, applied to the goal as Hardcover has
    /// it at the time of the write, so changes made elsewhere since it was
    /// loaded stay.
    pub fn update(
        &mut self,
        id: GoalId,
        edit: impl FnOnce(&mut Goal) + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        self.write(
            move |client| client.update_goal(id, edit),
            |goals, saved| match goals.iter_mut().find(|g| g.id == saved.id) {
                Some(g) => *g = saved,
                None => goals.push(saved),
            },
            cx,
        )
    }

    pub fn delete(
        &mut self,
        id: GoalId,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        self.write(
            move |client| client.delete_goal(id),
            move |goals, ()| goals.retain(|g| g.id != id),
            cx,
        )
    }
}

/// The order Hardcover lists goals in: the latest end date first.
fn sort(goals: &mut [Goal]) {
    goals.sort_by(|a, b| b.end.cmp(&a.end).then(b.id.cmp(&a.id)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext as _, TestAppContext};
    use hardcover_core::Client;
    use hardcover_core::store::LibraryCache;
    use std::sync::Arc;

    /// A library and its goals on `client`, over `cache`, as the workspace
    /// starts them, with everything settled (the reload delay included).
    fn launch(
        client: Client,
        cache: &Arc<LibraryCache>,
        cx: &mut TestAppContext,
    ) -> gpui_kit::Entity<GoalsModel> {
        let session = SessionScope::for_tests(client, Some(cache.clone()));
        cx.update(|cx| session.install(cx));
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let goals = cx.new(|cx| GoalsModel::new(session, &library, cx));
        library.update(cx, LibraryModel::start);
        for _ in 0..3 {
            cx.executor().advance_clock(RELOAD_DELAY * 2);
            cx.run_until_parked();
        }
        goals
    }

    /// The goals were fetched twice at every launch: the sync that found
    /// the cached library unchanged counted as a change.
    #[gpui_kit::test]
    fn goals_are_fetched_once_at_launch(cx: &mut TestAppContext) {
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        launch(hardcover_core::demo::client(), &cache, cx);
        let goals = launch(hardcover_core::demo::client(), &cache, cx);
        goals.read_with(cx, |m, _| {
            assert!(!m.goals().is_empty());
            assert_eq!(m.goals.started(), 1);
        });
    }

    /// B7: offline, the goals of the last launch show, with the failure.
    #[gpui_kit::test]
    fn the_last_goals_show_offline(cx: &mut TestAppContext) {
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let online = launch(hardcover_core::demo::client(), &cache, cx);
        let shown = online.read_with(cx, |m, _| m.goals().to_vec());
        assert!(!shown.is_empty());

        let faults = hardcover_core::demo::Faults::parse("offline").unwrap();
        let offline = launch(hardcover_core::demo::client_with(&faults), &cache, cx);
        offline.read_with(cx, |m, _| {
            assert_eq!(m.goals(), shown.as_slice());
            assert!(m.has_loaded());
            assert!(matches!(m.status(), LoadStatus::Failed(_)));
        });
    }

    /// A goal write that lands after signing out is dropped: nothing to
    /// show, rather than a silent success.
    #[gpui_kit::test]
    fn a_write_landing_after_sign_out_is_dropped(cx: &mut gpui_kit::TestAppContext) {
        let session = SessionScope::for_tests(hardcover_core::demo::client(), None);
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let goals = cx.new(|cx| GoalsModel::new(session.clone(), &library, cx));
        // The call runs inline; its result waits to be picked up.
        let task = goals.update(cx, |m, cx| m.delete(GoalId::new(1), cx));
        session.end();
        let task = cx.foreground_executor().spawn(task);
        cx.run_until_parked();
        assert_eq!(gpui_kit::block_on(task), None);
        assert!(
            !goals.read_with(cx, |m, _| m.is_writing()),
            "the write ended"
        );
    }
    use hardcover_core::{
        BookId, BookSummary, GoalMetric, ReadId, ReadSession, ReadingStatus, UserBookId, UserId,
    };

    fn entry(id: i64, finished: Option<Date>) -> LibraryEntry {
        let mut e = LibraryEntry::new(
            UserBookId::new(id),
            UserId::new(1),
            BookSummary::new(BookId::new(id), format!("Book {id}")),
            ReadingStatus::Read,
        );
        let mut read = ReadSession::new(ReadId::new(id));
        read.finished_at = finished;
        e.reads.push(read);
        e
    }

    #[test]
    fn finished_in_counts_latest_reads_within_the_dates() {
        let d = |y, m, d| Date::from_ymd_opt(y, m, d).unwrap();
        let goal = Goal::new(
            GoalId::new(1),
            GoalMetric::Books,
            12,
            d(2026, 1, 1),
            d(2026, 12, 31),
        );
        let entries = [
            entry(1, Some(d(2026, 3, 1))),
            entry(2, Some(d(2025, 12, 31))),
            entry(3, None),
            entry(4, Some(d(2026, 12, 31))),
            entry(5, Some(d(2026, 1, 1))),
        ];
        let ids: Vec<i64> = GoalsModel::finished_in(&goal, &entries)
            .iter()
            .map(|(e, _)| e.user_book_id.get())
            .collect();
        assert_eq!(ids, [4, 1, 5]);
    }

    #[test]
    fn goals_sort_by_end_date_then_newest() {
        let d = |y| Date::from_ymd_opt(y, 12, 31).unwrap();
        let goal = |id, end| Goal::new(GoalId::new(id), GoalMetric::Books, 1, d(2020), end);
        let mut goals = vec![goal(1, d(2025)), goal(2, d(2026)), goal(3, d(2026))];
        sort(&mut goals);
        let ids: Vec<i64> = goals.iter().map(|g| g.id.get()).collect();
        assert_eq!(ids, [3, 2, 1]);
    }
}
