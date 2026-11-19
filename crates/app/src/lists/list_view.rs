//! A list's page: its name, details and books in the list's order. Enter or a
//! click opens a book; Delete or a row's remove button takes it off the list.
//! Books load a page at a time as the list scrolls.

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::link::Link;
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LISTS};
use hardcover_core::{LIST_BOOKS_PER_PAGE, ListBook, ListBookId, ListId, UserList};

use super::list_dialog::open_edit_list;
use super::model::{ListsEvent, ListsModel};
use crate::RemoveFromList;
use crate::app_state::Services;
use crate::bidi::WrappingText;
use crate::catalog::book_list::row_details;
use crate::catalog::navigation::{self, Page};
use crate::format::{authors_text, needs_permission};
use crate::library::book_row::BookRow;
use crate::library::library_view::{empty_state, empty_state_with_action};
use crate::library::model::LibraryModel;

/// Key context of a list's page, where Delete takes the selected book off.
pub const KEY_CONTEXT: &str = "ListPage";

/// The books of one list loaded so far.
#[derive(Default)]
struct BooksPage {
    books: Vec<ListBook>,
    /// The last page fetched was full, so there may be more.
    more: bool,
    loading: bool,
    error: Option<SharedString>,
}

impl BooksPage {
    /// Showing the list should fetch: nothing loaded yet, or the last fetch
    /// failed. (`fetch` itself stops at the end of the list.)
    fn needs_fetch(&self) -> bool {
        self.books.is_empty() || self.error.is_some()
    }

    /// Takes one row off. True if a fetch in flight must start again: it
    /// asked from an offset that now skips a book.
    fn remove(&mut self, row: ListBookId) -> bool {
        let before = self.books.len();
        self.books.retain(|b| b.id != row);
        let restart = self.loading && self.books.len() != before;
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
    _fetch: Option<Task<()>>,
}

impl ListBooks {
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
        page.loading = true;
        page.error = None;
        let offset = page.books.len() as u32;
        let client = Services::global(cx).client().clone();
        cx.notify();
        // Replacing the task drops a fetch for a list no longer shown.
        self._fetch = Some(cx.spawn_in(window, async move |state, cx| {
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
                    Ok(books) => {
                        page.more = books.len() as u32 >= LIST_BOOKS_PER_PAGE;
                        page.books.extend(books);
                    }
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
            self._fetch = None;
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
        self._fetch = None;
        self.fetch(window, cx);
    }

    /// Forgets a list's books, so it loads again when shown.
    fn forget(&mut self, id: ListId) {
        self.pages.remove(&id);
        if self.current == Some(id) {
            self._fetch = None;
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
            self._fetch = None;
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
        self.page().map_or(0, |p| p.books.len())
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
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
        Some(
            ListItem::new(("list-book", book.id.get() as u64))
                .accessibility_label(book.book.title.clone())
                .child(BookRow::for_catalog(&book.book, details, status))
                .suffix(move |_, _| {
                    let (lists, book) = (lists.clone(), book.clone());
                    Button::new(("remove-list-book", book.id.get() as u64))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip(remove_tooltip.clone())
                        .disabled(busy || !can_write)
                        .on_click(move |_, window, cx| remove_book(&lists, list, &book, window, cx))
                }),
        )
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
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
                    .small()
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
    _subscriptions: Vec<Subscription>,
}

impl ListView {
    pub fn new(
        lists: Entity<ListsModel>,
        library: Entity<LibraryModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = cx.new(|cx| {
            ListState::new(
                ListBooks {
                    library: library.clone(),
                    lists: lists.clone(),
                    current: None,
                    pages: HashMap::new(),
                    _fetch: None,
                },
                window,
                cx,
            )
        });
        let subscriptions = vec![
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
                            delegate.removed(*list, *row, window, cx)
                        }
                        ListsEvent::Deleted(id) => delegate.forget(*id),
                        ListsEvent::Reloaded => delegate.reload(window, cx),
                    }
                    cx.notify();
                });
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
            _subscriptions: subscriptions,
        }
    }

    /// The list shown, if any.
    pub fn current(&self, cx: &App) -> Option<ListId> {
        self.list.read(cx).delegate().current
    }

