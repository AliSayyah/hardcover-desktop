//! The Series destination (`hardcover-design`, Screens → Series): every
//! series the reader has started, under its group, each with its books and
//! the one next step.
//!
//! - **Column:** inset 32, filling the region. The strip takes the width
//!   (as many books as fit on a line, `row::strip_capacity`), and the 240
//!   next lanes stay lined up on the trailing edge. A strip that doesn't fit
//!   is one line until its "Show the other N books" opens it; the series
//!   opened are kept for the session.
//! - **Header:** the `page-title` "Series" with
//!   the shown count, the line under it, and on the trailing edge the text
//!   filter, the filter select and the sort select (168 each), which wrap
//!   under the title, and onto a second line, in a narrow window. It sits
//!   above the list, not in it: it stays put while the list scrolls under
//!   it, so the filters are always at hand, and scrolling never takes the
//!   focused filter away. The filter and the sort are kept for the session.
//! - **List:** a virtual list (about 200 series in a large library): with
//!   the "Status" sort each group's label rides on its first row, so every
//!   item is a row and Tab moves from one to the next (`row::SeriesRow`);
//!   the other sorts are one list with no labels. A failed reload's line
//!   sits under the header, above the list, over the series still shown.
//! - **Hiding:** a row's "more" menu hides its series (kept in the
//!   preferences, so for good), and a notification says so with Undo; it
//!   then shows only under the "Hidden" filter, whose rows' menus show it
//!   again. A row hidden here keeps its place and fades out (`ui::leaving`):
//!   the list stays as it was while it fades, then closes the gap at once.
//! - **Foot:** with "Not finished", "1 finished series isn't shown." and a
//!   small ghost "Show finished", which switches the select to "All series".
//! - **States:** skeleton rows while the first load runs; a failed load is
//!   one muted sentence with "Try again" (`ui::LoadFailed`), and the
//!   library's own failure fills the page when it has never loaded; with no
//!   series started, the empty state.

use std::collections::{BTreeSet, HashSet};
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{
    Icon, IconName, IndexPath, VirtualListScrollHandle, h_flex, v_virtual_list,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Rems, Render, SharedString,
    Size, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px, rems,
    size as pt_size,
};
use hardcover_core::{Date, SeriesId};

use super::model::SeriesModel;
use super::progress::{Counts, Filter, SeriesSummary, Shown, Sort, counts, hidden_line, sections};
use super::row::{
    OnHide, OnToggle, RowHiding, RowStrip, RowTop, SeriesRow, row_height, skeleton_row,
    strip_capacity, strip_lines,
};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::library::model::LibraryModel;
use crate::preferences::AppPreferences;
use crate::resource::LoadStatus;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::leaving::{Leaving, ROW_LEAVE};
use crate::ui::{EmptyState, LoadFailed, PageHeader, region_width};
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

/// One choice of the sort select.
#[derive(Clone)]
struct SortOption(Sort);

impl SelectItem for SortOption {
    type Value = Sort;

    fn title(&self) -> SharedString {
        self.0.label().into()
    }

    fn value(&self) -> &Sort {
        &self.0
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

/// One item of the virtual list, under the header.
#[derive(Clone)]
enum Item {
    /// The filters leave no series.
    Nothing,
    /// A series, with what sits over it: its group's label and count when
    /// it is the first of a group.
    Row(SeriesSummary, RowTop),
    /// The foot: the finished series left out, or just the page's bottom
    /// inset.
    Foot { hidden: usize },
}

/// What the list was built from. The summaries are kept, not only their
/// address, so a new list is never taken for the old one at the same place.
#[derive(Clone)]
struct ListKey {
    summaries: Rc<Vec<SeriesSummary>>,
    filter: Filter,
    sort: Sort,
    hidden: BTreeSet<SeriesId>,
    query: String,
    rem: Pixels,
    /// Books a strip's line has room for, and which strips are opened: the
    /// rows' heights follow both.
    capacity: usize,
    opened: Rc<HashSet<SeriesId>>,
}

impl ListKey {
    /// Whether `other` differs only in the series hidden: the same rows,
    /// some of them hidden or shown again, so a row that leaves fades.
    fn same_rows_as(&self, other: &Self) -> bool {
        Self {
            hidden: other.hidden.clone(),
            ..self.clone()
        } == *other
    }
}

/// The series rows among a list's items, in order.
fn row_ids(items: &[Item]) -> Vec<SeriesId> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Row(series, _) => Some(series.id),
            _ => None,
        })
        .collect()
}

