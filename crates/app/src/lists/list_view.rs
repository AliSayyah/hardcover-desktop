//! A list's page: its name, details and books in the list's order. Enter or a
//! click opens a book; Delete or a row's remove button takes it off the list.
//! Books load a page at a time as the list scrolls. In a wide window the
//! selected book shows in the book pane beside the list, and a click only
//! selects (`catalog::book_list::ListPane`).
//!
//! The header has the list's name as the page title with Edit… and Delete…
//! on its trailing edge, the facts, the description (the reader's own words,
//! in `reader`) and "Open on Hardcover"; the books are catalogue rows in the
//! catalogue pages' frame (`catalog::book_list`). The list is a tab stop
//! after the header's controls; each row's remove button follows it.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::ScrollHandle;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::list::{ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, IconName, IndexPath, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LISTS};
use hardcover_core::{ListBook, ListBookId, ListId, OffsetPage, UserList};

use super::list_dialog::open_edit_list;
use super::model::{ListsEvent, ListsModel};
use crate::RemoveFromList;
use crate::app_state::Services;
use crate::bidi::WrappingText;
use crate::catalog::book_list::{
    ListPane, follow_pane, load_failed, page_body, page_header_scrolling, page_list, page_loading,
    reset_scroll, row_details, select_on_click,
};
use crate::catalog::navigation::{self, Page};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{authors_text, needs_permission};
use crate::library::book_row::BookRow;
use crate::library::model::LibraryModel;
use crate::ui::kit::{SmallControl as _, SmallIconButton as _};
use crate::ui::kit::{open_alert_dialog, row_action};
use crate::ui::{
    ExternalLink, ListRow as _, RowFocus, empty_state, empty_state_with_action, on_keyboard_entry,
    select_first_row,
};

/// Key context of a list's page, where Delete takes the selected book off.
pub const KEY_CONTEXT: &str = "ListPage";

/// The books of one list loaded so far.
#[derive(Default)]
struct BooksPage {
    books: Vec<ListBook>,
    /// Hardcover has more rows after the ones fetched.
    more: bool,
    /// Where the next page starts: the rows Hardcover has sent, less the
    /// ones taken off since. Rows whose book didn't resolve count too, so
    /// this can be more than `books.len()`.
    next_offset: u32,
    loading: bool,
    error: Option<SharedString>,
}

impl BooksPage {
    /// Showing the list should fetch: nothing loaded yet, or the last fetch
    /// failed. (`fetch` itself stops at the end of the list.)
    fn needs_fetch(&self) -> bool {
        self.books.is_empty() || self.error.is_some()
    }

    /// Adds a fetched page.
    fn receive(&mut self, page: OffsetPage<ListBook>) {
        self.more = page.has_more;
        self.next_offset = page.next_offset;
        self.books.extend(page.items);
    }

    /// Takes one row off. True if a fetch in flight must start again: it
    /// asked from an offset that now skips a book.
    fn remove(&mut self, row: ListBookId) -> bool {
        let before = self.books.len();
        self.books.retain(|b| b.id != row);
        let removed = self.books.len() != before;
        if removed {
            // The rows after it moved up one on Hardcover.
            self.next_offset = self.next_offset.saturating_sub(1);
        }
        let restart = self.loading && removed;
        if restart {
            self.loading = false;
        }
        restart
    }
}

pub struct ListBooks {
    library: Entity<LibraryModel>,
    lists: Entity<ListsModel>,
    current: Option<ListId>,
    /// Lists opened this session, so going back to one costs no request.
    pages: HashMap<ListId, BooksPage>,
    fetch: Option<Task<()>>,
    focus: RowFocus,
    /// The book pane is beside the list ([`ListPane`]).
    beside: Rc<Cell<bool>>,
}

impl ListBooks {
    /// The book in row `ix`, for the [`ListPane`].
    fn book_at(&self, ix: usize) -> Option<hardcover_core::BookSummary> {
        self.book(ix).map(|b| b.book.clone())
    }

    fn page(&self) -> Option<&BooksPage> {
        self.pages.get(&self.current?)
    }

