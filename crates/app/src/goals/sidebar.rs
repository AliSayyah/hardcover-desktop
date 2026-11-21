//! The sidebar's "Reading goal" section: each current goal as a goal row
//! (`ui::GoalRow`: the period, "9 of 12 books" and a meter with its pace
//! tick), or a way to set one.

use chrono::Datelike as _;
use gpui_kit::component::IconName;
use gpui_kit::{
    App, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Subscription,
    Window, div,
};
use hardcover_core::auth::scopes::{READ_GOALS, WRITE_GOALS};
use hardcover_core::{Date, Goal};

use super::goal_view::{open_goal, open_new_goal};
use super::model::{GoalsModel, GoalsStatus};
use crate::app_state::Services;
use crate::format::{goal_pace, goal_period, goal_progress, needs_permission};
use crate::library::model::LibraryModel;
use crate::ui::{GoalRow, NavRow, sidebar_label};

pub struct GoalsSection {
    goals: Entity<GoalsModel>,
    library: Entity<LibraryModel>,
    _subscription: Subscription,
}

impl GoalsSection {
    pub fn new(
        goals: Entity<GoalsModel>,
        library: Entity<LibraryModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _subscription: cx.observe(&goals, |_, _, cx| cx.notify()),
            goals,
            library,
        }
    }

    /// True if the section has something to show: the session can read goals,
    /// and they have loaded (or failed to) at least once.
    pub fn is_visible(&self, cx: &App) -> bool {
        let model = self.goals.read(cx);
        Services::global(cx).has_scope(READ_GOALS)
            && !matches!(model.status(), GoalsStatus::Idle)
            && model.has_loaded()
    }
}

/// Where steady progress would put the goal today, as a percentage of its
/// target: the meter's pace tick. `None` once the goal is reached or outside
/// its dates, where a tick means nothing.
pub fn pace_percent(goal: &Goal, today: Date) -> Option<f32> {
    let reached = goal.progress >= f64::from(goal.target);
    (!reached && goal.is_current(today)).then(|| {
        let expected = goal.expected_by(today) / f64::from(goal.target.max(1));
        crate::format::bar_percent(expected)
    })
}

/// The goal row's label: the year for a calendar-year goal ("2026"), else
/// its end ("By Jun 30"), since both dates don't fit beside the count.
fn row_label(goal: &Goal, today: Date) -> String {
    let period = goal_period(goal.start, goal.end);
    if period.len() == 4 {
        period
    } else if goal.end.year() == today.year() {
        format!("By {}", goal.end.format("%b %-d"))
    } else {
        format!("By {}", goal.end.format("%b %-d, %Y"))
    }
}

impl Render for GoalsSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today = chrono::Local::now().date_naive();
        let model = self.goals.read(cx);
        let can_write = Services::global(cx).has_scope(WRITE_GOALS);
        let goals = self.goals.clone();
        let library = self.library.clone();

        let mut rows: Vec<gpui_kit::AnyElement> = model
            .current(today)
            .map(|goal| {
                let period = goal_period(goal.start, goal.end);
                let count = goal_progress(goal);
                let accessible: SharedString = match goal_pace(goal, today) {
                    Some(pace) => format!("{period} reading goal, {count}, {pace}").into(),
                    None => format!("{period} reading goal, {count}").into(),
                };
                let percent = crate::format::bar_percent(goal.fraction());
                let (goals, library, id) = (goals.clone(), library.clone(), goal.id);
                GoalRow::new(
                    SharedString::from(format!("goal-row-{id}")),
                    row_label(goal, today),
                    count,
                    percent,
                )
                .pace(pace_percent(goal, today))
                .accessibility_label(accessible)
                .on_click(move |_, window, cx| {
                    open_goal(goals.clone(), library.clone(), id, window, cx);
                })
                .into_any_element()
            })
            .collect();
        let count = rows.len();
        if rows.is_empty() {
            rows.push(match model.status() {
                GoalsStatus::Failed(message) if model.goals().is_empty() => {
                    let goals = goals.clone();
                    NavRow::new(
                        "goal-retry",
                        IconName::CircleAlert,
                        "Try loading your goal again",
                    )
                    .tooltip(Some(format!("Couldn't load your reading goal. {message}")))
                    .on_click(move |_, _, cx| goals.update(cx, GoalsModel::load))
                    .into_any_element()
                }
                _ => {
                    let goals = goals.clone();
                    NavRow::new("goal-new", IconName::Plus, "Set a reading goal…")
                        .muted(true)
                        .disabled(!can_write)
                        .tooltip((!can_write).then(|| needs_permission(WRITE_GOALS)))
                        .on_click(move |_, window, cx| open_new_goal(goals.clone(), window, cx))
                        .into_any_element()
                }
            });
        }
        let label = if count > 1 {
            "Reading goals"
        } else {
            "Reading goal"
        };
        div().child(sidebar_label(label, cx)).children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{GoalId, GoalMetric};

    fn goal(start: Date, end: Date) -> Goal {
        Goal::new(GoalId::new(1), GoalMetric::Books, 12, start, end)
    }

    #[test]
    fn a_goal_row_names_its_year_or_its_end() {
        let d = |y, m, d| Date::from_ymd_opt(y, m, d).unwrap();
        let today = d(2026, 11, 20);
        assert_eq!(
            row_label(&goal(d(2026, 1, 1), d(2026, 12, 31)), today),
            "2026"
        );
        assert_eq!(
            row_label(&goal(d(2026, 3, 1), d(2026, 6, 30)), today),
            "By Jun 30"
        );
        assert_eq!(
            row_label(&goal(d(2026, 9, 1), d(2027, 3, 31)), today),
            "By Mar 31, 2027"
        );
    }
}
