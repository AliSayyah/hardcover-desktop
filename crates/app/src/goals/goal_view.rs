//! The goal dialogs: one goal's progress, pace and finished books, with its
//! target and deletion; and the form that creates a goal.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::date_picker::{DatePicker, DatePickerState};
use gpui_kit::component::form::{Field, Form};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    App, AppContext as _, Axis, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::WRITE_GOALS;
use hardcover_core::{Date, Goal, GoalId, GoalMetric, NewGoal};

use super::model::GoalsModel;
use crate::app_state::Services;
use crate::format::{
    goal_description, goal_pace, goal_period, goal_progress, goal_unit, needs_permission,
    retargeted_description,
};
use crate::library::model::LibraryModel;
use crate::ui::Meter;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::{open_alert_dialog, open_dialog};

fn today() -> Date {
    chrono::Local::now().date_naive()
}

/// Digits only, up to a million (a page goal can be large).
fn digits_only(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii_digit()) && text.len() <= 7
}

fn parse_target(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|n| *n > 0)
}

fn dialog_width(window: &Window) -> Pixels {
    rems(30.).to_pixels(window.rem_size())
}

fn label_width(window: &Window) -> Pixels {
    rems(5.).to_pixels(window.rem_size())
}

/// Opens the dialog for one of the model's goals.
pub fn open_goal(
    goals: Entity<GoalsModel>,
    library: Entity<LibraryModel>,
    id: GoalId,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(goal) = goals.read(cx).goal(id) else {
        return;
    };
    let title: SharedString = goal
        .description
        .clone()
        .unwrap_or_else(|| format!("{} reading goal", goal_period(goal.start, goal.end)))
        .into();
    let view = cx.new(|cx| GoalView::new(goals, library, id, window, cx));
    open_dialog(window, cx, move |dialog, window, _| {
        dialog
            .title(title.clone())
            .w(dialog_width(window))
            .child(view.clone())
    });
}

/// Opens the form that creates a goal.
pub fn open_new_goal(goals: Entity<GoalsModel>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| NewGoalView::new(goals, window, cx));
    open_dialog(window, cx, move |dialog, window, _| {
        dialog
            .title("New reading goal")
            .w(dialog_width(window))
            .child(view.clone())
    });
}

// ----- one goal -----

pub struct GoalView {
    goals: Entity<GoalsModel>,
    library: Entity<LibraryModel>,
    id: GoalId,
    target: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl GoalView {
    fn new(
        goals: Entity<GoalsModel>,
        library: Entity<LibraryModel>,
        id: GoalId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let current = goals.read(cx).goal(id).map(|g| g.target.to_string());
        let target = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("Target")
                .validate(|text, _| digits_only(text));
            if let Some(current) = current {
                state.set_value(current, window, cx);
            }
            state
        });
        let subscriptions = vec![
            cx.observe(&goals, |_, _, cx| cx.notify()),
            cx.observe(&library, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &target,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.save_target(window, cx),
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        Self {
            goals,
            library,
            id,
            target,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// The target typed in, if it is a valid new target.
    fn new_target(&self, goal: &Goal, cx: &App) -> Option<u32> {
        parse_target(&self.target.read(cx).value()).filter(|t| *t != goal.target)
    }

    fn save_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(goal) = self.goals.read(cx).goal(self.id).cloned() else {
            return;
        };
        let Some(target) = self.new_target(&goal, cx) else {
            return;
        };
        let mut edited = goal.clone();
        edited.target = target;
        edited.description = Some(retargeted_description(&goal, target));
        let task = self.goals.update(cx, |m, cx| m.update(edited, cx));
        cx.spawn_in(window, async move |this, cx| {
            if let Err(message) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(message);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(goal) = self.goals.read(cx).goal(self.id) else {
            return;
        };
        let title: SharedString = format!(
            "Delete the {} reading goal?",
            goal_period(goal.start, goal.end)
        )
        .into();
        let goals = self.goals.clone();
        let id = self.id;
        // One overlay at a time: the confirmation replaces this dialog.
        window.close_all_dialogs(cx);
        open_alert_dialog(window, cx, move |alert, _, _| {
            let goals = goals.clone();
            alert
                .title(title.clone())
                .description("It's deleted from Hardcover. Your books and reading dates stay.")
                .confirm()
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let task = goals.update(cx, |m, cx| m.delete(id, cx));
                    window
                        .spawn(cx, async move |cx| {
                            if let Err(message) = task.await {
                                let _ = cx.update(|window, cx| {
                                    window.push_notification(
                                        Notification::error(message)
                                            .title("Couldn't delete the goal"),
                                        cx,
                                    );
                                });
                            }
                        })
                        .detach();
                    true
                })
        });
    }

    fn render_summary(&self, goal: &Goal, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let percent = (goal.fraction() * 100.0).clamp(0.0, 999.0);
        let dates = format!(
            "{} – {}",
            goal.start.format("%b %-d, %Y"),
            goal.end.format("%b %-d, %Y")
        );
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .items_end()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(goal_progress(goal)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("{percent:.0}%")),
                    ),
            )
            .child(
                Meter::new("goal-progress", percent.min(100.0) as f32)
                    .accessibility_label("Progress toward the goal"),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .children(goal_pace(goal, today()))
                    .child(dates),
            )
            .when(!goal.conditions.is_empty(), |this| {
                this.child(div().text_xs().text_color(muted).child(
                    "Only some books count toward this goal; its filters are set on hardcover.app.",
                ))
            })
    }

