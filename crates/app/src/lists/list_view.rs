//! A list's page: its name, details and books in the list's order. Enter or a
//! click opens a book; Delete (while the list has focus) or a row's remove
//! button takes it off the list, and the notification that says so offers
//! Undo.
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
use std::sync::Arc;

use gpui_kit::ScrollHandle;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::list::{ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, IconName, IndexPath, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::WRITE_LISTS;
use hardcover_core::{ListBook, ListBookId, ListId, UserList};

use super::list_dialog::open_edit_list;
use super::model::{ListsEvent, ListsModel};
use crate::RemoveFromList;
use crate::bidi::WrappingText;
use crate::catalog::book_list::{
    ListPane, follow_pane, page_body, page_header_scrolling, page_list, page_loading, reset_scroll,
    row_details, select_on_click,
};
use crate::catalog::navigation::{self, Page};
use crate::design::{TypeRoles as _, size};
use crate::format::{authors_text, needs_permission};
use crate::library::book_row::BookRow;
use crate::library::model::LibraryModel;
use crate::notify::{notify_failure, notify_outcome};
use crate::resource::{Catalog, Demand, Paged, SharedChanged};
use crate::session;
use crate::ui::kit::{SmallControl as _, SmallIconButton as _};
use crate::ui::kit::{open_alert_dialog, row_action};
use crate::ui::leaving::{Leaving, ROW_LEAVE};
use crate::ui::{
    ExternalLink, ListRow as _, LoadFailed, PageHeader, RowFocus, empty_state,
    empty_state_with_action, on_keyboard_entry, select_first_row,
};

/// Key context of a list's books, where Delete takes the selected book off
/// while the list has focus.
pub const KEY_CONTEXT: &str = "ListPage";

/// The shown list's books as the catalogue holds them
/// (`resource::Catalog::list_books`), copied when they change so the list's
/// rows read them without the app.
#[derive(Default)]
struct BooksShown {
    /// The list these are the books of.
    list: Option<ListId>,
    page: Option<Arc<Paged<ListBook>>>,
    loading: bool,
    /// The first page failed.
    error: Option<SharedString>,
    /// A later page failed.
    more_error: Option<SharedString>,
}

pub struct ListBooks {
    library: Entity<LibraryModel>,
    lists: Entity<ListsModel>,
    catalog: Option<Catalog>,
    current: Option<ListId>,
    shown: BooksShown,
    /// Rows fading out as they leave the list (`ui::leaving`).
    leaving: Leaving<ListBookId>,
    /// While a row fades out: the rows shown, it included.
    fading: Option<Vec<ListBook>>,
    focus: RowFocus,
    /// The book pane is beside the list ([`ListPane`]).
    beside: Rc<Cell<bool>>,
}

impl ListBooks {
    /// The book in row `ix`, for the [`ListPane`].
    fn book_at(&self, ix: usize) -> Option<hardcover_core::BookSummary> {
        self.book(ix).map(|b| b.book.clone())
    }

    fn books(&self) -> &[ListBook] {
        match &self.fading {
            Some(rows) => rows,
            None => self
                .shown
                .page
                .as_deref()
                .map_or(&[], |p| p.items.as_slice()),
        }
    }

    fn book(&self, ix: usize) -> Option<&ListBook> {
        self.books().get(ix)
    }

    /// Copies the shown list's books from the catalogue. A book that left
    /// the list shown fades out in its place first (`ui::leaving`).
    fn follow_store(&mut self, cx: &mut Context<ListState<Self>>) {
        let old_fading = self.fading.take();
        let previous = std::mem::take(&mut self.shown);
        let old = match &old_fading {
            Some(rows) => rows.as_slice(),
            None => previous
                .page
                .as_deref()
                .map_or(&[][..], |p| p.items.as_slice()),
        };
        if let (Some(id), Some(catalog)) = (self.current, &self.catalog) {
            let store = catalog.list_books.read(cx).resource();
            self.shown = BooksShown {
                list: Some(id),
                page: store.value(&id).cloned(),
                loading: store.is_loading(&id),
                error: store.error(&id).cloned(),
                more_error: store.more_error(&id).cloned(),
            };
        }
        let animate = !cx.reduce_motion()
            && previous.list.is_some()
            && previous.list == self.shown.list
            && previous.page.is_some()
            && self.shown.page.is_some();
        let new = self
            .shown
            .page
            .as_deref()
            .map_or(&[][..], |p| p.items.as_slice());
        let shown = self.leaving.rows(
            &old.iter().map(|b| b.id).collect::<Vec<_>>(),
            new.iter().map(|b| b.id).collect(),
            animate,
            cx.background_executor().now(),
        );
        self.fading = (shown.rows.len() != new.len()).then(|| {
            // The new rows' books, and the leaving ones' as they were.
            let mut new = new.iter().map(|b| (b.id, b)).collect::<HashMap<_, _>>();
            let mut old = old.iter().map(|b| (b.id, b)).collect::<HashMap<_, _>>();
            shown
                .rows
                .iter()
                .filter_map(|id| new.remove(id).or_else(|| old.remove(id)).cloned())
                .collect()
        });
        if shown.started {
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

    fn request(&self, demand: Demand, cx: &mut App) {
        if let (Some(id), Some(catalog)) = (self.current, &self.catalog) {
            catalog.request_list_books(id, demand, cx);
        }
    }

    fn show(&mut self, id: ListId, cx: &mut Context<ListState<Self>>) {
        self.current = Some(id);
        self.request(Demand::Shown, cx);
        self.follow_store(cx);
    }

    /// Fetches again after a failure: the first page, or the next one.
    fn retry(&mut self, cx: &mut Context<ListState<Self>>) {
        match (&self.shown.page, self.current, &self.catalog) {
            (Some(_), Some(id), Some(catalog)) => catalog.more_list_books(id, cx),
            _ => self.request(Demand::Now, cx),
        }
    }

    /// Takes the book at `ix` off the list shown. A row already fading out
    /// is off it.
    fn remove(&mut self, ix: usize, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let (Some(list), Some(book)) = (self.current, self.book(ix).cloned()) else {
            return;
        };
        if self.leaving.is_leaving(&book.id) {
            return;
        }
        remove_book(&self.lists, list, &book, window, cx);
    }
}

/// Takes one row off `list`. Once it's off, a notification says so with
/// Undo, which puts the book back as the same edition, at its place on a
/// ranked list; a failure is notified instead.
fn remove_book(
    lists: &Entity<ListsModel>,
    list: ListId,
    book: &ListBook,
    window: &mut Window,
    cx: &mut App,
) {
    let (book_id, row, edition) = (book.book.id, book.id, book.edition_id);
    // A ranked list's place; others have none, and Undo sends none.
    let position = book.position;
    let title = book.book.title.clone();
    let list_name = lists.read(cx).list(list).map(|l| l.name.clone());
    let task = lists.update(cx, |m, cx| m.remove_rows(list, book_id, vec![row], cx));
    let lists = lists.clone();
    notify_outcome(
        task,
        "Couldn't remove the book",
        move |(), _, _| {
            let message = removed_text(&title, list_name.as_deref());
            Some(
                // One per list: removing another book replaces it.
                Notification::new()
                    .id1::<RemovedFromList>(SharedString::from(format!("removed-{list}")))
                    .message(message)
                    .action(move |_, _, cx| {
                        let lists = lists.clone();
                        let note = cx.entity().downgrade();
                        Button::new("undo-remove")
                            .label("Undo")
                            .on_click(move |_, window, cx| {
                                let task = lists.update(cx, |m, cx| {
                                    m.add(list, book_id, edition, position, cx)
                                });
                                notify_failure(task, "Couldn't put the book back", window, cx);
                                let _ = note.update(cx, |note, cx| note.dismiss(window, cx));
                            })
                    })
                    // Undo is offered for a while, not until dismissed.
                    .autohide(true),
            )
        },
        window,
        cx,
    );
}

/// The notification id of "Removed … from …".
struct RemovedFromList;

/// What the notification says once a book is off a list.
fn removed_text(title: &str, list: Option<&str>) -> String {
    match list {
        Some(list) => format!("Removed “{title}” from “{list}”."),
        None => format!("Removed “{title}” from the list."),
    }
}

impl ListDelegate for ListBooks {
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
        let list = self.current?;
        let book = self.book(ix.row)?.clone();
        let opacity = self
            .leaving
            .opacity(&book.id, cx.background_executor().now());
        // The ring is drawn outside the row's opacity: a leaving row has none.
        let focused = opacity.is_none() && self.focus.shows_focus(ix, window);
        let lists = self.lists.read(cx);
        let ranked = lists.list(list).is_some_and(|l| l.ranked);
        let busy = lists.is_busy(book.book.id);
        let can_write = session::has_scope(cx, WRITE_LISTS);
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
                        .disabled(busy || !can_write || opacity.is_some())
                        .on_click(move |_, window, cx| {
                            remove_book(&lists, list, &book, window, cx);
                        }),
                )
            });
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
            Some(message) => empty_state_with_action(
                IconName::CircleAlert,
                "Couldn't load this list".into(),
                message,
                Button::new("retry-list-empty")
                    .small_control()
                    .label("Try again")
                    .on_click(cx.listener(|state, _, _, cx| {
                        state.delegate_mut().retry(cx);
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
        self.shown.page.is_none() && self.shown.error.is_none()
    }

    fn render_loading(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        page_loading()
    }

    fn has_more(&self, _: &App) -> bool {
        self.shown.more_error.is_none() && self.shown.page.as_ref().is_some_and(|p| p.has_more)
    }

    fn load_more_threshold(&self) -> usize {
        20
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let (Some(id), Some(catalog)) = (self.current, &self.catalog)
            && !self.shown.loading
        {
            catalog.more_list_books(id, cx);
        }
    }
}

pub struct ListView {
    lists: Entity<ListsModel>,
    catalog: Option<Catalog>,
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
        let catalog = Catalog::global(cx);
        let list = cx.new(|cx| {
            ListState::new(
                ListBooks {
                    library: library.clone(),
                    lists: lists.clone(),
                    catalog: catalog.clone(),
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
        let mut subscriptions = Vec::new();
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe(&catalog.list_books, {
                let list = list.clone();
                move |_, _, SharedChanged(id): &SharedChanged<ListId>, cx| {
                    list.update(cx, |state, cx| {
                        if state.delegate().current == Some(*id) {
                            state.delegate_mut().follow_store(cx);
                            cx.notify();
                        }
                    });
                }
            }));
        }
        subscriptions.extend([
            follow_pane(&list, &pane, ListBooks::book_at, window, cx),
            cx.observe(&lists, |_, _, cx| cx.notify()),
            // Status tags follow library changes.
            cx.observe(&library, {
                let list = list.clone();
                move |_, _, cx| list.update(cx, |_, cx| cx.notify())
            }),
            cx.subscribe(&lists, |this, _, event: &ListsEvent, cx| {
                this.list_changed(event, cx);
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
        ]);
        Self {
            catalog,
            lists,
            library,
            list,
            pane,
            header_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// A change made to the lists elsewhere, in the catalogue's copy of a
    /// list's books.
    fn list_changed(&mut self, event: &ListsEvent, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        let current = self.current(cx);
        match event {
            // A book went onto it from a book page: a page showing it
            // reloads, and another loads again when shown.
            ListsEvent::BookAdded(id) => {
                catalog.list_books.update(cx, |s, _| s.invalidate(id));
                if current == Some(*id) {
                    catalog.request_list_books(*id, Demand::Shown, cx);
                }
            }
            ListsEvent::BookRemoved { list, row } => {
                catalog
                    .list_books
                    .update(cx, |s, cx| s.update(list, |p| p.remove(*row), cx));
            }
            ListsEvent::Deleted(id) => catalog.list_books.update(cx, |s, cx| s.remove(id, cx)),
            // Refresh, a new grant, or a write whose effect on the lists'
            // books isn't known: every list's books load again when shown.
            ListsEvent::Reloaded => {
                catalog.list_books.update(cx, |s, _| s.invalidate_all());
                if let Some(id) = current {
                    catalog.request_list_books(id, Demand::Shown, cx);
                }
            }
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
            state.delegate_mut().show(id, cx);
            if changed {
                reset_scroll(&header_scroll);
                state.set_selected_index(None, window, cx);
                state.scroll_to_item(IndexPath::new(0), gpui_kit::ScrollStrategy::Top, window, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    /// Asks for the list's books again where they are stale (`resource`; a
    /// failed first page only when `shown_again`), and for the book in the
    /// pane beside the list.
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        let demand = crate::catalog::book_list::demand(shown_again);
        self.list
            .update(cx, |state, cx| state.delegate().request(demand, cx));
        self.pane.revalidate(shown_again, cx);
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::enter_list(&self.list, window, cx);
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.list
            .update(cx, |state, cx| state.delegate_mut().retry(cx));
    }

    /// Delete: takes the selected book off the list, only while the list
    /// itself has focus (not a row's own button, which says what it does).
    fn remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.list.read(cx);
        if !list.focus_handle(cx).is_focused(window) {
            return;
        }
        let Some(ix) = list.selected_index() else {
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
                    notify_failure(task, "Couldn't delete the list", window, cx);
                    true
                })
        });
    }

    fn list_actions(&self, list: &UserList, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        let can_write = session::has_scope(cx, WRITE_LISTS);
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
        // An empty list shows a first page's error in place of the books;
        // a later page's shows here.
        let error = self.list.read(cx).delegate().shown.more_error.clone();
        let actions = self.list_actions(list, cx);
        let mut header = PageHeader::new(list.name.clone())
            .wraps()
            .facts(facts.join(" · "));
        if let Some(actions) = actions {
            header = header.actions(actions);
        }
        if list.is_owned() {
            header = header.line("The editions you own. Mark a book owned, or not, from its page.");
        }
        v_flex()
            .gap_4()
            .child(header)
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
                this.child(
                    LoadFailed::new("retry-list", message)
                        .on_retry(cx.listener(|this, _, _, cx| this.retry(cx))),
                )
            })
    }
}

impl Render for ListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self
            .current(cx)
            .and_then(|id| self.lists.read(cx).list(id).cloned());
        let column =
            v_flex()
                .id("list-page")
                .size_full()
                .min_w_0()
                .when_some(list, |this, list| {
                    this.child(page_header_scrolling(
                        "list-page-header",
                        &self.header_scroll,
                        self.render_header(&list, cx),
                    ))
                    .child(
                        page_body().child(
                            // Delete belongs to the list: with focus on the
                            // header's buttons it must not take a book off.
                            div()
                                .size_full()
                                .key_context(KEY_CONTEXT)
                                .on_action(cx.listener(|this, _: &RemoveFromList, window, cx| {
                                    this.remove_selected(window, cx);
                                }))
                                .child(page_list(&self.list)),
                        ),
                    )
                });
        self.pane
            .layout(&self.list, ListBooks::book_at, column, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::session::SessionScope;
    use crate::ui::test_support::{has_focus, press, redraw, shift_tab};
    use gpui_kit::component::{Root, WindowExt as _};
    use gpui_kit::{TestAppContext, VisualTestContext};
    use hardcover_core::store::LibraryCache;

    fn settle(cx: &mut VisualTestContext) {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        redraw(cx);
    }

    fn notifications(cx: &mut VisualTestContext) -> usize {
        cx.update(|window, cx| window.notifications(cx).len())
    }

    #[gpui_kit::test]
    fn delete_takes_a_book_off_only_while_the_list_has_focus(cx: &mut TestAppContext) {
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client(), cache);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::bind_keys(cx);
            crate::ui::kit::bind_keys(cx);
            session.install(cx);
            Catalog::init(&session, cx);
        });
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let lists = cx.update(|cx| ListsModel::init(session, &library, cx));
        lists.update(cx, ListsModel::load);
        cx.run_until_parked();
        let id = lists.read_with(cx, |m, _| m.user_lists().next().map(|l| l.id).unwrap());
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| ListView::new(lists.clone(), library.clone(), window, cx));
            Root::new(view, window, cx)
        });
        let view = view.read_with(cx, |root, _| {
            root.view().clone().downcast::<ListView>().unwrap()
        });
        cx.update(|window, cx| view.update(cx, |v, cx| v.show(id, window, cx)));
        settle(cx);

        // A book selected, then Shift+Tab to the header's controls (Delete…,
        // "Open on Hardcover"): Delete does nothing there.
        cx.update(|window, cx| {
            view.update(cx, |v, cx| {
                v.focus(window, cx);
                assert!(
                    v.list.read(cx).selected_index().is_some(),
                    "a book is selected"
                );
            });
        });
        redraw(cx);
        shift_tab(cx);
        let list_focused = cx.update(|window, cx| {
            view.read(cx)
                .list
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert!(!list_focused, "focus left the list");
        assert!(has_focus(cx), "on a header control");
        press(cx, "delete");
        settle(cx);
        assert_eq!(notifications(cx), 0, "nothing was asked of Hardcover");

        // With the list focused, Delete asks. The demo refuses every write,
        // so the answer is the failure's notification.
        cx.update(|window, cx| view.update(cx, |v, cx| v.focus(window, cx)));
        redraw(cx);
        press(cx, "delete");
        settle(cx);
        assert_eq!(notifications(cx), 1);
    }

    #[test]
    fn the_notification_names_the_book_and_the_list() {
        assert_eq!(
            removed_text("Dune", Some("Book club")),
            "Removed “Dune” from “Book club”."
        );
        assert_eq!(removed_text("Dune", None), "Removed “Dune” from the list.");
    }
}
