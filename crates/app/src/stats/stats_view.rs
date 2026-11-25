//! The Stats destination: what the reader finished, how, what it was, and
//! how they rated it, computed from the library on this computer
//! (`reading_stats`), and the genres and moods of the books from their tags
//! (`tag_stats`). Recomputed only when the library, the date, the period
//! or the tags change.
//!
//! A page (inset 32, filling the region): the page title with the period
//! select on its trailing edge ("All time", then each year with a finished
//! read, newest first; kept for the session), the headline figures in
//! `display-30` over muted words, then the sections 32 apart (`sections`):
//! Reading over time, Pace, Genres and moods, Formats, Book length, Your
//! ratings, Authors, Series, Rereads, From adding to finishing, Publication
//! years, Shelves. A section with nothing to say for the period isn't
//! drawn; one about a mix says when the period has too few books.
//!
//! Charts draw their bars in `accent` on a `track` grid with muted labels;
//! each bar carries its count. Counts are tabular. A chart fills its
//! column, its bars widening up to `size::CHART_BAR_MAX`; Reading over time
//! for all time gives each year at least `size::CHART_YEAR_MIN`, so a wider
//! chart reaches further back ([`years_shown`]), never fewer than
//! [`YEARS_AT_LEAST`]; the books finished before the first year shown
//! lead both charts as one "Earlier" bar. A year shows its twelve months.
//! What a section leaves out is a footnote in `caption` under it, never
//! between its parts.
//!
//! **Genres and moods** need the tags of the books finished in the period
//! (`ShelfTagsModel::show_books`: one background request per 100 books,
//! saved in the library cache): skeleton rows until they land, "Try again"
//! when they fail. A genre or mood opens the Read shelf filtered by it.
//! Without `read:catalog` the section hides.
//!
//! **Keyboard:** the page takes focus when shown; while it has focus itself
//! it scrolls from the keys (`ui::page_scroll`): Page Up and Page Down (and
//! Space) by most of a screen, Home and End to the ends, the arrows by a
//! line. Tab goes through the select and the page's buttons, kept in view.
//!
//! **Loading:** until the library has loaded once, the title and skeleton
//! figures and a chart's place.
//!
//! **Wide region** ([`two_columns`], from the window less the sidebar and
//! the insets): when two columns of at least 544 fit (a 1424 window), the
//! sections sit in two columns 32 apart, each filling its half. The figures
//! stay one row across both, 32 apart, so the third starts on the second
//! column's spine. Two lists of a section (genres and moods) sit side by
//! side when each gets `size::STATS_PAIR_MIN`.

use std::rc::Rc;