    fn render_target(
        &self,
        goal: &Goal,
        label_width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let services = Services::global(cx);
        let can_write = services.has_scope(WRITE_GOALS);
        let editable = goal.conditions.to_input().is_some();
        let writing = self.goals.read(cx).is_writing();
        let changed = self.new_target(goal, cx).is_some();
        let unit = goal_unit(&goal.metric, f64::from(goal.target)).to_owned();
        let reason: Option<SharedString> = if !can_write {
            Some(needs_permission(WRITE_GOALS).into())
        } else if !editable {
            Some("This goal has filters the app can't keep. Change it on hardcover.app.".into())
        } else {
            None
        };
        Form::new()
            .label_layout(Axis::Horizontal)
            .label_width(label_width)
            .child(
                Field::new().label("Target").child(
                    h_flex()
                        .gap_2()
                        .child(
                            div().w(rems(7.)).child(
                                Input::new(&self.target)
                                    .disabled(reason.is_some() || writing)
                                    .raised_field(reason.is_some() || writing, cx),
                            ),
                        )
                        .child(div().text_sm().child(unit))
                        .child(
                            Button::new("save-goal-target")
                                .label("Save")
                                .loading(writing)
                                .disabled(reason.is_some() || writing || !changed)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_target(window, cx)),
                                ),
                        ),
                ),
            )
            .when_some(reason, |this, reason| {
                this.child(
                    Field::new().child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(reason),
                    ),
                )
            })
    }

    fn render_finished(&self, goal: &Goal, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let library = self.library.read(cx);
        let finished = GoalsModel::finished_in(goal, library.entries());
        let rows = finished.iter().map(|(entry, finished)| {
            let date = finished.format("%b %-d").to_string();
            h_flex()
                .id(SharedString::from(format!(
                    "goal-book-{}",
                    entry.user_book_id
                )))
                .gap_2()
                .justify_between()
                .text_sm()
                .child(div().min_w_0().truncate().child(entry.book.title.clone()))
                .child(div().flex_shrink_0().text_color(muted).child(date))
        });
        let count = finished.len();
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(format!("Finished in this period ({count})")),
            )
            .child(div().text_xs().text_color(muted).child(
                "From your library: books with a read finished between these dates. \
                 Hardcover counts the goal itself, so its number can differ.",
            ))
            .map(|this| {
                if count == 0 {
                    this.child(div().text_sm().text_color(muted).child("None yet."))
                } else {
                    this.child(
                        v_flex()
                            .id("goal-finished-books")
                            .gap_1()
                            .max_h(rems(12.))
                            .overflow_y_scrollbar()
                            .children(rows),
                    )
                }
            })
    }
}

impl Render for GoalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(goal) = self.goals.read(cx).goal(self.id).cloned() else {
            // Deleted meanwhile (e.g. from another window); nothing to show.
            return v_flex().child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("This goal no longer exists."),
            );
        };
        let can_write = Services::global(cx).has_scope(WRITE_GOALS);
        let writing = self.goals.read(cx).is_writing();
        v_flex()
            .gap_5()
            .child(self.render_summary(&goal, cx))
            .child(self.render_target(&goal, label_width(window), cx))
            .child(self.render_finished(&goal, cx))
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                h_flex().child(
                    Button::new("delete-goal")
                        .ghost()
                        .danger()
                        .label("Delete goal…")
                        .disabled(!can_write || writing)
                        .when(!can_write, |this| {
                            this.tooltip(needs_permission(WRITE_GOALS))
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.confirm_delete(window, cx)),
                        ),
                ),
            )
    }
}

// ----- a new goal -----

