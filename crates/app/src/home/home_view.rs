//! The Home destination, shown at launch: the books being read with their
//! pace, books finished lately to rate, the reading goal, what to read next,
//! and what is coming soon in the reader's series. Everything comes from the
//! library on this computer and the goals and upcoming books already loaded;
//! Home asks Hardcover for nothing until the reader acts.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Pixels, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription,
    Window, div, img, prelude::FluentBuilder as _, px, relative, rems,
};
use hardcover_core::auth::scopes::{READ_GOALS, WRITE_LIBRARY};
use hardcover_core::{
    Client, Date, Error, Goal, GoalMetric, LibraryEntry, Rating, ReadingStatus, UserBookId,
};

use super::coming_soon::coming_soon_card;
use super::digest::{
    Digest, Reading, SeriesNext, ToRate, compute, finished_per_month, finishing_by, to_rate,
};
use super::progress_dialog::open_progress;
use crate::app_state::{Services, cover_thumbnail};
use crate::book::star_rating::StarRating;
use crate::catalog::navigation::{self, Page};
use crate::format::{
    authors_text, goal_amount, goal_pace, goal_period, goal_unit, needs_permission,
    progress_summary, rating_text, series_position_text,
};
use crate::goals::goal_view::open_goal;
use crate::goals::model::GoalsModel;
use crate::library::library_view::empty_state;
use crate::library::model::{LibraryModel, WriteKind};
use crate::upcoming::model::UpcomingModel;
use crate::{ShowPaused, ShowWantToRead};

pub struct HomeView {
    model: Entity<LibraryModel>,
    goals: Entity<GoalsModel>,
    upcoming: Entity<UpcomingModel>,
    focus_handle: FocusHandle,
    /// The digest and the library revision, upcoming books' revision and
    /// date it was computed for.
    computed: Option<((u64, u64, Date), Rc<Digest>)>,
    /// Books rated on Home since it was last opened from the sidebar: their
    /// rows stay, so the rating can still be changed.
    rated_here: HashSet<UserBookId>,
    /// A rating being saved, shown until the write lands.
    saving_ratings: HashMap<UserBookId, Option<Rating>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for HomeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl HomeView {
    pub fn new(
        model: Entity<LibraryModel>,
        goals: Entity<GoalsModel>,
        upcoming: Entity<UpcomingModel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe(&goals, |_, _, cx| cx.notify()),
            cx.observe(&upcoming, |_, _, cx| cx.notify()),
        ];
        Self {
            model,
            goals,
            upcoming,
            focus_handle: cx.focus_handle(),
            computed: None,
            rated_here: HashSet::new(),
            saving_ratings: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    /// Home was opened from the sidebar: books rated on the last visit leave
    /// "Rate what you finished". Coming back from a book page keeps them.
    pub fn opened(&mut self, cx: &mut Context<Self>) {
        if !self.rated_here.is_empty() {
            self.rated_here.clear();
            cx.notify();
        }
    }

    fn digest(&mut self, today: Date, cx: &App) -> Rc<Digest> {
        let model = self.model.read(cx);
        let upcoming = self.upcoming.read(cx);
        let key = (model.revision(), upcoming.revision(), today);
        match &self.computed {
            Some((k, digest)) if *k == key => digest.clone(),
            _ => {
                let unreleased = upcoming.unreleased(today);
                let digest = Rc::new(compute(model.entries(), today, &unreleased));
                self.computed = Some((key, digest.clone()));
                digest
            }
        }
    }

    /// The goal Home shows: the current one counted in books, else the first
    /// current one. The sidebar lists them all.
    fn goal(&self, today: Date, cx: &App) -> Option<Goal> {
        if !Services::global(cx).has_scope(READ_GOALS) {
            return None;
        }
        let goals = self.goals.read(cx);
        goals
            .current(today)
            .find(|g| g.metric == GoalMetric::Books)
            .or_else(|| goals.current(today).next())
            .cloned()
    }

    /// Runs a write on `id` in the background; the model applies the entry it
    /// returns. A failure is shown as a notification.
    fn write(
        &mut self,
        id: UserBookId,
        kind: WriteKind,
        failure_title: &'static str,
        op: impl FnOnce(&Client) -> Result<LibraryEntry, Error> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model = self.model.clone();
        if !model.update(cx, |m, cx| m.begin_write(id, kind, cx)) {
            return;
        }
        let client = Services::global(cx).client().clone();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx.background_spawn(async move { op(&client) }).await;
            model.update(cx, |m, cx| {
                m.end_write(id, cx);
                if let Ok(entry) = &result {
                    m.apply_written(entry.clone(), cx);
                }
            });
            if let Err(error) = result {
                let _ = cx.update(|window, cx| {
                    let message = Services::error_message(&error, Some(WRITE_LIBRARY), cx);
                    window.push_notification(Notification::error(message).title(failure_title), cx);
                });
            }
        })
        .detach();
    }

