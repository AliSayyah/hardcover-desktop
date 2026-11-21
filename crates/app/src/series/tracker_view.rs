//! The Series destination (`hardcover-design`, Screens → Series): every
//! series the reader has started, under its group, each with its books and
//! the one next step.
//!
//! - **Column:** inset 32, filling the region. The strip takes the width
//!   (as many books as fit, `row::strip_capacity`), and the 240 next lanes
//!   stay lined up on the trailing edge.
//! - **Header:** the `page-title` "Series" with
//!   the shown count, the line under it, and on the trailing edge the text
//!   filter and the filter select (168 each). It stays put while the list
//!   scrolls under it, so the filters are always at hand.
//! - **List:** a virtual list (about 200 series in a large library): each
//!   group's label rides on its first row, so every item is a row and Tab
//!   moves from one to the next (`row::SeriesRow`).
//! - **Foot:** with "Not finished", "1 finished series isn't shown." and a
//!   small ghost "Show finished", which switches the select to "All series".
//! - **States:** skeleton rows while the first load runs; a failed load is
//!   one muted sentence with "Try again"; with no series started, the empty
//!   state.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{
    Icon, IconName, IndexPath, VirtualListScrollHandle, h_flex, v_virtual_list,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Rems, Render, SharedString,
    Size, Styled as _, Subscription, Window, div, px, rems, size as pt_size,
};
use hardcover_core::Date;

use super::model::{SeriesModel, SeriesStatus};
use super::progress::{Counts, Filter, Group, SeriesSummary, counts, grouped, hidden_line};
use super::row::{SeriesRow, row_height, skeleton_row, strip_capacity};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::library::model::LibraryModel;
use crate::ui::focus::{keyed_focus, reveal_when};
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::{EmptyState, PageHeader, region_width};
use crate::upcoming::model::UpcomingModel;

/// Skeleton rows while the first load runs.
const SKELETON_ROWS: usize = 3;
/// Above the header (32).
const HEADER_ABOVE: Rems = rems(2.);
/// Between the title and the filters, beside it or wrapped under it (16).
const FILTERS_GAP: Rems = rems(1.);
/// Above the failure line (16).
const FAILED_ABOVE: Rems = rems(1.);
/// Above the "nothing shown" line and the foot's line (24).
const LINE_ABOVE: Rems = rems(1.5);
/// The narrowest the title and its line get beside the filters; below it the
/// filters wrap under them.
const TITLE_MIN: Rems = rems(22.);
/// Between the filter field and the select (8).
const BETWEEN_FILTERS: Rems = rems(0.5);

/// One choice of the filter select, with its count.
#[derive(Clone)]
struct FilterOption {
    filter: Filter,
    count: usize,
}

impl SelectItem for FilterOption {
    type Value = Filter;

    fn title(&self) -> SharedString {
        self.filter.label().into()
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_2()
            .child(div().flex_1().child(self.filter.label()))
            .child(
                div()
                    .caption(cx)
                    .baseline_on(TypeRole::Caption, TypeRole::Body)
                    .tabular()
                    .child(crate::format::count_text(self.count)),
            )
    }

    fn value(&self) -> &Filter {
        &self.filter
    }
}

fn options(counts: Counts) -> Vec<FilterOption> {
    Filter::ALL
        .into_iter()
        .map(|filter| FilterOption {
            filter,
            count: counts.of(filter),
        })
        .collect()
}

/// One item of the virtual list. The header scrolls with the rows, as on
/// every page.
#[derive(Clone)]
enum Item {
    Header,
    /// A reload failed; the series shown are the previous ones.
    Failed(SharedString),
    /// The filters leave no series.
    Nothing,
    /// A series, with its group's label and count when it is the first.
    Row(SeriesSummary, Option<(Group, usize)>),
    /// The foot: the finished series left out, or just the page's bottom
    /// inset.
    Foot {
        hidden: usize,
    },
}

