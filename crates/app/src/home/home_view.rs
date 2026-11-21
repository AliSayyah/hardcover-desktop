//! The Home destination, shown at launch (`hardcover-design`, Screens →
//! Home): the books being read with their pace, books finished lately to
//! rate, the reading goal, what to read next, and what is coming soon in the
//! reader's series. Everything comes from the library on this computer and
//! the goals and upcoming books already loaded; Home asks Hardcover for
//! nothing until the reader acts.
//!
//! - **Header:** the date and the greeting (`ui::Greeting`), inset 32.
//! - **Main column:** "Continue reading": the lead book (the most recently
//!   updated one, xl cover), the other books being read and the latest paused
//!   one, as a sequence; then "Rate what you finished" and "Coming soon".
//! - **Rail:** the reading goal, Up next (the next book in a series) and the
//!   newest on Want to read. A section that doesn't apply disappears; the
//!   rail folds under the main column in a narrow window (`ui::Columns`).
//! - **Width:** Home fills the width it is given; its rows fill their
//!   columns.
//! - **Wide region** ([`home_tier`], from the window less the sidebar and
//!   the insets): when two content columns of at least 448 fit beside the
//!   rail (a 1536 window), "Coming soon", "Rate what you finished" and
//!   "Newest on Want to read" move to a second column; the two content
//!   columns share the width beside the rail. There the shelf is one row of
//!   md covers, as many as the column holds ([`want_to_read_shown`]). With
//!   nothing for the second column, Home keeps its two columns.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use crate::ui::reveal::RevealScroll as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, IconName, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, HighlightStyle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, RenderImage,
    SharedString, Styled as _, StyledText, Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::{READ_GOALS, WRITE_LIBRARY};
use hardcover_core::{
    Date, Goal, GoalId, GoalMetric, LibraryEntry, Rating, ReadingStatus, UserBookId,
};

use super::coming_soon::{ComingSoon, coming_soon, coming_soon_section};
use super::digest::{
    Digest, Reading, SeriesNext, UP_NEXT_IN_RAIL, compute, finished_per_month, finishing_by, lead,
    to_rate,
};
use crate::app_state::release_cover_image;
use crate::bidi::is_rtl;
use crate::book::progress_dialog::open_progress;
use crate::book::star_rating::StarRating;
use crate::catalog::navigation::Page;
use crate::chip::StatusChip;
use crate::cover::{Cover, CoverArt, CoverSize, detail_art};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::{
    authors_text, books_count_text, count_text, goal_amount, goal_pace, goal_unit,
    needs_permission, paused_text, percent_label, progress_summary, rating_text,
    series_position_text, short_date,
};
use crate::goals::goal_view::open_goal;
use crate::goals::model::GoalsModel;
use crate::library::model::{LibraryModel, LibraryWrite, WriteKind};
use crate::library::writes;
use crate::session;
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::open_alert_dialog;
use crate::ui::{
    Columns, CoverShelf, Greeting, Lane, Meter, MonthBars, RailSection, RowSpacing, SectionHeader,
    Sequence, Skeleton, TextButton, Tier, books_that_fit, content_column_width, date_line,
    empty_state, greeting_at, region_width, see_all, tier,
};
use crate::upcoming::model::UpcomingModel;
use crate::{ShowPaused, ShowWantToRead};

/// What Home shows, worked out once per change of the library, the
/// upcoming books, the day, the books rated here or the write scope, so a
/// frame only reads it: the digest, and the entries and upcoming books its
/// sections draw.
struct HomeData {
    digest: Digest,
    /// Continue reading, the lead first.
    reading: Vec<(Reading, LibraryEntry)>,
    paused: Option<LibraryEntry>,
    /// Rate what you finished: when each was finished, and the entry.
    to_rate: Vec<(Date, LibraryEntry)>,
    /// How many books finished lately still need a rating.
    unrated: usize,
    series_next: Option<(SeriesNext, LibraryEntry)>,
    /// The newest on Want to read.
    want: Vec<LibraryEntry>,
    coming_soon: Option<ComingSoon>,
}

/// What [`HomeData`] was worked out from: the library's and the upcoming
/// books' revisions, the day, the generation of the books rated here, and
/// whether the session can write the library.
type HomeKey = (u64, u64, Date, u64, bool);

impl HomeData {
    fn new(
        library: &LibraryModel,
        upcoming: &UpcomingModel,
        today: Date,
        rated_here: &HashSet<UserBookId>,
        can_write: bool,
    ) -> Self {
        let entry = |id: UserBookId| library.entry(id).cloned();
        let unreleased = upcoming.unreleased(today);
        let digest = compute(library.entries(), today, &unreleased);
        let (to_rate, unrated) = if can_write {
            to_rate(library.entries(), today, rated_here)
        } else {
            (Vec::new(), 0)
        };
        Self {
            reading: digest
                .reading
                .iter()
                .filter_map(|r| Some((r.clone(), entry(r.id)?)))
                .collect(),
            paused: digest.paused.and_then(entry),
            to_rate: to_rate
                .iter()
                .filter_map(|r| Some((r.finished, entry(r.id)?)))
                .collect(),
            unrated,
            series_next: digest
                .series_next
                .as_ref()
                .and_then(|next| Some((next.clone(), entry(next.id)?))),
            want: digest.up_next.iter().filter_map(|id| entry(*id)).collect(),
            coming_soon: coming_soon(upcoming, library, today),
            digest,
        }
    }
}