    fn set_status(
        &mut self,
        id: UserBookId,
        status: ReadingStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.write(
            id,
            WriteKind::Status,
            "Couldn't change the status",
            move |client| client.set_status(id, status),
            window,
            cx,
        );
    }

    fn rate(
        &mut self,
        id: UserBookId,
        rating: Option<Rating>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model = self.model.read(cx);
        // A change while the last one saves is dropped: the stars stay
        // enabled during a save only so that they keep the keyboard focus.
        if model.pending_write(id).is_some() {
            return;
        }
        let Some(keep) = model.entry(id).map(|e| e.status) else {
            return;
        };
        self.rated_here.insert(id);
        self.saving_ratings.insert(id, rating);
        // Sending the status keeps Hardcover from starting a new read.
        self.write(
            id,
            WriteKind::Rating,
            "Couldn't save the rating",
            move |client| client.set_rating(id, rating, keep),
            window,
            cx,
        );
    }

    // ----- sections -----

    fn render_reading(
        &self,
        digest: &Digest,
        today: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let model = self.model.read(cx);
        let books: Vec<(Reading, LibraryEntry)> = digest
            .reading
            .iter()
            .filter_map(|r| Some((r.clone(), model.entry(r.id)?.clone())))
            .collect();
        let rows: Vec<AnyElement> = books
            .iter()
            .enumerate()
            .map(|(ix, (reading, entry))| {
                self.reading_row(ix, reading, entry, today, window, cx)
                    .into_any_element()
            })
            .collect();
        let paused = digest
            .paused
            .and_then(|id| self.model.read(cx).entry(id).cloned())
            .map(|entry| self.paused_row(&entry, digest.paused_count, today, window, cx));
        let empty = rows.is_empty();
        section("Continue reading", (!empty).then_some(rows.len()), cx)
            .when(empty, |this| {
                this.child(note(
                    "You're not reading anything right now. Start a book from Up next, or from any book's page.",
                    cx,
                ))
            })
            .child(v_flex().children(rows))
            .children(paused)
    }

    fn reading_row(
        &self,
        ix: usize,
        reading: &Reading,
        entry: &LibraryEntry,
        today: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = entry.user_book_id;
        let muted = cx.theme().muted_foreground;
        let progress = progress_summary(entry);
        let pending = self.model.read(cx).pending_write(id);
        let can_write = Services::global(cx).has_scope(WRITE_LIBRARY);
        let disabled_reason = (!can_write).then(|| needs_permission(WRITE_LIBRARY));

        let mut position = progress.text();
        if let Some(pct) = progress.percent
            && (progress.page.is_some() || progress.seconds.is_some())
        {
            position.push_str(&format!(" · {}%", pct.round() as u32));
        }
        let mut meta: Vec<AnyElement> = vec![
            div()
                .text_color(cx.theme().foreground)
                .child(position)
                .into_any_element(),
        ];
        if let Some(pace) = reading.pace {
            meta.push(
                div()
                    .child(pages_a_day(pace.pages_per_day))
                    .into_any_element(),
            );
            if reading.idle_days.is_none()
                && let Some(finish) = pace.finish
            {
                meta.push(
                    h_flex()
                        .gap_1()
                        .child("Done around")
                        .child(
                            div()
                                .text_color(cx.theme().foreground)
                                .child(short_date(finish, today)),
                        )
                        .into_any_element(),
                );
            }
        }
        if let Some(days) = reading.idle_days {
            // The icon carries the colour: amber text is hard to read.
            meta.push(
                h_flex()
                    .gap_1()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .xsmall()
                            .text_color(cx.theme().warning),
                    )
                    .child(format!("Last updated {days} days ago"))
                    .into_any_element(),
            );
        }