const METRICS: [(GoalMetric, &str); 3] = [
    (GoalMetric::Books, "Books"),
    (GoalMetric::Pages, "Pages"),
    (GoalMetric::Hours, "Hours"),
];

pub struct NewGoalView {
    goals: Entity<GoalsModel>,
    metric: usize,
    target: Entity<InputState>,
    start: Entity<DatePickerState>,
    end: Entity<DatePickerState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl NewGoalView {
    fn new(goals: Entity<GoalsModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let year = chrono::Datelike::year(&today());
        let target = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. 24")
                .validate(|text, _| digits_only(text))
        });
        let picker = |date: Option<Date>, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut state = DatePickerState::new(window, cx).date_format("%b %-d, %Y");
                state.set_date(CalendarDate::Single(date), window, cx);
                state
            })
        };
        let start = picker(Date::from_ymd_opt(year, 1, 1), window, cx);
        let end = picker(Date::from_ymd_opt(year, 12, 31), window, cx);
        let subscriptions = vec![
            cx.observe(&goals, |_, _, cx| cx.notify()),
            cx.observe(&start, |_, _, cx| cx.notify()),
            cx.observe(&end, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &target,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.create(window, cx),
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        target.update(cx, |input, cx| input.focus(window, cx));
        Self {
            goals,
            metric: 0,
            target,
            start,
            end,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// The goal as entered, or why it can't be created.
    fn draft(&self, cx: &App) -> Result<NewGoal, &'static str> {
        let target =
            parse_target(&self.target.read(cx).value()).ok_or("Enter a target above 0.")?;
        let start = self
            .start
            .read(cx)
            .date()
            .start()
            .ok_or("Choose a start date.")?;
        let end = self
            .end
            .read(cx)
            .date()
            .start()
            .ok_or("Choose an end date.")?;
        if end < start {
            return Err("The end date is before the start date.");
        }
        let metric = METRICS[self.metric].0.clone();
        Ok(NewGoal {
            description: goal_description(&metric, target, start, end),
            metric,
            target,
            start,
            end,
        })
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let goal = match self.draft(cx) {
            Ok(goal) => goal,
            Err(message) => {
                self.error = Some(message.into());
                cx.notify();
                return;
            }
        };
        let task = self.goals.update(cx, |m, cx| m.create(goal, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => window.close_dialog(cx),
                Err(message) => {
                    this.error = Some(message);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for NewGoalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writing = self.goals.read(cx).is_writing();
        let can_write = Services::global(cx).has_scope(WRITE_GOALS);
        let metric = METRICS[self.metric].0.clone();
        let unit = goal_unit(&metric, 2.0).to_owned();
        let view = cx.entity().downgrade();
        let metrics = RadioGroup::horizontal("goal-metric")
            .children(METRICS.map(|(_, label)| Radio::new(SharedString::from(label)).label(label)))
            .selected_index(Some(self.metric))
            .disabled(writing)
            .on_change(move |ix, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.metric = *ix;
                    cx.notify();
                });
            });
        v_flex()
            .gap_4()
            .child(
                Form::new()
                    .label_layout(Axis::Horizontal)
                    .label_width(label_width(window))
                    .child(Field::new().label("Count").child(metrics))
                    .child(
                        Field::new().label("Target").child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div().w(rems(7.)).child(
                                        Input::new(&self.target)
                                            .disabled(writing)
                                            .raised_field(writing, cx),
                                    ),
                                )
                                .child(div().text_sm().child(unit)),
                        ),
                    )
                    .child(
                        Field::new().label("From").child(
                            div().w_full().max_w_56().child(
                                DatePicker::new(&self.start)
                                    .cleanable(false)
                                    .disabled(writing),
                            ),
                        ),
                    )
                    .child(
                        Field::new().label("To").child(
                            div().w_full().max_w_56().child(
                                DatePicker::new(&self.end)
                                    .cleanable(false)
                                    .disabled(writing),
                            ),
                        ),
                    ),
            )
            .when(matches!(metric, GoalMetric::Hours), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Hours count time read and listened, as Hardcover tracks it."),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-new-goal")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("create-goal")
                            .primary()
                            .label("Create")
                            .loading(writing)
                            .disabled(writing || !can_write)
                            .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_whole_numbers_above_zero() {
        assert_eq!(parse_target(" 24 "), Some(24));
        assert_eq!(parse_target("0"), None);
        assert_eq!(parse_target(""), None);
        assert_eq!(parse_target("1.5"), None);
        assert!(digits_only("1200"));
        assert!(!digits_only("12a"));
        assert!(!digits_only("12345678"));
    }
}
