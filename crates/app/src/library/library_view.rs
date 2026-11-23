//! A library shelf (`hardcover-design`, Screens → Shelf): the filtered,
//! sorted collection beside the book pane.
//!
//! - **Header:** padding 12/16 and a hairline under it: the shelf's name with
//!   its count ("2 of 8" while filtered, with "Clear filters" on the trailing
//!   edge), then the filter (filling the width) and the sort, 8 apart, then
//!   the genre and mood chips ([`FilterBar`]), 12 under them.
//! - **Rows:** list rows, inset 8 ([`BookRow`] in a `ui::ListRow`). The
//!   text filter, the chips and the sort work together; with nothing left,
//!   "No matches" and "Clear filters". While a chip is picked, a foot line
//!   says how many books no chip finds.
//! - **Split:** the list is 384 wide by default, 272 to 640, and the pane
//!   takes the rest, at least 288. As the list narrows, the sort gives way
//!   before the filter (`size::LIST_MIN`, `size::FILTER_MIN`).

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::{ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{
    Icon, IconName, IndexPath, h_flex, h_resizable, resizable_panel, v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, ScrollStrategy,
    SharedString, StyleRefinement, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::{LibraryEntry, ReadingStatus, UserBookId};

use super::book_row::{BookRow, loading_rows};
use super::collection::{SortOrder, shelf_rows};
use super::filter_bar::{FilterBar, PickChanged};
use super::model::LibraryModel;
use super::shelf_tags::ShelfTagsModel;
use super::tag_filter::{TagPick, no_matches};
use crate::book::detail_view::{BookDetailView, Subject};
use crate::chip::status_icon;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::status_label;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::{SmallControl as _, gutter_list};
use crate::ui::leaving::{Leaving, ROW_LEAVE};
use crate::ui::{
    ListRow as _, PageHeader, RowFocus, empty_state, empty_state_with_action, on_keyboard_entry,
};

#[derive(Clone)]
pub struct SortOption(SortOrder, SharedString);

impl SelectItem for SortOption {
    type Value = SortOrder;

    fn title(&self) -> SharedString {
        self.1.clone()
    }

    fn value(&self) -> &SortOrder {
        &self.0
    }
}

/// The empty state the list shows, decided by the view.
#[derive(Clone)]
enum Empty {
    /// Nothing on the shelf, or nothing matching the filter.
    Copy {
        icon: IconName,
        title: SharedString,
        description: SharedString,
    },
    /// The filters leave nothing of a shelf that has books: why, with
    /// "Clear filters".
    NoMatches(SharedString),
    /// The library has never loaded: why, with "Try again".
    Failed(SharedString),
}

/// What "Clear filters" does: clears the chips and the text.
type ClearFilters = Rc<dyn Fn(&mut Window, &mut App)>;

/// Rows of one shelf for the virtualized list: the entries' ids, each row
/// drawn from the model's entry, so a new revision copies no entries.
pub struct ShelfDelegate {
    model: Entity<LibraryModel>,
    rows: Vec<UserBookId>,
    /// How many rows the filters leave, without those leaving.
    matches: usize,
    loading: bool,
    empty: Empty,
    clear: ClearFilters,
    focus: RowFocus,
    /// Rows fading out as they leave the shelf (`ui::leaving`).
    leaving: Leaving<UserBookId>,
    /// What the leaving rows are drawn from: each as it was when it left.
    gone: Vec<LibraryEntry>,
}

impl ShelfDelegate {
    fn id_at(&self, ix: IndexPath) -> Option<UserBookId> {
        self.rows.get(ix.row).copied()
    }
}

impl ListDelegate for ShelfDelegate {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.focus.stop_when_rows(self.rows.len());
        self.rows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let id = *self.rows.get(ix.row)?;
        let opacity = self.leaving.opacity(&id, cx.background_executor().now());
        let model = self.model.read(cx);
        let entry = match opacity {
            Some(_) => self
                .gone
                .iter()
                .find(|e| e.user_book_id == id)
                .or_else(|| model.entry(id))?,
            None => model.entry(id)?,
        };
        let title = entry.book.title.clone();
        let row = BookRow::for_entry(entry).selected(self.focus.is_selected(ix));
        let mut item = ListItem::new(("library-book", id.get().cast_unsigned()))
            .list_row(
                // The ring is drawn outside the row's opacity: a leaving
                // row has none.
                opacity.is_none() && self.focus.shows_focus(ix, window),
                cx,
            )
            .accessibility_label(title)
            .child(row);
        if let Some(opacity) = opacity {
            window.request_animation_frame();
            item = item.opacity(opacity);
        }
        Some(item)
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        // Selection is owned by `LibraryView`, keyed by `user_book_id`; the
        // rows only draw it.
        self.focus.select(ix);
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        match &self.empty {
            Empty::Copy {
                icon,
                title,
                description,
            } => empty_state(icon.clone(), title.clone(), description.clone()).into_any_element(),
            Empty::NoMatches(description) => {
                let clear = self.clear.clone();
                empty_state_with_action(
                    IconName::Search,
                    "No matches".into(),
                    description.clone(),
                    Button::new("shelf-clear-filters")
                        .label("Clear filters")
                        .on_click(move |_, window, cx| clear(window, cx)),
                )
                .into_any_element()
            }
            Empty::Failed(message) => {
                crate::library::load_failed("shelf-retry", &self.model, message.clone())
                    .into_any_element()
            }
        }
    }

    fn render_loading(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        loading_rows()
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }
}

/// What the shelf's rows and empty state were built from.
#[derive(Clone, PartialEq)]
struct RowsKey {
    revision: u64,
    status: ReadingStatus,
    query: SharedString,
    pick: TagPick,
    /// The shelf tags' revision, while a pick is set (without one the tags
    /// change nothing).
    tags: u64,
    order: SortOrder,
    loading: bool,
    failure: Option<SharedString>,
}

impl RowsKey {
    /// Whether `next` shows the same shelf, filter and sort with loaded rows,
    /// so a row that leaves it fades out (`ui::leaving`).
    fn same_rows_as(&self, next: &RowsKey) -> bool {
        self.status == next.status
            && self.query == next.query
            && self.pick == next.pick
            && self.order == next.order
            && !self.loading
            && !next.loading
            && self.failure.is_none()
            && next.failure.is_none()
    }
}

pub struct LibraryView {
    model: Entity<LibraryModel>,
    tags: Entity<ShelfTagsModel>,
    status: ReadingStatus,
    filter: Entity<InputState>,
    bar: Entity<FilterBar>,
    order: SortOrder,
    list: Entity<ListState<ShelfDelegate>>,
    pane: Entity<ShelfPane>,
    detail: Entity<BookDetailView>,
    /// The selected entry per shelf, so switching shelves keeps each one's place.
    selection: HashMap<ReadingStatus, UserBookId>,
    /// Inputs of the rows currently in the list, to skip needless rebuilds.
    rows_key: Option<RowsKey>,
    /// The selected entry as the shelf last drew it, so the row can fade out
    /// as it was when the book leaves the shelf, or the library.
    selected_entry: Option<LibraryEntry>,
    /// A leaving row has faded: work the rows out again, though nothing
    /// they're built from changed.
    leaving_due: bool,
    _subscriptions: Vec<Subscription>,
}

impl LibraryView {
    pub fn new(
        model: Entity<LibraryModel>,
        tags: Entity<ShelfTagsModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let bar = cx.new(|cx| FilterBar::new(model.clone(), tags.clone(), window, cx));
        let clear: ClearFilters = {
            let this = cx.entity().downgrade();
            Rc::new(move |window, cx| {
                let _ = this.update(cx, |this, cx| this.clear_filters(window, cx));
            })
        };
        let options: Vec<SortOption> = SortOrder::ALL
            .into_iter()
            .map(|o| SortOption(o, o.label().into()))
            .collect();
        let sort = cx.new(|cx| SelectState::new(options, Some(IndexPath::new(0)), window, cx));
        let list = cx.new(|cx| {
            ListState::new(
                ShelfDelegate {
                    model: model.clone(),
                    rows: Vec::new(),
                    matches: 0,
                    loading: true,
                    empty: Empty::Copy {
                        icon: IconName::Inbox,
                        title: SharedString::default(),
                        description: SharedString::default(),
                    },
                    focus: RowFocus::default(),
                    leaving: Leaving::default(),
                    gone: Vec::new(),
                    clear: clear.clone(),
                },
                window,
                cx,
            )
        });
        RowFocus::attach(&list, |d| &mut d.focus, cx);
        let detail = cx.new(|cx| BookDetailView::new(model.clone(), window, cx));
        let pane = cx.new(|cx| ShelfPane {
            _subscriptions: vec![
                cx.observe(&model, |_, _, cx| cx.notify()),
                cx.observe(&bar, |_, _, cx| cx.notify()),
            ],
            model: model.clone(),
            status: ReadingStatus::CurrentlyReading,
            filter: filter.clone(),
            sort: sort.clone(),
            bar: bar.clone(),
            list: list.clone(),
            clear,
        });

        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| {
                this.refresh_rows(window, cx);
                if !this.selection.contains_key(&this.status) {
                    this.select_first(window, cx);
                }
                cx.notify();
            }),
            // The chips' books, while a pick narrows the shelf.
            cx.observe_in(&tags, window, |this, _, window, cx| {
                this.refresh_rows(window, cx);
            }),
            cx.subscribe_in(&bar, window, |this, _, _: &PickChanged, window, cx| {
                this.refresh_rows(window, cx);
                this.keep_selection_shown(window, cx);
            }),
            cx.subscribe_in(
                &filter,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.refresh_rows(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &sort,
                window,
                |this, _, event: &SelectEvent<Vec<SortOption>>, window, cx| {
                    if let SelectEvent::Confirm(Some(order)) = event {
                        this.order = *order;
                        this.refresh_rows(window, cx);
                        this.scroll_to_selection(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &list,
                window,
                |this, list, event: &ListEvent, window, cx| {
                    let ix = match event {
                        ListEvent::Select(ix) | ListEvent::Confirm(ix) => *ix,
                        ListEvent::Cancel => return,
                    };
                    let id = list.read(cx).delegate().id_at(ix);
                    if let Some(id) = id {
                        this.select(id, window, cx);
                    }
                },
            ),
            on_keyboard_entry(&list, window, cx, |this, window, cx| {
                this.select_first(window, cx);
            }),
        ];

        let mut this = Self {
            model,
            tags,
            status: ReadingStatus::CurrentlyReading,
            filter,
            bar,
            order: SortOrder::RecentlyUpdated,
            list,
            pane,
            detail,
            selection: HashMap::new(),
            rows_key: None,
            selected_entry: None,
            leaving_due: false,
            _subscriptions: subscriptions,
        };
        this.refresh_rows(window, cx);
        this
    }

    /// Switches to `status`, restoring that shelf's selection.
    pub fn show_shelf(
        &mut self,
        status: ReadingStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.status != status {
            self.status = status;
            self.pane.update(cx, |pane, cx| {
                pane.status = status;
                cx.notify();
            });
            self.filter.update(cx, |f, cx| f.set_value("", window, cx));
            self.bar.update(cx, |bar, cx| bar.set_status(status, cx));
            self.refresh_rows(window, cx);
        }
        self.tags.update(cx, |tags, cx| tags.show_shelf(status, cx));
        match self.selection.get(&status).copied() {
            Some(id) => self.select(id, window, cx),
            None => self.select_first(window, cx),
        }
        self.scroll_to_selection(window, cx);
        cx.notify();
    }

    /// Fetches the pane's book again where it is stale ([`crate::resource`]).
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.revalidate(shown_again, cx));
        let status = self.status;
        self.tags.update(cx, |tags, cx| {
            if shown_again {
                tags.show_shelf(status, cx);
            } else {
                tags.revalidate(status, cx);
            }
        });
    }

    /// "Clear filters": the chips and the text. Focus goes to the list,
    /// since the button that asked is gone.
    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bar.update(cx, FilterBar::clear);
        self.filter.update(cx, |f, cx| f.set_value("", window, cx));
        self.refresh_rows(window, cx);
        self.focus_list(window, cx);
    }

    /// The chips changed: a selected book they leave out gives way to the
    /// first that stays, or to none.
    fn keep_selection_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selection.get(&self.status).copied();
        let shown = selected.is_some_and(|id| self.list.read(cx).delegate().rows.contains(&id));
        if !shown {
            self.select_first(window, cx);
        }
        self.scroll_to_selection(window, cx);
    }

    pub fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| list.focus(window, cx));
    }

    fn select(&mut self, id: UserBookId, window: &mut Window, cx: &mut Context<Self>) {
        self.selection.insert(self.status, id);
        self.selected_entry = self.model.read(cx).entry(id).cloned();
        self.sync_list_selection(window, cx);
        self.detail.update(cx, |d, cx| {
            d.set_subject(Some(Subject::Entry(id)), window, cx);
        });
    }

    fn select_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self.list.read(cx).delegate().rows.first().copied();
        match first {
            Some(id) => self.select(id, window, cx),
            None => self
                .detail
                .update(cx, |d, cx| d.set_subject(None, window, cx)),
        }
    }

    /// Rebuilds the shelf rows from the model, the filters and the sort order.
    fn refresh_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        let pick = self.bar.read(cx).pick(cx);
        let tags = self.tags.read(cx);
        let model = self.model.read(cx);
        let failure = model.load_failure().cloned();
        // Loading until the library loads, unless its first sync failed.
        let loading = !model.is_loaded() && failure.is_none();
        let key = RowsKey {
            revision: model.revision(),
            status: self.status,
            query: query.clone(),
            pick: pick.clone(),
            tags: if pick.is_empty() { 0 } else { tags.revision() },
            order: self.order,
            loading,
            failure: failure.clone(),
        };
        let due = std::mem::take(&mut self.leaving_due);
        if !due && self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let animate = !cx.reduce_motion()
            && self
                .rows_key
                .as_ref()
                .is_some_and(|previous| previous.same_rows_as(&key));
        let entries = model.entries();
        let rows: Vec<UserBookId> = shelf_rows(entries, self.status, &query, self.order)
            .into_iter()
            .filter(|ix| pick.keeps(tags.facts(entries[*ix].book.id)))
            .map(|ix| entries[ix].user_book_id)
            .collect();
        let matches = rows.len();
        let shelf_name = status_label(self.status);
        let empty = if let Some(message) = failure {
            Empty::Failed(message)
        } else if (!query.trim().is_empty() || !pick.is_empty()) && model.count(self.status) > 0 {
            Empty::NoMatches(no_matches(shelf_name, &pick, &query).into())
        } else {
            Empty::Copy {
                icon: status_icon(self.status),
                title: format!("No books in {shelf_name}").into(),
                description: match self.status {
                    ReadingStatus::Ignored => {
                        "Books you hide from recommendations on Hardcover appear here."
                    }
                    _ => "Books you shelve here on Hardcover or from Search appear here.",
                }
                .into(),
            }
        };
        self.rows_key = Some(key);
        let now = cx.background_executor().now();
        let model = self.model.clone();
        let selected = self.selected_entry.take();
        let started = self.list.update(cx, |list, cx| {
            let delegate = list.delegate_mut();
            let shown = delegate.leaving.rows(&delegate.rows, rows, animate, now);
            delegate.rows = shown.rows;
            let leaving = &delegate.leaving;
            delegate
                .gone
                .retain(|e| leaving.is_leaving(&e.user_book_id));
            for id in &delegate.rows {
                if !leaving.is_leaving(id) || delegate.gone.iter().any(|e| e.user_book_id == *id) {
                    continue;
                }
                // The selected row as the shelf drew it: its entry has moved
                // on, or is gone from the library.
                let entry = match &selected {
                    Some(entry) if entry.user_book_id == *id => Some(entry.clone()),
                    _ => model.read(cx).entry(*id).cloned(),
                };
                delegate.gone.extend(entry);
            }
            delegate.loading = loading;
            delegate.matches = matches;
            delegate.empty = empty;
            cx.notify();
            shown.started
        });
        // The title's "2 of 8".
        self.pane.update(cx, |_, cx| cx.notify());
        if started {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(ROW_LEAVE).await;
                this.update_in(cx, |this, window, cx| {
                    this.leaving_due = true;
                    this.refresh_rows(window, cx);
                })
                .ok();
            })
            .detach();
        }
        self.selected_entry = self
            .selection
            .get(&self.status)
            .and_then(|id| self.model.read(cx).entry(*id).cloned());
        self.sync_list_selection(window, cx);
    }

    /// Points the list's positional selection at the selected id, if it is shown.
    fn sync_list_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.selection.get(&self.status).copied();
        self.list.update(cx, |list, cx| {
            let pos = id.and_then(|id| list.delegate().rows.iter().position(|row| *row == id));
            let ix = pos.map(IndexPath::new);
            if list.selected_index() != ix {
                list.set_selected_index(ix, window, cx);
                // The list sits in the cached shelf pane, so make sure it redraws.
                cx.notify();
            }
        });
    }

    fn scroll_to_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| {
            if let Some(ix) = list.selected_index() {
                list.scroll_to_item(ix, ScrollStrategy::Center, window, cx);
            }
        });
    }
}