/// What the list was built from.
#[derive(Clone, PartialEq)]
struct ListKey {
    summaries: usize,
    failed: Option<SharedString>,
    wrapped: bool,
    filter: Filter,
    query: String,
    rem: Pixels,
}

pub struct SeriesTrackerView {
    focus_handle: FocusHandle,
    model: Entity<SeriesModel>,
    library: Entity<LibraryModel>,
    filter_input: Entity<InputState>,
    select: Entity<SelectState<Vec<FilterOption>>>,
    /// The select's choice, kept for the session.
    filter: Filter,
    counts: Counts,
    scroll: VirtualListScrollHandle,
    list: Option<(ListKey, Rc<Vec<Item>>, ItemSizes)>,
    _subscriptions: Vec<Subscription>,
}

/// Each list item's height, for the virtual list.
type ItemSizes = Rc<Vec<Size<Pixels>>>;

impl Focusable for SeriesTrackerView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SeriesTrackerView {
    pub fn new(
        model: Entity<SeriesModel>,
        library: Entity<LibraryModel>,
        upcoming: &Entity<UpcomingModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter series"));
        let select = cx.new(|cx| {
            SelectState::new(
                options(Counts::default()),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| {
                this.changed(window, cx);
            }),
            cx.observe_in(&library, window, |this, _, window, cx| {
                this.changed(window, cx);
            }),
            cx.observe_in(upcoming, window, |this, _, window, cx| {
                this.changed(window, cx);
            }),
            cx.subscribe_in(
                &filter_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.changed(window, cx);
                        this.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                    }
                },
            ),
            cx.subscribe_in(
                &select,
                window,
                |this, _, event: &SelectEvent<Vec<FilterOption>>, _, cx| {
                    if let SelectEvent::Confirm(Some(filter)) = event {
                        this.filter = *filter;
                        this.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                        cx.notify();
                    }
                },
            ),
        ];
        Self {
            focus_handle: cx.focus_handle(),
            model,
            library,
            filter_input,
            select,
            filter: Filter::default(),
            counts: Counts::default(),
            scroll: VirtualListScrollHandle::new(),
            list: None,
            _subscriptions: subscriptions,
        }
    }

    /// Focuses the page, so the keyboard starts at the filter.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    fn today() -> Date {
        crate::dates::today()
    }

    fn summaries(&self, cx: &mut App) -> Rc<Vec<SeriesSummary>> {
        let today = Self::today();
        self.model.update(cx, |m, cx| m.summaries(today, cx))
    }

    fn query(&self, cx: &App) -> String {
        self.filter_input.read(cx).value().to_string()
    }

    /// The data or the text filter changed: the counts in the select follow.
    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let summaries = self.summaries(cx);
        let counts = counts(summaries.iter(), &self.query(cx));
        if counts != self.counts {
            self.counts = counts;
            self.select
                .update(cx, |s, cx| s.set_items(options(counts), window, cx));
        }
        cx.notify();
    }

    /// "Show finished": every series, in the select too.
    fn show_finished(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = Filter::All;
        self.select
            .update(cx, |s, cx| s.set_selected_value(&Filter::All, window, cx));
        cx.notify();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, SeriesModel::load);
    }

    /// The list's items and their heights, rebuilt when what they show
    /// changes.
    fn list(
        &mut self,
        summaries: &Rc<Vec<SeriesSummary>>,
        query: &str,
        failed: Option<SharedString>,
        wrapped: bool,
        rem: Pixels,
    ) -> (Rc<Vec<Item>>, Rc<Vec<Size<Pixels>>>) {
        let key = ListKey {
            summaries: Rc::as_ptr(summaries) as usize,
            failed: failed.clone(),
            wrapped,
            filter: self.filter,
            query: query.to_owned(),
            rem,
        };
        if let Some((k, items, sizes)) = &self.list
            && *k == key
        {
            return (items.clone(), sizes.clone());
        }
        let mut items = vec![Item::Header];
        items.extend(failed.map(Item::Failed));
        let groups = grouped(summaries, self.filter, query);
        if groups.is_empty() {
            items.push(Item::Nothing);
        }
        for (group, rows) in groups {
            let count = rows.len();
            for (ix, series) in rows.into_iter().enumerate() {
                items.push(Item::Row(
                    series.clone(),
                    (ix == 0).then_some((group, count)),
                ));
            }
        }
        let hidden = if self.filter == Filter::NotFinished {
            counts(summaries.iter(), query).finished
        } else {
            0
        };
        items.push(Item::Foot { hidden });
        let body_line = px(TypeRole::Body.scale().1);
        let sizes: Vec<Size<Pixels>> = items
            .iter()
            .map(|item| {
                let height = match item {
                    Item::Header => header_height(wrapped, rem),
                    // The small button is taller than the line beside it.
                    Item::Failed(_) => (FAILED_ABOVE + size::CONTROL_SMALL).to_pixels(rem),
                    Item::Nothing => LINE_ABOVE.to_pixels(rem) + body_line,
                    Item::Row(_, label) => row_height(label.is_some(), rem),
                    // The page's inset under the last row.
                    Item::Foot { hidden: 0 } => size::PAGE_INSET.to_pixels(rem),
                    Item::Foot { .. } => {
                        (LINE_ABOVE + size::CONTROL_SMALL + size::PAGE_INSET).to_pixels(rem)
                    }
                };
                pt_size(px(0.), height)
            })
            .collect();
        let (items, sizes) = (Rc::new(items), Rc::new(sizes));
        self.list = Some((key, items.clone(), sizes.clone()));
        (items, sizes)
    }

    /// The width the column gets: the window less the sidebar and the
    /// insets. It fills the region.
    fn column_width(window: &Window) -> Pixels {
        region_width(window, size::PAGE_INSET)
    }

    fn header(
        &self,
        shown: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        // Shift+Tab from the first row comes back to the filters.
        let focus = keyed_focus("series-header", window, cx);
        let reveal = reveal_when(
            "series-header",
            focus.contains_focused(window, cx),
            self.scroll.base_handle(),
            px(0.),
            window,
            cx,
        );
        let title = match shown {
            Some(n) => PageHeader::new("Series").count(n),
            None => PageHeader::new("Series"),
        };
        column()
            .id("series-header")
            .track_focus(&focus)
            .relative()
            .flex_none()
            .pt(HEADER_ABOVE)
            .child(reveal)
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap(FILTERS_GAP)
                    .child(
                        div().flex_1().min_w(TITLE_MIN).child(
                            title.line("The series you've started, and what's next in each."),
                        ),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap(BETWEEN_FILTERS)
                            .child(
                                div().w(size::SELECT).child(
                                    Input::new(&self.filter_input)
                                        .raised_field(false, cx)
                                        .cleanable(true)
                                        .prefix(Icon::new(IconName::Search).text_color(muted)),
                                ),
                            )
                            .child(
                                div()
                                    .w(size::SELECT)
                                    .child(Select::new(&self.select).raised_field(false, cx)),
                            ),
                    ),
            )
    }

    /// "Couldn't load your series…" and "Try again", in one muted line.
    fn failed(message: SharedString, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_wrap()
            .gap_2()
            .child(div().body().text_color(colors(cx).fg_muted).child(message))
            .child(
                Button::new("series-retry")
                    .small_control()
                    .label("Try again")
                    .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
            )
    }

    fn foot(hidden: usize, cx: &mut Context<Self>) -> AnyElement {
        if hidden == 0 {
            return div().into_any_element();
        }
        column()
            .child(
                h_flex()
                    .pt(LINE_ABOVE)
                    .gap_2()
                    .child(
                        div()
                            .body()
                            .text_color(colors(cx).fg_muted)
                            .child(hidden_line(hidden)),
                    )
                    .child(
                        Button::new("series-show-finished")
                            .ghost()
                            .small_control()
                            .label("Show finished")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.show_finished(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// One muted line where the rows would be, when the filters leave none.
    fn nothing_shown(&self, query: &str, cx: &App) -> impl IntoElement {
        let text = if !query.trim().is_empty() {
            format!("No series match “{}”.", query.trim())
        } else if self.filter == Filter::Finished {
            "You haven't finished a series yet.".to_owned()
        } else {
            "Every series you've started is finished.".to_owned()
        };
        div()
            .pt(LINE_ABOVE)
            .body()
            .text_color(colors(cx).fg_muted)
            .child(text)
    }
}

/// Whether the filters wrap under the title in a column `column` wide.
fn header_wraps(column: Pixels, rem: Pixels) -> bool {
    let beside = (TITLE_MIN + FILTERS_GAP + size::SELECT * 2. + BETWEEN_FILTERS).to_pixels(rem);
    column < beside
}

/// The header's height: the title and its line, the filters (a control)
/// beside them, or under them when they wrap.
fn header_height(wrapped: bool, rem: Pixels) -> Pixels {
    let title = PageHeader::height(1, rem);
    let filters = size::CONTROL.to_pixels(rem);
    let below = if wrapped {
        title + FILTERS_GAP.to_pixels(rem) + filters
    } else {
        title.max(filters)
    };
    HEADER_ABOVE.to_pixels(rem) + below
}

/// The page's column: inset on the sides, filling the page.
fn column() -> gpui_kit::Div {
    div().flex().flex_col().w_full().px(size::PAGE_INSET)
}

impl Render for SeriesTrackerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today = Self::today();
        let summaries = self.summaries(cx);
        let model = self.model.read(cx);
        let status = model.status().clone();
        let loaded = model.is_loaded();
        let query = self.query(cx);
        let root = div()
            .id("series-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col();
        let rem = window.rem_size();
        let column_width = Self::column_width(window);

        if !loaded || summaries.is_empty() {
            let body: AnyElement = match status {
                SeriesStatus::Failed(message) if !loaded => column()
                    .pt_4()
                    .child(Self::failed(message, cx))
                    .into_any_element(),
                _ if !loaded => column()
                    .pt_6()
                    .children((0..SKELETON_ROWS).map(|_| skeleton_row()))
                    .into_any_element(),
                _ => EmptyState::new(crate::icons::library(), "No series yet")
                    .description(
                        "When you read a book that belongs to a series, the series shows here with what comes next.",
                    )
                    .into_any_element(),
            };
            return root
                .child(self.header(None, window, cx))
                .child(div().flex_1().min_h_0().flex().flex_col().child(body))
                .into_any_element();
        }

        let failed = match status {
            SeriesStatus::Failed(message) => Some(message),
            _ => None,
        };
        let (items, sizes) = self.list(
            &summaries,
            &query,
            failed,
            header_wraps(column_width, rem),
            rem,
        );
        let shown = items.iter().filter(|i| matches!(i, Item::Row(..))).count();
        let capacity = strip_capacity(column_width, rem);
        let library = self.library.clone();
        let scroll = self.scroll.base_handle().clone();
        let list = v_virtual_list(
            cx.entity(),
            "series-list",
            sizes,
            move |this, range, window, cx| {
                range
                    .map(|ix| match &items[ix] {
                        Item::Header => this.header(Some(shown), window, cx).into_any_element(),
                        Item::Failed(message) => column()
                            .pt(FAILED_ABOVE)
                            .child(Self::failed(message.clone(), cx))
                            .into_any_element(),
                        Item::Nothing => column()
                            .child(this.nothing_shown(&query, cx))
                            .into_any_element(),
                        Item::Row(series, label) => column()
                            .child(SeriesRow::new(
                                series.clone(),
                                *label,
                                library.clone(),
                                today,
                                capacity,
                                scroll.clone(),
                            ))
                            .into_any_element(),
                        Item::Foot { hidden } => Self::foot(*hidden, cx),
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll)
        .size_full();

        root.child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .vertical_scrollbar(&self.scroll)
                .child(list),
        )
        .into_any_element()
    }
}
