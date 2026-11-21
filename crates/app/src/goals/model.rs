//! The user's reading goals as an entity: loaded once signed in, reloaded
//! shortly after the library changes (Hardcover recalculates progress from
//! the user's reading), and the writes that change them.
//!
//! Everything here needs the goal scopes; without `read:goals` the model
//! stays empty and the views hide the goal section.

use std::time::Duration;

use gpui_kit::{AppContext as _, Context, Entity, SharedString, Subscription, Task};
use hardcover_core::auth::scopes::{READ_GOALS, WRITE_GOALS};
use hardcover_core::{Date, Error, Goal, GoalId, LibraryEntry, NewGoal};

use crate::app_state::Services;
use crate::format::requested_scope;
use crate::library::model::{LibraryModel, SyncStatus};

/// How long after a library change to reload: Hardcover recalculates goal
/// progress after a write, and several writes in a row need one reload.
const RELOAD_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Default)]
pub enum GoalsStatus {
    /// Not loaded, or the session can't read goals.
    #[default]
    Idle,
    Loading,
    Loaded,
    /// The last load failed; the goals shown (if any) are the previous ones.
    Failed(SharedString),
}

pub struct GoalsModel {
    goals: Vec<Goal>,
    status: GoalsStatus,
    /// A load has finished (or failed) at least once. Reloads keep the old
    /// goals on screen, so the sidebar doesn't flicker while they run.
    loaded_once: bool,
    /// A write is in flight; the controls that write are disabled meanwhile.
    writing: bool,
    /// The library revision the goals were last loaded after.
    seen_revision: u64,
    load_task: Option<Task<()>>,
    reload_task: Option<Task<()>>,
    _subscription: Subscription,
}

impl GoalsModel {
    pub fn new(library: &Entity<LibraryModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(library, |this, library, cx| {
            let library = library.read(cx);
            let revision = library.revision();
            // Pages of a running sync are not news; its result is.
            if revision != this.seen_revision && *library.sync_status() != SyncStatus::Syncing {
                this.seen_revision = revision;
                this.schedule_reload(cx);
            }
        });
        Self {
            seen_revision: library.read(cx).revision(),
            goals: Vec::new(),
            status: GoalsStatus::Idle,
            loaded_once: false,
            writing: false,
            load_task: None,
            reload_task: None,
            _subscription: subscription,
        }
    }

    // ----- reads -----

    pub fn goals(&self) -> &[Goal] {
        &self.goals
    }

    pub fn goal(&self, id: GoalId) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }

    /// Goals whose dates include `today`, the latest end date first.
    pub fn current(&self, today: Date) -> impl Iterator<Item = &Goal> {
        self.goals.iter().filter(move |g| g.is_current(today))
    }

    pub fn status(&self) -> &GoalsStatus {
        &self.status
    }

    pub fn has_loaded(&self) -> bool {
        self.loaded_once
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

    /// Loads the goals, keeping the ones shown until the new ones arrive.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        if !Services::global(cx).has_scope(READ_GOALS) {
            return;
        }
        self.status = GoalsStatus::Loading;
        cx.notify();
        let client = Services::global(cx).client().clone();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { client.goals() }).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(goals) => {
                    this.goals = goals;
                    this.status = GoalsStatus::Loaded;
                    this.loaded_once = true;
                    cx.notify();
                }
                // A scope the app never requests falls through to the
                // generic message: signing in again could not grant it.
                Err(Error::InsufficientScope { scope })
                    if scope.is_none() || requested_scope(scope.as_deref()).is_some() =>
                {
                    this.goals.clear();
                    this.status = GoalsStatus::Idle;
                    let scope = requested_scope(scope.as_deref()).unwrap_or(READ_GOALS);
                    Services::scope_refused(scope, cx);
                    cx.notify();
                }
                Err(e) => {
                    this.loaded_once = true;
                    this.status = GoalsStatus::Failed(Services::error_message(&e, None, cx).into());
                    cx.notify();
                }
            });
        }));
    }

    fn schedule_reload(&mut self, cx: &mut Context<Self>) {
        if !Services::global(cx).has_scope(READ_GOALS) {
            return;
        }
        self.reload_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_DELAY).await;
            let _ = this.update(cx, GoalsModel::load);
        }));
    }

    // ----- writes -----

    /// Runs one goal write in the background and applies its result. `Err`
    /// carries the message to show; the goals are unchanged then.
    fn write<T: Send + 'static>(
        &mut self,
        run: impl FnOnce(&hardcover_core::Client) -> Result<T, Error> + Send + 'static,
        apply: impl FnOnce(&mut Self, T) + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        if self.writing {
            return Task::ready(Err(
                "Another change is still being saved. Try again in a moment.".into(),
            ));
        }
        if !Services::global(cx).has_scope(WRITE_GOALS) {
            return Task::ready(Err(crate::format::needs_permission(WRITE_GOALS).into()));
        }
        self.writing = true;
        cx.notify();
        let client = Services::global(cx).client().clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { run(&client) }).await;
            this.update(cx, |this, cx| {
                this.writing = false;
                cx.notify();
                match result {
                    Ok(value) => {
                        // A load that started before the write would bring
                        // back the old goals; the write's result is newer.
                        // It may also carry a library change the goals
                        // haven't seen, so load again afterwards. A reload
                        // still waiting on its timer runs as planned.
                        if this.status == GoalsStatus::Loading {
                            this.load_task = None;
                            this.schedule_reload(cx);
                        }
                        this.status = GoalsStatus::Loaded;
                        apply(this, value);
                        Ok(())
                    }
                    Err(e) => Err(Services::error_message(&e, Some(WRITE_GOALS), cx).into()),
                }
            })
            .unwrap_or(Ok(()))
        })
    }

    pub fn create(
        &mut self,
        goal: NewGoal,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        self.write(
            move |client| client.create_goal(&goal),
            |this, created| {
                this.goals.push(created);
                sort(&mut this.goals);
            },
            cx,
        )
    }

    /// Saves an edited copy of one of the model's goals.
    pub fn update(&mut self, goal: Goal, cx: &mut Context<Self>) -> Task<Result<(), SharedString>> {
        self.write(
            move |client| client.update_goal(&goal),
            |this, saved| {
                match this.goals.iter_mut().find(|g| g.id == saved.id) {
                    Some(g) => *g = saved,
                    None => this.goals.push(saved),
                }
                sort(&mut this.goals);
            },
            cx,
        )
    }

    pub fn delete(&mut self, id: GoalId, cx: &mut Context<Self>) -> Task<Result<(), SharedString>> {
        self.write(
            move |client| client.delete_goal(id),
            move |this, ()| this.goals.retain(|g| g.id != id),
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