    fn book(&self, ix: usize) -> Option<&ListBook> {
        self.page()?.books.get(ix)
    }

    /// Fetches the current list's next page, unless one is on its way.
    fn fetch(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(id) = self.current else { return };
        let page = self.pages.entry(id).or_insert_with(|| BooksPage {
            more: true,
            ..BooksPage::default()
        });
        if page.loading || !page.more {
            return;
        }
        let offset = page.next_offset;
        page.loading = true;
        page.error = None;
        let client = Services::global(cx).client().clone();
        cx.notify();
        // Replacing the task drops a fetch for a list no longer shown.
        self.fetch = Some(cx.spawn_in(window, async move |state, cx| {
            let result = cx
                .background_spawn(async move { client.list_books(id, offset) })
                .await;
            let _ = state.update_in(cx, |state, _, cx| {
                let message = match &result {
                    Err(e) => Some(Services::error_message(e, Some(READ_LISTS), cx).into()),
                    Ok(_) => None,
                };
                let delegate = state.delegate_mut();
                let Some(page) = delegate.pages.get_mut(&id) else {
                    return;
                };
                page.loading = false;
                match result {
                    Ok(books) => page.receive(books),
                    Err(_) => page.error = message,
                }
                cx.notify();
            });
        }));
    }

    fn show(&mut self, id: ListId, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        if self.current != Some(id) {
            // A fetch still running for another list is dropped; that list
            // loads again from where it stopped.
            if let Some(page) = self.current.and_then(|c| self.pages.get_mut(&c)) {
                page.loading = false;
            }
            self.fetch = None;
            self.current = Some(id);
        }
        if self.page().is_none_or(BooksPage::needs_fetch) {
            self.fetch(window, cx);
        }
    }

    /// Fetches again after a failure: the first page, or the next one.
    fn retry(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let Some(page) = self.current.and_then(|id| self.pages.get_mut(&id)) {
            page.error = None;
        }
        self.fetch(window, cx);
    }

    /// Forgets every list's books (Refresh), loading the one shown again.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.pages.clear();
        self.fetch = None;
        self.fetch(window, cx);
    }

    /// Forgets a list's books, so it loads again when shown.
    fn forget(&mut self, id: ListId) {
        self.pages.remove(&id);
        if self.current == Some(id) {
            self.fetch = None;
        }
    }

    fn removed(
        &mut self,
        list: ListId,
        row: ListBookId,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        let restart = self.pages.get_mut(&list).is_some_and(|p| p.remove(row));
        if restart && self.current == Some(list) {
            self.fetch = None;
            self.fetch(window, cx);
        }
    }

    /// Takes the book at `ix` off the list shown.
    fn remove(&mut self, ix: usize, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let (Some(list), Some(book)) = (self.current, self.book(ix).cloned()) else {
            return;
        };
        remove_book(&self.lists, list, &book, window, cx);
    }
}

/// Takes one row off `list`, with a notification if that fails.
fn remove_book(
    lists: &Entity<ListsModel>,
    list: ListId,
    book: &ListBook,
    window: &mut Window,
    cx: &mut App,
) {
    let (book_id, row) = (book.book.id, book.id);
    let task = lists.update(cx, |m, cx| m.remove_rows(list, book_id, vec![row], cx));
    window
        .spawn(cx, async move |cx| {
            if let Err(message) = task.await {
                let _ = cx.update(|window, cx| {
                    window.push_notification(
                        Notification::error(message).title("Couldn't remove the book"),
                        cx,
                    );
                });
            }
        })
        .detach();
}

