//! A vibe's page (`hardcover-design`, A vibe's page): "For you", "Top
//! picks" or a vibe Hardcover features, over Discover.
//!
//! - **Header:** the vibe's name ([`super::vibe_name`]), the muted intro, and
//!   the facts line, "250 books · Updated today" ([`super::facts_line`]).
//!   From the eighth day the line says why it is old and "Open on
//!   Hardcover" shows on the title's line: a visit there is what rebuilds
//!   the list.
//! - **Rows:** catalogue rows in Hardcover's order, the author muted and a
//!   book in the library with its StatusChip. They load a page of ids at a
//!   time as the list scrolls (`resource::Catalog::vibe_books`). In a wide
//!   window the selected book shows in the pane beside the list, and a
//!   click only selects (`catalog::book_list::ListPane`).
//! - **Context menu:** "Open", "Want to read" for a book not in the
//!   library, and, on the user's own Recommendations and Top Picks,
//!   "Never recommend…" (disabled without `write:vibes`, with the reason).
//!   The keyboard reaches the same command through the book's "more" menu,
//!   in the pane or on the book page opened from here. Once Hardcover
//!   accepts it, the row fades out (`ui::leaving`): the book is hidden from
//!   every vibe built before (`VibeExclusion`).
//! - **Not built yet:** the empty state, with "Open on Hardcover".

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::list::{ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{IconName, IndexPath, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::{WRITE_LIBRARY, WRITE_VIBES};
use hardcover_core::{BookId, BookSummary, ReadingStatus, Vibe, VibeId};

use super::model::VibesModel;
use super::{confirm_never_recommend, days_since, facts_line, is_stale, vibe_intro, vibe_name};
use crate::catalog::book_list::{
    ListPane, demand, follow_pane, page_body, page_header_scrolling, page_list, page_loading,
    reset_scroll, select_on_click,
};
use crate::catalog::navigation::{self, Page};
use crate::chip::status_icon;
use crate::format::{authors_text, needs_permission};
use crate::library::book_row::BookRow;
use crate::library::model::LibraryModel;
use crate::library::writes;
use crate::preferences::AppPreferences;
use crate::resource::catalog::{VibeBooks, VibeBooksKey};
use crate::resource::{Catalog, Demand, LoadStatus, SharedChanged};
use crate::session;
use crate::ui::leaving::{Leaving, ROW_LEAVE};
use crate::ui::{
    EmptyState, ExternalLink, ListRow as _, LoadFailed, PageHeader, RowFocus, empty_state,
    on_keyboard_entry, select_first_row,
};

/// The shown vibe's books as the catalogue holds them, less the books left
/// out ([`VibesModel::shown_books`]), copied when either changes so the
/// rows read them without the app.
#[derive(Default)]
struct BooksShown {
    key: Option<VibeBooksKey>,
    books: Vec<BookSummary>,
    /// The first page is in.
    loaded: bool,
    has_more: bool,
    loading: bool,
    /// The first page failed.
    error: Option<SharedString>,
    /// A later page failed.
    more_error: Option<SharedString>,
}

/// What a row's context menu needs of the vibe shown.
#[derive(Clone, Copy)]
struct RowVibe {
    id: VibeId,
    /// The user's own Recommendations or Top Picks: "Never recommend…".
    system: bool,
}

pub struct VibeRows {
    library: Entity<LibraryModel>,
    catalog: Option<Catalog>,
    vibes: Option<Entity<VibesModel>>,
    current: Option<VibeId>,
    shown: BooksShown,
    /// Rows fading out as they leave the list (`ui::leaving`).
    leaving: Leaving<BookId>,
    /// While a row fades out: the rows shown, it included.
    fading: Option<Vec<BookSummary>>,
    focus: RowFocus,
    /// The book pane is beside the list ([`ListPane`]).
    beside: Rc<Cell<bool>>,
}

impl VibeRows {
    fn books(&self) -> &[BookSummary] {
        self.fading.as_deref().unwrap_or(&self.shown.books)
    }

    /// The book in row `ix`, for the [`ListPane`].
    fn book_at(&self, ix: usize) -> Option<BookSummary> {
        self.books().get(ix).cloned()
    }

    fn vibe<'a>(&self, cx: &'a App) -> Option<&'a Vibe> {
        let id = self.current?;
        self.vibes.as_ref()?.read(cx).vibe(id)
    }

    /// Copies the shown vibe's books from the catalogue, less the books
    /// left out. A book that left the list shown fades out in its place
    /// first.
    fn follow_store(&mut self, cx: &mut Context<ListState<Self>>) {
        let old_fading = self.fading.take();
        let previous = std::mem::take(&mut self.shown);
        let old: Vec<BookSummary> = old_fading.unwrap_or_else(|| previous.books.clone());
        if let (Some(vibe), Some(catalog)) = (self.vibe(cx).cloned(), &self.catalog) {
            let key = VibeBooksKey {
                vibe: vibe.id,
                built: vibe.built_at,
            };
            let shown: HashSet<BookId> = VibesModel::shown_books(&vibe, cx).into_iter().collect();
            let store = catalog.vibe_books.read(cx).resource();
            let value: Option<&VibeBooks> = store.value(&key).map(|v| &**v);
            self.shown = BooksShown {
                key: Some(key),
                books: value.map_or_else(Vec::new, |v| {
                    v.books
                        .items
                        .iter()
                        .filter(|b| shown.contains(&b.id))
                        .cloned()
                        .collect()
                }),
                loaded: value.is_some(),
                has_more: value.is_some_and(|v| v.books.has_more),
                loading: store.is_loading(&key),
                error: store.error(&key).cloned(),
                more_error: store.more_error(&key).cloned(),
            };
        }
        let animate = !cx.reduce_motion()
            && previous.key.is_some()
            && previous.key == self.shown.key
            && previous.loaded
            && self.shown.loaded;
        let new = &self.shown.books;
        let rows = self.leaving.rows(
            &old.iter().map(|b| b.id).collect::<Vec<_>>(),
            new.iter().map(|b| b.id).collect(),
            animate,
            cx.background_executor().now(),
        );
        self.fading = (rows.rows.len() != new.len()).then(|| {
            rows.rows
                .iter()
                .filter_map(|id| {
                    new.iter()
                        .find(|b| b.id == *id)
                        .or_else(|| old.iter().find(|b| b.id == *id))
                        .cloned()
                })
                .collect()
        });
        if rows.started {
            cx.spawn(async move |state, cx| {
                cx.background_executor().timer(ROW_LEAVE).await;
                state
                    .update(cx, |state, cx| {
                        state.delegate_mut().follow_store(cx);
                        cx.notify();
                    })
                    .ok();
            })
            .detach();
        }
    }

    /// Asks for the vibe, and for its books once it is built.
    fn request(&self, demand: Demand, cx: &mut App) {
        let (Some(id), Some(vibes)) = (self.current, &self.vibes) else {
            return;
        };
        vibes.update(cx, |m, cx| m.request_vibe(id, demand, cx));
        self.request_books(demand, cx);
    }

    fn request_books(&self, demand: Demand, cx: &mut App) {
        let Some(catalog) = &self.catalog else {
            return;
        };
        let Some(vibe) = self.vibe(cx).filter(|v| v.is_built()) else {
            return;
        };
        let key = VibeBooksKey {
            vibe: vibe.id,
            built: vibe.built_at,
        };
        let ids = vibe.book_ids.clone();
        catalog.request_vibe_books(key, ids, demand, cx);
    }

    /// Fetches again after a failure: the vibe, its first page, or the next.
    fn retry(&mut self, cx: &mut Context<ListState<Self>>) {
        match (self.shown.loaded, self.shown.key, &self.catalog) {
            (true, Some(key), Some(catalog)) => catalog.more_vibe_books(key, cx),
            _ => self.request(Demand::Now, cx),
        }
    }

    fn row_vibe(&self, cx: &App) -> Option<RowVibe> {
        self.vibe(cx).map(|v| RowVibe {
            id: v.id,
            system: v.kind.is_system(),
        })
    }
}

