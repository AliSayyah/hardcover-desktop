//! A tag's page (`hardcover-design`, A tag's page): the books Hardcover's
//! readers file under a mood or a genre, the most votes first, over the
//! page it was opened from (a book's mood line, its Votes popover, a genre
//! chip).
//!
//! - **Header:** the tag's name as the page title, the muted intro
//!   ([`tag::intro`]) at the prose measure, and on the title's line one
//!   select (`size::SELECT`) for the other kind: "Any genre" and Browse's
//!   genres (with their icons) on a mood's page, "Any mood" and the 14
//!   moods on a genre's. The choice is kept with the page in the page stack
//!   (`navigation::replace`), so Back finds it.
//! - **Order:** always by votes, for the mood whenever one is involved
//!   ([`tag::request_key`]); no sort control.
//! - **Rows:** catalogue rows ("Travis Baldree · 2022 · 1,204 votes") with
//!   each book's StatusChip, a page of `TAG_BOOKS_PER_PAGE` at a time as the
//!   list nears its end (`resource::Catalog::tag_books`). The shared frame
//!   of `book_list`: the book pane beside the list in a wide window, a
//!   click only selecting there, Enter or a double click opening the book.
//! - **States:** skeleton rows while the first page loads, `ui::LoadFailed`
//!   (filling the region) when it failed, "No matches" when a choice has
//!   no books, and a later page's failure as a line under the header.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::list::ListState;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, RenderOnce, ScrollHandle, SharedString, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::TagBook;

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, LoadMore, PageRows, demand,
    focus_list_if_fallback_is_focused, follow_list, new_list, page_body, page_header_scrolling,
    page_list, page_loading, reset_scroll,
};
use super::navigation::{self, Page};
use super::tag::{self, Tag};
use crate::chip::genre::genre_icon;
use crate::design::{colors, size};
use crate::library::model::LibraryModel;
use crate::resource::catalog::{Paged, TagBooksKey};
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::ui::kit::RaisedField as _;
use crate::ui::{LoadFailed, PageHeader};

/// One choice of the page's select: `None` narrows nothing.
#[derive(Clone)]
pub struct Choice {
    within: Option<Tag>,
    title: SharedString,
}

impl Choice {
    fn new(page: Tag, within: Option<Tag>) -> Self {
        let title = within.map_or(page.any_other(), Tag::name).into();
        Self { within, title }
    }

    fn label(&self) -> ChoiceLabel {
        ChoiceLabel {
            icon: match self.within {
                Some(Tag::Genre(genre)) => genre_icon(genre.name),
                _ => None,
            },
            title: self.title.clone(),
        }
    }
}

/// A choice as drawn, in the dropdown and the trigger: a genre with its
/// icon in `accent`, as its chip has it.
#[derive(IntoElement)]
struct ChoiceLabel {
    icon: Option<IconName>,
    title: SharedString,
}

impl RenderOnce for ChoiceLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .gap_2()
            .min_w_0()
            .when_some(self.icon, |this, icon| {
                this.child(
                    Icon::new(icon)
                        .xsmall()
                        .flex_shrink_0()
                        .text_color(colors(cx).accent),
                )
            })
            .child(div().min_w_0().truncate().child(self.title))
    }
}

impl SelectItem for Choice {
    type Value = Option<Tag>;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Option<Tag> {
        &self.within
    }

    fn display_title(&self) -> Option<AnyElement> {
        Some(self.label().into_any_element())
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.label()
    }
}

/// The select's choices on `page`'s page: narrowing nothing, then the
/// other kind.
fn choices(page: Tag) -> Vec<Choice> {
    std::iter::once(None)
        .chain(page.others().into_iter().map(Some))
        .map(|within| Choice::new(page, within))
        .collect()
}