impl ListDelegate for ListBooks {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        let rows = self.page().map_or(0, |p| p.books.len());
        self.focus.stop_when_rows(rows);
        rows
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let focused = self.focus.shows_focus(ix, window);
        let list = self.current?;
        let book = self.book(ix.row)?.clone();
        let lists = self.lists.read(cx);
        let ranked = lists.list(list).is_some_and(|l| l.ranked);
        let busy = lists.is_busy(book.book.id);
        let can_write = Services::global(cx).has_scope(WRITE_LISTS);
        let status = self
            .library
            .read(cx)
            .entry_for_book(book.book.id)
            .map(|e| e.status);
        let mut lead = Vec::new();
        if ranked {
            // Hardcover numbers from 0 or 1 depending on how the list was
            // made; the row's place is what the reader sees.
            lead.push(format!("#{}", ix.row + 1));
        }
        lead.push(authors_text(&book.book.authors));
        let details = row_details(&lead, &book.book);
        let remove_tooltip: SharedString = if can_write {
            "Remove from list".into()
        } else {
            needs_permission(WRITE_LISTS).into()
        };
        let lists = self.lists.clone();
        let item = ListItem::new(("list-book", book.id.get().cast_unsigned()))
            .list_row(focused, cx)
            .accessibility_label(book.book.title.clone())
            .child(BookRow::for_catalog(&book.book, details, status))
            .suffix(move |_, _| {
                let (lists, book) = (lists.clone(), book.clone());
                row_action(
                    Button::new(("remove-list-book", book.id.get().cast_unsigned()))
                        .ghost()
                        .small_icon_button()
                        .icon(IconName::Close)
                        .tooltip(remove_tooltip.clone())
                        .disabled(busy || !can_write)
                        .on_click(move |_, window, cx| {
                            remove_book(&lists, list, &book, window, cx);
                        }),
                )
            });
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
        match self.page().and_then(|p| p.error.clone()) {
            Some(message) => empty_state_with_action(
                IconName::CircleAlert,
                "Couldn't load this list".into(),
                message,
                Button::new("retry-list-empty")
                    .small_control()
                    .label("Try again")
                    .on_click(cx.listener(|state, _, window, cx| {
                        state.delegate_mut().retry(window, cx);
                    })),
            )
            .into_any_element(),
            None => empty_state(
                IconName::BookOpen,
                "No books on this list".into(),
                "Add one from a book's page with Lists.".into(),
            )
            .into_any_element(),
        }
    }

    fn loading(&self, _: &App) -> bool {
        self.page().is_none_or(|p| p.loading && p.books.is_empty())
    }

    fn render_loading(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        page_loading()
    }

    fn has_more(&self, _: &App) -> bool {
        self.page().is_some_and(|p| p.more && p.error.is_none())
    }

    fn load_more_threshold(&self) -> usize {
        20
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.fetch(window, cx);
    }
}