use crate::ui::reveal::RevealScroll as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{IconName, IndexPath, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{Date, GoalMetric};

use super::reading_stats::{Period, ReadingStats, compute, finish_years};
use super::sections::{self, Ctx, GoalPace, TagActions, TagState};
use super::tag_stats::{self, TagStats};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::count_text;
use crate::goals::model::GoalsModel;
use crate::library::model::LibraryModel;
use crate::library::shelf_tags::{ShelfTags, ShelfTagsModel};
use crate::resource::Demand;
use crate::session;
use crate::ui::kit::RaisedField as _;
use crate::ui::page_scroll::scroll_page;
use crate::ui::{PageHeader, Skeleton, empty_state, region_width};

/// One choice of the period select.
#[derive(Clone)]
struct PeriodOption(Period);

impl SelectItem for PeriodOption {
    type Value = Period;

    fn title(&self) -> SharedString {
        self.0.label().into()
    }

    fn value(&self) -> &Period {
        &self.0
    }
}

fn options(years: &[i32]) -> Vec<PeriodOption> {
    std::iter::once(Period::All)
        .chain(years.iter().map(|y| Period::Year(*y)))
        .map(PeriodOption)
        .collect()
}

pub struct StatsView {
    model: Entity<LibraryModel>,
    goals: Entity<GoalsModel>,
    tags: Entity<ShelfTagsModel>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    period: Period,
    select: Entity<SelectState<Vec<PeriodOption>>>,
    /// The years the select offers, and the library revision and day they
    /// were worked out for.
    years: Vec<i32>,
    years_for: Option<(u64, Date)>,
    /// The stats and the library revision, date and period they were
    /// computed for.
    computed: Option<(u64, Date, Period, Rc<ReadingStats>)>,
    /// The genres and moods, for the stats above and the tags' revision.
    tag_stats: Option<(u64, usize, Rc<TagStats>)>,
    /// "Show N more" pressed on the genres, the moods.
    expanded: (bool, bool),
    _subscriptions: Vec<Subscription>,
}

impl Focusable for StatsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl StatsView {
    pub fn new(
        model: Entity<LibraryModel>,
        goals: Entity<GoalsModel>,
        tags: Entity<ShelfTagsModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let select =
            cx.new(|cx| SelectState::new(options(&[]), Some(IndexPath::new(0)), window, cx));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe(&goals, |_, _, cx| cx.notify()),
            cx.observe(&tags, |_, _, cx| cx.notify()),
            // "Sign in again…" may grant the tags.
            session::observe_grant(cx, |_, cx| cx.notify()),
            cx.subscribe_in(
                &select,
                window,
                |this, _, event: &SelectEvent<Vec<PeriodOption>>, _, cx| {
                    if let SelectEvent::Confirm(Some(period)) = event {
                        this.set_period(*period, cx);
                    }
                },
            ),
        ];
        Self {
            model,
            goals,
            tags,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            period: Period::All,
            select,
            years: Vec::new(),
            years_for: None,
            computed: None,
            tag_stats: None,
            expanded: (false, false),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    /// Stats is shown: asks for the tags of the period's books.
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.request_tags(Demand::Shown, cx);
    }

    /// Asks again for what is stale (`Workspace::revalidate_visible`).
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.request_tags(crate::catalog::book_list::demand(shown_again), cx);
    }

    fn request_tags(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if !session::has_scope(cx, READ_CATALOG) {
            return;
        }
        let books = self.stats(cx).books.clone();
        self.tags
            .update(cx, |tags, cx| tags.show_books(books, demand, cx));
    }

    fn set_period(&mut self, period: Period, cx: &mut Context<Self>) {
        if self.period == period {
            return;
        }
        self.period = period;
        self.expanded = (false, false);
        self.scroll
            .set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)));
        self.request_tags(Demand::Shown, cx);
        cx.notify();
    }

    /// The library or the day changed: the select offers the years it now
    /// has (worked out as the page is drawn, once per revision and day). A year
    /// that no longer has a finished read goes back to all time.
    fn follow_years(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let key = (model.revision(), crate::dates::today());
        if !model.is_loaded() || self.years_for == Some(key) {
            return;
        }
        self.years_for = Some(key);
        let years = finish_years(model.entries(), key.1);
        if years == self.years {
            return;
        }
        self.years = years;
        if let Period::Year(year) = self.period
            && !self.years.contains(&year)
        {
            self.period = Period::All;
        }
        let period = self.period;
        let items = options(&self.years);
        self.select.update(cx, |s, cx| {
            s.set_items(items, window, cx);
            s.set_selected_value(&period, window, cx);
        });
    }

    /// Scrolls the page from the keyboard while it has focus itself.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if !scroll_page(&self.scroll, event, true, window) {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn stats(&mut self, cx: &App) -> Rc<ReadingStats> {
        let model = self.model.read(cx);
        let revision = model.revision();
        let today = crate::dates::today();
        let period = self.period;
        match &self.computed {
            Some((r, d, p, stats)) if *r == revision && *d == today && *p == period => {
                stats.clone()
            }
            _ => {
                let stats = Rc::new(compute(model.entries(), today, period));
                self.computed = Some((revision, today, period, stats.clone()));
                stats
            }
        }
    }

    /// Where the genres and moods stand for `stats`' books.
    fn tag_state(&mut self, stats: &Rc<ReadingStats>, cx: &App) -> TagState {
        let tags = self.tags.read(cx);
        match tags.shelf(&stats.books) {
            ShelfTags::Unavailable => TagState::Hidden,
            ShelfTags::Loading => TagState::Loading,
            ShelfTags::Failed(message) => TagState::Failed(message),
            ShelfTags::Ready { failure } => {
                let revision = tags.revision();
                // Keyed by the stats' address: the stats are recomputed only
                // when their key changes, and kept while shown.
                let key = Rc::as_ptr(stats) as usize;
                let tag_stats = match &self.tag_stats {
                    Some((r, k, t)) if *r == revision && *k == key => t.clone(),
                    _ => {
                        let t = Rc::new(tag_stats::compute(&stats.books, |id| tags.facts(id)));
                        self.tag_stats = Some((revision, key, t.clone()));
                        t
                    }
                };
                TagState::Ready {
                    stats: tag_stats,
                    failure,
                }
            }
        }
    }

    /// The books goal for the period's year, if it runs today.
    fn goal_pace(&self, today: Date, cx: &App) -> Option<GoalPace> {
        let Period::Year(year) = self.period else {
            return None;
        };
        self.goals
            .read(cx)
            .current(today)
            .find(|g| g.metric == GoalMetric::Books && g.last_day().year() == year)
            .map(|g| GoalPace {
                ahead: g.pace(today),
            })
    }

    fn tag_actions(&self, cx: &mut Context<Self>) -> TagActions {
        let this = cx.entity().downgrade();
        let retry = {
            let this = this.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        let books = this.stats(cx).books.clone();
                        this.tags.update(cx, |t, cx| t.retry_books(&books, cx));
                    });
                }
            }) as Rc<dyn Fn(&mut Window, &mut App)>
        };
        let expand = |genres: bool| {
            let this = this.clone();
            Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        if genres {
                            this.expanded.0 = true;
                        } else {
                            this.expanded.1 = true;
                        }
                        // The button goes: focus stays on the page, which
                        // keeps scrolling from the keys.
                        window.focus(&this.focus_handle, cx);
                        cx.notify();
                    });
                }
            }) as Rc<dyn Fn(&mut Window, &mut App)>)
        };
        TagActions {
            retry,
            more_genres: expand(true),
            more_moods: expand(false),
        }
    }
}

