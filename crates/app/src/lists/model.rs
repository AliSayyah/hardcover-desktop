//! The user's lists as an entity: the lists themselves, loaded once signed in,
//! and an index of which books are on which list, loaded the first time a book
//! page needs it. Writes update both locally from their own results.
//!
//! Everything here needs `read:lists`; without it the model stays empty and
//! the views hide the lists. Any view reaches the model through
//! [`ListsModel::global`], so the book page needs no new constructor argument.

use std::collections::{HashMap, HashSet};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Task};
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LIBRARY, WRITE_LISTS};
use hardcover_core::{
    BookId, Client, EditionId, Error, ListBookId, ListDraft, ListId, ListMembership,
    ListMemberships, UserList,
};

use crate::app_state::Services;
use crate::format::{needs_permission, requested_scope};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ListsStatus {
    /// Not loaded, or the session can't read lists.
    #[default]
    Idle,
    Loading,
    Loaded,
    /// The last load failed; the lists shown (if any) are the previous ones.
    Failed(SharedString),
}

/// What a list's page needs to hear about changes made elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub enum ListsEvent {
    /// A book went onto `list` (from a book page): a page showing it reloads.
    BookAdded(ListId),
    /// One row is off its list.
    BookRemoved { list: ListId, row: ListBookId },
    /// The list was deleted.
    Deleted(ListId),
}

pub struct ListsModel {
    /// The Owned list first, then by name.
    lists: Vec<UserList>,
    status: ListsStatus,
    /// A load has finished (or failed) at least once.
    loaded_once: bool,
    /// Which books are on which list; `None` until first needed.
    memberships: Option<ListMemberships>,
    memberships_loading: bool,
    /// Books looked up one at a time because the index was cut short.
    book_rows: HashMap<BookId, Vec<ListMembership>>,
    book_rows_loading: HashSet<BookId>,
    /// Books with a list change in flight; their switches and menu items wait.
    busy_books: HashSet<BookId>,
    /// A list is being created, changed or deleted.
    writing: bool,
    /// Counts local writes. A load that started before one would bring back
    /// the old state, so it is run again instead of applied.
    generation: u64,
    _load: Option<Task<()>>,
    _memberships: Option<Task<()>>,
    _book_rows: Vec<Task<()>>,
}

impl EventEmitter<ListsEvent> for ListsModel {}

struct ListsGlobal(Entity<ListsModel>);

impl Global for ListsGlobal {}

impl ListsModel {
    /// Creates the model for a new workspace, replacing any earlier one.
    pub fn init(cx: &mut App) -> Entity<Self> {
        let model = cx.new(|_| Self::new());
        cx.set_global(ListsGlobal(model.clone()));
        model
    }