pub struct HomeView {
    model: Entity<LibraryModel>,
    goals: Entity<GoalsModel>,
    upcoming: Entity<UpcomingModel>,
    focus_handle: FocusHandle,
    /// What Home shows, and what it was worked out from.
    computed: Option<(HomeKey, Rc<HomeData>)>,
    /// Books finished in each month of the goal shown, by library revision
    /// and the goal's id and dates.
    months: Option<((u64, GoalId, Date, Date), Option<[u32; 12]>)>,
    /// Books rated on Home since it was last opened from the sidebar: their
    /// rows stay, so the rating can still be changed.
    rated_here: HashSet<UserBookId>,
    /// Bumped whenever [`Self::rated_here`] changes.
    rated_generation: u64,
    /// A rating being saved, shown until the write lands.
    saving_ratings: HashMap<UserBookId, Option<Rating>>,
    /// The lead book's detail cover (the xl cover needs more than a
    /// thumbnail), released when the lead changes.
    lead_cover: Option<(String, Option<Arc<RenderImage>>)>,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, model, window, cx| {
                let model = model.read(cx);
                // A rating stays on screen only while it is being saved.
                this.saving_ratings
                    .retain(|id, _| model.pending_write(*id) == Some(WriteKind::Rating));
                // The lead's detail cover goes once another book leads.
                let lead = lead(model.entries()).and_then(|e| e.cover().map(|c| c.url.clone()));
                if this.lead_cover.as_ref().map(|(url, _)| url) != lead.as_ref()
                    && let Some((url, image)) = this.lead_cover.take()
                {
                    release_cover_image(&url, image, window, cx);
                }
                cx.notify();
            }),
            cx.observe(&goals, |_, _, cx| cx.notify()),
            cx.observe(&upcoming, |_, _, cx| cx.notify()),
        ];
        Self {
            model,
            goals,
            upcoming,
            focus_handle: cx.focus_handle(),
            computed: None,
            months: None,
            rated_here: HashSet::new(),
            rated_generation: 0,
            saving_ratings: HashMap::new(),
            lead_cover: None,
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
            self.rated_generation += 1;
            cx.notify();
        }
    }

    /// What Home shows today, worked out again only when what it comes
    /// from changed.
    fn data(&mut self, today: Date, cx: &App) -> Rc<HomeData> {
        let model = self.model.read(cx);
        let upcoming = self.upcoming.read(cx);
        let can_write = session::has_scope(cx, WRITE_LIBRARY);
        let key = (
            model.revision(),
            upcoming.revision(),
            today,
            self.rated_generation,
            can_write,
        );
        match &self.computed {
            Some((k, data)) if *k == key => data.clone(),
            _ => {
                let data = Rc::new(HomeData::new(
                    model,
                    upcoming,
                    today,
                    &self.rated_here,
                    can_write,
                ));
                self.computed = Some((key, data.clone()));
                data
            }
        }
    }

    /// Books finished in each month of `goal`'s calendar year, worked out
    /// again only when the library or the goal changed.
    fn months(&mut self, goal: &Goal, cx: &App) -> Option<[u32; 12]> {
        let model = self.model.read(cx);
        let key = (model.revision(), goal.id, goal.start, goal.end);
        match &self.months {
            Some((k, months)) if *k == key => *months,
            _ => {
                let months = (goal.metric == GoalMetric::Books)
                    .then(|| finished_per_month(goal, model.entries()))
                    .flatten();
                self.months = Some((key, months));
                months
            }
        }
    }

    /// The goal Home shows: the current one counted in books, else the first
    /// current one. The sidebar lists them all.
    fn goal(&self, today: Date, cx: &App) -> Option<Goal> {
        if !session::has_scope(cx, READ_GOALS) {
            return None;
        }
        let goals = self.goals.read(cx);
        goals
            .current(today)
            .find(|g| g.metric == GoalMetric::Books)
            .or_else(|| goals.current(today).next())
            .cloned()
    }

    /// The lead's detail cover, kept so it can be released once another
    /// book leads (the model's observer in [`Self::new`] does that).
    fn lead_art(&mut self, url: Option<String>, window: &mut Window, cx: &mut App) -> CoverArt {
        let Some(url) = url else {
            return CoverArt::None;
        };
        let art = detail_art(&url, window, cx);
        self.lead_cover = Some((url, art.image()));
        art
    }

    fn set_status(
        &mut self,
        id: UserBookId,
        status: ReadingStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        writes::set_status(&self.model, id, status, window, cx);
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
        if self.rated_here.insert(id) {
            self.rated_generation += 1;
        }
        self.saving_ratings.insert(id, rating);
        // Sending the status keeps Hardcover from starting a new read.
        let write = LibraryWrite::entry(
            id,
            WriteKind::Rating,
            "Couldn't save the rating",
            move |client| client.set_rating(id, rating, keep),
        );
        drop(self.model.update(cx, |m, cx| m.write(write, window, cx)));
    }

    /// Asks before finishing a book: the read ends today, and moving the
    /// book back to Currently Reading would start another read.
    fn confirm_finished(&mut self, id: UserBookId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.model.read(cx).entry(id).map(|e| e.book.title.clone()) else {
            return;
        };
        let view = cx.entity().downgrade();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title(format!("Mark “{title}” as read?"))
                .description("It moves to your Read shelf, and this read ends today.")
                .confirm()
                .ok_text("Mark as read")
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.set_status(id, ReadingStatus::Read, window, cx);
                    });
                    true
                })
        });
    }

    // ----- Continue reading -----

    fn render_reading(
        &mut self,
        data: &HomeData,
        today: Date,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let paused = data
            .paused
            .as_ref()
            .map(|entry| Self::paused_row(entry, data.digest.paused_count, today, cx));

        let lead_url = data
            .reading
            .first()
            .and_then(|(_, entry)| entry.cover())
            .map(|c| c.url.clone());
        let lead_art = self.lead_art(lead_url, window, cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut books = data.reading.iter();
        if let Some((reading, entry)) = books.next() {
            rows.push(self.lead(reading, entry, lead_art, today, cx));
        }
        rows.extend(books.map(|(reading, entry)| self.reading_row(reading, entry, today, cx)));
        let reading_nothing = rows.is_empty();
        rows.extend(paused);

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(SectionHeader::new("Continue reading"))
            .when(reading_nothing, |this| {
                this.child(muted_line(
                    "You're not reading anything right now. Start a book from Up next, or from any book's page.",
                    cx,
                ))
            })
            .when(!rows.is_empty(), |this| {
                this.child(
                    Sequence::new()
                        .spacing(RowSpacing::Reviews)
                        .flush_top()
                        .children(rows),
                )
            })
            .into_any_element()
    }

    /// The two commands of a book being read: "Update progress…" (default)
    /// and "Mark as read…" (ghost). Small inside a row.
    fn reading_actions(
        &self,
        entry: &LibraryEntry,
        small: bool,
        cx: &mut Context<Self>,
    ) -> [AnyElement; 2] {
        let id = entry.user_book_id;
        let pending = self.model.read(cx).pending_write(id);
        let can_write = session::has_scope(cx, WRITE_LIBRARY);
        let disabled_reason = (!can_write).then(|| needs_permission(WRITE_LIBRARY));
        let model = self.model.clone();
        let entry_for_dialog = entry.clone();
        let size = |button: Button| {
            if small {
                button.small_control()
            } else {
                button
            }
        };
        [
            size(Button::new(SharedString::from(format!("home-update-{id}"))))
                .label("Update progress…")
                .loading(pending == Some(WriteKind::Progress))
                .disabled(pending.is_some() || !can_write)
                .when_some(disabled_reason.clone(), Button::tooltip)
                .on_click(move |_, window, cx| {
                    open_progress(model.clone(), &entry_for_dialog, window, cx);
                })
                .into_any_element(),
            size(Button::new(SharedString::from(format!("home-finish-{id}"))))
                .ghost()
                .label("Mark as read…")
                .loading(pending == Some(WriteKind::Status))
                .disabled(pending.is_some() || !can_write)
                .when_some(disabled_reason, Button::tooltip)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.confirm_finished(id, window, cx)),
                )
                .into_any_element(),
        ]
    }

    /// The most recently updated book being read: an xl cover, the title in
    /// `display-24`, the progress block and its two commands.
    fn lead(
        &self,
        reading: &Reading,
        entry: &LibraryEntry,
        art: CoverArt,
        today: Date,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.user_book_id;
        let muted = colors(cx).fg_muted;
        let progress = progress_summary(entry);
        let author = authors_text(&entry.book.authors);
        let pace = pace_line(reading, today);
        let [update, finish] = self.reading_actions(entry, false, cx);

        div()
            .flex()
            .flex_row()
            .gap_6()
            .child(
                Cover::new(CoverSize::Xl, entry.book.title.clone())
                    .author(entry.book.authors.first().cloned())
                    .art(art)
                    .shadow(true)
                    .opens(
                        SharedString::from(format!("home-cover-{id}")),
                        Page::Book(entry.book.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        TextButton::book(
                            SharedString::from(format!("home-title-{id}")),
                            &entry.book,
                        )
                        .display_24()
                        .wrap(),
                    )
                    .when(!author.is_empty(), |this| {
                        this.child(
                            div()
                                .mt_1()
                                .body()
                                .text_color(muted)
                                .truncate()
                                .child(author),
                        )
                    })
                    // The progress block sits at the foot of the cover.
                    .child(div().flex_1().min_h_4())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .max_w(size::METER_MAX)
                            .gap_2()
                            .child(
                                h_flex()
                                    // Both runs are 14 px, so centring them shares a baseline.
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .body_strong()
                                            .tabular()
                                            .child(progress.text()),
                                    )
                                    .children(progress.percent.map(|pct| {
                                        div()
                                            .flex_none()
                                            .body()
                                            .tabular()
                                            .text_color(muted)
                                            .child(percent_label(pct))
                                    })),
                            )
                            .child(Meter::new(
                                SharedString::from(format!("home-progress-{id}")),
                                progress.percent.unwrap_or(0.0),
                                "Reading progress",
                            ))
                            .children(
                                pace.map(|pace| {
                                    div().body().tabular().text_color(muted).child(pace)
                                }),
                            ),
                    )
                    .child(h_flex().mt_4().gap_2().child(update).child(finish)),
            )
            .into_any_element()
    }

    /// Another book being read: an md cover, the title in `title-18`, a 2 px
    /// meter with the percentage, a caption, and the two commands stacked on
    /// the trailing edge.
    fn reading_row(
        &self,
        reading: &Reading,
        entry: &LibraryEntry,
        today: Date,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.user_book_id;
        let muted = colors(cx).fg_muted;
        let progress = progress_summary(entry);
        let author = authors_text(&entry.book.authors);
        let mut facts = vec![progress.text()];
        facts.extend(pace_facts(reading, today));
        let [update, finish] = self.reading_actions(entry, true, cx);

        // The book and its meter beside the two commands.
        let top = h_flex()
            .w_full()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        TextButton::book(
                            SharedString::from(format!("home-title-{id}")),
                            &entry.book,
                        )
                        .title_18(),
                    )
                    .when(!author.is_empty(), |this| {
                        this.child(div().body().text_color(muted).truncate().child(author))
                    })
                    .children(progress.percent.map(|pct| {
                        h_flex()
                            .mt_2()
                            .gap_2()
                            .child(
                                Meter::new(
                                    SharedString::from(format!("home-progress-{id}")),
                                    pct,
                                    "Reading progress",
                                )
                                .thin()
                                .w(size::METER_IN_ROW),
                            )
                            .child(div().caption(cx).tabular().child(percent_label(pct)))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_end()
                    .gap_1()
                    .child(update)
                    .child(finish),
            );

        // The caption runs under the commands too, so it keeps to one line
        // in the default window; narrower, it wraps between its facts.
        h_flex()
            .w_full()
            .gap_4()
            .child(cover(entry, CoverSize::Md, true))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(top)
                    .child(caption_facts(facts, cx).mt_1()),
            )
            .into_any_element()
    }

    /// The most recently paused book, so it isn't forgotten: an sm row with
    /// the Paused chip. Opening it is the way back: moving it to Currently
    /// Reading would start a new read.
    fn paused_row(entry: &LibraryEntry, count: usize, today: Date, cx: &App) -> AnyElement {
        let progress = progress_summary(entry);
        let since = entry.updated_at.map(crate::dates::local_day);
        h_flex()
            .w_full()
            .gap_3()
            .child(cover(entry, CoverSize::Sm, false))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(TextButton::book(
                        SharedString::from(format!("home-paused-{}", entry.user_book_id)),
                        &entry.book,
                    ))
                    .child(
                        div()
                            .caption(cx)
                            .tabular()
                            .truncate()
                            .child(paused_text(&progress, since, today)),
                    ),
            )
            .child(
                Lane::fit()
                    .stacked()
                    .child(StatusChip(ReadingStatus::Paused))
                    .when(count > 1, |this| {
                        this.child(
                            Button::new("home-all-paused")
                                .ghost()
                                .small_control()
                                .label(format!("All {} paused", count_text(count)))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ShowPaused), cx);
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    // ----- Rate what you finished -----

    /// Books finished lately with no rating, each with its stars. Hidden
    /// without `write:library`, as there is nothing to do in it then.
    fn render_rate(
        &self,
        books: &[(Date, LibraryEntry)],
        unrated: usize,
        today: Date,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = books
            .iter()
            .map(|(finished, entry)| self.rate_row(entry, *finished, today, cx))
            .collect::<Vec<_>>();
        let mut header = SectionHeader::new("Rate what you finished");
        if unrated > 0 {
            header = header.action(
                div()
                    .caption(cx)
                    .tabular()
                    .child(books_count_text(u32::try_from(unrated).unwrap_or(u32::MAX))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(Sequence::new().spacing(RowSpacing::Reviews).children(rows))
            .into_any_element()
    }

    fn rate_row(
        &self,
        entry: &LibraryEntry,
        finished: Date,
        today: Date,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.user_book_id;
        let pending = self.model.read(cx).pending_write(id);
        let rating = match self.saving_ratings.get(&id) {
            Some(saving) if pending == Some(WriteKind::Rating) => *saving,
            _ => entry.rating,
        };
        let byline = finished_byline(&entry.book.authors, finished, today);
        h_flex()
            .w_full()
            .gap_3()
            .child(cover(entry, CoverSize::Sm, false))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(TextButton::book(
                        SharedString::from(format!("home-rate-title-{id}")),
                        &entry.book,
                    ))
                    .child(
                        div()
                            .body()
                            .tabular()
                            .text_color(colors(cx).fg_muted)
                            .truncate()
                            .child(byline),
                    ),
            )
            // The stars show the new rating at once; it saves behind them.
            .child(
                StarRating::new(SharedString::from(format!("home-stars-{id}")))
                    .small()
                    .value(rating)
                    .on_change(
                        cx.listener(move |this, rating: &Option<Rating>, window, cx| {
                            this.rate(id, *rating, window, cx);
                        }),
                    ),
            )
            .child(
                Lane::trailing().child(
                    div()
                        .body_strong()
                        .tabular()
                        .children(rating.map(rating_text)),
                ),
            )
            .into_any_element()
    }

    // ----- the rail -----

    /// The reading goal: the figure, the goal meter with its pace tick, one
    /// sentence, and books per month for a calendar-year goal counted in
    /// books.
    fn goal_section(
        &self,
        goal: &Goal,
        months: Option<[u32; 12]>,
        data: &HomeData,
        today: Date,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = colors(cx).fg_muted;
        let title = match crate::format::goal_year(goal.start, goal.end) {
            Some(year) => format!("{year} reading goal"),
            None => "Reading goal".to_owned(),
        };
        let reached = goal.progress >= f64::from(goal.target);
        let finishing: Vec<String> = if reached {
            Vec::new()
        } else {
            finishing_by(goal, &data.digest, today)
                .into_iter()
                .filter_map(|id| {
                    let (_, entry) = data.reading.iter().find(|(r, _)| r.id == id)?;
                    Some(entry.book.title.clone())
                })
                .collect()
        };
        let sentence = goal_sentence(goal, today, &finishing);

        let goals = self.goals.clone();
        let model = self.model.clone();
        let goal_id = goal.id;
        let target_text = format!(
            "of {} {}",
            count_text(goal.target),
            goal_unit(&goal.metric, f64::from(goal.target))
        );

        RailSection::new(title)
            .action(
                Button::new("home-goal-details")
                    .ghost()
                    .small_control()
                    .label("Details…")
                    .on_click(move |_, window, cx| {
                        open_goal(goals.clone(), model.clone(), goal_id, window, cx);
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .display_30()
                            .child(goal_amount(&goal.metric, goal.progress)),
                    )
                    .child(
                        div()
                            .body()
                            .baseline_on(TypeRole::Body, TypeRole::Display30)
                            .tabular()
                            .text_color(muted)
                            .child(target_text),
                    ),
            )
            .child(
                div().pt_1().child(
                    Meter::new(
                        "home-goal-bar",
                        crate::format::bar_percent(goal.fraction()),
                        "Progress toward the goal",
                    )
                    .pace(crate::goals::sidebar::pace_percent(goal, today)),
                ),
            )
            .children(sentence.map(|(text, title)| {
                let fg = colors(cx).fg;
                let highlights = title.map(|range| {
                    (
                        range,
                        HighlightStyle {
                            color: Some(fg),
                            font_weight: Some(FontWeight::MEDIUM),
                            ..Default::default()
                        },
                    )
                });
                div()
                    .pt_1()
                    .body()
                    .text_color(muted)
                    .child(StyledText::new(text).with_highlights(highlights))
            }))
            .children(months.map(|months| div().pt_2().child(month_bars(&months, today, goal))))
            .into_any_element()
    }

    /// The next book in a series the reader has started.
    fn up_next_section(
        &self,
        next: &SeriesNext,
        entry: &LibraryEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.user_book_id;
        let pending = self.model.read(cx).pending_write(id);
        let can_write = session::has_scope(cx, WRITE_LIBRARY);
        let author = authors_text(&entry.book.authors);
        RailSection::new("Up next")
            .child(
                h_flex()
                    .items_start()
                    .gap_4()
                    .child(cover(entry, CoverSize::Md, true))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .caption(cx)
                                    .tabular()
                                    .truncate()
                                    .child(series_next_text(next)),
                            )
                            .child(
                                TextButton::book(
                                    SharedString::from(format!("home-series-next-{id}")),
                                    &entry.book,
                                )
                                .title_18()
                                .wrap(),
                            )
                            .when(!author.is_empty(), |this| {
                                this.child(
                                    div()
                                        .body()
                                        .text_color(colors(cx).fg_muted)
                                        .truncate()
                                        .child(author),
                                )
                            })
                            .child(
                                h_flex().mt_2().child(
                                    Button::new("home-start-next")
                                        .small_control()
                                        .label("Start reading")
                                        .loading(pending == Some(WriteKind::Status))
                                        .disabled(pending.is_some() || !can_write)
                                        .when(!can_write, |b| {
                                            b.tooltip(needs_permission(WRITE_LIBRARY))
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.set_status(
                                                id,
                                                ReadingStatus::CurrentlyReading,
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The newest books on Want to read, as a shelf of covers: in the rail,
    /// or (`in_column`) as a section of the second content column, one row
    /// of `shown` books.
    fn want_to_read_section(&self, data: &HomeData, shown: usize, in_column: bool) -> AnyElement {
        let books = data.want.iter().take(shown).map(|e| {
            (
                cover(e, CoverSize::Md, false),
                TextButton::book(
                    SharedString::from(format!("home-want-{}", e.user_book_id)),
                    &e.book,
                ),
            )
        });
        let count = u32::try_from(data.digest.want_to_read).unwrap_or(u32::MAX);
        let all = see_all(
            "home-all-want",
            (count > 1).then_some(count),
            |_, window, cx| window.dispatch_action(Box::new(ShowWantToRead), cx),
        );
        let shelf = CoverShelf::new(CoverSize::Md).books(books);
        if in_column {
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(SectionHeader::new("Newest on Want to read").action(all))
                .child(shelf.one_row())
                .into_any_element();
        }
        RailSection::new("Newest on Want to read")
            .action(all)
            .child(div().pt_1().child(shelf))
            .into_any_element()
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = crate::dates::now();
        let today = now.date_naive();
        let header = Greeting::new(date_line(today), greeting_at(&now));
        let model = self.model.read(cx);
        let (loaded, failure) = (model.is_loaded(), model.load_failure().cloned());
        let root = div().id("home").track_focus(&self.focus_handle).size_full();
        if let Some(message) = failure {
            return root
                .child(crate::library::load_failed(
                    "home-retry",
                    &self.model,
                    message,
                ))
                .into_any_element();
        }
        if !loaded {
            return root
                .child(
                    page()
                        .child(header)
                        .child(div().pt_6().child(lead_skeleton())),
                )
                .into_any_element();
        }
        let data = self.data(today, cx);
        let goal = self.goal(today, cx);
        let coming_soon = data
            .coming_soon
            .as_ref()
            .map(|soon| coming_soon_section(soon, &self.model, today, cx));
        if data.digest.is_empty()
            && data.to_rate.is_empty()
            && goal.is_none()
            && coming_soon.is_none()
        {
            return root
                .child(empty_state(
                    IconName::LayoutDashboard,
                    "Nothing to show yet".into(),
                    "Find books with Search or Discover and add them to your shelves. What you're reading and what's next show up here."
                        .into(),
                ))
                .into_any_element();
        }

        let reading = self.render_reading(&data, today, window, cx);
        let rate = (!data.to_rate.is_empty())
            .then(|| self.render_rate(&data.to_rate, data.unrated, today, cx));

        let goal_section = goal.map(|g| {
            let months = self.months(&g, cx);
            self.goal_section(&g, months, &data, today, cx)
        });
        let series_next = data
            .series_next
            .as_ref()
            .map(|(next, entry)| self.up_next_section(next, entry, cx));
        // The sidebar is fixed, so the columns get the rest of the window
        // less the page's insets.
        let rem = window.rem_size();
        let width = region_width(window, size::PAGE_INSET);
        let has_second = rate.is_some() || coming_soon.is_some();
        let tier = home_tier(width, rem, has_second);

        // Want to Read books that are all still to come leave nothing to
        // suggest. In three columns the shelf leaves the rail for the second
        // column, under Rate what you finished.
        let has_want = !data.want.is_empty();
        let (want_in_rail, want_in_column) = if tier == Tier::Three {
            let has_rail = goal_section.is_some() || series_next.is_some();
            let shown = want_to_read_shown(width, rem, has_rail);
            let section =
                (has_want && shown > 0).then(|| self.want_to_read_section(&data, shown, true));
            (None, section)
        } else {
            let section =
                has_want.then(|| self.want_to_read_section(&data, UP_NEXT_IN_RAIL, false));
            (section, None)
        };
        let rail: Vec<AnyElement> = [goal_section, series_next, want_in_rail]
            .into_iter()
            .flatten()
            .collect();
        let column = |sections: Vec<Option<AnyElement>>| {
            div()
                .flex()
                .flex_col()
                .gap_6()
                .children(sections.into_iter().flatten())
        };
        let columns = if tier == Tier::Three {
            // Coming soon leads the second column; Rate what you finished
            // and the newest on Want to read follow it.
            Columns::new(tier, reading).second(column(vec![coming_soon, rate, want_in_column]))
        } else {
            Columns::new(tier, column(vec![Some(reading), rate, coming_soon]))
        };
        root.scroll_y_revealing_focus()
            .child(
                page()
                    .child(header)
                    .child(div().pt_6().child(columns.wide_gap().rail(rail))),
            )
            .into_any_element()
    }
}

/// Home's tier for columns `width` wide: three columns only when there is
/// something for the second (Coming soon, or Rate what you finished).
fn home_tier(width: Pixels, rem: Pixels, has_second: bool) -> Tier {
    match tier(width, rem, true) {
        Tier::Three if !has_second => Tier::Rail,
        t => t,
    }
}

/// How many of the newest Want to Read books the second column's one row
/// of md covers holds, in three columns `width` wide (the rail beside them
/// if `has_rail`).
fn want_to_read_shown(width: Pixels, rem: Pixels, has_rail: bool) -> usize {
    books_that_fit(
        content_column_width(width, rem, true, has_rail),
        CoverSize::Md,
        rem,
    )
}

// ----- pieces -----

/// Home's column: the whole width, inset 32.
fn page() -> gpui_kit::Div {
    div().flex().flex_col().w_full().p_8()
}

/// The lead while the library loads.
fn lead_skeleton() -> impl IntoElement {
    h_flex()
        .items_start()
        .gap_6()
        .child(Skeleton::cover(CoverSize::Xl))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_2()
                .child(Skeleton::line().w_2_3())
                .child(Skeleton::line().w_1_3()),
        )
}

fn muted_line(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .body()
        .text_color(colors(cx).fg_muted)
        .child(text.into())
        .into_any_element()
}

/// A book's cover; a click opens the book's page. `shadow` where the cover
/// leads its row.
fn cover(entry: &LibraryEntry, size: CoverSize, shadow: bool) -> Cover {
    Cover::new(size, entry.book.title.clone())
        .author(entry.book.authors.first().cloned())
        .thumbnail(entry.cover().map(|c| c.url.clone()))
        .shadow(shadow)
        .opens(
            SharedString::from(format!("home-cover-{}", entry.user_book_id)),
            Page::Book(entry.book.clone()),
        )
}

/// The pace line of a book being read: "16 pages a day · done around Dec 1",
/// or "Last updated 8 days ago" for a book left idle, which gets no finish
/// date. `None` when there is nothing to say.
pub fn pace_line(reading: &Reading, today: Date) -> Option<String> {
    let parts = pace_facts(reading, today);
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The pace line's facts, each a phrase a caption keeps on one line.
fn pace_facts(reading: &Reading, today: Date) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pace) = reading.pace {
        parts.push(crate::format::pages_per_day_text(
            pace.pages_per_day,
            "Under a page a day",
        ));
        if reading.idle_days.is_none()
            && let Some(finish) = pace.finish
        {
            parts.push(format!("done around {}", short_date(finish, today)));
        }
    }
    if let Some(days) = reading.idle_days {
        parts.push(format!("Last updated {days} days ago"));
    }
    parts
}

/// A row's caption made of facts ("Page 418 of 1,007 · 13 pages a day ·
/// done around Jan 5, 2027"). It wraps between facts only, never inside one
/// (a date keeps its year), and a wrapped line never starts with the dot.
fn caption_facts(facts: Vec<String>, cx: &App) -> gpui_kit::Div {
    let last = facts.len().saturating_sub(1);
    h_flex()
        .flex_wrap()
        .caption(cx)
        .tabular()
        .children(facts.into_iter().enumerate().map(move |(i, fact)| {
            let text = if i < last {
                format!("{fact} ·\u{a0}")
            } else {
                fact
            };
            div().whitespace_nowrap().child(text)
        }))
}

/// A rating row's second line: "Andy Weir · Finished Nov 3".
fn finished_byline(authors: &[String], finished: Date, today: Date) -> String {
    format!(
        "{} · Finished {}",
        authors_text(authors),
        short_date(finished, today)
    )
}

/// Up next's caption: "Next in Dune · #2".
pub fn series_next_text(next: &SeriesNext) -> String {
    format!(
        "Next in {} · {}",
        next.series,
        series_position_text(next.position)
    )
}

/// The goal's one sentence, and where the title it names sits in it:
/// "2 behind pace, 3 to go in 41 days. Project Hail Mary should finish in
/// time." `finishing` holds the titles of the books being read that should
/// finish before the goal ends. One left-to-right title is named; more books,
/// or a right-to-left title (GPUI can't wrap one inside a sentence), are
/// counted. A goal not running today says how it stands ("Goal reached.",
/// "Starts Jan 1, 2027."). `None` for a goal with nothing to say.
pub fn goal_sentence(
    goal: &Goal,
    today: Date,
    finishing: &[String],
) -> Option<(String, Option<Range<usize>>)> {
    let reached = goal.progress >= f64::from(goal.target);
    let standing = goal_pace(goal, today)?;
    if reached || !goal.is_current(today) {
        return Some((format!("{standing}."), None));
    }
    let pace = goal.pace(today).round();
    let standing = if pace == 0.0 {
        "On pace".to_owned()
    } else {
        let side = if pace > 0.0 { "ahead of" } else { "behind" };
        format!("{} {side} pace", pace_amount(goal, pace.abs()))
    };
    let left = pace_amount(goal, (f64::from(goal.target) - goal.progress).ceil());
    let to_go = match (goal.last_day() - today).num_days() {
        0 => format!("{left} to go today"),
        1 => format!("{left} to go in 1 day"),
        n => format!("{left} to go in {} days", count_text(n.unsigned_abs())),
    };
    let mut text = format!("{standing}, {to_go}.");
    let mut title = None;
    if !finishing.is_empty() {
        text.push(' ');
        match finishing {
            [one] if !is_rtl(one) => {
                title = Some(text.len()..text.len() + one.len());
                write!(text, "{one} should finish in time.").expect("writing to a String");
            }
            [_] => text.push_str("1 book you're reading should finish in time."),
            many => write!(
                text,
                "{} books you're reading should finish in time.",
                many.len()
            )
            .expect("writing to a String"),
        }
    }
    Some((text, title))
}

/// An amount in the goal's terms: a bare number for books, which the figure
/// above already names ("2"), else with its unit ("120 pages").
fn pace_amount(goal: &Goal, n: f64) -> String {
    let amount = goal_amount(&goal.metric, n);
    match goal.metric {
        GoalMetric::Books => amount,
        _ => format!("{amount} {}", goal_unit(&goal.metric, n)),
    }
}

/// Books finished each month of the goal's year ([`MonthBars`]), months
/// still to come marked as such.
fn month_bars(months: &[u32; 12], today: Date, goal: &Goal) -> MonthBars {
    use chrono::Datelike as _;
    let future_from = match goal.start.year().cmp(&today.year()) {
        std::cmp::Ordering::Greater => Some(0),
        std::cmp::Ordering::Equal => Some(today.month0() as usize + 1),
        std::cmp::Ordering::Less => None,
    };
    MonthBars::new("home-months", *months).future_from(future_from)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::home::digest::Pace;
    use crate::session::SessionScope;
    use gpui_kit::{AppContext as _, TestAppContext};
    use hardcover_core::store::LibraryCache;

    fn day(s: &str) -> Date {
        s.parse().unwrap()
    }

    #[gpui_kit::test]
    fn a_rating_shown_while_saving_goes_when_the_write_ends_without_a_frame(
        cx: &mut TestAppContext,
    ) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        cx.update(|cx| {
            gpui_kit::init(cx);
            session.install(cx);
        });
        let model = cx.new(|_| LibraryModel::new(session.clone()));
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let goals = cx.new(|cx| GoalsModel::new(session.clone(), &model, cx));
        let upcoming = cx.new(|cx| UpcomingModel::new(session, model.clone(), cx));
        let id = model.read_with(cx, |m, _| m.entries()[0].user_book_id);
        let (home, cx) = cx.add_window_view(|window, cx| {
            HomeView::new(model.clone(), goals, upcoming, window, cx)
        });
        cx.update(|window, cx| {
            home.update(cx, |home, cx| home.rate(id, Rating::new(4.0), window, cx));
        });
        assert!(home.read_with(cx, |h, _| h.saving_ratings.contains_key(&id)));
        // The demo refuses the write; its end clears the rating shown, with
        // no frame drawn in between.
        cx.run_until_parked();
        assert!(home.read_with(cx, |h, _| h.saving_ratings.is_empty()));
        assert!(home.read_with(cx, |h, _| h.rated_here.contains(&id)));
    }

    #[test]
    fn home_takes_three_columns_on_a_wide_region_with_something_for_the_second() {
        let rem = gpui_kit::px(16.);
        let at =
            |window: f32, second: bool| home_tier(gpui_kit::px(window - 240. - 64.), rem, second);
        // The README's windows keep today's layout.
        assert_eq!(at(800., true), Tier::Folded);
        assert_eq!(at(1200., true), Tier::Rail);
        assert_eq!(at(1535., true), Tier::Rail);
        assert_eq!(at(1536., true), Tier::Three);
        assert_eq!(at(1600., true), Tier::Three);
        assert_eq!(at(2560., true), Tier::Three);
        // Nothing to rate and nothing coming: two columns, however wide.
        assert_eq!(at(2560., false), Tier::Rail);
        assert_eq!(at(800., false), Tier::Folded);
    }

    #[test]
    fn the_second_column_holds_as_many_want_to_read_covers_as_fit() {
        let rem = gpui_kit::px(16.);
        let at = |window: f32, rail: bool| {
            want_to_read_shown(gpui_kit::px(window - 240. - 64.), rem, rail)
        };
        // At the three-column threshold each column is 448: five md covers.
        assert_eq!(at(1536., true), 5);
        // A 1600 laptop panel: (1296 - 336) / 2 = 480.
        assert_eq!(at(1600., true), 6);
        // A 4K monitor at 1.5: (2256 - 336) / 2 = 960.
        assert_eq!(at(2560., true), 12);
        // With an empty rail the columns share its width too.
        assert_eq!(at(2560., false), 14);
    }

    #[test]
    fn the_pace_line_names_the_pace_and_the_finish() {
        let today = day("2026-11-20");
        let mut reading = Reading {
            id: UserBookId::new(1),
            pace: Some(Pace {
                pages_per_day: 16.2,
                finish: Some(day("2026-12-01")),
            }),
            idle_days: None,
        };
        assert_eq!(
            pace_line(&reading, today).as_deref(),
            Some("16 pages a day · done around Dec 1")
        );
        // An idle book gets no finish date.
        reading.idle_days = Some(8);
        assert_eq!(
            pace_line(&reading, today).as_deref(),
            Some("16 pages a day · Last updated 8 days ago")
        );
        reading.pace = None;
        assert_eq!(
            pace_line(&reading, today).as_deref(),
            Some("Last updated 8 days ago")
        );
        reading.idle_days = None;
        assert_eq!(pace_line(&reading, today), None);
    }

    #[test]
    fn a_rating_row_says_when_the_book_was_finished() {
        let today = day("2026-11-20");
        assert_eq!(
            finished_byline(&["Andy Weir".to_owned()], day("2026-11-03"), today),
            "Andy Weir · Finished Nov 3"
        );
        assert_eq!(
            finished_byline(&[], day("2026-11-03"), today),
            "Unknown author · Finished Nov 3"
        );
    }

    #[test]
    fn up_next_names_the_series_and_position() {
        let next = SeriesNext {
            id: UserBookId::new(2),
            series: "Dune".to_owned(),
            position: 2.0,
            after: UserBookId::new(1),
            after_position: 1.0,
        };
        assert_eq!(series_next_text(&next), "Next in Dune · #2");
    }

    fn books_goal(progress: f64) -> Goal {
        let mut goal = Goal::new(
            GoalId::new(1),
            GoalMetric::Books,
            12,
            day("2026-01-01"),
            day("2026-12-31"),
        );
        goal.progress = progress;
        goal
    }

    #[test]
    fn the_goal_sentence_names_one_book_and_counts_more() {
        let today = day("2026-11-20");
        let goal = books_goal(9.0);
        let (text, title) = goal_sentence(&goal, today, &["Project Hail Mary".to_owned()]).unwrap();
        assert_eq!(
            text,
            "2 behind pace, 3 to go in 41 days. Project Hail Mary should finish in time."
        );
        assert_eq!(&text[title.unwrap()], "Project Hail Mary");

        let (text, title) = goal_sentence(&goal, today, &["A".to_owned(), "B".to_owned()]).unwrap();
        assert_eq!(
            text,
            "2 behind pace, 3 to go in 41 days. 2 books you're reading should finish in time."
        );
        assert_eq!(title, None);

        // A right-to-left title is counted, not named.
        let (text, title) = goal_sentence(&goal, today, &["ملت عشق".to_owned()]).unwrap();
        assert_eq!(
            text,
            "2 behind pace, 3 to go in 41 days. 1 book you're reading should finish in time."
        );
        assert_eq!(title, None);

        let (text, title) = goal_sentence(&goal, today, &[]).unwrap();
        assert_eq!(text, "2 behind pace, 3 to go in 41 days.");
        assert_eq!(title, None);
    }

    #[test]
    fn the_goal_sentence_says_ahead_on_pace_and_the_days_left() {
        // 11 by Nov 20 is about the 10.9 expected.
        let (text, _) = goal_sentence(&books_goal(11.0), day("2026-11-20"), &[]).unwrap();
        assert_eq!(text, "On pace, 1 to go in 41 days.");
        let (text, _) = goal_sentence(&books_goal(10.0), day("2026-06-01"), &[]).unwrap();
        assert_eq!(text, "5 ahead of pace, 2 to go in 213 days.");
        let (text, _) = goal_sentence(&books_goal(11.0), day("2026-12-30"), &[]).unwrap();
        assert_eq!(text, "1 behind pace, 1 to go in 1 day.");
        let (text, _) = goal_sentence(&books_goal(11.0), day("2026-12-31"), &[]).unwrap();
        assert_eq!(text, "1 behind pace, 1 to go today.");
    }

    #[test]
    fn a_goal_in_pages_names_its_unit() {
        let mut goal = Goal::new(
            GoalId::new(2),
            GoalMetric::Pages,
            1000,
            day("2026-01-01"),
            day("2026-12-31"),
        );
        goal.progress = 400.0;
        let (text, _) = goal_sentence(&goal, day("2026-07-02"), &[]).unwrap();
        assert!(text.ends_with("pages to go in 182 days."), "{text}");
        assert!(text.contains("pages behind pace"), "{text}");
    }

    #[test]
    fn a_goal_not_running_today_says_how_it_stands() {
        let (text, title) =
            goal_sentence(&books_goal(3.0), day("2025-12-01"), &["Dune".to_owned()]).unwrap();
        assert_eq!(text, "Starts Jan 1, 2026.");
        assert_eq!(title, None);
        let (text, _) = goal_sentence(&books_goal(3.0), day("2027-01-05"), &[]).unwrap();
        assert_eq!(text, "Ended 9 books short.");
    }

    #[test]
    fn a_reached_goal_says_so_and_nothing_more() {
        let today = day("2026-11-20");
        let (text, title) = goal_sentence(&books_goal(12.0), today, &["Dune".to_owned()]).unwrap();
        assert_eq!(text, "Goal reached.");
        assert_eq!(title, None);
    }
}