pub struct TagView {
    /// The session's catalogue, which holds the pages; `None` once signed
    /// out.
    catalog: Option<Catalog>,
    /// The page's tag and the select's choice.
    current: Option<(Tag, Option<Tag>)>,
    /// Holds focus while the first page loads; see [`Self::focus`].
    focus_when_ready: Option<FocusHandle>,
    select: Entity<SelectState<Vec<Choice>>>,
    list: Entity<ListState<CatalogBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// Which books the list holds, and each choice's selected book.
    rows: PageRows<TagBooksKey, Paged<TagBook>>,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl TagView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = ListPane::new(model.clone(), window, cx);
        let list = new_list(model.clone(), "", &pane, window, cx);
        let select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let mut subscriptions = follow_list(&list, &model, &pane, window, cx);
        let catalog = Catalog::global(cx);
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe_in(
                &catalog.tag_books,
                window,
                |this, _, SharedChanged(key): &SharedChanged<TagBooksKey>, window, cx| {
                    if this.key() == Some(*key) {
                        this.page_changed(window, cx);
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &select,
            window,
            |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                if let SelectEvent::Confirm(Some(within)) = event {
                    this.narrow(*within, window, cx);
                }
            },
        ));
        Self {
            catalog,
            current: None,
            focus_when_ready: None,
            select,
            list,
            pane,
            rows: PageRows::default(),
            header_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `tag`'s page, narrowed by `within`.
    pub fn show(
        &mut self,
        tag: Tag,
        within: Option<Tag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.current.map(|(t, _)| t);
        if previous != Some(tag) {
            reset_scroll(&self.header_scroll);
        }
        self.select.update(cx, |select, cx| {
            select.set_items(choices(tag), window, cx);
            select.set_selected_value(&within, window, cx);
        });
        self.current = Some((tag, within));
        self.request(Demand::Shown, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// The select chose `within`: the page narrows in place, and the page
    /// stack keeps the choice for Back.
    fn narrow(&mut self, within: Option<Tag>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tag, current)) = self.current else {
            return;
        };
        if current == within {
            return;
        }
        self.current = Some((tag, within));
        self.request(Demand::Shown, cx);
        self.fill_list(window, cx);
        navigation::replace(Page::Tag { tag, within }, cx);
        cx.notify();
    }

    /// Focuses the book list, which renders only once the first page is in.
    /// Until then `fallback` holds focus, so Escape and Back keep working,
    /// and the list takes over when the page arrives if `fallback` still
    /// has it.
    pub fn focus(&mut self, fallback: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        if self.page(cx).is_some_and(|p| !p.items.is_empty()) {
            self.focus_when_ready = None;
            crate::ui::enter_list(&self.list, window, cx);
        } else {
            self.focus_when_ready = Some(fallback.clone());
            window.focus(fallback, cx);
        }
    }

    /// What the page asks Hardcover for.
    fn key(&self) -> Option<TagBooksKey> {
        self.current
            .map(|(tag, within)| tag::request_key(tag, within))
    }

    /// The books shown, once the first page is in.
    fn page(&self, cx: &App) -> Option<Arc<Paged<TagBook>>> {
        let key = self.key()?;
        self.catalog
            .as_ref()?
            .tag_books
            .read(cx)
            .resource()
            .value(&key)
            .cloned()
    }

    fn request(&self, demand: Demand, cx: &mut App) {
        if let (Some(key), Some(catalog)) = (self.key(), &self.catalog) {
            catalog.request_tag_books(key, demand, cx);
        }
    }

    /// The books shown changed in the catalogue: a page landed, failed, or
    /// was fetched again.
    fn page_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_list(window, cx);
        if self.page(cx).is_some_and(|p| !p.items.is_empty()) {
            focus_list_if_fallback_is_focused(&self.list, self.focus_when_ready.take(), window, cx);
        }
        cx.notify();
    }

    /// Fetches again after a failure: the first page, or the next one.
    fn retry(&mut self, cx: &mut Context<Self>) {
        match (self.page(cx), self.key(), &self.catalog) {
            (Some(_), Some(key), Some(catalog)) => catalog.more_tag_books(key, cx),
            _ => self.request(Demand::Now, cx),
        }
        cx.notify();
    }