    /// The workspace's model, once signed in.
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<ListsGlobal>().map(|g| g.0.clone())
    }

    fn new() -> Self {
        Self {
            lists: Vec::new(),
            status: ListsStatus::Idle,
            loaded_once: false,
            memberships: None,
            memberships_loading: false,
            book_rows: HashMap::new(),
            book_rows_loading: HashSet::new(),
            busy_books: HashSet::new(),
            writing: false,
            generation: 0,
            _load: None,
            _memberships: None,
            _book_rows: Vec::new(),
        }
    }

    // ----- reads -----

    pub fn lists(&self) -> &[UserList] {
        &self.lists
    }

    pub fn list(&self, id: ListId) -> Option<&UserList> {
        self.lists.iter().find(|l| l.id == id)
    }

    /// The list Hardcover keeps owned editions on, once it exists.
    pub fn owned_list(&self) -> Option<&UserList> {
        self.lists.iter().find(|l| l.is_owned())
    }

    /// The lists a book can be put on from its page: all but Owned.
    pub fn user_lists(&self) -> impl Iterator<Item = &UserList> {
        self.lists.iter().filter(|l| !l.is_owned())
    }

    pub fn status(&self) -> &ListsStatus {
        &self.status
    }

    pub fn has_loaded(&self) -> bool {
        self.loaded_once
    }

    pub fn is_writing(&self) -> bool {
        self.writing
    }

    pub fn is_busy(&self, book: BookId) -> bool {
        self.busy_books.contains(&book)
    }

    /// Every row of `book` on the user's lists, or `None` while that is unknown.
    pub fn rows_for(&self, book: BookId) -> Option<Vec<ListMembership>> {
        if let Some(rows) = self.book_rows.get(&book) {
            return Some(rows.clone());
        }
        let index = self.memberships.as_ref()?;
        index.complete.then(|| {
            index
                .rows
                .iter()
                .filter(|r| r.book_id == book)
                .copied()
                .collect()
        })
    }

    /// Whether `book` is on `list`, or `None` while that is unknown.
    pub fn is_on(&self, list: ListId, book: BookId) -> Option<bool> {
        self.rows_for(book)
            .map(|rows| rows.iter().any(|r| r.list_id == list))
    }

    /// The Owned list's rows for `book` (one per owned edition), or `None`
    /// while that is unknown. Empty when there is no Owned list yet.
    pub fn owned_rows(&self, book: BookId) -> Option<Vec<ListMembership>> {
        if !self.loaded_once {
            return None;
        }
        let rows = self.rows_for(book)?;
        Some(match self.owned_list() {
            Some(owned) => rows.into_iter().filter(|r| r.list_id == owned.id).collect(),
            None => Vec::new(),
        })
    }

    // ----- loading -----

    /// Loads the lists, keeping the ones shown until the new ones arrive.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        if !Services::global(cx).has_scope(READ_LISTS) {
            return;
        }
        self.status = ListsStatus::Loading;
        cx.notify();
        let client = Services::global(cx).client().clone();
        let generation = self.generation;
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { client.lists() }).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation && result.is_ok() {
                    this.load(cx);
                    return;
                }
                match result {
                    Ok(lists) => {
                        this.lists = lists;
                        sort(&mut this.lists);
                        this.status = ListsStatus::Loaded;
                        this.loaded_once = true;
                    }
                    Err(e) => this.failed(e, cx),
                }
                cx.notify();
            });
        }));
    }

    /// Clears what the session can no longer read, or records a failure.
    fn failed(&mut self, error: Error, cx: &mut Context<Self>) {
        match error {
            // A scope the app never requests falls through to the generic
            // message: signing in again could not grant it.
            Error::InsufficientScope { scope }
                if scope.is_none() || requested_scope(scope.as_deref()).is_some() =>
            {
                self.lists.clear();
                self.memberships = None;
                self.book_rows.clear();
                self.status = ListsStatus::Idle;
                let scope = requested_scope(scope.as_deref()).unwrap_or(READ_LISTS);
                Services::scope_refused(scope, cx);
            }
            e => {
                self.loaded_once = true;
                self.status = ListsStatus::Failed(Services::error_message(&e, None, cx).into());
            }
        }
    }

    /// Reloads the lists and, if it was loaded, the index (the Refresh command).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
        self.book_rows.clear();
        if self.memberships.is_some() {
            self.memberships = None;
            self.load_memberships(cx);
        }
    }

    /// Loads the index of the user's books on lists, if it isn't loaded yet.
    pub fn ensure_memberships(&mut self, cx: &mut Context<Self>) {
        if self.memberships.is_none() && !self.memberships_loading {
            self.load_memberships(cx);
        }
    }

    fn load_memberships(&mut self, cx: &mut Context<Self>) {
        if !Services::global(cx).has_scope(READ_LISTS) {
            return;
        }
        self.memberships_loading = true;
        let client = Services::global(cx).client().clone();
        let generation = self.generation;
        self._memberships = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.list_memberships() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.memberships_loading = false;
                if this.generation != generation && result.is_ok() {
                    this.load_memberships(cx);
                    return;
                }
                match result {
                    Ok(index) => this.memberships = Some(index),
                    // The lists themselves report the failure; the book page
                    // keeps its list controls waiting, and Refresh retries.
                    Err(e) => {
                        let _ = Services::error_message(&e, Some(READ_LISTS), cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Looks `book` up on its own when the index was cut short and doesn't
    /// say. Also loads the index first if needed.
    pub fn ensure_book(&mut self, book: BookId, cx: &mut Context<Self>) {
        self.ensure_memberships(cx);
        let needed = self.memberships.as_ref().is_some_and(|m| !m.complete)
            && !self.book_rows.contains_key(&book)
            && !self.book_rows_loading.contains(&book);
        if !needed {
            return;
        }
        self.book_rows_loading.insert(book);
        let client = Services::global(cx).client().clone();
        let generation = self.generation;
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.book_list_memberships(book) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.book_rows_loading.remove(&book);
                match result {
                    Ok(rows) if this.generation == generation => {
                        this.book_rows.insert(book, rows);
                    }
                    Ok(_) => this.ensure_book(book, cx),
                    Err(e) => {
                        let _ = Services::error_message(&e, Some(READ_LISTS), cx);
                    }
                }
                cx.notify();
            });
        });
        // Finished lookups are dropped with the next one; there are few.
        self._book_rows.retain(|t| !t.is_ready());
        self._book_rows.push(task);
    }

    // ----- local changes -----

    fn insert_row(&mut self, row: ListMembership) {
        let index_rows = self.memberships.as_mut().map(|m| &mut m.rows);
        let book_rows = self.book_rows.get_mut(&row.book_id);
        let mut is_new = false;
        for rows in [index_rows, book_rows].into_iter().flatten() {
            if !rows.iter().any(|r| r.id == row.id) {
                rows.push(row);
                is_new = true;
            }
        }
        // Without an index, count it anyway: the list's count is all there is.
        if (is_new || self.memberships.is_none())
            && let Some(list) = self.lists.iter_mut().find(|l| l.id == row.list_id)
        {
            list.books_count += 1;
        }
    }

    fn remove_row(&mut self, list: ListId, row: ListBookId) {
        if let Some(index) = self.memberships.as_mut() {
            index.rows.retain(|r| r.id != row);
        }
        for rows in self.book_rows.values_mut() {
            rows.retain(|r| r.id != row);
        }
        if let Some(list) = self.lists.iter_mut().find(|l| l.id == list) {
            list.books_count = list.books_count.saturating_sub(1);
        }
    }

    // ----- list writes -----

    /// Runs one list write in the background. `Err` carries the message to
    /// show; the lists are unchanged then.
    fn write_list<T: Send + 'static, R: 'static>(
        &mut self,
        run: impl FnOnce(&Client) -> Result<T, Error> + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Context<Self>) -> R + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<R, SharedString>> {
        if self.writing {
            return Task::ready(Err("Another change is still being saved.".into()));
        }
        if !Services::global(cx).has_scope(WRITE_LISTS) {
            return Task::ready(Err(needs_permission(WRITE_LISTS).into()));
        }
        self.writing = true;
        self.generation += 1;
        cx.notify();
        let client = Services::global(cx).client().clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { run(&client) }).await;
            this.update(cx, |this, cx| {
                this.writing = false;
                cx.notify();
                match result {
                    Ok(value) => Ok(apply(this, value, cx)),
                    Err(e) => Err(Services::error_message(&e, Some(WRITE_LISTS), cx).into()),
                }
            })
            .unwrap_or_else(|_| Err("The window closed before the change was saved.".into()))
        })
    }

    /// Creates a list; `then_add` also puts that book on it. `Ok(Some(_))`
    /// says why the book isn't on the new list: the list itself was created.
    pub fn create(
        &mut self,
        draft: ListDraft,
        then_add: Option<BookId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<SharedString>, SharedString>> {
        self.write_list(
            move |client| {
                let list = client.create_list(&draft)?;
                // The list stands even if adding the book fails; say so.
                let added = then_add.map(|book| client.add_to_list(list.id, book, None));
                Ok((list, added))
            },
            |this, (mut list, added), cx| {
                let id = list.id;
                let mut failure = None;
                match added {
                    Some(Ok(row)) => {
                        list.books_count = list.books_count.max(1);
                        this.lists.push(list);
                        // Counted above: the new list's count came from the server.
                        if let Some(index) = this.memberships.as_mut() {
                            index.rows.push(row);
                        }
                        if let Some(rows) = this.book_rows.get_mut(&row.book_id) {
                            rows.push(row);
                        }
                    }
                    Some(Err(e)) => {
                        this.lists.push(list);
                        failure = Some(e);
                    }
                    None => this.lists.push(list),
                }
                sort(&mut this.lists);
                cx.emit(ListsEvent::BookAdded(id));
                failure.map(|e| Services::error_message(&e, Some(WRITE_LISTS), cx).into())
            },
            cx,
        )
    }

    /// Saves the draft over one of the model's lists.
    pub fn update(
        &mut self,
        id: ListId,
        draft: ListDraft,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        let Some(list) = self.list(id).cloned() else {
            return Task::ready(Err("This list no longer exists.".into()));
        };
        self.write_list(
            move |client| client.update_list(&list, &draft),
            |this, saved, _| {
                match this.lists.iter_mut().find(|l| l.id == saved.id) {
                    Some(l) => *l = saved,
                    None => this.lists.push(saved),
                }
                sort(&mut this.lists);
            },
            cx,
        )
    }

    pub fn delete(&mut self, id: ListId, cx: &mut Context<Self>) -> Task<Result<(), SharedString>> {
        self.write_list(
            move |client| client.delete_list(id),
            move |this, (), cx| {
                this.lists.retain(|l| l.id != id);
                if let Some(index) = this.memberships.as_mut() {
                    index.rows.retain(|r| r.list_id != id);
                }
                for rows in this.book_rows.values_mut() {
                    rows.retain(|r| r.list_id != id);
                }
                cx.emit(ListsEvent::Deleted(id));
            },
            cx,
        )
    }

    // ----- book writes -----

    /// Runs one change to `book`'s lists in the background.
    /// `apply` may still fail the write, e.g. when only part of it went through.
    fn write_book<T: Send + 'static>(
        &mut self,
        book: BookId,
        scope: &'static str,
        run: impl FnOnce(&Client) -> Result<T, Error> + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Context<Self>) -> Result<(), SharedString> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        if self.busy_books.contains(&book) {
            return Task::ready(Err(
                "Another change to this book is still being saved.".into()
            ));
        }
        if !Services::global(cx).has_scope(scope) {
            return Task::ready(Err(needs_permission(scope).into()));
        }
        self.busy_books.insert(book);
        self.generation += 1;
        cx.notify();
        let client = Services::global(cx).client().clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { run(&client) }).await;
            this.update(cx, |this, cx| {
                this.busy_books.remove(&book);
                cx.notify();
                match result {
                    Ok(value) => apply(this, value, cx),
                    Err(e) => Err(Services::error_message(&e, Some(scope), cx).into()),
                }
            })
            .unwrap_or_else(|_| Err("The window closed before the change was saved.".into()))
        })
    }

    /// Puts `book` on `list`.
    pub fn add(
        &mut self,
        list: ListId,
        book: BookId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        self.write_book(
            book,
            WRITE_LISTS,
            move |client| client.add_to_list(list, book, None),
            move |this, row, cx| {
                this.insert_row(row);
                cx.emit(ListsEvent::BookAdded(row.list_id));
                Ok(())
            },
            cx,
        )
    }

    /// Takes `book` off `list`: every row it has there.
    pub fn remove(
        &mut self,
        list: ListId,
        book: BookId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        let Some(rows) = self.rows_for(book) else {
            return Task::ready(Err("Your lists are still loading.".into()));
        };
        let rows: Vec<ListBookId> = rows
            .iter()
            .filter(|r| r.list_id == list)
            .map(|r| r.id)
            .collect();
        self.remove_rows(list, book, rows, cx)
    }

    /// Takes rows of `book` off `list`, one request each. Rows already
    /// removed stay removed if a later one fails.
    pub fn remove_rows(
        &mut self,
        list: ListId,
        book: BookId,
        rows: Vec<ListBookId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        self.write_book(
            book,
            WRITE_LISTS,
            move |client| {
                let mut removed = Vec::new();
                for row in rows {
                    match client.remove_from_list(row) {
                        Ok(()) => removed.push(row),
                        Err(e) => return Ok((removed, Some(e))),
                    }
                }
                Ok((removed, None))
            },
            move |this, (removed, failed): (Vec<ListBookId>, Option<Error>), cx| {
                for row in removed {
                    this.remove_row(list, row);
                    cx.emit(ListsEvent::BookRemoved { list, row });
                }
                match failed {
                    Some(e) => Err(Services::error_message(&e, Some(WRITE_LISTS), cx).into()),
                    None => Ok(()),
                }
            },
            cx,
        )
    }

    /// Marks `book` owned (as `edition`, through Hardcover's `edition_owned`)
    /// or takes every edition of it off the Owned list.
    pub fn set_owned(
        &mut self,
        book: BookId,
        edition: Option<EditionId>,
        owned: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SharedString>> {
        if !owned {
            let (Some(list), Some(rows)) = (self.owned_list().map(|l| l.id), self.owned_rows(book))
            else {
                return Task::ready(Err("Your Owned list is still loading.".into()));
            };
            return self.remove_rows(list, book, rows.iter().map(|r| r.id).collect(), cx);
        }
        let Some(edition) = edition else {
            return Task::ready(Err(
                "Choose an edition first: Hardcover owns editions.".into()
            ));
        };
        self.write_book(
            book,
            WRITE_LIBRARY,
            move |client| client.mark_owned(edition),
            move |this, row, cx| {
                match row {
                    Some(row) => {
                        let known = this.list(row.list_id).is_some();
                        this.insert_row(row);
                        cx.emit(ListsEvent::BookAdded(row.list_id));
                        // The first owned edition creates the Owned list.
                        if !known {
                            this.load(cx);
                        }
                    }
                    None => this.reload(cx),
                }
                Ok(())
            },
            cx,
        )
    }
}