impl PartialEq for ListKey {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.summaries, &other.summaries)
            && self.filter == other.filter
            && self.sort == other.sort
            && self.hidden == other.hidden
            && self.query == other.query
            && self.rem == other.rem
            && self.capacity == other.capacity
            && self.opened == other.opened
    }
}

pub struct SeriesTrackerView {
    focus_handle: FocusHandle,
    model: Entity<SeriesModel>,
    library: Entity<LibraryModel>,
    filter_input: Entity<InputState>,
    select: Entity<SelectState<Vec<FilterOption>>>,
    sort_select: Entity<SelectState<Vec<SortOption>>>,
    /// The select's choice, kept for the session.
    filter: Filter,
    /// The sort select's choice, kept for the session.
    sort: Sort,
    /// The series whose strips show every book, kept for the session. A new
    /// set on every change.
    opened: Rc<HashSet<SeriesId>>,
    counts: Counts,
    scroll: VirtualListScrollHandle,
    list: Option<(ListKey, Rc<Vec<Item>>, ItemSizes)>,
    /// Rows fading out as they are hidden (`ui::leaving`): while they fade
    /// the list stays as it was, then it closes the gap at once.
    leaving: Leaving<SeriesId>,
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
        let sort_select = cx.new(|cx| {
            SelectState::new(
                Sort::ALL.map(SortOption).to_vec(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let preferences = AppPreferences::global(cx).clone();
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
            // Hiding a series, or showing it again, here or in another window.
            cx.observe_in(&preferences, window, |this, _, window, cx| {
                this.changed(window, cx);
            }),
            cx.subscribe_in(
                &sort_select,
                window,
                |this, _, event: &SelectEvent<Vec<SortOption>>, _, cx| {
                    if let SelectEvent::Confirm(Some(sort)) = event {
                        this.sort = *sort;
                        this.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                        cx.notify();
                    }
                },
            ),
            // The rows' commands follow the grant (`write:library`).
            crate::session::observe_grant(cx, |_, cx| cx.notify()),
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
            sort_select,
            filter: Filter::default(),
            sort: Sort::default(),
            opened: Rc::default(),
            counts: Counts::default(),
            scroll: VirtualListScrollHandle::new(),
            list: None,
            leaving: Leaving::default(),
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

    fn summaries(&self, cx: &App) -> Rc<Vec<SeriesSummary>> {
        self.model.read(cx).summaries(Self::today(), cx)
    }

    fn query(&self, cx: &App) -> String {
        self.filter_input.read(cx).value().to_string()
    }

    /// The series the reader hid.
    fn hidden(cx: &App) -> BTreeSet<SeriesId> {
        AppPreferences::global(cx).read(cx).hidden_series().clone()
    }

    /// The data, the text filter or the hidden series changed: the counts in
    /// the select follow.
    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let summaries = self.summaries(cx);
        let counts = counts(summaries.iter(), &self.query(cx), &Self::hidden(cx));
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

    /// "Show the other N books" or "Show fewer" on a series' strip.
    fn toggle(&mut self, id: SeriesId, cx: &mut Context<Self>) {
        let mut opened = (*self.opened).clone();
        if !opened.remove(&id) {
            opened.insert(id);
        }
        self.opened = Rc::new(opened);
        cx.notify();
    }

    /// "Hide series" or "Show series again" from a row's menu. The row
    /// leaves the list, so focus goes to the page; hiding says so with Undo,
    /// since the series is then out of sight.
    fn set_hidden(
        &mut self,
        id: SeriesId,
        hide: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .summaries(cx)
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone());
        let preferences = AppPreferences::global(cx).clone();
        preferences.update(cx, |p, cx| p.set_series_hidden(id, hide, cx));
        window.focus(&self.focus_handle, cx);
        if !hide {
            return;
        }
        let note = Notification::new()
            // One at a time: hiding another series replaces it.
            .id::<SeriesHidden>()
            .message(hidden_text(name.as_deref()))
            .action(move |_, _, cx| {
                let preferences = preferences.clone();
                let note = cx.entity().downgrade();
                Button::new("undo-hide-series")
                    .label("Undo")
                    .on_click(move |_, window, cx| {
                        preferences.update(cx, |p, cx| p.set_series_hidden(id, false, cx));
                        let _ = note.update(cx, |note, cx| note.dismiss(window, cx));
                    })
            })
            .autohide(true);
        window.push_notification(note, cx);
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, SeriesModel::load);
    }

    /// The list's items and their heights, rebuilt when what they show
    /// changes.
    fn list(
        &mut self,
        key: ListKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Rc<Vec<Item>>, Rc<Vec<Size<Pixels>>>) {
        if let Some((k, items, sizes)) = &self.list
            && *k == key
        {
            return (items.clone(), sizes.clone());
        }
        let (items, sizes) = self.build_list(&key);
        // A row hidden here keeps its place and fades: the list stays as it
        // was until the fade ends, then closes the gap at once.
        let previous = self.list.take();
        let animate = !cx.reduce_motion()
            && previous
                .as_ref()
                .is_some_and(|(k, _, _)| k.same_rows_as(&key));
        let old = previous
            .as_ref()
            .map(|(_, items, _)| row_ids(items))
            .unwrap_or_default();
        let new = row_ids(&items);
        let now = cx.background_executor().now();
        let shown = self.leaving.rows(&old, new.clone(), animate, now);
        if shown.rows != new
            && let Some((_, old_items, old_sizes)) = previous
        {
            if shown.started {
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(ROW_LEAVE).await;
                    this.update(cx, |this, cx| {
                        // Built again, without the rows that faded.
                        this.list = None;
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            self.list = Some((key, old_items.clone(), old_sizes.clone()));
            return (old_items, old_sizes);
        }
        self.list = Some((key, items.clone(), sizes.clone()));
        (items, sizes)
    }

    /// The list's items and their heights for `key`.
    fn build_list(&self, key: &ListKey) -> (Rc<Vec<Item>>, ItemSizes) {
        let (summaries, query, hidden) = (&key.summaries, key.query.as_str(), &key.hidden);
        let (rem, capacity) = (key.rem, key.capacity);
        let mut items = Vec::new();
        let shown = Shown {
            filter: self.filter,
            sort: self.sort,
            query,
            hidden,
        };
        let sections = sections(summaries, shown);
        if sections.is_empty() {
            items.push(Item::Nothing);
        }
        for (group, rows) in sections {
            let count = rows.len();
            for (ix, series) in rows.into_iter().enumerate() {
                let top = match group {
                    Some(group) if ix == 0 => RowTop::Label(group, count),
                    // The first row of a list without groups sits right
                    // under the header.
                    None if ix == 0 => RowTop::Bare,
                    _ => RowTop::Hairline,
                };
                items.push(Item::Row(series.clone(), top));
            }
        }
        let finished_left_out = if self.filter == Filter::NotFinished {
            counts(summaries.iter(), query, hidden).finished
        } else {
            0
        };
        items.push(Item::Foot {
            hidden: finished_left_out,
        });
        let sizes: Vec<Size<Pixels>> = items
            .iter()
            .map(|item| {
                let height = match item {
                    Item::Nothing => (LINE_ABOVE + TypeRole::Body.line_height()).to_pixels(rem),
                    Item::Row(series, top) => row_height(
                        *top,
                        strip_lines(series, capacity, self.opened.contains(&series.id)),
                        rem,
                    ),
                    // The page's inset under the last row.
                    Item::Foot { hidden: 0 } => size::PAGE_INSET.to_pixels(rem),
                    Item::Foot { .. } => {
                        (LINE_ABOVE + size::CONTROL_SMALL + size::PAGE_INSET).to_pixels(rem)
                    }
                };
                pt_size(px(0.), height)
            })
            .collect();
        (Rc::new(items), Rc::new(sizes))
    }

    /// The width the column gets: the window less the sidebar and the
    /// insets. It fills the region.
    fn column_width(window: &Window) -> Pixels {
        region_width(window, size::PAGE_INSET)
    }

    fn header(&self, shown: Option<usize>, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        let title = match shown {
            Some(n) => PageHeader::new("Series").count(n),
            None => PageHeader::new("Series"),
        };
        column()
            .id("series-header")
            .flex_none()
            .pt(HEADER_ABOVE)
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
                            .flex_wrap()
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
                            )
                            .child(
                                div()
                                    .w(size::SELECT)
                                    .child(Select::new(&self.sort_select).raised_field(false, cx)),
                            ),
                    ),
            )
    }

    /// "Couldn't load your series…" and "Try again", under the header.
    fn failed(message: SharedString, cx: &mut Context<Self>) -> impl IntoElement {
        column().pt(FAILED_ABOVE).child(
            LoadFailed::new("series-retry", message)
                .on_retry(cx.listener(|this, _, _, cx| this.retry(cx))),
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
        let text = nothing_text(self.filter, query, self.counts);
        div()
            .pt(LINE_ABOVE)
            .body()
            .text_color(colors(cx).fg_muted)
            .child(text)
    }
}

/// The line where the rows would be, when the filters leave none. `counts`
/// are the select's (the same text filter).
fn nothing_text(filter: Filter, query: &str, counts: Counts) -> String {
    if !query.trim().is_empty() {
        return format!("No series match “{}”.", query.trim());
    }
    match filter {
        Filter::Hidden => "You haven't hidden a series. Hide one from its menu.".to_owned(),
        Filter::Finished if counts.hidden > 0 => "No finished series to show.".to_owned(),
        Filter::Finished => "You haven't finished a series yet.".to_owned(),
        Filter::All => "Every series you've started is hidden.".to_owned(),
        Filter::NotFinished if counts.hidden > 0 => {
            "Every series you've started is finished or hidden.".to_owned()
        }
        Filter::NotFinished => "Every series you've started is finished.".to_owned(),
    }
}

/// "“Mistborn” is hidden. It's under Hidden in the filter."
fn hidden_text(name: Option<&str>) -> String {
    let what = name.map_or_else(|| "The series".to_owned(), |n| format!("“{n}”"));
    format!("{what} is hidden. It's under Hidden in the filter.")
}

/// The notification id of "… is hidden".
struct SeriesHidden;

/// The page's column: inset on the sides, filling the page.
fn column() -> gpui_kit::Div {
    div().flex().flex_col().w_full().px(size::PAGE_INSET)
}

impl Render for SeriesTrackerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let today = Self::today();
        let summaries = self.summaries(cx);
        let model = self.model.read(cx);
        let status = model.status();
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

        if let Some(message) = self.library.read(cx).load_failure().cloned() {
            return root
                .child(self.header(None, cx))
                .child(div().flex_1().min_h_0().child(crate::library::load_failed(
                    "series-library-retry",
                    &self.library,
                    message,
                )))
                .into_any_element();
        }
        if !loaded || summaries.is_empty() {
            let body: AnyElement = match status {
                LoadStatus::Failed(message) if !loaded => {
                    Self::failed(message, cx).into_any_element()
                }
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
                .child(self.header(None, cx))
                .child(div().flex_1().min_h_0().flex().flex_col().child(body))
                .into_any_element();
        }

        let failed = match status {
            LoadStatus::Failed(message) => Some(message),
            _ => None,
        };
        let capacity = strip_capacity(column_width, rem);
        let hidden = Self::hidden(cx);
        let key = ListKey {
            summaries: summaries.clone(),
            filter: self.filter,
            sort: self.sort,
            hidden: hidden.clone(),
            query: query.clone(),
            rem,
            capacity,
            opened: self.opened.clone(),
        };
        let (items, sizes) = self.list(key, window, cx);
        let shown = items.iter().filter(|i| matches!(i, Item::Row(..))).count();
        let library = self.library.clone();
        let opened = self.opened.clone();
        let view = cx.entity().downgrade();
        let on_toggle: OnToggle = Rc::new({
            let view = view.clone();
            move |id, _, cx| {
                view.update(cx, |this, cx| this.toggle(id, cx)).ok();
            }
        });
        let on_hide: OnHide = Rc::new(move |id, hide, window, cx| {
            view.update(cx, |this, cx| this.set_hidden(id, hide, window, cx))
                .ok();
        });
        let scroll = self.scroll.base_handle().clone();
        let list = v_virtual_list(
            cx.entity(),
            "series-list",
            sizes,
            move |this, range, window, cx| {
                let now = cx.background_executor().now();
                range
                    .map(|ix| match &items[ix] {
                        Item::Nothing => column()
                            .child(this.nothing_shown(&query, cx))
                            .into_any_element(),
                        Item::Row(series, top) => column()
                            .when_some(this.leaving.opacity(&series.id, now), |row, opacity| {
                                window.request_animation_frame();
                                row.opacity(opacity)
                            })
                            .child(SeriesRow::new(
                                series.clone(),
                                *top,
                                library.clone(),
                                today,
                                RowStrip {
                                    capacity,
                                    open: opened.contains(&series.id),
                                    on_toggle: on_toggle.clone(),
                                },
                                RowHiding {
                                    hidden: hidden.contains(&series.id),
                                    on_hide: on_hide.clone(),
                                },
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

        root.child(self.header(Some(shown), cx))
            .children(failed.map(|message| Self::failed(message, cx)))
            .child(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(summaries: &Rc<Vec<SeriesSummary>>) -> ListKey {
        ListKey {
            summaries: summaries.clone(),
            filter: Filter::default(),
            sort: Sort::default(),
            hidden: BTreeSet::new(),
            query: String::new(),
            rem: px(16.),
            capacity: 12,
            opened: Rc::default(),
        }
    }

    #[test]
    fn the_list_is_built_again_for_new_summaries_even_if_equal() {
        let first = Rc::new(Vec::new());
        let again = Rc::new(Vec::new());
        assert!(key(&first) == key(&first), "the same summaries: kept");
        // Worked out afresh (a sync, a day): built again, wherever the new
        // summaries were allocated.
        assert!(key(&first) != key(&again));
        // Another sort, or a series hidden: built again.
        let sorted = ListKey {
            sort: Sort::Name,
            ..key(&first)
        };
        assert!(sorted != key(&first));
        let hid = ListKey {
            hidden: [SeriesId::new(1)].into(),
            ..key(&first)
        };
        assert!(hid != key(&first));
        // Only hiding keeps the rows (so a hidden row fades); a sort doesn't.
        assert!(key(&first).same_rows_as(&hid));
        assert!(!key(&first).same_rows_as(&sorted));
        assert!(!key(&first).same_rows_as(&key(&again)));
    }

    #[test]
    fn the_empty_line_says_why() {
        let none = Counts::default();
        let some_hidden = Counts {
            hidden: 2,
            ..Counts::default()
        };
        assert_eq!(
            nothing_text(Filter::NotFinished, "", some_hidden),
            "Every series you've started is finished or hidden."
        );
        assert_eq!(
            nothing_text(Filter::NotFinished, "", none),
            "Every series you've started is finished."
        );
        assert_eq!(
            nothing_text(Filter::Hidden, " ", none),
            "You haven't hidden a series. Hide one from its menu."
        );
        assert_eq!(
            nothing_text(Filter::All, "", some_hidden),
            "Every series you've started is hidden."
        );
        assert_eq!(
            nothing_text(Filter::All, "dune", none),
            "No series match “dune”."
        );
        assert_eq!(
            hidden_text(Some("Mistborn")),
            "“Mistborn” is hidden. It's under Hidden in the filter."
        );
    }
}
