//! The user's lists as an entity: the lists themselves, loaded once signed in,
//! and an index of which books are on which list, loaded the first time a book
//! page needs it. Writes update both locally from their own results.
//!
//! Everything here needs `read:lists`; without it the model stays empty and
//! the views hide the lists. Any view reaches the model through
//! [`ListsModel::global`], so the book page needs no new constructor argument.

use std::collections::{HashMap, HashSet};

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Task, WeakEntity,
};
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LIBRARY, WRITE_LISTS};
use hardcover_core::{
    BookId, Client, EditionId, Error, ListBookId, ListDraft, ListId, ListMembership,
    ListMemberships, OWNED_LIST_SLUG, UserList,
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
    /// Refresh, or a new grant: list pages load their books again.
    Reloaded,
}

pub struct ListsModel {
    /// The Owned list first, then by name.
    lists: Vec<UserList>,
    status: ListsStatus,
    /// A load has finished (or failed) at least once.
    loaded_once: bool,
    /// A load has succeeded: `lists` is the user's, so a missing Owned list
    /// means there is none yet.
    lists_known: bool,
    /// Which books are on which list; `None` until first needed.
    memberships: Option<ListMemberships>,
    memberships_loading: bool,
    /// Books looked up one at a time because the index was cut short.
    book_rows: HashMap<BookId, Vec<ListMembership>>,
    book_rows_loading: HashSet<BookId>,
    /// Books a book page asked about before the index could answer; they are
    /// looked up when an index load finishes.
    pending_books: HashSet<BookId>,
    /// The book the book page asked about last, looked up again on reload.
    shown_book: Option<BookId>,
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

/// Weak, so the model (and the user's lists) goes with the workspace that
/// owns it.
struct ListsGlobal(WeakEntity<ListsModel>);

impl Global for ListsGlobal {}

/// The session's services, or `None` once signed out: a result that lands
/// after signing out is dropped.
fn services(cx: &App) -> Option<&Services> {
    cx.try_global::<Services>()
}

fn can_read(cx: &App) -> bool {
    services(cx).is_some_and(|s| s.has_scope(READ_LISTS))
}

impl ListsModel {
    /// Creates the model for a new workspace, replacing any earlier one. The
    /// caller owns it.
    pub fn init(cx: &mut App) -> Entity<Self> {
        let model = cx.new(|_| Self::new());
        cx.set_global(ListsGlobal(model.downgrade()));
        model
    }

    /// Forgets the workspace's model, on signing out.
    pub fn release(cx: &mut App) {
        if cx.has_global::<ListsGlobal>() {
            cx.remove_global::<ListsGlobal>();
        }
    }

    /// The workspace's model, once signed in.
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<ListsGlobal>()?.0.upgrade()
    }