        let mut byline = authors_text(&entry.book.authors);
        if let Some(series) = entry.series.first() {
            let name = match series.position {
                Some(p) => format!("{} {}", series.name, series_position_text(p)),
                None => series.name.clone(),
            };
            byline = if byline.is_empty() {
                name
            } else {
                format!("{byline} · {name}")
            };
        }

        let model = self.model.clone();
        let entry_for_dialog = entry.clone();
        h_flex()
            .id(SharedString::from(format!("home-reading-{id}")))
            .gap_4()
            .py_2()
            .when(ix > 0, |this| {
                this.border_t_1().border_color(cx.theme().border)
            })
            .child(cover(entry, CoverSize::Large, window, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(title_button("home-title", entry, FontWeight::SEMIBOLD))
                    .when(!byline.is_empty(), |this| {
                        this.child(div().text_sm().text_color(muted).truncate().child(byline))
                    })
                    .child(
                        div().pt_1().child(
                            Progress::new(SharedString::from(format!("home-progress-{id}")))
                                .small()
                                .value(progress.percent.unwrap_or(0.0))
                                .accessibility_label("Reading progress"),
                        ),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x_4()
                            .gap_y_0p5()
                            .text_sm()
                            .text_color(muted)
                            .children(meta),
                    ),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .w(rems(9.5))
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("home-update-{id}")))
                            .outline()
                            .small()
                            .w_full()
                            .label("Update progress…")
                            .loading(pending == Some(WriteKind::Progress))
                            .disabled(pending.is_some() || !can_write)
                            .when_some(disabled_reason.clone(), |b, r| b.tooltip(r))
                            .on_click(move |_, window, cx| {
                                open_progress(model.clone(), &entry_for_dialog, window, cx)
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("home-finish-{id}")))
                            .ghost()
                            .small()
                            .w_full()
                            .label("Mark as read")
                            .loading(pending == Some(WriteKind::Status))
                            .disabled(pending.is_some() || !can_write)
                            .when_some(disabled_reason, |b, r| b.tooltip(r))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_finished(id, window, cx)
                            })),
                    ),
            )
    }

    /// Asks before finishing a book: the read ends today, and moving the
    /// book back to Currently Reading would start another read.
    fn confirm_finished(&mut self, id: UserBookId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.model.read(cx).entry(id).map(|e| e.book.title.clone()) else {
            return;
        };
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title(format!("Mark “{title}” as read?"))
                .description("It moves to your Read shelf, and this read ends today.")
                .confirm()
                .ok_text("Mark as read")
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.set_status(id, ReadingStatus::Read, window, cx)
                    });
                    true
                })
        });
    }

    /// The most recently paused book, so it isn't forgotten. Opening it is
    /// the way back: moving it to Currently Reading would start a new read.
    fn paused_row(
        &self,
        entry: &LibraryEntry,
        count: usize,
        today: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let progress = progress_summary(entry);
        let mut details = String::from("Paused");
        if let Some(pct) = progress.percent {
            details.push_str(&format!(" at {}%", pct.round() as u32));
        }
        if let Some(updated) = entry.updated_at {
            let day = updated.with_timezone(&chrono::Local).date_naive();
            details.push_str(&format!(", since {}", short_date(day, today)));
        }
        h_flex()
            .mt_2()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius_lg)
            .bg(cx.theme().muted)
            .child(cover(entry, CoverSize::Small, window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(title_button("home-paused", entry, FontWeight::MEDIUM))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(muted)
                            .child(details),
                    ),
            )
            .when(count > 1, |this| {
                this.child(
                    Button::new("home-all-paused")
                        .ghost()
                        .small()
                        .label(format!("All {count} paused"))
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowPaused), cx)),
                )
            })
            .into_any_element()
    }

    /// Books finished lately with no rating, each with its stars. Hidden
    /// without `write:library`, as there is nothing to do in it then.
    fn render_rate(
        &self,
        rows: &[ToRate],
        unrated: usize,
        today: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let library = self.model.read(cx);
        let books: Vec<(Date, LibraryEntry)> = rows
            .iter()
            .filter_map(|r| Some((r.finished, library.entry(r.id)?.clone())))
            .collect();
        let rows = books
            .iter()
            .enumerate()
            .map(|(ix, (finished, entry))| self.rate_row(ix, entry, *finished, today, window, cx))
            .collect::<Vec<_>>();
        section(
            "Rate what you finished",
            (unrated > 0).then_some(unrated),
            cx,
        )
        .child(v_flex().children(rows))
    }

    fn rate_row(
        &self,
        ix: usize,
        entry: &LibraryEntry,
        finished: Date,
        today: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.user_book_id;
        let pending = self.model.read(cx).pending_write(id);
        let rating = match self.saving_ratings.get(&id) {
            Some(saving) if pending == Some(WriteKind::Rating) => *saving,
            _ => entry.rating,
        };
        let muted = cx.theme().muted_foreground;
        let mut byline = authors_text(&entry.book.authors);
        let finished_text = format!("Finished {}", short_date(finished, today));
        byline = if byline.is_empty() {
            finished_text
        } else {
            format!("{byline} · {finished_text}")
        };
        h_flex()
            .id(SharedString::from(format!("home-rate-{id}")))
            .gap_3()
            .py_1p5()
            .when(ix > 0, |this| {
                this.border_t_1().border_color(cx.theme().border)
            })
            .child(cover(entry, CoverSize::Small, window, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(title_button("home-rate-title", entry, FontWeight::MEDIUM))
                    .child(div().text_sm().text_color(muted).truncate().child(byline)),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .child(
                        StarRating::new(SharedString::from(format!("home-stars-{id}")))
                            .value(rating)
                            .on_change(cx.listener(
                                move |this, rating: &Option<Rating>, window, cx| {
                                    this.rate(id, *rating, window, cx)
                                },
                            )),
                    )
                    // Fixed width, so the stars line up from row to row.
                    .child(
                        div()
                            .w(rems(1.75))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .map(|this| {
                                if pending == Some(WriteKind::Rating) {
                                    this.child(Spinner::new().small())
                                } else {
                                    this.children(rating.map(rating_text))
                                }
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_goal(
        &self,
        goal: &Goal,
        digest: &Digest,
        today: Date,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let muted = cx.theme().muted_foreground;
        let period = goal_period(goal.start, goal.end);
        let title = if period.len() == 4 {
            format!("{period} reading goal")
        } else {
            "Reading goal".to_owned()
        };
        let fraction = goal.fraction().clamp(0.0, 1.0) as f32;
        let reached = goal.progress >= f64::from(goal.target);
        let expected =
            (goal.expected_by(today) / f64::from(goal.target.max(1))).clamp(0.0, 1.0) as f32;
        let library = self.model.read(cx);
        let months = (goal.metric == GoalMetric::Books)
            .then(|| finished_per_month(goal, library.entries()))
            .flatten();

        let mut status = goal_pace(goal, today).unwrap_or_default();
        let days_left = (goal.end - today).num_days();
        if !reached && goal.is_current(today) {
            status.push_str(&match days_left {
                0 => ", and today is the last day".to_owned(),
                1 => ", with 1 day left".to_owned(),
                n => format!(", with {n} days left"),
            });
        }
        let mut notes: Vec<String> = Vec::new();
        let finishing = finishing_by(goal, digest, today);
        if !reached && finishing > 0 {
            let total = goal.progress.floor() as usize + finishing;
            notes.push(match finishing {
                1 => {
                    format!("1 book you're reading should be done by the end, which makes {total}.")
                }
                n => format!(
                    "{n} books you're reading should be done by the end, which makes {total}."
                ),
            });
        }
        let goals = self.goals.clone();
        let model = self.model.clone();
        let goal_id = goal.id;
        let progress_text = goal_amount(&goal.metric, goal.progress);
        let target_text = format!(
            "of {} {}",
            goal.target,
            goal_unit(&goal.metric, f64::from(goal.target))
        );

        card(cx)
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .child(
                        v_flex()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                            .child(
                                h_flex()
                                    .items_baseline()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(progress_text),
                                    )
                                    .child(div().text_sm().text_color(muted).child(target_text)),
                            ),
                    )
                    .child(
                        Button::new("home-goal-details")
                            .ghost()
                            .xsmall()
                            .label("Details…")
                            .on_click(move |_, window, cx| {
                                open_goal(goals.clone(), model.clone(), goal_id, window, cx)
                            }),
                    ),
            )
            .child(goal_bar(
                fraction,
                (!reached && goal.is_current(today)).then_some(expected),
                cx,
            ))
            .child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .child(div().font_weight(FontWeight::MEDIUM).child(status))
                    .children(notes.into_iter().map(|n| div().text_color(muted).child(n))),
            )
            .when_some(months, |this, months| {
                this.child(month_bars(&months, today, goal, cx))
            })
    }

    fn render_up_next(
        &self,
        digest: &Digest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let library = self.model.read(cx);
        let pick = digest.series_next.as_ref().and_then(|n| {
            Some((
                n.clone(),
                library.entry(n.id)?.clone(),
                library.entry(n.after)?.clone(),
            ))
        });
        let rows: Vec<AnyElement> = digest
            .up_next
            .iter()
            .filter_map(|id| library.entry(*id))
            .map(|e| {
                compact_row(
                    "home-next",
                    e,
                    e.total_pages().map(|p| format!("{p} pages")),
                    cx,
                )
            })
            .collect();
        let count = digest.want_to_read;
        card(cx)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Up next"))
            .children(
                pick.map(|(next, entry, after)| {
                    self.series_pick(&next, &entry, &after, window, cx)
                }),
            )
            .child(v_flex().gap_1().children(rows))
            .child(
                h_flex().child(
                    Button::new("home-all-want")
                        .ghost()
                        .small()
                        .label(match count {
                            1 => "Want to Read shelf".to_owned(),
                            n => format!("All {n} on Want to Read"),
                        })
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(ShowWantToRead), cx)
                        }),
                ),
            )
    }

    fn series_pick(
        &self,
        next: &SeriesNext,
        entry: &LibraryEntry,
        after: &LibraryEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let id = entry.user_book_id;
        let pending = self.model.read(cx).pending_write(id);
        let can_write = Services::global(cx).has_scope(WRITE_LIBRARY);
        let after_text = match after.rating.map(rating_text) {
            Some(r) if r == "1" => format!("you rated {} 1 star", after.book.title),
            Some(r) => format!("you rated {} {r} stars", after.book.title),
            None => format!("after {}", after.book.title),
        };
        let details = format!(
            "{} {} · {after_text}",
            next.series,
            series_position_text(next.position)
        );
        h_flex()
            .gap_3()
            .p_3()
            .rounded(cx.theme().radius_lg)
            .bg(cx.theme().muted)
            .child(cover(entry, CoverSize::Medium, window, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_xs().text_color(muted).child("Next in a series"))
                    .child(title_button(
                        "home-series-next",
                        entry,
                        FontWeight::SEMIBOLD,
                    ))
                    .child(div().text_xs().text_color(muted).truncate().child(details)),
            )
            .child(
                Button::new("home-start-next")
                    .outline()
                    .small()
                    .label("Start reading")
                    .loading(pending == Some(WriteKind::Status))
                    .disabled(pending.is_some() || !can_write)
                    .when(!can_write, |b| b.tooltip(needs_permission(WRITE_LIBRARY)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_status(id, ReadingStatus::CurrentlyReading, window, cx)
                    })),
            )
            .into_any_element()
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today = chrono::Local::now().date_naive();
        let loaded = self.model.read(cx).is_loaded();
        let digest = self.digest(today, cx);
        let root = div().id("home").track_focus(&self.focus_handle).size_full();
        if !loaded {
            return root.into_any_element();
        }
        let goal = self.goal(today, cx);
        let model = self.model.read(cx);
        let (rate_rows, unrated) = if Services::global(cx).has_scope(WRITE_LIBRARY) {
            to_rate(model.entries(), today, &self.rated_here)
        } else {
            (Vec::new(), 0)
        };
        // A rating stays on screen only while it is being saved.
        self.saving_ratings
            .retain(|id, _| model.pending_write(*id) == Some(WriteKind::Rating));
        let coming_soon = coming_soon_card(&self.upcoming, &self.model, today, cx);
        if digest.is_empty() && rate_rows.is_empty() && goal.is_none() && coming_soon.is_none() {
            return root
                .child(empty_state(
                    IconName::LayoutDashboard,
                    "Nothing to show yet".into(),
                    "Find books with Search or Discover and add them to your shelves. What you're reading and what's next show up here."
                        .into(),
                ))
                .into_any_element();
        }
        // Want to Read books that are all still to come leave nothing to suggest.
        let up_next = (digest.series_next.is_some() || !digest.up_next.is_empty())
            .then(|| self.render_up_next(&digest, window, cx));
        let goal_card = goal.map(|g| self.render_goal(&g, &digest, today, cx));
        let reading = self.render_reading(&digest, today, window, cx);
        let rate = (!rate_rows.is_empty())
            .then(|| self.render_rate(&rate_rows, unrated, today, window, cx));
        root.overflow_y_scrollbar()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(56.))
                    .px_6()
                    .py_5()
                    .gap_5()
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap_3()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Home"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(today.format("%A, %B %-d").to_string()),
                            ),
                    )
                    .child(reading)
                    .children(rate)
                    .when(goal_card.is_some() || up_next.is_some(), |this| {
                        this.child(
                            h_flex()
                                .flex_wrap()
                                .items_start()
                                .gap_4()
                                .children(goal_card.map(|c| c.flex_1().min_w(rems(18.))))
                                .children(up_next.map(|c| c.flex_1().min_w(rems(18.)))),
                        )
                    })
                    .children(coming_soon),
            )
            .into_any_element()
    }
}

