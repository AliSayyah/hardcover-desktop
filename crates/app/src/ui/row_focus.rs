//! Row focus (`hardcover-design`, Rows and States): what a collection's
//! `ListDelegate` needs to draw its rows' selected and focus-visible states
//! with [`super::ListRow`].
//!
//! The kit's list owns the selection and the focus, but a delegate renders
//! its rows while the list is borrowed, so it can't ask. [`RowFocus`] keeps
//! a copy: the list's focus handle, given once after the list is made, and
//! the selected index, from the delegate's `set_selected_index` (which the
//! list calls on every change, a click and a confirm included).
//!
//! - [`RowFocus::shows_focus`]: the row is the selected one and the list
//!   shows keyboard focus, so it takes the focus treatment.
//! - [`RowFocus::is_selected`]: the row is the selected one, so its trailing
//!   figures turn from `fg-muted` to `fg`, as on a selected nav row.
//!
//! It also puts the list in the Tab order (the kit's list is not a tab stop
//! of its own), between the controls before and after it, as it is drawn:
//! - [`RowFocus::attach`] makes the list's focus handle a tab stop. Arrow
//!   keys then move within it; Tab and Shift+Tab leave it.
//! - [`RowFocus::stop_when_rows`], from the delegate's `items_count`, takes
//!   an empty list out of the order, so Tab never lands on nothing.
//! - [`on_keyboard_entry`]: when the keyboard brings focus to a list with no
//!   row selected, the view selects the first, so the focus shows where it
//!   landed. A click selects its own row, so it doesn't run then.
//! - [`enter_list`]: where a view focuses its list itself (a page opening,
//!   a shortcut), it lands on a row the same way.
//!
//! ```ignore
//! // In the delegate:
//! fn set_selected_index(&mut self, ix: Option<IndexPath>, _: &mut Window, _: &mut Context<..>) {
//!     self.focus.select(ix);
//! }
//! fn render_item(&mut self, ix: IndexPath, window: &mut Window, cx: &mut Context<..>) -> .. {
//!     ListItem::new(id).list_row(self.focus.shows_focus(ix, window), cx)
//! }
//! fn items_count(&self, _: usize, _: &App) -> usize {
//!     self.focus.stop_when_rows(self.rows.len());
//!     self.rows.len()
//! }
//! // After making the list:
//! RowFocus::attach(&list, |d| &mut d.focus, cx);
//! // In the view that shows it:
//! subscriptions.push(on_keyboard_entry(&list, window, cx, |this, window, cx| {
//!     this.select_first(window, cx)
//! }));
//! ```

use gpui_kit::component::IndexPath;
use gpui_kit::component::list::{ListDelegate, ListState};
use gpui_kit::{App, Context, Entity, FocusHandle, Focusable as _, Subscription, Window};

use super::focus::focus_visible;

/// A delegate's copy of its list's focus handle and selection.
#[derive(Default)]
pub struct RowFocus {
    handle: Option<FocusHandle>,
    selected: Option<IndexPath>,
}

impl RowFocus {
    /// Gives the delegate of `list` its list's focus handle, and makes the
    /// list a tab stop. `field` finds the delegate's [`RowFocus`].
    pub fn attach<D: ListDelegate>(
        list: &Entity<ListState<D>>,
        field: fn(&mut D) -> &mut RowFocus,
        cx: &mut App,
    ) {
        list.update(cx, |list, cx| {
            // The flag lives in the window's focus map, which every clone of
            // the handle (the one the list tracks included) reads.
            let handle = list.focus_handle(cx).tab_stop(true);
            field(list.delegate_mut()).handle = Some(handle);
        });
    }

    /// Keeps the list a tab stop only while it has rows. Call it from the
    /// delegate's `items_count`, which the list asks before it draws.
    pub fn stop_when_rows(&self, rows: usize) {
        if let Some(handle) = &self.handle {
            let _ = handle.clone().tab_stop(rows > 0);
        }
    }

    /// Records the list's selection; call it from `set_selected_index`.
    pub fn select(&mut self, ix: Option<IndexPath>) {
        self.selected = ix;
    }

    /// Whether `ix` is the selected row.
    pub fn is_selected(&self, ix: IndexPath) -> bool {
        self.selected.is_some_and(|s| s.eq_row(ix))
    }

    /// Whether `ix` is the selected row of a list that shows keyboard focus.
    pub fn shows_focus(&self, ix: IndexPath, window: &Window) -> bool {
        self.is_selected(ix)
            && self
                .handle
                .as_ref()
                .is_some_and(|handle| focus_visible(handle, window))
    }
}

/// Runs `select_first` when the keyboard brings focus to `list` (Tab,
/// Shift+Tab, a shortcut) and no row is selected. See the module doc.
pub fn on_keyboard_entry<V: 'static, D: ListDelegate>(
    list: &Entity<ListState<D>>,
    window: &mut Window,
    cx: &mut Context<V>,
    select_first: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> Subscription {
    let handle = list.read(cx).focus_handle(cx);
    let list = list.downgrade();
    cx.on_focus(&handle, window, move |view, window, cx| {
        let unselected = list
            .upgrade()
            .is_some_and(|list| list.read(cx).selected_index().is_none());
        if unselected && window.last_input_was_keyboard() {
            select_first(view, window, cx);
            cx.notify();
            window.refresh();
        }
    })
}