pub struct ListView {
    lists: Entity<ListsModel>,
    library: Entity<LibraryModel>,
    list: Entity<ListState<ListBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ListView {
    pub fn new(
        lists: Entity<ListsModel>,
        library: Entity<LibraryModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pane = ListPane::new(library.clone(), window, cx);
        let list = cx.new(|cx| {
            ListState::new(
                ListBooks {
                    library: library.clone(),
                    lists: lists.clone(),
                    current: None,
                    pages: HashMap::new(),
                    fetch: None,
                    focus: RowFocus::default(),
                    beside: pane.beside().clone(),
                },
                window,
                cx,
            )
        });
        RowFocus::attach(&list, |d| &mut d.focus, cx);
        let subscriptions = vec![
            follow_pane(&list, &pane, ListBooks::book_at, window, cx),
            cx.observe(&lists, |_, _, cx| cx.notify()),
            // Status tags follow library changes.
            cx.observe(&library, {
                let list = list.clone();
                move |_, _, cx| list.update(cx, |_, cx| cx.notify())
            }),
            cx.subscribe_in(&lists, window, |this, _, event: &ListsEvent, window, cx| {
                this.list.update(cx, |state, cx| {
                    let delegate = state.delegate_mut();
                    match event {
                        ListsEvent::BookAdded(id) => {
                            delegate.forget(*id);
                            if delegate.current == Some(*id) {
                                delegate.fetch(window, cx);
                            }
                        }
                        ListsEvent::BookRemoved { list, row } => {
                            delegate.removed(*list, *row, window, cx);
                        }
                        ListsEvent::Deleted(id) => delegate.forget(*id),
                        ListsEvent::Reloaded => delegate.reload(window, cx),
                    }
                    cx.notify();
                });
            }),
            on_keyboard_entry(&list, window, cx, |this, window, cx| {
                select_first_row(&this.list, window, cx);
            }),
            cx.subscribe(&list, |_, list, event: &ListEvent, cx| match event {
                ListEvent::Confirm(ix) => {
                    let book = list.read(cx).delegate().book(ix.row).cloned();
                    if let Some(book) = book {
                        navigation::open(Page::Book(book.book), cx);
                    }
                }
                ListEvent::Select(_) | ListEvent::Cancel => {}
            }),
        ];
        Self {
            lists,
            library,
            list,
            pane,
            header_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The list shown, if any.
    pub fn current(&self, cx: &App) -> Option<ListId> {
        self.list.read(cx).delegate().current
    }

    pub fn show(&mut self, id: ListId, window: &mut Window, cx: &mut Context<Self>) {
        let header_scroll = self.header_scroll.clone();
        self.list.update(cx, |state, cx| {
            let changed = state.delegate().current != Some(id);
            state.delegate_mut().show(id, window, cx);
            if changed {
                reset_scroll(&header_scroll);
                state.set_selected_index(None, window, cx);
                state.scroll_to_item(IndexPath::new(0), gpui_kit::ScrollStrategy::Top, window, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    /// Fetches the book in the pane beside the list again where it is stale
    /// ([`crate::freshness`]). The list's own books follow Refresh
    /// (`ListsModel::reload`).
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.pane.revalidate(shown_again, cx);
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::enter_list(&self.list, window, cx);
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list
            .update(cx, |state, cx| state.delegate_mut().retry(window, cx));
    }

    fn remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.list.read(cx).selected_index() else {
            return;
        };
        self.list.update(cx, |state, cx| {
            state.delegate_mut().remove(ix.row, window, cx);
        });
    }

    fn confirm_delete(&mut self, list: &UserList, window: &mut Window, cx: &mut Context<Self>) {
        let title: SharedString = format!("Delete “{}”?", list.name).into();
        let lists = self.lists.clone();
        let id = list.id;
        open_alert_dialog(window, cx, move |alert, _, _| {
            let lists = lists.clone();
            alert
                .title(title.clone())
                .description("The list is deleted from Hardcover. Its books stay in your library.")
                .confirm()
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let task = lists.update(cx, |m, cx| m.delete(id, cx));
                    window
                        .spawn(cx, async move |cx| {
                            if let Err(message) = task.await {
                                let _ = cx.update(|window, cx| {
                                    window.push_notification(
                                        Notification::error(message)
                                            .title("Couldn't delete the list"),
                                        cx,
                                    );
                                });
                            }
                        })
                        .detach();
                    true
                })
        });
    }

    fn list_actions(&self, list: &UserList, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        let can_write = Services::global(cx).has_scope(WRITE_LISTS);
        let writing = self.lists.read(cx).is_writing();
        let permission: Option<SharedString> =
            (!can_write).then(|| needs_permission(WRITE_LISTS).into());
        (!list.is_owned()).then(|| {
            let edit_lists = self.lists.clone();
            let id = list.id;
            let delete_list = list.clone();
            h_flex()
                .flex_shrink_0()
                .gap_2()
                .child(
                    Button::new("edit-list")
                        .small_control()
                        .label("Edit…")
                        .disabled(!can_write || writing)
                        .when_some(permission.clone(), Button::tooltip)
                        .on_click(move |_, window, cx| {
                            open_edit_list(edit_lists.clone(), id, window, cx);
                        }),
                )
                .child(
                    Button::new("delete-list")
                        .danger()
                        .small_control()
                        .label("Delete…")
                        .disabled(!can_write || writing)
                        .when_some(permission, Button::tooltip)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_delete(&delete_list, window, cx);
                        })),
                )
        })
    }

    fn render_header(&self, list: &UserList, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        let mut facts = vec![crate::format::books_count_text(list.books_count)];
        if list.ranked {
            facts.push("Ranked".into());
        }
        if let Some(privacy) = list.privacy() {
            facts.push(privacy.label().into());
        }
        let url = self
            .library
            .read(cx)
            .user()
            .and_then(|u| list.web_url(&u.username));
        let error = self
            .list
            .read(cx)
            .delegate()
            .page()
            .and_then(|p| p.error.clone())
            .filter(|_| {
                // An empty list shows its error in place of the books.
                self.list
                    .read(cx)
                    .delegate()
                    .page()
                    .is_some_and(|p| !p.books.is_empty())
            });
        let actions = self.list_actions(list, cx);
        v_flex()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_start()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .min_w_0()
                                    .page_title()
                                    .child(WrappingText::new(list.name.clone())),
                            )
                            .children(actions),
                    )
                    .child(
                        div()
                            .body()
                            .tabular()
                            .text_color(muted)
                            .child(facts.join(" · ")),
                    )
                    .when(list.is_owned(), |this| {
                        this.child(div().body().text_color(muted).child(
                            "The editions you own. Mark a book owned, or not, from its page.",
                        ))
                    }),
            )
            .when_some(list.description.clone(), |this, text| {
                this.child(
                    div()
                        .max_w(size::PROSE)
                        .reader()
                        .child(WrappingText::new(text).line_clamp(3)),
                )
            })
            .when_some(url, |this, url| {
                this.child(ExternalLink::new(
                    "list-on-hardcover",
                    "Open on Hardcover",
                    url,
                ))
            })
            .when_some(error, |this, message| {
                this.child(load_failed(
                    message,
                    Button::new("retry-list")
                        .small_control()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
                    cx,
                ))
            })
    }
}