// ----- pieces -----

/// A section heading, with a count beside it when there is one.
fn section(title: &'static str, count: Option<usize>, cx: &App) -> gpui_kit::Div {
    v_flex().gap_2().child(
        h_flex()
            .items_baseline()
            .gap_2()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .when_some(count, |this, n| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(n.to_string()),
                )
            }),
    )
}

pub(super) fn card(cx: &App) -> gpui_kit::Div {
    v_flex()
        .gap_2p5()
        .p_4()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius_lg)
}

fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// "16 pages a day"; "Under a page a day" below one.
fn pages_a_day(pages: f64) -> String {
    match pages.round() as u32 {
        0 => "Under a page a day".to_owned(),
        1 => "1 page a day".to_owned(),
        n => format!("{n} pages a day"),
    }
}

/// "Nov 30", with the year when it isn't this year's.
fn short_date(date: Date, today: Date) -> String {
    use chrono::Datelike as _;
    if date.year() == today.year() {
        date.format("%b %-d").to_string()
    } else {
        date.format("%b %-d, %Y").to_string()
    }
}

#[derive(Clone, Copy)]
enum CoverSize {
    Small,
    Medium,
    Large,
}

/// A book's cover at a fixed 2:3 size; a click opens the book's page.
fn cover(entry: &LibraryEntry, size: CoverSize, window: &mut Window, cx: &mut App) -> AnyElement {
    let width = match size {
        CoverSize::Small => rems(1.5),
        CoverSize::Medium => rems(2.5),
        CoverSize::Large => rems(3.),
    };
    let image = entry
        .cover()
        .and_then(|c| cover_thumbnail(&c.url, window, cx));
    let page = Page::Book(entry.book.clone());
    div()
        .id(SharedString::from(format!(
            "home-cover-{}",
            entry.user_book_id
        )))
        .flex_shrink_0()
        .w(width)
        .h(width * 1.5)
        .rounded(cx.theme().radius)
        .overflow_hidden()
        .bg(cx.theme().secondary)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_click(move |_, _, cx| navigation::open(page.clone(), cx))
        .map(|this| match image {
            Some(image) => this.child(img(image).size_full().object_fit(ObjectFit::Cover)),
            None => this.child(
                Icon::new(IconName::BookOpen)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            ),
        })
        .into_any_element()
}