    fn new() -> Self {
        Self {
            lists: Vec::new(),
            status: ListsStatus::Idle,
            loaded_once: false,
            lists_known: false,
            memberships: None,
            memberships_loading: false,
            book_rows: HashMap::new(),
            book_rows_loading: HashSet::new(),
            pending_books: HashSet::new(),
            shown_book: None,
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

    /// Whether a page showing list `id` has nothing left to show: the lists
    /// no longer include it, or can't be read (`Idle` after a refused scope).
    /// While they load, or after a failed load, the page stays.
    pub fn is_gone(&self, id: ListId) -> bool {
        match self.status {
            ListsStatus::Idle => true,
            ListsStatus::Loaded => self.list(id).is_none(),
            ListsStatus::Loading | ListsStatus::Failed(_) => false,
        }
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
        if !self.lists_known {
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
        let Some(client) = services(cx)
            .filter(|s| s.has_scope(READ_LISTS))
            .map(|s| s.client().clone())
        else {
            return;
        };
        self.status = ListsStatus::Loading;
        cx.notify();
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
                        this.lists_known = true;
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
                self.lists_known = false;
                self.memberships = None;
                self.book_rows.clear();
                self.pending_books.clear();
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

    /// Reloads the lists, and the index if a book page has needed it
    /// (Refresh, or a new grant). The old index answers until the new one
    /// arrives; this also retries one that failed.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if !can_read(cx) {
            return;
        }
        self.load(cx);
        if self.plan_reload() {
            self.load_memberships(cx);
        }
        cx.emit(ListsEvent::Reloaded);
    }

    /// Forgets the books looked up alone, queueing the one shown to be looked
    /// up again. Whether the index should load again: only if it was loaded
    /// or wanted, so a Refresh with no book page opened costs no index load.
    fn plan_reload(&mut self) -> bool {
        self.book_rows.clear();
        if let Some(book) = self.shown_book {
            self.pending_books.insert(book);
        }
        self.memberships.is_some() || self.memberships_loading || !self.pending_books.is_empty()
    }

    /// Loads the index of the user's books on lists, if it isn't loaded yet.
    pub fn ensure_memberships(&mut self, cx: &mut Context<Self>) {
        if self.memberships.is_none() && !self.memberships_loading {
            self.load_memberships(cx);
        }
    }

    fn load_memberships(&mut self, cx: &mut Context<Self>) {
        let Some(client) = services(cx)
            .filter(|s| s.has_scope(READ_LISTS))
            .map(|s| s.client().clone())
        else {
            return;
        };
        self.memberships_loading = true;
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
                // Complete or not, the index (old or new) now decides which
                // books waiting on it need a lookup of their own.
                for book in this.take_lookups() {
                    this.look_up(book, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Makes sure `book`'s lists become known for its page: loads the index
    /// if needed, and looks the book up alone if the index was cut short.
    pub fn ensure_book(&mut self, book: BookId, cx: &mut Context<Self>) {
        self.shown_book = Some(book);
        self.ensure_memberships(cx);
        if self.memberships_loading || self.memberships.is_none() {
            // Decided when the index lands.
            self.pending_books.insert(book);
            return;
        }
        self.look_up(book, cx);
    }

    /// The waiting books that need a lookup of their own, now that an index
    /// load has finished. Without any index they keep waiting (Refresh
    /// retries); a complete index answers for them all.
    fn take_lookups(&mut self) -> Vec<BookId> {
        let Some(index) = &self.memberships else {
            return Vec::new();
        };
        let complete = index.complete;
        let books = std::mem::take(&mut self.pending_books);
        if complete {
            return Vec::new();
        }
        books
            .into_iter()
            .filter(|b| !self.book_rows.contains_key(b) && !self.book_rows_loading.contains(b))
            .collect()
    }

    /// Looks `book` up on its own when the index was cut short and doesn't say.
    fn look_up(&mut self, book: BookId, cx: &mut Context<Self>) {
        let needed = self.memberships.as_ref().is_some_and(|m| !m.complete)
            && !self.book_rows.contains_key(&book)
            && !self.book_rows_loading.contains(&book);
        if !needed {
            return;
        }
        let Some(client) = services(cx)
            .filter(|s| s.has_scope(READ_LISTS))
            .map(|s| s.client().clone())
        else {
            return;
        };
        self.book_rows_loading.insert(book);
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
                    Ok(_) => this.look_up(book, cx),
                    // Looked up again after the next index load (Refresh).
                    Err(e) => {
                        this.pending_books.insert(book);
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

    /// Records `row` in the index and the book's own rows, where missing.
    /// True if neither held it: the book wasn't known to be on the list.
    fn record_row(&mut self, row: ListMembership) -> bool {
        let index_rows = self.memberships.as_mut().map(|m| &mut m.rows);
        let book_rows = self.book_rows.get_mut(&row.book_id);
        let mut known = false;
        for rows in [index_rows, book_rows].into_iter().flatten() {
            if rows.iter().any(|r| r.id == row.id) {
                known = true;
            } else {
                rows.push(row);
            }
        }
        !known
    }

    /// Records a new row and counts it on its list. Without an index it is
    /// counted anyway: the list's count is all there is.
    fn insert_row(&mut self, row: ListMembership) {
        if self.record_row(row)
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

    /// When the lists are known and `id` isn't one of them, it is the Owned
    /// list that marking an edition owned just created: stands in for it
    /// until the lists reload, so the switch shows on (and can't post the
    /// edition again). True if it did.
    fn add_owned_placeholder(&mut self, id: ListId) -> bool {
        if !self.lists_known || self.list(id).is_some() {
            return false;
        }
        let mut owned = UserList::new(id, "Owned");
        owned.slug = Some(OWNED_LIST_SLUG.into());
        self.lists.push(owned);
        sort(&mut self.lists);
        true
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
            return Task::ready(Err(
                "Another change is still being saved. Try again in a moment.".into(),
            ));
        }
        let Some(services) = services(cx) else {
            return Task::ready(Err(SIGNED_OUT.into()));
        };
        if !services.has_scope(WRITE_LISTS) {
            return Task::ready(Err(needs_permission(WRITE_LISTS).into()));
        }
        let client = services.client().clone();
        self.writing = true;
        self.generation += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { run(&client) }).await;
            this.update(cx, |this, cx| {
                this.writing = false;
                // A load that started while the write ran may predate it too.
                this.generation += 1;
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
                        this.record_row(row);
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
                "Another change to this book is still being saved. Try again in a moment.".into(),
            ));
        }
        let Some(services) = services(cx) else {
            return Task::ready(Err(SIGNED_OUT.into()));
        };
        if !services.has_scope(scope) {
            return Task::ready(Err(needs_permission(scope).into()));
        }
        let client = services.client().clone();
        self.busy_books.insert(book);
        self.generation += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { run(&client) }).await;
            this.update(cx, |this, cx| {
                this.busy_books.remove(&book);
                this.generation += 1;
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
                "Choose an edition first. Hardcover marks editions as owned, not books.".into(),
            ));
        };
        self.write_book(
            book,
            WRITE_LIBRARY,
            move |client| client.mark_owned(edition),
            move |this, row, cx| {
                match row {
                    Some(row) => {
                        // The first owned edition creates the Owned list.
                        let created = this.add_owned_placeholder(row.list_id);
                        this.insert_row(row);
                        cx.emit(ListsEvent::BookAdded(row.list_id));
                        if created {
                            this.load(cx);
                        }
                    }
                    // `Reloaded` also has the Owned list's page load again.
                    None => this.reload(cx),
                }
                Ok(())
            },
            cx,
        )
    }
}

const SIGNED_OUT: &str = "You signed out before the change could be saved.";

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
        m.lists_known = true;
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

    #[test]
    fn a_row_known_from_the_book_lookup_is_not_counted_again() {
        let mut m = ListsModel::new();
        let mut picks = list(2, "Picks", "picks");
        picks.books_count = 5;
        m.lists = vec![picks];
        // The index was cut short before this book; its own lookup has it.
        m.memberships = Some(ListMemberships {
            rows: Vec::new(),
            complete: false,
        });
        m.book_rows.insert(BookId::new(10), vec![row(100, 2, 10)]);
        // Adding it again returns the same row.
        m.insert_row(row(100, 2, 10));
        assert_eq!(m.lists[0].books_count, 5);
        assert_eq!(m.memberships.as_ref().unwrap().rows, [row(100, 2, 10)]);
        // A row neither holds is new.
        m.insert_row(row(101, 2, 11));
        assert_eq!(m.lists[0].books_count, 6);
    }

    #[test]
    fn books_asked_about_early_are_looked_up_once_a_cut_short_index_lands() {
        let mut m = ListsModel::new();
        let (a, b) = (BookId::new(10), BookId::new(11));
        m.pending_books.extend([a, b]);
        // No index yet (still loading, or it failed): they keep waiting.
        assert!(m.take_lookups().is_empty());
        assert_eq!(m.pending_books.len(), 2);

        m.memberships = Some(ListMemberships {
            rows: Vec::new(),
            complete: false,
        });
        m.book_rows.insert(b, Vec::new());
        assert_eq!(m.take_lookups(), [a]);
        assert!(m.pending_books.is_empty());

        // A complete index answers for every book.
        m.pending_books.insert(a);
        m.memberships.as_mut().unwrap().complete = true;
        assert!(m.take_lookups().is_empty());
        assert!(m.pending_books.is_empty());
    }

    #[test]
    fn reload_skips_an_index_nobody_needed_and_looks_the_shown_book_up_again() {
        let mut m = ListsModel::new();
        assert!(!m.plan_reload());

        let book = BookId::new(10);
        m.shown_book = Some(book);
        m.memberships = Some(ListMemberships {
            rows: Vec::new(),
            complete: false,
        });
        m.book_rows.insert(book, vec![row(100, 2, 10)]);
        assert!(m.plan_reload());
        assert!(m.book_rows.is_empty());
        assert!(m.pending_books.contains(&book));
    }

    #[test]
    fn the_first_owned_edition_shows_owned_before_the_lists_reload() {
        let mut m = ListsModel::new();
        m.lists_known = true;
        m.status = ListsStatus::Loaded;
        m.lists = vec![list(2, "Picks", "picks")];
        m.memberships = Some(ListMemberships {
            rows: Vec::new(),
            complete: true,
        });
        let book = BookId::new(10);
        assert_eq!(m.owned_rows(book).map(|r| r.len()), Some(0));

        assert!(m.add_owned_placeholder(ListId::new(7)));
        m.insert_row(row(100, 7, 10));
        assert_eq!(m.owned_rows(book).map(|r| r.len()), Some(1));
        assert_eq!(
            m.owned_list().map(|l| (l.id, l.books_count)),
            Some((ListId::new(7), 1))
        );
        // Only once, and never for a list already known.
        assert!(!m.add_owned_placeholder(ListId::new(7)));
        assert!(!m.add_owned_placeholder(ListId::new(2)));
        assert_eq!(m.lists.len(), 2);
    }

    #[test]
    fn the_first_owned_edition_adds_nothing_while_the_lists_are_unknown() {
        let mut m = ListsModel::new();
        assert!(!m.add_owned_placeholder(ListId::new(7)));
        assert!(m.lists.is_empty());
    }

    #[test]
    fn a_list_page_gives_way_when_the_list_is_gone_or_unreadable() {
        let mut m = ListsModel::new();
        let id = ListId::new(2);
        // A refused scope leaves the lists Idle and empty.
        assert!(m.is_gone(id));
        m.status = ListsStatus::Loading;
        assert!(!m.is_gone(id));
        m.status = ListsStatus::Failed("offline".into());
        assert!(!m.is_gone(id));
        m.status = ListsStatus::Loaded;
        assert!(m.is_gone(id));
        m.lists = vec![list(2, "Picks", "picks")];
        assert!(!m.is_gone(id));
    }
}