    fn fill_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((tag, within)), Some(key)) = (self.current, self.key()) else {
            return;
        };
        let page = self.page(cx);
        self.rows.show(
            key,
            page.as_ref(),
            Self::rows,
            false,
            &self.list,
            window,
            cx,
        );
        let more: Option<LoadMore> = match (&self.catalog, &page) {
            (Some(catalog), Some(page)) if page.has_more && self.more_error(cx).is_none() => {
                let catalog = catalog.clone();
                Some(Rc::new(move |cx: &mut App| catalog.more_tag_books(key, cx)))
            }
            _ => None,
        };
        self.list.update(cx, |list, cx| {
            let books = list.delegate_mut();
            books.set_empty("No matches", tag::empty_text(tag, within));
            books.set_more(more);
            cx.notify();
        });
    }

    /// The list's rows for a loaded page.
    fn rows(page: &Paged<TagBook>) -> Vec<CatalogRow> {
        page.items
            .iter()
            .map(|b| CatalogRow {
                details: tag::row_line(&b.book, b.votes).into(),
                book: b.book.clone(),
            })
            .collect()
    }

    /// A later page's failure, shown under the header.
    fn more_error(&self, cx: &App) -> Option<SharedString> {
        let key = self.key()?;
        self.catalog
            .as_ref()?
            .tag_books
            .read(cx)
            .resource()
            .more_error(&key)
            .cloned()
    }

    /// The first page's failure, in the list's place.
    fn error(&self, cx: &App) -> Option<SharedString> {
        let key = self.key()?;
        let store = self.catalog.as_ref()?.tag_books.read(cx);
        let store = store.resource();
        store
            .error(&key)
            .filter(|_| !store.is_loading(&key))
            .cloned()
    }

    /// Asks for the books again if they are stale (`resource`; a failed
    /// page only when `shown_again`), and for the book in the pane.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.request(demand(shown_again), cx);
        self.pane.revalidate(shown_again, cx);
    }

    fn render_header(&self, tag: Tag, within: Option<Tag>, cx: &mut Context<Self>) -> AnyElement {
        let select = div().w(size::SELECT).child(
            Select::new(&self.select)
                .accessibility_label(match tag {
                    Tag::Mood(_) => "Genre",
                    Tag::Genre(_) => "Mood",
                })
                .raised_field(false, cx),
        );
        let more_error = self.more_error(cx);
        v_flex()
            .gap_4()
            .child(
                PageHeader::new(tag.name())
                    .wraps()
                    .actions(select)
                    .line(tag::intro(tag, within)),
            )
            .when_some(more_error, |this, message| {
                this.child(
                    LoadFailed::new("retry-tag-books", message)
                        .on_retry(cx.listener(|this, _, _, cx| this.retry(cx))),
                )
            })
            .into_any_element()
    }
}

