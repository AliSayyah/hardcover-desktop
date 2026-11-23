//! Catalogue pages (a series, an author, a book, the books coming soon, a
//! vibe, a tag, Trending) and the history of the ones opened since the last sidebar
//! destination.
//!
//! Any view can open a page through the [`Navigator`] global; the workspace
//! owns the stack, shows its top page over the destination and handles
//! `GoBack`.

use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global};
use hardcover_core::{AuthorId, BookSummary, SeriesId, TrendingPeriod, VibeId};

use super::tag::Tag;
use crate::ui::focus::FocusReturn;

/// A catalogue page.
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Series(SeriesId),
    Author(AuthorId),
    /// A book opened from a series, an author or similar books. Shown as the
    /// library entry when the book is in the library.
    Book(BookSummary),
    /// Every book still to come in the reader's series, opened from Home.
    Upcoming,
    /// A vibe's books ("For you", "Top picks"), opened from Discover.
    Vibe(VibeId),
    /// A mood's or a genre's books, opened from a book's mood line, its
    /// Votes popover or a genre chip; `within` is the page's select, the
    /// other kind narrowing it, kept with the page so Back finds it.
    Tag {
        tag: Tag,
        within: Option<Tag>,
    },
    /// The books trending on Hardcover in a period, opened from Discover's
    /// "Trending this month"; the period is the page's segments' choice,
    /// kept with the page so Back finds it.
    Trending(TrendingPeriod),
}

impl Page {
    /// Whether `other` is this page, however it is narrowed (a book by id,
    /// a tag whatever its choice, Trending whatever its period).
    pub fn same_target(&self, other: &Page) -> bool {
        match (self, other) {
            (Page::Book(a), Page::Book(b)) => a.id == b.id,
            // However it is narrowed: the tag's page is shown.
            (Page::Tag { tag: a, .. }, Page::Tag { tag: b, .. }) => a == b,
            // Whatever its period: Trending is shown.
            (Page::Trending(_), Page::Trending(_)) => true,
            _ => self == other,
        }
    }
}

/// Pages opened since the last sidebar destination, oldest first, each
/// with where focus was when it opened, for Back to return to.
#[derive(Default)]
pub struct PageStack {
    pages: Vec<(Page, Option<FocusReturn>)>,
}

impl PageStack {
    /// Back goes this far at most; older pages are forgotten.
    const MAX: usize = 50;

    pub fn current(&self) -> Option<&Page> {
        self.pages.last().map(|(page, _)| page)
    }

    /// The page under the current one: where Back goes. `None` when Back
    /// returns to the destination.
    pub fn previous(&self) -> Option<&Page> {
        self.pages.len().checked_sub(2).map(|ix| &self.pages[ix].0)
    }

    /// Opens `page` from where focus is (`from`). Opening the page already
    /// shown does nothing; returns whether the current page changed.
    pub fn push(&mut self, page: Page, from: Option<FocusReturn>) -> bool {
        if self.current().is_some_and(|p| p.same_target(&page)) {
            return false;
        }
        if self.pages.len() == Self::MAX {
            self.pages.remove(0);
        }
        self.pages.push((page, from));
        true
    }

    /// Puts `page` in the current page's place, keeping where focus was
    /// when it opened: the same page, changed in place (a tag's page
    /// narrowed). Returns whether there was a page to replace.
    pub fn replace_current(&mut self, page: Page) -> bool {
        match self.pages.last_mut() {
            Some((current, _)) => {
                *current = page;
                true
            }
            None => false,
        }
    }

    /// Leaves the current page: `None` if there was none, else where focus was
    /// when it opened.
    #[expect(
        clippy::option_option,
        reason = "None means no page, while Some(None) means a page without an opener"
    )]
    pub fn pop(&mut self) -> Option<Option<FocusReturn>> {
        self.pages.pop().map(|(_, from)| from)
    }

    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

/// What a view asks the workspace to do.
#[derive(Debug, Clone)]
pub enum NavigationEvent {
    Open(Page),
    /// The page shown changed in place: keep it in its stead, for Back.
    Replace(Page),
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

/// Records that the page shown is now `page` (a tag's page narrowed by its
/// select, Trending's period), without showing it again or moving focus.
pub fn replace(page: Page, cx: &mut App) {
    Navigator::emit(NavigationEvent::Replace(page), cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::BookId;

    #[test]
    fn push_skips_the_current_page_and_pop_returns() {
        let mut stack = PageStack::default();
        assert!(stack.current().is_none());
        assert!(stack.push(Page::Series(SeriesId::new(1)), None));
        assert!(stack.previous().is_none(), "Back goes to the destination");
        assert!(!stack.push(Page::Series(SeriesId::new(1)), None));
        assert!(stack.push(Page::Author(AuthorId::new(2)), None));
        let book = BookSummary::new(BookId::new(3), "A Book");
        assert!(stack.push(Page::Book(book.clone()), None));
        let mut renamed = book.clone();
        renamed.title = "Same book, other copy".into();
        assert!(
            !stack.push(Page::Book(renamed), None),
            "the same book by id"
        );
        assert!(
            stack.push(Page::Series(SeriesId::new(1)), None),
            "may recur deeper"
        );

        assert_eq!(stack.previous(), Some(&Page::Book(book.clone())));
        assert!(stack.pop().is_some());
        assert_eq!(stack.current(), Some(&Page::Book(book)));
        assert!(stack.pop().is_some());
        assert!(stack.pop().is_some());
        assert_eq!(stack.current(), Some(&Page::Series(SeriesId::new(1))));
        assert!(stack.pop().is_some());
        assert!(stack.pop().is_none());
        assert!(stack.current().is_none());
    }

    #[test]
    fn a_tag_page_narrowed_in_place_keeps_its_opener_and_its_choice() {
        use super::super::tag::Tag;
        use hardcover_core::tags::{Mood, browse_genre};
        let relaxing = Tag::Mood(Mood::Relaxing);
        let mystery = Tag::Genre(browse_genre("Mystery").unwrap());
        let mut stack = PageStack::default();
        assert!(!stack.replace_current(Page::Upcoming), "no page to replace");
        stack.push(Page::Author(AuthorId::new(1)), None);
        stack.push(
            Page::Tag {
                tag: relaxing,
                within: None,
            },
            None,
        );
        assert!(stack.replace_current(Page::Tag {
            tag: relaxing,
            within: Some(mystery),
        }));
        assert!(
            !stack.push(
                Page::Tag {
                    tag: relaxing,
                    within: None
                },
                None
            ),
            "the tag's page is already shown"
        );
        stack.push(Page::Book(BookSummary::new(BookId::new(3), "A")), None);
        stack.pop();
        assert_eq!(
            stack.current(),
            Some(&Page::Tag {
                tag: relaxing,
                within: Some(mystery)
            }),
            "Back finds the choice"
        );
    }

    #[test]
    fn history_is_bounded_and_clears() {
        let mut stack = PageStack::default();
        for id in 0..(i64::try_from(PageStack::MAX).unwrap() + 10) {
            stack.push(Page::Author(AuthorId::new(id)), None);
        }
        let mut depth = 0;
        while stack.pop().is_some() {
            depth += 1;
        }
        assert_eq!(depth, PageStack::MAX);
        stack.push(Page::Author(AuthorId::new(1)), None);
        stack.clear();
        assert!(stack.current().is_none());
    }
}
