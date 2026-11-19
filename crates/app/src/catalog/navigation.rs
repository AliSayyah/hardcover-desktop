//! Catalogue pages (a series, an author, a book) and the history of the ones
//! opened since the last sidebar destination.
//!
//! Any view can open a page through the [`Navigator`] global; the workspace
//! owns the stack, shows its top page over the destination and handles
//! `GoBack`.

use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global};
use hardcover_core::{AuthorId, BookSummary, SeriesId};

/// A catalogue page.
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Series(SeriesId),
    Author(AuthorId),
    /// A book opened from a series, an author or similar books. Shown as the
    /// library entry when the book is in the library.
    Book(BookSummary),
}

impl Page {
    fn same_target(&self, other: &Page) -> bool {
        match (self, other) {
            (Page::Book(a), Page::Book(b)) => a.id == b.id,
            _ => self == other,
        }
    }
}

/// Pages opened since the last sidebar destination, oldest first.
#[derive(Debug, Default)]
pub struct PageStack {
    pages: Vec<Page>,
}

impl PageStack {
    /// Back goes this far at most; older pages are forgotten.
    const MAX: usize = 50;

    pub fn current(&self) -> Option<&Page> {
        self.pages.last()
    }

    /// Opens `page`. Opening the page already shown does nothing; returns
    /// whether the current page changed.
    pub fn push(&mut self, page: Page) -> bool {
        if self.current().is_some_and(|p| p.same_target(&page)) {
            return false;
        }
        if self.pages.len() == Self::MAX {
            self.pages.remove(0);
        }
        self.pages.push(page);
        true
    }

    /// Leaves the current page. `false` if there was none.
    pub fn pop(&mut self) -> bool {
        self.pages.pop().is_some()
    }

    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

/// What a view asks the workspace to do.
#[derive(Debug, Clone)]
pub enum NavigationEvent {
    Open(Page),
}

/// Carries [`NavigationEvent`]s from any view to the workspace.
pub struct Navigator;

impl EventEmitter<NavigationEvent> for Navigator {}

struct NavigatorGlobal(Entity<Navigator>);

impl Global for NavigatorGlobal {}

impl Navigator {
    /// Creates the navigator for a new workspace, replacing any earlier one.
    pub fn init(cx: &mut App) -> Entity<Navigator> {
        let navigator = cx.new(|_| Navigator);
        cx.set_global(NavigatorGlobal(navigator.clone()));
        navigator
    }

    fn emit(event: NavigationEvent, cx: &mut App) {
        if let Some(navigator) = cx.try_global::<NavigatorGlobal>().map(|g| g.0.clone()) {
            navigator.update(cx, |_, cx| cx.emit(event));
        }
    }
}

/// Opens a catalogue page.
pub fn open(page: Page, cx: &mut App) {
    Navigator::emit(NavigationEvent::Open(page), cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::BookId;

    #[test]
    fn push_skips_the_current_page_and_pop_returns() {
        let mut stack = PageStack::default();
        assert!(stack.current().is_none());
        assert!(stack.push(Page::Series(SeriesId::new(1))));
        assert!(!stack.push(Page::Series(SeriesId::new(1))));
        assert!(stack.push(Page::Author(AuthorId::new(2))));
        let book = BookSummary::new(BookId::new(3), "A Book");
        assert!(stack.push(Page::Book(book.clone())));
        let mut renamed = book.clone();
        renamed.title = "Same book, other copy".into();
        assert!(!stack.push(Page::Book(renamed)), "the same book by id");
        assert!(
            stack.push(Page::Series(SeriesId::new(1))),
            "may recur deeper"
        );

        assert!(stack.pop());
        assert_eq!(stack.current(), Some(&Page::Book(book)));
        assert!(stack.pop());
        assert!(stack.pop());
        assert_eq!(stack.current(), Some(&Page::Series(SeriesId::new(1))));
        assert!(stack.pop());
        assert!(!stack.pop());
        assert!(stack.current().is_none());
    }

    #[test]
    fn history_is_bounded_and_clears() {
        let mut stack = PageStack::default();
        for id in 0..(PageStack::MAX as i64 + 10) {
            stack.push(Page::Author(AuthorId::new(id)));
        }
        let mut depth = 0;
        while stack.pop() {
            depth += 1;
        }
        assert_eq!(depth, PageStack::MAX);
        stack.push(Page::Author(AuthorId::new(1)));
        stack.clear();
        assert!(stack.current().is_none());
    }
}
