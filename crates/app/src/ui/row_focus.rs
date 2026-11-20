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
//!   shows keyboard focus, so it takes the focus edge.
//! - [`RowFocus::is_selected`]: the row is the selected one, so its trailing
//!   figures turn from `fg-muted` to `fg`, as on a selected nav row.
//!
//! ```ignore
//! // In the delegate:
//! fn set_selected_index(&mut self, ix: Option<IndexPath>, _: &mut Window, _: &mut Context<..>) {
//!     self.focus.select(ix);
//! }
//! fn render_item(&mut self, ix: IndexPath, window: &mut Window, cx: &mut Context<..>) -> .. {
//!     ListItem::new(id).list_row(self.focus.shows_focus(ix, window), cx)
//! }
//! // After making the list:
//! RowFocus::attach(&list, |d| &mut d.focus, cx);
//! ```

use gpui_kit::component::IndexPath;
use gpui_kit::component::list::{ListDelegate, ListState};
use gpui_kit::{App, Entity, FocusHandle, Focusable as _, Window};

use super::focus::focus_visible;

/// A delegate's copy of its list's focus handle and selection.
#[derive(Default)]
pub struct RowFocus {
    handle: Option<FocusHandle>,
    selected: Option<IndexPath>,
}

impl RowFocus {
    /// Gives the delegate of `list` its list's focus handle. `field` finds
    /// the delegate's [`RowFocus`].
    pub fn attach<D: ListDelegate>(
        list: &Entity<ListState<D>>,
        field: fn(&mut D) -> &mut RowFocus,
        cx: &mut App,
    ) {
        list.update(cx, |list, cx| {
            let handle = list.focus_handle(cx);
            field(list.delegate_mut()).handle = Some(handle);
        });
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

#[cfg(test)]
mod tests {
    use super::*;

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