/// The book's title as a quiet button that opens its page: reachable by
/// keyboard, unlike the cover.
fn title_button(prefix: &str, entry: &LibraryEntry, weight: FontWeight) -> AnyElement {
    let page = Page::Book(entry.book.clone());
    // A row, so the button keeps its content's width instead of stretching
    // (and centring the title); it may still shrink, and the title truncates.
    h_flex()
        .min_w_0()
        .child(
            Button::new(SharedString::from(format!(
                "{prefix}-{}",
                entry.user_book_id
            )))
            .text()
            .min_w_0()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(weight)
                    .child(entry.book.title.clone()),
            )
            .on_click(move |_, _, cx| navigation::open(page.clone(), cx)),
        )
        .into_any_element()
}

/// One line for a book: small cover, title, and something about it.
fn compact_row(
    prefix: &str,
    entry: &LibraryEntry,
    trailing: Option<String>,
    cx: &App,
) -> AnyElement {
    let page = Page::Book(entry.book.clone());
    let image_url = entry.cover().map(|c| c.url.clone());
    let muted = cx.theme().muted_foreground;
    h_flex()
        .id(SharedString::from(format!(
            "{prefix}-row-{}",
            entry.user_book_id
        )))
        .gap_2()
        .child(
            div()
                .id(SharedString::from(format!(
                    "{prefix}-cover-{}",
                    entry.user_book_id
                )))
                .flex_shrink_0()
                .w(rems(1.25))
                .h(rems(1.875))
                .rounded(px(2.))
                .overflow_hidden()
                .bg(cx.theme().secondary)
                .cursor_pointer()
                .on_click({
                    let page = page.clone();
                    move |_, _, cx| navigation::open(page.clone(), cx)
                })
                .child(CompactCover(image_url)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(title_button(prefix, entry, FontWeight::NORMAL)),
        )
        .when_some(trailing, |this, t| {
            this.child(div().flex_shrink_0().text_sm().text_color(muted).child(t))
        })
        .into_any_element()
}

/// A small cover image, loaded where the window is at hand.
#[derive(IntoElement)]
struct CompactCover(Option<String>);

impl gpui_kit::RenderOnce for CompactCover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let image = self
            .0
            .as_deref()
            .and_then(|url| cover_thumbnail(url, window, cx));
        div().size_full().when_some(image, |this, image| {
            this.child(img(image).size_full().object_fit(ObjectFit::Cover))
        })
    }
}