/// A row's context menu: Open, Want to read for a book not in the library,
/// and Never recommend… on the user's own vibes. Every command has another
/// path: Enter opens the book, and its page's "more" menu has the rest.
fn row_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    book: &BookSummary,
    status: Option<ReadingStatus>,
    vibe: RowVibe,
    library: &Entity<LibraryModel>,
    cx: &App,
) -> gpui_kit::component::menu::PopupMenu {
    let open = book.clone();
    let mut menu = menu.item(
        PopupMenuItem::new("Open")
            .icon(IconName::BookOpen)
            .on_click(move |_, _, cx| navigation::open(Page::Book(open.clone()), cx)),
    );
    if status.is_none() {
        let (library, id) = (library.clone(), book.id);
        menu = menu.item(
            PopupMenuItem::new("Want to read")
                .icon(status_icon(ReadingStatus::WantToRead))
                .disabled(!session::has_scope(cx, WRITE_LIBRARY))
                .on_click(move |_, window, cx| {
                    writes::add_to_library(&library, id, ReadingStatus::WantToRead, window, cx);
                }),
        );
    }
    if !vibe.system {
        return menu;
    }
    let can_write = session::has_scope(cx, WRITE_VIBES);
    let writing = VibesModel::global(cx).is_some_and(|m| m.read(cx).is_excluding(vibe.id, book.id));
    let (id, title) = (book.id, book.title.clone());
    menu.separator()
        .item(
            PopupMenuItem::new("Never recommend…")
                .icon(IconName::ThumbsDown)
                .disabled(!can_write || writing)
                .on_click(move |_, window, cx| {
                    confirm_never_recommend(vibe.id, id, &title, window, cx);
                }),
        )
        .when(!can_write, |menu| {
            menu.item(crate::book::detail_view::reason_row(needs_permission(
                WRITE_VIBES,
            )))
        })
}