/// The shelf's header, filter, sort and list, as a view of its own so
/// [`LibraryView`] can cache it. Frames driven by the detail pane (its loading
/// animation, its cover) then reuse the list instead of rendering every row
/// again. Whatever changes the pane notifies it: the list, the filter and the
/// sort are views inside it, and it observes the model for the count and
/// the filter bar for whether it shows; the shelf notifies it when its rows
/// change ("2 of 8").
struct ShelfPane {
    model: Entity<LibraryModel>,
    status: ReadingStatus,
    filter: Entity<InputState>,
    sort: Entity<SelectState<Vec<SortOption>>>,
    bar: Entity<FilterBar>,
    list: Entity<ListState<ShelfDelegate>>,
    clear: ClearFilters,
    _subscriptions: Vec<Subscription>,
}

impl Render for ShelfPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        // No count until the library has loaded: 0 would say it's empty.
        let count = model.is_loaded().then(|| model.count(self.status));
        let bar = self.bar.read(cx);
        let filtered = !bar.pick(cx).is_empty() || !self.filter.read(cx).value().trim().is_empty();
        let bar_shown = bar.is_shown(cx);
        let foot = bar.foot(cx);
        let matches = self.list.read(cx).delegate().matches;
        let colors = colors(cx);
        let header = match count {
            Some(count) if filtered => {
                PageHeader::new(status_label(self.status)).count_of(matches, count)
            }
            Some(count) => PageHeader::new(status_label(self.status)).count(count),
            None => PageHeader::new(status_label(self.status)),
        };
        let clear = self.clear.clone();
        v_flex()
            .size_full()
            .min_w_0()
            .child(
                v_flex()
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        h_flex()
                            .h(size::CONTROL)
                            .gap_2()
                            .justify_between()
                            .child(div().min_w_0().child(header))
                            .when(filtered, |this| {
                                this.child(
                                    Button::new("shelf-clear-filters-title")
                                        .ghost()
                                        .small_control()
                                        .label("Clear filters")
                                        .flex_none()
                                        .on_click(move |_, window, cx| clear(window, cx)),
                                )
                            }),
                    )
                    .child(
                        // As the list narrows the sort gives way first, down
                        // to a small select; the filter keeps a usable width,
                        // and the list's minimum leaves room for both.
                        h_flex()
                            .gap_2()
                            .child(
                                div().flex_1().min_w(size::FILTER_MIN).child(
                                    Input::new(&self.filter)
                                        .raised_field(false, cx)
                                        .cleanable(true)
                                        .prefix(
                                            Icon::new(IconName::Search).text_color(colors.fg_muted),
                                        ),
                                ),
                            )
                            .child(
                                div()
                                    .flex_basis(size::SELECT)
                                    .flex_shrink_1()
                                    .min_w(size::SELECT_SMALL)
                                    .child(Select::new(&self.sort).raised_field(false, cx)),
                            ),
                    )
                    .when(bar_shown, |this| this.child(self.bar.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(gutter_list("shelf-scrollbar", &self.list, cx)),
            )
            .when_some(foot, |this, foot| {
                this.child(div().flex_none().px_4().py_2().caption(cx).child(foot))
            })
    }
}

impl Render for LibraryView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        // Keyed by the rem: the group keeps its panels' widths in px, so a new
        // interface scale (`ui_scale`) starts it again at the default width.
        h_resizable(("library-split", f32::from(rem).to_bits() as usize))
            .child(
                resizable_panel()
                    .size(rem * size::LIST.0)
                    .size_range(rem * size::LIST_MIN.0..rem * size::LIST_MAX.0)
                    .child(
                        self.pane
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    ),
            )
            .child(
                resizable_panel()
                    .size_range(rem * size::PANE_MIN.0..gpui_kit::Pixels::MAX)
                    .child(BookDetailView::embedded(&self.detail)),
            )
    }
}