/// The goal's progress, with a tick where steady progress would be today.
fn goal_bar(fraction: f32, expected: Option<f32>, cx: &App) -> impl IntoElement {
    let height = px(8.);
    div()
        .relative()
        .w_full()
        .h(height)
        .rounded_full()
        .bg(cx.theme().progress_bar.opacity(0.2))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(fraction))
                .rounded_full()
                .bg(cx.theme().progress_bar),
        )
        .when_some(expected, |this, expected| {
            this.child(
                div()
                    .absolute()
                    .top(px(-3.))
                    .h(height + px(6.))
                    .w(px(2.))
                    .left(relative(expected))
                    .bg(cx.theme().muted_foreground),
            )
        })
}

/// Books finished each month of the goal's year; months still to come are
/// outlined.
fn month_bars(months: &[u32; 12], today: Date, goal: &Goal, cx: &App) -> impl IntoElement {
    use chrono::Datelike as _;
    let max = months.iter().copied().max().unwrap_or(0).max(1);
    let this_year = goal.start.year() == today.year();
    let bar_area: Pixels = px(32.);
    let names = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
    let full = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let bars = months.iter().enumerate().map(|(ix, &n)| {
        let future = this_year && ix > today.month0() as usize || goal.start.year() > today.year();
        let height = if n == 0 {
            px(2.)
        } else {
            bar_area * (n as f32 / max as f32)
        };
        let tip: SharedString = match n {
            1 => format!("{}: 1 book", full[ix]).into(),
            n => format!("{}: {n} books", full[ix]).into(),
        };
        v_flex()
            .id(SharedString::from(format!("home-month-{ix}")))
            .flex_1()
            .gap_1()
            .items_center()
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
            .child(
                div()
                    .h(bar_area)
                    .w_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(
                        div()
                            .w_full()
                            .h(if future { bar_area } else { height })
                            .rounded_t(px(2.))
                            .map(|this| {
                                if future {
                                    this.border_1()
                                        .border_dashed()
                                        .border_color(cx.theme().border)
                                } else if n == 0 {
                                    this.bg(cx.theme().border)
                                } else {
                                    this.bg(cx.theme().progress_bar)
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(names[ix]),
            )
    });
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Finished each month"),
        )
        .child(h_flex().gap_1().items_end().children(bars))
}