impl ListDelegate for VibeRows {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        let rows = self.books().len();
        self.focus.stop_when_rows(rows);
        rows
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let vibe = self.row_vibe(cx)?;
        let book = self.books().get(ix.row)?.clone();
        let opacity = self
            .leaving
            .opacity(&book.id, cx.background_executor().now());
        // The ring is drawn outside the row's opacity: a leaving row has none.
        let focused = opacity.is_none() && self.focus.shows_focus(ix, window);
        let status = self
            .library
            .read(cx)
            .entry_for_book(book.id)
            .map(|e| e.status);
        let library = self.library.clone();
        let menu_book = book.clone();
        let row = div()
            .id(("vibe-book-row", book.id.get().cast_unsigned()))
            .w_full()
            .min_w_0()
            .child(BookRow::for_catalog(
                &book,
                authors_text(&book.authors),
                status,
            ));
        // A leaving row takes no more commands.
        let row = if opacity.is_none() {
            row.context_menu(move |menu, _, cx| {
                row_menu(menu, &menu_book, status, vibe, &library, cx)
            })
            .into_any_element()
        } else {
            row.into_any_element()
        };
        let item = ListItem::new(("vibe-book", book.id.get().cast_unsigned()))
            .list_row(focused, cx)
            .accessibility_label(book.title.clone())
            .child(row);
        let item = match opacity {
            Some(opacity) => {
                window.request_animation_frame();
                item.opacity(opacity)
            }
            None => item,
        };
        Some(select_on_click(item, ix, &self.beside, cx))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.focus.select(ix);
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        match self.shown.error.clone() {
            Some(message) => LoadFailed::new("retry-vibe-books-empty", message)
                .region("Couldn't load this list")
                .on_retry(cx.listener(|state, _, _, cx| state.delegate_mut().retry(cx)))
                .into_any_element(),
            // Built, but every book on it is one the reader left out.
            None => empty_state(
                IconName::BookOpen,
                "No books to show".into(),
                "Hardcover updates this list when you open it on hardcover.app.".into(),
            )
            .into_any_element(),
        }
    }

    fn loading(&self, _: &App) -> bool {
        !self.shown.loaded && self.shown.error.is_none()
    }

    fn render_loading(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        page_loading()
    }

    fn has_more(&self, _: &App) -> bool {
        self.shown.more_error.is_none() && self.shown.has_more
    }

    fn load_more_threshold(&self) -> usize {
        10
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let (Some(key), Some(catalog)) = (self.shown.key, &self.catalog)
            && !self.shown.loading
        {
            catalog.more_vibe_books(key, cx);
        }
    }
}

