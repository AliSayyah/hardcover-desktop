//! A library shelf (`hardcover-design`, Screens → Shelf): the filtered,
//! sorted collection beside the book pane.
//!
//! - **Header:** padding 12/16 and a hairline under it: the shelf's name with
//!   its count, then the filter (filling the width) and the sort, 8 apart.
//! - **Rows:** list rows, inset 8 ([`BookRow`] in a `ui::ListRow`).
//! - **Split:** the list is 384 wide by default, 272 to 640, and the pane
//!   takes the rest, at least 288. As the list narrows, the sort gives way
//!   before the filter (`size::LIST_MIN`, `size::FILTER_MIN`).

use std::collections::HashMap;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::{ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{
    Icon, IconName, IndexPath, h_flex, h_resizable, resizable_panel, v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, ScrollStrategy,
    SharedString, StyleRefinement, Styled as _, Subscription, Window, div,
};
use hardcover_core::{ReadingStatus, UserBookId};

use super::book_row::{BookRow, loading_rows};
use super::collection::{SortOrder, shelf_rows};
use super::model::LibraryModel;
use crate::book::detail_view::{BookDetailView, Subject};
use crate::chip::status_icon;
use crate::design::{colors, size};
use crate::format::status_label;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::gutter_list;
use crate::ui::{ListRow as _, PageHeader, RowFocus, empty_state, on_keyboard_entry};

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
    /// The library has never loaded: why, with "Try again".
    Failed(SharedString),
}

/// Rows of one shelf for the virtualized list: the entries' ids, each row
/// drawn from the model's entry, so a new revision copies no entries.
pub struct ShelfDelegate {
    model: Entity<LibraryModel>,
    rows: Vec<UserBookId>,
    loading: bool,
    empty: Empty,
    focus: RowFocus,
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
        let entry = self.model.read(cx).entry(id)?;
        let title = entry.book.title.clone();
        let row = BookRow::for_entry(entry).selected(self.focus.is_selected(ix));
        Some(
            ListItem::new(("library-book", id.get().cast_unsigned()))
                .list_row(self.focus.shows_focus(ix, window), cx)
                .accessibility_label(title)
                .child(row),
        )
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
    order: SortOrder,
    loading: bool,
    failure: Option<SharedString>,
}

pub struct LibraryView {
    model: Entity<LibraryModel>,
    status: ReadingStatus,
    filter: Entity<InputState>,
    order: SortOrder,
    list: Entity<ListState<ShelfDelegate>>,
    pane: Entity<ShelfPane>,
    detail: Entity<BookDetailView>,
    /// The selected entry per shelf, so switching shelves keeps each one's place.
    selection: HashMap<ReadingStatus, UserBookId>,
    /// Inputs of the rows currently in the list, to skip needless rebuilds.
    rows_key: Option<RowsKey>,
    _subscriptions: Vec<Subscription>,
}

impl LibraryView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
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
                    loading: true,
                    empty: Empty::Copy {
                        icon: IconName::Inbox,
                        title: SharedString::default(),
                        description: SharedString::default(),
                    },
                    focus: RowFocus::default(),
                },
                window,
                cx,
            )
        });
        RowFocus::attach(&list, |d| &mut d.focus, cx);
        let detail = cx.new(|cx| BookDetailView::new(model.clone(), window, cx));
        let pane = cx.new(|cx| ShelfPane {
            _model: cx.observe(&model, |_, _, cx| cx.notify()),
            model: model.clone(),
            status: ReadingStatus::CurrentlyReading,
            filter: filter.clone(),
            sort: sort.clone(),
            list: list.clone(),
        });

        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| {
                this.refresh_rows(window, cx);
                if !this.selection.contains_key(&this.status) {
                    this.select_first(window, cx);
                }
                cx.notify();
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
            status: ReadingStatus::CurrentlyReading,
            filter,
            order: SortOrder::RecentlyUpdated,
            list,
            pane,
            detail,
            selection: HashMap::new(),
            rows_key: None,
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
            self.refresh_rows(window, cx);
        }
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
    }

    pub fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| list.focus(window, cx));
    }

    fn select(&mut self, id: UserBookId, window: &mut Window, cx: &mut Context<Self>) {
        self.selection.insert(self.status, id);
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

    /// Rebuilds the shelf rows from the model, the filter and the sort order.
    fn refresh_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        let model = self.model.read(cx);
        let failure = model.load_failure().cloned();
        // Loading until the library loads, unless its first sync failed.
        let loading = !model.is_loaded() && failure.is_none();
        let key = RowsKey {
            revision: model.revision(),
            status: self.status,
            query: query.clone(),
            order: self.order,
            loading,
            failure: failure.clone(),
        };
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let entries = model.entries();
        let rows: Vec<UserBookId> = shelf_rows(entries, self.status, &query, self.order)
            .into_iter()
            .map(|ix| entries[ix].user_book_id)
            .collect();
        let shelf_name = status_label(self.status);
        let empty = if let Some(message) = failure {
            Empty::Failed(message)
        } else if !query.trim().is_empty() {
            Empty::Copy {
                icon: IconName::Search,
                title: "No matches".into(),
                description: format!("No books in {shelf_name} match “{}”.", query.trim()).into(),
            }
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
        self.list.update(cx, |list, cx| {
            let delegate = list.delegate_mut();
            delegate.rows = rows;
            delegate.loading = loading;
            delegate.empty = empty;
            cx.notify();
        });
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
/// sort are views inside it, and it observes the model for the count.
struct ShelfPane {
    model: Entity<LibraryModel>,
    status: ReadingStatus,
    filter: Entity<InputState>,
    sort: Entity<SelectState<Vec<SortOption>>>,
    list: Entity<ListState<ShelfDelegate>>,
    _model: Subscription,
}

impl Render for ShelfPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        // No count until the library has loaded: 0 would say it's empty.
        let count = model.is_loaded().then(|| model.count(self.status));
        let colors = colors(cx);
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
                    .child(h_flex().h(size::CONTROL).child(match count {
                        Some(count) => PageHeader::new(status_label(self.status)).count(count),
                        None => PageHeader::new(status_label(self.status)),
                    }))
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
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(gutter_list("shelf-scrollbar", &self.list, cx)),
            )
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