use chrono::Datelike as _;

impl Render for StatsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let (loaded, failure) = (model.is_loaded(), model.load_failure().cloned());
        let root = div()
            .id("stats")
            .track_focus(&self.focus_handle)
            .size_full();
        if let Some(message) = failure {
            return root
                .child(crate::library::load_failed(
                    "stats-retry",
                    &self.model,
                    message,
                ))
                .into_any_element();
        }
        if !loaded {
            return root.child(loading_page()).into_any_element();
        }
        self.follow_years(window, cx);
        let stats = self.stats(cx);
        if stats.is_empty() {
            return root
                .child(empty_state(
                    IconName::ChartPie,
                    "No stats yet".into(),
                    "Add books to your library and finish them, and your reading is counted here."
                        .into(),
                ))
                .into_any_element();
        }
        let today = crate::dates::today();
        let rem = window.rem_size();
        let width = region_width(window, size::PAGE_INSET);
        let wide = two_columns(width, rem);
        let column = chart_column(width, rem);
        let ctx = Ctx {
            stats: &stats,
            bar: size::CHART_BAR_MAX.to_pixels(rem),
            years: years_shown(column, rem),
            pair: column >= size::STATS_PAIR_MIN.to_pixels(rem) * 2. + rems(2.).to_pixels(rem),
        };
        let tag_state = self.tag_state(&stats, cx);
        let actions = self.tag_actions(cx);
        let goal = self.goal_pace(today, cx);
        let expanded = self.expanded;

        let over_time = sections::over_time(&ctx, cx);
        let pace = sections::pace(&ctx, goal, cx);
        let tags = sections::genres_and_moods(&ctx, tag_state, expanded, actions, cx);
        let formats = sections::formats(&ctx, cx);
        let lengths = sections::lengths(&ctx, cx);
        let ratings = sections::ratings(&ctx, cx);
        let authors = sections::authors(&ctx, cx);
        let series = sections::series(&ctx, cx);
        let rereads = sections::rereads(&ctx, cx);
        let wait = sections::wait(&ctx, cx);
        let decades = sections::decades(&ctx, cx);
        let shelves = Some(sections::shelves(&ctx, cx));

        let header = PageHeader::new("Stats")
            .line("From your library on this computer.")
            .actions(
                div()
                    .w(size::SELECT)
                    .child(Select::new(&self.select).raised_field(false, cx)),
            );
        let page = v_flex()
            .w_full()
            .p_8()
            .gap_8()
            .child(header)
            .child(render_figures(&stats, wide, cx));
        let page = if wide {
            let column = |parts: Vec<Option<AnyElement>>| {
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_8()
                    .children(parts.into_iter().flatten())
            };
            page.child(
                h_flex()
                    .items_start()
                    .gap_8()
                    .child(column(vec![over_time, tags, authors, series, decades]))
                    .child(column(vec![
                        pace, formats, lengths, ratings, rereads, wait, shelves,
                    ])),
            )
        } else {
            page.children(
                [
                    over_time, pace, tags, formats, lengths, ratings, authors, series, rereads,
                    wait, decades, shelves,
                ]
                .into_iter()
                .flatten(),
            )
        };
        // The scrollbar overlays a wrapper: inside the scroller it would
        // scroll away with the page.
        div()
            .id("stats-region")
            .relative()
            .size_full()
            .child(
                root.on_key_down(cx.listener(Self::on_key_down))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(page)
                    .reveals_focus(&self.scroll),
            )
            .vertical_scrollbar(&self.scroll)
            .into_any_element()
    }
}