pub struct VibeView {
    list: Entity<ListState<VibeRows>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
    /// Holds focus while the vibe loads, so shortcuts keep working.
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for VibeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl VibeView {
    pub fn new(library: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = ListPane::new(library.clone(), window, cx);
        let catalog = Catalog::global(cx);
        let vibes = VibesModel::global(cx);
        let list = cx.new(|cx| {
            ListState::new(
                VibeRows {
                    library: library.clone(),
                    catalog: catalog.clone(),
                    vibes: vibes.clone(),
                    current: None,
                    shown: BooksShown::default(),
                    leaving: Leaving::default(),
                    fading: None,
                    focus: RowFocus::default(),
                    beside: pane.beside().clone(),
                },
                window,
                cx,
            )
        });
        RowFocus::attach(&list, |d| &mut d.focus, cx);
        let follow = |list: &Entity<ListState<VibeRows>>| {
            let list = list.clone();
            move |_: &mut Self, cx: &mut Context<Self>| {
                list.update(cx, |state, cx| {
                    state.delegate_mut().follow_store(cx);
                    cx.notify();
                });
                cx.notify();
            }
        };
        let mut subscriptions = Vec::new();
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe(&catalog.vibe_books, {
                let follow = follow(&list);
                let list = list.clone();
                move |this, _, SharedChanged(key): &SharedChanged<VibeBooksKey>, cx| {
                    if list.read(cx).delegate().shown.key == Some(*key) {
                        follow(this, cx);
                    }
                }
            }));
        }
        if let Some(vibes) = &vibes {
            // The vibe landed (or was built again): its books follow.
            subscriptions.push(cx.observe(vibes, {
                let follow = follow(&list);
                let list = list.clone();
                move |this, _, cx| {
                    list.update(cx, |state, cx| {
                        state.delegate().request_books(Demand::Active, cx);
                    });
                    follow(this, cx);
                }
            }));
        }
        let preferences = AppPreferences::global(cx).clone();
        subscriptions.extend([
            // A book left out leaves the list.
            cx.observe(&preferences, {
                let follow = follow(&list);
                move |this, _, cx| follow(this, cx)
            }),
            session::observe_grant(cx, |_, cx| cx.notify()),
            follow_pane(&list, &pane, VibeRows::book_at, window, cx),
            // Status tags follow library changes.
            cx.observe(&library, {
                let list = list.clone();
                move |_, _, cx| list.update(cx, |_, cx| cx.notify())
            }),
            on_keyboard_entry(&list, window, cx, |this, window, cx| {
                select_first_row(&this.list, window, cx);
            }),
            cx.subscribe(&list, |_, list, event: &ListEvent, cx| match event {
                ListEvent::Confirm(ix) => {
                    let book = list.read(cx).delegate().book_at(ix.row);
                    if let Some(book) = book {
                        navigation::open(Page::Book(book), cx);
                    }
                }
                ListEvent::Select(_) | ListEvent::Cancel => {}
            }),
        ]);
        Self {
            list,
            pane,
            header_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The name of vibe `id` as the page shows it, for a Back label.
    pub fn name(&self, id: VibeId, cx: &App) -> Option<String> {
        VibesModel::global(cx)?.read(cx).vibe(id).map(vibe_name)
    }

    pub fn show(&mut self, id: VibeId, window: &mut Window, cx: &mut Context<Self>) {
        let header_scroll = self.header_scroll.clone();
        self.pane.set_from_vibe(Some(id), cx);
        self.list.update(cx, |state, cx| {
            let changed = state.delegate().current != Some(id);
            state.delegate_mut().current = Some(id);
            state.delegate().request(Demand::Shown, cx);
            state.delegate_mut().follow_store(cx);
            if changed {
                reset_scroll(&header_scroll);
                state.set_selected_index(None, window, cx);
                state.scroll_to_item(IndexPath::new(0), gpui_kit::ScrollStrategy::Top, window, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    /// Asks for the vibe and its books again where they are stale
    /// (`resource`; a failure only when `shown_again`), and for the book in
    /// the pane beside the list.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        let demand = demand(shown_again);
        self.list
            .update(cx, |state, cx| state.delegate().request(demand, cx));
        self.pane.revalidate(shown_again, cx);
    }

    /// The list once it has books, else the page itself.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.read(cx).delegate().books().is_empty() {
            window.focus(&self.focus_handle, cx);
        } else {
            crate::ui::enter_list(&self.list, window, cx);
        }
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.list
            .update(cx, |state, cx| state.delegate_mut().retry(cx));
    }

    fn render_header(&self, vibe: &Vibe, cx: &mut Context<Self>) -> impl IntoElement {
        let today = crate::dates::today();
        let days = vibe.built_at.map(|b| days_since(b, today));
        let shown = VibesModel::shown_books(vibe, cx).len();
        let mut header = PageHeader::new(vibe_name(vibe)).wraps();
        if days.is_some_and(is_stale) {
            header = header.actions(ExternalLink::new(
                "vibe-on-hardcover",
                "Open on Hardcover",
                vibe.web_url(),
            ));
        }
        if let Some(intro) = vibe_intro(vibe) {
            header = header.line(intro);
        }
        if let Some(days) = days.filter(|_| vibe.is_built()) {
            header = header.facts(facts_line(shown, days));
        }
        // An empty list shows a first page's error in place of the books;
        // a later page's shows here.
        let error = self.list.read(cx).delegate().shown.more_error.clone();
        v_flex()
            .gap_4()
            .child(header)
            .when_some(error, |this, message| {
                this.child(
                    LoadFailed::new("retry-vibe-books", message)
                        .on_retry(cx.listener(|this, _, _, cx| this.retry(cx))),
                )
            })
    }

    /// Not built yet: Hardcover builds the list when the reader opens it on
    /// hardcover.app.
    fn render_not_built(vibe: &Vibe) -> impl IntoElement {
        let title = if vibe.kind == hardcover_core::VibeKind::Recommendations {
            "Your recommendations aren't ready"
        } else {
            "This list isn't ready"
        };
        EmptyState::new(crate::icons::sparkles(), title)
            .description(
                "Hardcover builds this list when you open it on hardcover.app. It shows here \
                 after that.",
            )
            .action(ExternalLink::new(
                "build-vibe-on-hardcover",
                "Open on Hardcover",
                vibe.web_url(),
            ))
    }

    fn render_column(&self, cx: &mut Context<Self>) -> AnyElement {
        let delegate = self.list.read(cx).delegate();
        let Some(id) = delegate.current else {
            return div().into_any_element();
        };
        let vibes = VibesModel::global(cx);
        let vibe = vibes.as_ref().and_then(|m| m.read(cx).vibe(id).cloned());
        let Some(vibe) = vibe else {
            let status = vibes.map_or(LoadStatus::Idle, |m| m.read(cx).vibe_status(id));
            return match status {
                LoadStatus::Failed(message) => div()
                    .size_full()
                    .child(
                        LoadFailed::new("retry-vibe", message)
                            .region("Couldn't load this list")
                            .on_retry(cx.listener(|this, _, _, cx| this.retry(cx))),
                    )
                    .into_any_element(),
                _ => v_flex()
                    .size_full()
                    .pt_8()
                    .child(page_loading())
                    .into_any_element(),
            };
        };
        let header = page_header_scrolling(
            "vibe-page-header",
            &self.header_scroll,
            self.render_header(&vibe, cx),
        );
        let body = if vibe.is_built() {
            page_body().child(page_list(&self.list)).into_any_element()
        } else {
            page_body()
                .child(Self::render_not_built(&vibe))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .min_w_0()
            .child(header)
            .child(body)
            .into_any_element()
    }
}

impl Render for VibeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let column = v_flex()
            .id("vibe-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .child(self.render_column(cx));
        self.pane
            .layout(&self.list, VibeRows::book_at, column, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::{TestAppContext, VisualTestContext};
    use hardcover_core::demo::Faults;
    use hardcover_core::prefs::Preferences;
    use hardcover_core::store::LibraryCache;

    use super::*;
    use crate::session::SessionScope;

    /// The demo reader's Recommendations ("For you").
    const FOR_YOU: VibeId = VibeId::new(501);

    fn settle(cx: &mut VisualTestContext) {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    fn vibe_view<'a>(
        faults: &str,
        cx: &'a mut TestAppContext,
    ) -> (
        Entity<VibeView>,
        Entity<VibesModel>,
        &'a mut VisualTestContext,
    ) {
        let faults = Faults::parse(faults).unwrap();
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client_with(&faults), cache);
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let vibes = cx.update(|cx| {
            gpui_kit::init(cx);
            session.install(cx);
            Catalog::init(&session, cx);
            crate::preferences::init(Preferences::default(), None, cx);
            VibesModel::init(session.clone(), &library, cx)
        });
        let (view, cx) = cx.add_window_view(|window, cx| VibeView::new(library, window, cx));
        cx.update(|window, cx| view.update(cx, |v, cx| v.show(FOR_YOU, window, cx)));
        settle(cx);
        (view, vibes, cx)
    }

    fn shown(view: &Entity<VibeView>, cx: &mut VisualTestContext) -> Vec<BookId> {
        view.read_with(cx, |view, cx| {
            view.list
                .read(cx)
                .delegate()
                .books()
                .iter()
                .map(|b| b.id)
                .collect()
        })
    }

    #[gpui_kit::test]
    fn a_book_never_recommended_leaves_the_page(cx: &mut TestAppContext) {
        let (view, vibes, cx) = vibe_view("", cx);
        let before = shown(&view, cx);
        assert_eq!(before.len(), 11, "the demo's eleven recommendations");
        let book = before[2];

        let task = vibes.update(cx, |m, cx| m.exclude(FOR_YOU, book, cx));
        settle(cx);
        let result = gpui_kit::block_on(task);
        assert_eq!(result, Some(Ok(())));
        settle(cx);

        let after = shown(&view, cx);
        assert_eq!(after.len(), 10);
        assert!(!after.contains(&book), "its row left");
        let left_out =
            cx.update(|_, cx| AppPreferences::global(cx).read(cx).vibe_exclusions().len());
        assert_eq!(left_out, 1, "kept until Hardcover builds the list again");
    }

    #[gpui_kit::test]
    fn a_list_not_built_yet_has_no_rows(cx: &mut TestAppContext) {
        let (view, vibes, cx) = vibe_view("vibes=unbuilt", cx);
        let built = vibes.read_with(cx, |m, _| m.vibe(FOR_YOU).map(Vibe::is_built));
        assert_eq!(built, Some(false), "the vibe is there, empty");
        assert!(shown(&view, cx).is_empty());
    }
}