/// The Owned list first, then by name, as the sidebar shows them.
fn sort(lists: &mut [UserList]) {
    lists.sort_by(|a, b| {
        b.is_owned()
            .cmp(&a.is_owned())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(id: i64, name: &str, slug: &str) -> UserList {
        let mut l = UserList::new(ListId::new(id), name);
        l.slug = Some(slug.into());
        l
    }

    #[test]
    fn owned_comes_first_then_names_ignoring_case() {
        let mut lists = vec![
            list(1, "zebra reads", "zebra-reads"),
            list(2, "Apples", "apples"),
            list(3, "Owned", "owned"),
            list(4, "apples", "apples-2"),
        ];
        sort(&mut lists);
        let ids: Vec<i64> = lists.iter().map(|l| l.id.get()).collect();
        assert_eq!(ids, [3, 2, 4, 1]);
    }

    fn row(id: i64, list: i64, book: i64) -> ListMembership {
        ListMembership {
            id: ListBookId::new(id),
            list_id: ListId::new(list),
            book_id: BookId::new(book),
            edition_id: None,
        }
    }

    #[test]
    fn membership_is_unknown_until_the_index_says() {
        let mut m = ListsModel::new();
        m.loaded_once = true;
        m.status = ListsStatus::Loaded;
        m.lists = vec![list(1, "Owned", "owned"), list(2, "Picks", "picks")];
        let book = BookId::new(10);
        assert_eq!(m.is_on(ListId::new(2), book), None);

        m.memberships = Some(ListMemberships {
            rows: vec![row(100, 1, 10), row(101, 1, 10), row(102, 2, 11)],
            complete: true,
        });
        assert_eq!(m.is_on(ListId::new(2), book), Some(false));
        assert_eq!(m.owned_rows(book).map(|r| r.len()), Some(2));
        assert_eq!(m.owned_rows(BookId::new(11)).map(|r| r.len()), Some(0));

        // A cut-short index can't say "not on it"; a lookup of the book can.
        m.memberships.as_mut().unwrap().complete = false;
        assert_eq!(m.is_on(ListId::new(2), book), None);
        m.book_rows.insert(book, vec![row(103, 2, 10)]);
        assert_eq!(m.is_on(ListId::new(2), book), Some(true));
    }

    #[test]
    fn local_rows_keep_counts_without_counting_twice() {
        let mut m = ListsModel::new();
        m.lists = vec![list(2, "Picks", "picks")];
        m.memberships = Some(ListMemberships {
            rows: Vec::new(),
            complete: true,
        });
        m.insert_row(row(100, 2, 10));
        // Adding a book already on the list returns the same row.
        m.insert_row(row(100, 2, 10));
        assert_eq!(m.lists[0].books_count, 1);
        assert_eq!(m.is_on(ListId::new(2), BookId::new(10)), Some(true));
        m.remove_row(ListId::new(2), ListBookId::new(100));
        assert_eq!(m.lists[0].books_count, 0);
        assert_eq!(m.is_on(ListId::new(2), BookId::new(10)), Some(false));
        // Never below zero.
        m.remove_row(ListId::new(2), ListBookId::new(100));
        assert_eq!(m.lists[0].books_count, 0);
    }
}