impl Render for ListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self
            .current(cx)
            .and_then(|id| self.lists.read(cx).list(id).cloned());
        let column = v_flex()
            .id("list-page")
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &RemoveFromList, window, cx| {
                this.remove_selected(window, cx);
            }))
            .size_full()
            .min_w_0()
            .when_some(list, |this, list| {
                this.child(page_header_scrolling(
                    "list-page-header",
                    &self.header_scroll,
                    self.render_header(&list, cx),
                ))
                .child(page_body().child(page_list(&self.list)))
            });
        self.pane
            .layout(&self.list, ListBooks::book_at, column, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{BookId, BookSummary};

    fn book(row: i64) -> ListBook {
        ListBook::new(
            ListBookId::new(row),
            BookSummary::new(BookId::new(row + 1000), format!("Book {row}")),
        )
    }

    #[test]
    fn a_failed_or_empty_page_fetches_when_shown() {
        let mut page = BooksPage::default();
        assert!(page.needs_fetch());
        page.books.push(book(1));
        assert!(!page.needs_fetch());
        page.error = Some("offline".into());
        assert!(page.needs_fetch());
    }

    /// D7: a row whose book didn't resolve neither ends the list nor
    /// shifts the next page, and a removal moves the next page back one.
    #[test]
    fn the_next_page_starts_after_every_row_hardcover_sent() {
        let mut page = BooksPage::default();
        // 100 rows sent, one of them unresolved.
        let mut first = OffsetPage::default();
        first.items = (1..=99).map(book).collect();
        first.next_offset = 100;
        first.has_more = true;
        page.receive(first);
        assert_eq!((page.books.len(), page.next_offset), (99, 100));
        assert!(page.more);
        // Taking a loaded row off moves the rest up one on Hardcover.
        page.remove(ListBookId::new(5));
        assert_eq!(page.next_offset, 99);
        // A row not loaded here changes nothing.
        page.remove(ListBookId::new(500));
        assert_eq!(page.next_offset, 99);
    }

    #[test]
    fn removing_a_row_restarts_a_fetch_in_flight() {
        let mut page = BooksPage {
            books: vec![book(1), book(2)],
            more: true,
            next_offset: 2,
            loading: false,
            error: None,
        };
        assert!(!page.remove(ListBookId::new(1)));
        assert_eq!(page.books.len(), 1);

        page.loading = true;
        // A row not on this page changes no offset.
        assert!(!page.remove(ListBookId::new(9)));
        assert!(page.loading);
        assert!(page.remove(ListBookId::new(2)));
        assert_eq!(page.books, []);
        assert!(!page.loading);
    }
}
