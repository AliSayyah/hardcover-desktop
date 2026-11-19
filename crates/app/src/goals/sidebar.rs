//! The sidebar's "Reading goal" section: each current goal with its progress,
//! or a way to set one.

use gpui_kit::component::progress::Progress;
use gpui_kit::component::sidebar::{SidebarGroup, SidebarItem as _, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{IconName, Sizable as _};
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, rems,
};
use hardcover_core::auth::scopes::{READ_GOALS, WRITE_GOALS};

use super::goal_view::{open_goal, open_new_goal};
use super::model::{GoalsModel, GoalsStatus};
use crate::app_state::Services;
use crate::format::{goal_pace, goal_period, goal_progress};
use crate::library::model::LibraryModel;

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
    pub fn is_visible(&self, cx: &gpui_kit::App) -> bool {
        Services::global(cx).has_scope(READ_GOALS)
            && !matches!(self.goals.read(cx).status(), GoalsStatus::Idle)
            && !(self.goals.read(cx).goals().is_empty()
                && *self.goals.read(cx).status() == GoalsStatus::Loading)
    }
}

impl Render for GoalsSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today = chrono::Local::now().date_naive();
        let model = self.goals.read(cx);
        let can_write = Services::global(cx).has_scope(WRITE_GOALS);
        let goals = self.goals.clone();
        let library = self.library.clone();

        let mut items: Vec<SidebarMenuItem> = model
            .current(today)
            .map(|goal| {
                let period = goal_period(goal.start, goal.end);
                let progress = goal_progress(goal);
                // A calendar year names itself; other dates would not fit.
                let label = if period.len() == 4 {
                    format!("{period} · {progress}")
                } else {
                    progress
                };
                let percent = (goal.fraction() * 100.0).clamp(0.0, 100.0) as f32;
                let accessible: SharedString = match goal_pace(goal, today) {
                    Some(pace) => format!("{label}, {pace}").into(),
                    None => label.clone().into(),
                };
                let (goals, library, id) = (goals.clone(), library.clone(), goal.id);
                SidebarMenuItem::new(label)
                    .icon(IconName::ChartPie)
                    .accessibility_label(accessible)
                    .suffix(move |_, _| {
                        div().w(rems(2.5)).child(
                            Progress::new(SharedString::from(format!("goal-bar-{id}")))
                                .xsmall()
                                .value(percent),
                        )
                    })
                    .on_click(move |_, window, cx| {
                        open_goal(goals.clone(), library.clone(), id, window, cx)
                    })
            })
            .collect();
        if items.is_empty() {
            items.push(match model.status() {
                GoalsStatus::Failed(_) if model.goals().is_empty() => {
                    let goals = goals.clone();
                    SidebarMenuItem::new("Couldn't load goals. Retry")
                        .icon(IconName::CircleAlert)
                        .on_click(move |_, _, cx| goals.update(cx, |m, cx| m.load(cx)))
                }
                _ => {
                    let goals = goals.clone();
                    SidebarMenuItem::new("Set a reading goal…")
                        .icon(IconName::Plus)
                        .disable(!can_write)
                        .on_click(move |_, window, cx| open_new_goal(goals.clone(), window, cx))
                }
            });
        }
        let label = if items.len() > 1 {
            "Reading goals"
        } else {
            "Reading goal"
        };
        div().child(
            SidebarGroup::new(label)
                .child(SidebarMenu::new().children(items))
                .render("goals", window, cx),
        )
    }
}