/// Selects the first row of `list` if none is and it has one, and shows
/// it: what [`on_keyboard_entry`] does for a list whose selection only
/// highlights.
pub fn select_first_row<D: ListDelegate>(
    list: &Entity<ListState<D>>,
    window: &mut Window,
    cx: &mut App,
) {
    list.update(cx, |list, cx| {
        if list.selected_index().is_none() && list.delegate().items_count(0, cx) > 0 {
            list.set_selected_index(Some(IndexPath::default()), window, cx);
            list.scroll_to_selected_item(window, cx);
            cx.notify();
        }
    });
    // Focus listeners run as a frame finishes; draw the selection now, not
    // at the next input.
    window.refresh();
}

/// Focuses `list` on a row: the selected one, else the first. For a view
/// that moves focus to its list itself (opening a page, a shortcut), where
/// [`on_keyboard_entry`] can't tell how focus arrived.
pub fn enter_list<D: ListDelegate>(list: &Entity<ListState<D>>, window: &mut Window, cx: &mut App) {
    select_first_row(list, window, cx);
    list.update(cx, |list, cx| list.focus(window, cx));
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gpui_kit::component::button::Button;
    use gpui_kit::component::list::{List, ListItem};
    use gpui_kit::{
        AppContext as _, Context, IntoElement, ParentElement as _, Styled as _, TestAppContext, div,
    };

    use super::*;
    use crate::ui::test_support::{host, shift_tab, tab};

    struct Rows {
        rows: Rc<Cell<usize>>,
        focus: RowFocus,
    }

    impl ListDelegate for Rows {
        type Item = ListItem;

        fn items_count(&self, _: usize, _: &App) -> usize {
            self.focus.stop_when_rows(self.rows.get());
            self.rows.get()
        }

        fn render_item(
            &mut self,
            ix: IndexPath,
            _: &mut Window,
            _: &mut Context<ListState<Self>>,
        ) -> Option<ListItem> {
            Some(ListItem::new(("row", ix.row)).child("A book"))
        }

        fn set_selected_index(
            &mut self,
            ix: Option<IndexPath>,
            _: &mut Window,
            _: &mut Context<ListState<Self>>,
        ) {
            self.focus.select(ix);
        }
    }

    type SharedHandle = Rc<RefCell<Option<FocusHandle>>>;

    /// A button, the list, a button; returns the list's handle and how many
    /// rows it has.
    fn between_buttons(
        cx: &mut TestAppContext,
    ) -> (
        &mut gpui_kit::VisualTestContext,
        SharedHandle,
        Rc<Cell<usize>>,
    ) {
        let rows = Rc::new(Cell::new(3));
        let handle = Rc::new(RefCell::new(None));
        let list = Rc::new(RefCell::new(None));
        let cx = host(cx, {
            let (rows, handle) = (rows.clone(), handle.clone());
            move |window, cx| {
                let list = list
                    .borrow_mut()
                    .get_or_insert_with(|| {
                        let rows = rows.clone();
                        let list = cx.new(|cx| {
                            ListState::new(
                                Rows {
                                    rows,
                                    focus: RowFocus::default(),
                                },
                                window,
                                cx,
                            )
                        });
                        RowFocus::attach(&list, |d| &mut d.focus, cx);
                        *handle.borrow_mut() = Some(list.read(cx).focus_handle(cx));
                        list
                    })
                    .clone();
                div()
                    .size_full()
                    .child(Button::new("before").label("Before"))
                    .child(div().h_64().child(List::new(&list).size_full()))
                    .child(Button::new("after").label("After"))
                    .into_any_element()
            }
        });
        (cx, handle, rows)
    }

    #[gpui_kit::test]
    fn the_list_is_a_tab_stop_between_its_neighbours(cx: &mut TestAppContext) {
        let (cx, handle, _) = between_buttons(cx);
        let list_focused = |cx: &mut gpui_kit::VisualTestContext| {
            let handle = handle.borrow().clone().unwrap();
            cx.update(|window, _| handle.is_focused(window))
        };
        tab(cx); // Before
        assert!(!list_focused(cx));
        tab(cx);
        assert!(list_focused(cx), "Tab reaches the list");
        tab(cx); // After
        assert!(!list_focused(cx));
        shift_tab(cx);
        assert!(list_focused(cx), "Shift+Tab comes back to it");
        shift_tab(cx);
        assert!(!list_focused(cx), "and leaves it the other way");
    }

    #[gpui_kit::test]
    fn an_empty_list_is_no_tab_stop(cx: &mut TestAppContext) {
        let (cx, handle, rows) = between_buttons(cx);
        rows.set(0);
        crate::ui::test_support::redraw(cx);
        tab(cx); // Before
        tab(cx);
        let handle = handle.borrow().clone().unwrap();
        assert!(!cx.update(|window, _| handle.is_focused(window)));
    }

    #[test]
    fn selection_follows_the_list() {
        let mut focus = RowFocus::default();
        assert!(!focus.is_selected(IndexPath::new(0)));
        focus.select(Some(IndexPath::new(2).section(1)));
        assert!(focus.is_selected(IndexPath::new(2).section(1)));
        assert!(!focus.is_selected(IndexPath::new(2)));
        focus.select(None);
        assert!(!focus.is_selected(IndexPath::new(2).section(1)));
    }
}