    pub fn show(&mut self, id: ListId, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |state, cx| {
            let changed = state.delegate().current != Some(id);
            state.delegate_mut().show(id, window, cx);
            if changed {
                state.set_selected_index(None, window, cx);
                state.scroll_to_item(IndexPath::new(0), gpui_kit::ScrollStrategy::Top, window, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| list.focus(window, cx));
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
            state.delegate_mut().remove(ix.row, window, cx)
        });
    }

    fn confirm_delete(&mut self, list: &UserList, window: &mut Window, cx: &mut Context<Self>) {
        let title: SharedString = format!("Delete “{}”?", list.name).into();
        let lists = self.lists.clone();
        let id = list.id;
        window.open_alert_dialog(cx, move |alert, _, _| {
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

    fn render_header(&self, list: &UserList, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let can_write = Services::global(cx).has_scope(WRITE_LISTS);
        let writing = self.lists.read(cx).is_writing();
        let mut facts = vec![match list.books_count {
            1 => "1 book".to_owned(),
            n => format!("{n} books"),
        }];
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
        let permission: Option<SharedString> =
            (!can_write).then(|| needs_permission(WRITE_LISTS).into());
        let actions = (!list.is_owned()).then(|| {
            let edit_lists = self.lists.clone();
            let id = list.id;
            let delete_list = list.clone();
            h_flex()
                .flex_shrink_0()
                .gap_2()
                .child(
                    Button::new("edit-list")
                        .outline()
                        .small()
                        .label("Edit…")
                        .disabled(!can_write || writing)
                        .when_some(permission.clone(), |this, tip| this.tooltip(tip))
                        .on_click(move |_, window, cx| {
                            open_edit_list(edit_lists.clone(), id, window, cx)
                        }),
                )
                .child(
                    Button::new("delete-list")
                        .ghost()
                        .danger()
                        .small()
                        .label("Delete…")
                        .disabled(!can_write || writing)
                        .when_some(permission, |this, tip| this.tooltip(tip))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_delete(&delete_list, window, cx)
                        })),
                )
        });
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
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(WrappingText::new(list.name.clone())),
                    )
                    .children(actions),
            )
            .child(div().text_sm().text_color(muted).child(facts.join(" · ")))
            .when(list.is_owned(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("The editions you own. Mark a book owned, or not, from its page."),
                )
            })
            .when_some(list.description.clone(), |this, text| {
                this.child(
                    div()
                        .pt_2()
                        .text_sm()
                        .text_color(muted)
                        .child(WrappingText::new(text).line_clamp(3)),
                )
            })
            .when_some(url, |this, url| {
                this.child(
                    div().pt_2().child(
                        Link::new("list-on-hardcover")
                            .href(url)
                            .text_sm()
                            .child("Open on Hardcover"),
                    ),
                )
            })
            .when_some(error, |this, message| {
                this.child(
                    h_flex()
                        .pt_2()
                        .gap_2()
                        .text_sm()
                        .child(div().text_color(muted).child(message))
                        .child(
                            Button::new("retry-list")
                                .small()
                                .label("Try again")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.retry(window, cx)),
                                ),
                        ),
                )
            })
    }
}

impl Render for ListView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self
            .current(cx)
            .and_then(|id| self.lists.read(cx).list(id).cloned());
        v_flex()
            .id("list-page")
            .key_context(KEY_CONTEXT)
            .on_action(
                cx.listener(|this, _: &RemoveFromList, window, cx| {
                    this.remove_selected(window, cx)
                }),
            )
            .size_full()
            .min_w_0()
            .when_some(list, |this, list| {
                this.child(
                    div()
                        .w_full()
                        .max_w(rems(48.))
                        .px_6()
                        .pt_6()
                        .pb_3()
                        .child(self.render_header(&list, cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .max_w(rems(48.))
                        .px_3()
                        .child(List::new(&self.list).size_full().p_1()),
                )
            })
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

    #[test]
    fn removing_a_row_restarts_a_fetch_in_flight() {
        let mut page = BooksPage {
            books: vec![book(1), book(2)],
            more: true,
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
        assert!(page.books.is_empty());
        assert!(!page.loading);
    }
}