/// The page while the library loads: its title, skeleton figures and the
/// first chart's place.
fn loading_page() -> impl IntoElement {
    let figure = || {
        v_flex()
            .flex_1()
            .flex_basis(size::FIGURE)
            .min_w_0()
            .gap_1()
            .child(Skeleton::line_in(TypeRole::Display30).w_16())
            .child(Skeleton::line_in(TypeRole::Body).w_32())
    };
    v_flex()
        .w_full()
        .p_8()
        .gap_8()
        .child(PageHeader::new("Stats").line("From your library on this computer."))
        .child(
            h_flex()
                .flex_wrap()
                .items_start()
                .gap_x_6()
                .gap_y_6()
                .children((0..4).map(|_| figure())),
        )
        .child(
            v_flex()
                .gap_3()
                .child(Skeleton::line_in(TypeRole::Section).w_40())
                .child(Skeleton::line().h(size::CHART)),
        )
}

/// Whether Stats' sections sit in two columns in content `width` wide (the
/// window less the sidebar and the insets): both columns get at least
/// [`size::CHART_COLUMN_MIN`], 32 apart.
fn two_columns(width: Pixels, rem: Pixels) -> bool {
    width >= size::CHART_COLUMN_MIN.to_pixels(rem) * 2. + rems(2.).to_pixels(rem)
}

/// How wide a chart's column is in content `width` wide: the whole width,
/// or half of it less the gap in [`two_columns`].
fn chart_column(width: Pixels, rem: Pixels) -> Pixels {
    if two_columns(width, rem) {
        (width - rems(2.).to_pixels(rem)) / 2.
    } else {
        width
    }
}

/// The fewest years Reading over time shows, when the library has them.
const YEARS_AT_LEAST: usize = 12;

/// How many years Reading over time shows in a column `column` wide: one
/// per `size::CHART_YEAR_MIN`, at least [`YEARS_AT_LEAST`]. Fewer when the
/// library has fewer.
fn years_shown(column: Pixels, rem: Pixels) -> usize {
    let fits = (column / size::CHART_YEAR_MIN.to_pixels(rem)).floor();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a floored width ratio; the saturating cast bounds it"
    )]
    let fits = if fits.is_finite() && fits > 0. {
        fits as usize
    } else {
        0
    };
    fits.max(YEARS_AT_LEAST)
}

/// A headline figure in `display-30` over what it counts, muted. Each takes
/// an equal share of its row, at least `size::FIGURE` wide, so figures that
/// wrap keep their columns.
fn figure(value: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .flex_basis(size::FIGURE)
        .min_w_0()
        .child(div().display_30().tabular().child(value.into()))
        .child(
            div()
                .body()
                .text_color(colors(cx).fg_muted)
                .child(label.into()),
        )
        .into_any_element()
}