impl Render for TagView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((tag, within)) = self.current else {
            return div().into_any_element();
        };
        let header = page_header_scrolling(
            "tag-header",
            &self.header_scroll,
            self.render_header(tag, within, cx),
        );
        let body = match (self.page(cx), self.error(cx)) {
            (Some(_), _) => page_body().child(page_list(&self.list)).into_any_element(),
            (None, Some(message)) => page_body()
                .child(
                    LoadFailed::new("retry-tag-books-first", message)
                        .region("Couldn't load these books")
                        .on_retry(cx.listener(|this, _, _, cx| this.retry(cx))),
                )
                .into_any_element(),
            (None, None) => page_body().child(page_loading()).into_any_element(),
        };
        let column = v_flex()
            .id("tag-page")
            .size_full()
            .min_w_0()
            .child(header)
            .child(body);
        self.pane
            .layout(&self.list, CatalogBooks::book_at, column, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use gpui_kit::component::list::ListDelegate as _;
    use gpui_kit::{TestAppContext, VisualTestContext};
    use hardcover_core::BookId;
    use hardcover_core::store::LibraryCache;
    use hardcover_core::tags::{Mood, browse_genre};

    use super::*;
    use crate::catalog::navigation::{NavigationEvent, Navigator};
    use crate::session::SessionScope;

    fn settle(cx: &mut VisualTestContext) {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    fn genre(name: &str) -> Tag {
        Tag::Genre(browse_genre(name).unwrap())
    }

    /// The page, and the pages it asked the workspace to keep in its stead.
    fn tag_view(
        cx: &mut TestAppContext,
    ) -> (
        Entity<TagView>,
        Rc<RefCell<Vec<Page>>>,
        &mut VisualTestContext,
    ) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        let replaced: Rc<RefCell<Vec<Page>>> = Rc::default();
        cx.update(|cx| {
            gpui_kit::init(cx);
            session.install(cx);
            Catalog::init(&session, cx);
            let navigator = Navigator::init(cx);
            let replaced = replaced.clone();
            cx.subscribe(&navigator, move |_, event: &NavigationEvent, _| {
                if let NavigationEvent::Replace(page) = event {
                    replaced.borrow_mut().push(page.clone());
                }
            })
            .detach();
        });
        let model = cx.new(|_| LibraryModel::new(session));
        let (view, cx) = cx.add_window_view(|window, cx| TagView::new(model, window, cx));
        (view, replaced, cx)
    }

    fn show(view: &Entity<TagView>, tag: Tag, within: Option<Tag>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| view.update(cx, |v, cx| v.show(tag, within, window, cx)));
        settle(cx);
    }

    fn books(view: &Entity<TagView>, cx: &mut VisualTestContext) -> Vec<BookId> {
        view.read_with(cx, |view, cx| {
            let list = view.list.read(cx).delegate();
            (0..list.items_count(0, cx))
                .filter_map(|ix| list.book_at(ix).map(|b| b.id))
                .collect()
        })
    }

    #[gpui_kit::test]
    fn a_mood_page_lists_the_books_with_their_votes(cx: &mut TestAppContext) {
        let (view, _, cx) = tag_view(cx);
        show(&view, Tag::Mood(Mood::Adventurous), None, cx);
        let shown = books(&view, cx);
        assert!(shown.len() > 3, "the demo's adventurous books: {shown:?}");
        let line = view.read_with(cx, |view, cx| view.page(cx).unwrap().items[0].votes);
        assert!(line > 0, "ranked by votes");
    }

    #[gpui_kit::test]
    fn narrowing_keeps_the_choice_with_the_page(cx: &mut TestAppContext) {
        let (view, replaced, cx) = tag_view(cx);
        let mood = Tag::Mood(Mood::Adventurous);
        show(&view, mood, None, cx);
        let all = books(&view, cx);

        cx.update(|window, cx| {
            view.update(cx, |v, cx| v.narrow(Some(genre("Fantasy")), window, cx));
        });
        settle(cx);
        let fantasy = books(&view, cx);
        assert!(!fantasy.is_empty(), "the demo's adventurous fantasy");
        assert!(fantasy.len() < all.len(), "narrowed");
        assert!(fantasy.iter().all(|b| all.contains(b)));
        assert_eq!(
            *replaced.borrow(),
            vec![Page::Tag {
                tag: mood,
                within: Some(genre("Fantasy"))
            }],
            "the page stack keeps the choice for Back"
        );

        // Back to the page as it was kept: the select shows the choice,
        // and the rows are the narrowed ones.
        show(&view, Tag::Mood(Mood::Dark), None, cx);
        show(&view, mood, Some(genre("Fantasy")), cx);
        let chosen = view.read_with(cx, |view, cx| {
            view.select.read(cx).selected_value().copied()
        });
        assert_eq!(chosen, Some(Some(genre("Fantasy"))));
        assert_eq!(books(&view, cx), fantasy);
    }

    #[gpui_kit::test]
    fn a_choice_with_no_books_says_so(cx: &mut TestAppContext) {
        let (view, _, cx) = tag_view(cx);
        // Probe the demo for a mood and genre that share no book.
        let mut found = None;
        'search: for mood in Mood::ALL {
            for genre in hardcover_core::tags::BROWSE_GENRES {
                show(&view, Tag::Mood(mood), Some(Tag::Genre(genre)), cx);
                if books(&view, cx).is_empty() {
                    found = Some((mood, genre));
                    break 'search;
                }
            }
        }
        let (mood, genre) = found.expect("some choice has no demo books");
        let (loaded, empty) = view.read_with(cx, |view, cx| {
            (
                view.page(cx).is_some(),
                view.list.read(cx).delegate().empty_text().clone(),
            )
        });
        assert!(loaded, "loaded, not loading");
        assert_eq!(
            empty.as_ref(),
            format!(
                "Hardcover has no {} books readers call {}.",
                genre.name,
                mood.name().to_lowercase()
            )
        );
    }
}