/// The headline figures: books finished, pages read, hours listened (when
/// any) and the average rating; `wide`, 32 apart (the columns' gap), so
/// they line up with the two columns under them.
fn render_figures(stats: &ReadingStats, wide: bool, cx: &App) -> impl IntoElement {
    let within = match stats.period {
        Period::All => String::new(),
        Period::Year(year) => format!(" in {year}"),
    };
    let mut figures = vec![
        figure(
            count_text(stats.reads),
            match stats.reads {
                1 => format!("book finished{within}"),
                _ => format!("books finished{within}"),
            },
            cx,
        ),
        figure(count_text(stats.pages), format!("pages read{within}"), cx),
    ];
    let hours = stats.hours_listened();
    if hours > 0 {
        figures.push(figure(
            count_text(hours),
            match hours {
                1 => format!("hour listened{within}"),
                _ => format!("hours listened{within}"),
            },
            cx,
        ));
    }
    if let Some(avg) = stats.ratings.average {
        figures.push(figure(
            format!("{avg:.2}"),
            format!("average rating{within}"),
            cx,
        ));
    }
    h_flex()
        .flex_wrap()
        .items_start()
        .map(|this| if wide { this.gap_x_8() } else { this.gap_x_6() })
        .gap_y_6()
        .children(figures)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::session::SessionScope;
    use crate::ui::test_support::{press, redraw};
    use gpui_kit::{TestAppContext, px, size as gsize};
    use hardcover_core::store::LibraryCache;

    fn stats_window(
        cx: &mut TestAppContext,
    ) -> (Entity<StatsView>, &mut gpui_kit::VisualTestContext) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        cx.update(|cx| {
            gpui_kit::init(cx);
            session.install(cx);
        });
        let model = cx.new(|_| LibraryModel::new(session.clone()));
        let goals = cx.new(|cx| GoalsModel::new(session.clone(), &model, cx));
        let tags = cx.new(|cx| ShelfTagsModel::new(session.clone(), model.clone(), cx));
        model.update(cx, LibraryModel::start);
        cx.run_until_parked();
        cx.add_window_view(|window, cx| StatsView::new(model, goals, tags, window, cx))
    }

    #[gpui_kit::test]
    fn page_down_scrolls_stats_while_it_has_focus(cx: &mut TestAppContext) {
        let (view, cx) = stats_window(cx);
        cx.simulate_resize(gsize(px(900.), px(400.)));
        cx.update(|window, cx| view.update(cx, |v, cx| v.focus(window, cx)));
        redraw(cx);
        let offset =
            |cx: &mut gpui_kit::VisualTestContext| view.read_with(cx, |v, _| v.scroll.offset().y);
        assert_eq!(offset(cx), px(0.));
        press(cx, "pagedown");
        redraw(cx);
        assert!(offset(cx) < px(0.), "scrolled down");
        press(cx, "home");
        redraw(cx);
        assert_eq!(offset(cx), px(0.));
    }

    #[gpui_kit::test]
    fn the_select_offers_each_year_with_a_finished_read(cx: &mut TestAppContext) {
        let (view, cx) = stats_window(cx);
        redraw(cx);
        let years = view.read_with(cx, |v, _| v.years.clone());
        assert!(!years.is_empty(), "the demo has finished reads");
        assert!(years.windows(2).all(|w| w[0] > w[1]), "newest first");
        let year = years[0];
        view.update(cx, |v, cx| v.set_period(Period::Year(year), cx));
        redraw(cx);
        let stats = view.update(cx, |v, cx| v.stats(cx));
        assert_eq!(stats.period, Period::Year(year));
        assert!(stats.reads > 0);
    }

    #[test]
    fn stats_splits_into_two_columns_only_on_a_wide_region() {
        let rem = gpui_kit::px(16.);
        let at = |window: f32| two_columns(gpui_kit::px(window - 240. - 64.), rem);
        // 544 + 32 + 544 = 1120 of content: a 1424 window.
        assert!(two_columns(gpui_kit::px(1120.), rem));
        assert!(!two_columns(gpui_kit::px(1119.), rem));
        assert!(!at(800.));
        assert!(!at(1200.));
        assert!(!at(1423.));
        assert!(at(1424.));
        assert!(at(1600.));
        assert!(at(2560.));
    }

    #[test]
    fn each_column_fills_its_share_of_the_page() {
        let rem = gpui_kit::px(16.);
        let content = |window: f32| gpui_kit::px(window - 240. - 64.);
        // One column: the whole content.
        assert_eq!(chart_column(content(800.), rem), gpui_kit::px(496.));
        assert_eq!(chart_column(content(1200.), rem), gpui_kit::px(896.));
        assert_eq!(chart_column(content(1423.), rem), gpui_kit::px(1119.));
        // Two: each half, less half the 32 gap.
        assert_eq!(chart_column(content(1424.), rem), gpui_kit::px(544.));
        assert_eq!(chart_column(content(1600.), rem), gpui_kit::px(632.));
        assert_eq!(chart_column(content(2560.), rem), gpui_kit::px(1112.));
    }

    #[test]
    fn a_wider_chart_shows_more_years() {
        let rem = gpui_kit::px(16.);
        let years =
            |window: f32| years_shown(chart_column(gpui_kit::px(window - 240. - 64.), rem), rem);
        // Never fewer than twelve, as before.
        assert_eq!(years(800.), 12);
        assert_eq!(years(1600.), 12);
        // One per 64: 896 at the default window, 1112 per column at 2560.
        assert_eq!(years(1200.), 14);
        assert_eq!(years(2560.), 17);
        assert_eq!(years_shown(gpui_kit::px(64. * 20.), rem), 20);
        assert_eq!(years_shown(gpui_kit::px(64. * 20. - 1.), rem), 19);
        assert_eq!(years_shown(gpui_kit::px(0.), rem), 12);
    }
}
