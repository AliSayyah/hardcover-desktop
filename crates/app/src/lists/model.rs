//! The user's lists as an entity: the lists themselves, loaded once signed in,
//! and an index of which books are on which list, loaded the first time a book
//! page needs it. Writes update both locally from their own results.
//!
//! The lists, the index and books looked up alone are [`Resource`]s; the
//! lists and the index are kept in the library cache between launches
//! (`resource::saved`), so the last good ones show at once, offline too.
//!
//! Everything here needs `read:lists`; without it the model stays empty and
//! the views hide the lists. Any view reaches the model through
//! [`ListsModel::global`], so the book page needs no new constructor argument.
//! The model belongs to one session ([`SessionScope`]): once it ends,
//! nothing more is sent and results that land are dropped.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString, Subscription, Task,
    WeakEntity,
};
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LIBRARY, WRITE_LISTS};
use hardcover_core::{
    BookId, Client, EditionId, Error, ListBookId, ListDraft, ListEdit, ListId, ListMembership,
    ListMemberships, OWNED_LIST_SLUG, UserList,
};

use crate::format::needs_permission;
use crate::io::Priority;
use crate::library::model::{AccountChanged, LibraryModel, STILL_SAVING};
use crate::resource::{Binding, Debounce, Demand, Landing, LoadStatus, Resource, saved};
use crate::session::SessionScope;

/// How long after a write to load again what a load in flight missed:
/// Hardcover can answer a read right after a write with the old rows.
const RELOAD_DELAY: Duration = Duration::from_secs(2);

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
    session: SessionScope,
    /// The Owned list first, then by name.
    lists: Resource<(), Vec<UserList>>,
    /// Which books are on which list; asked for once a book page needs it.
    index: Resource<(), ListMemberships>,
    /// Books looked up one at a time because the index was cut short.
    book_rows: Resource<BookId, Vec<ListMembership>>,
    /// Books a book page asked about before the index could answer; they are
    /// looked up when an index load finishes.
    pending_books: HashSet<BookId>,
    /// The book the book page asked about last, looked up again on reload.
    shown_book: Option<BookId>,
    /// Books with a list change in flight; their switches and menu items wait.
    busy_books: HashSet<BookId>,
    /// A list is being created, changed or deleted.
    writing: bool,
    /// Loads again what a write made a load in flight drop.
    reload_dropped: Debounce,
    _hydrating: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ListsEvent> for ListsModel {}

/// Weak, so the model (and the user's lists) goes with the workspace that
/// owns it.
struct ListsGlobal(WeakEntity<ListsModel>);

impl Global for ListsGlobal {}

impl ListsModel {
    /// Creates the model for a new workspace, replacing any earlier one, and
    /// shows the lists of the last launch while they load. The caller owns
    /// it.
    pub fn init(
        session: SessionScope,
        library: &Entity<LibraryModel>,
        cx: &mut App,
    ) -> Entity<Self> {
        let model = cx.new(|cx| {
            let mut model = Self::new(session.clone());
            model._hydrating = vec![
                saved::hydrate(
                    &session,
                    "lists",
                    saved::parse_single,
                    Self::bind_lists(),
                    cx,
                ),
                saved::hydrate(
                    &session,
                    "list_memberships",
                    saved::parse_single,
                    Self::bind_index(),
                    cx,
                ),
            ];
            model._subscriptions =
                vec![cx.subscribe(library, |this, _, _: &AccountChanged, cx| {
                    this._hydrating.clear();
                    this.lists.forget_from_disk();
                    this.index.forget_from_disk();
                    cx.notify();
                })];
            model.lists = model
                .lists
                .persist(saved::saver(&session, "lists", saved::single));
            model.index =
                model
                    .index
                    .persist(saved::saver(&session, "list_memberships", saved::single));
            model
        });
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

    fn new(session: SessionScope) -> Self {
        Self {
            lists: Resource::new(session.clone(), READ_LISTS, Priority::Background),
            index: Resource::new(session.clone(), READ_LISTS, Priority::Background),
            book_rows: Resource::new(session.clone(), READ_LISTS, Priority::Interactive),
            session,
            pending_books: HashSet::new(),
            shown_book: None,
            busy_books: HashSet::new(),
            writing: false,
            reload_dropped: Debounce::default(),
            _hydrating: Vec::new(),
            _subscriptions: Vec::new(),
        }
    }

    fn bind_lists() -> Binding<Self, (), Vec<UserList>> {
        Binding {
            get: |m| &mut m.lists,
            landed: |this, _, landing, cx| this.landed(landing, cx),
        }
    }

    fn bind_index() -> Binding<Self, (), ListMemberships> {
        Binding {
            get: |m| &mut m.index,
            landed: |this, _, landing, cx| {
                // Complete or not, the index (old or new) now decides which
                // books waiting on it need a lookup of their own. A failed
                // one leaves them waiting (Refresh, or "Try again").
                if landing != Landing::Failed {
                    for book in this.take_lookups() {
                        this.look_up(book, Demand::Active, cx);
                    }
                }
                this.landed(landing, cx);
            },
        }
    }

    fn bind_book_rows() -> Binding<Self, BookId, Vec<ListMembership>> {
        Binding {
            get: |m| &mut m.book_rows,
            landed: |this, _, landing, cx| this.landed(landing, cx),
        }
    }

    /// A write came after a load started: its result was dropped, so what
    /// it would have brought loads again shortly.
    fn landed(&mut self, landing: Landing, cx: &mut Context<Self>) {
        if landing == Landing::Dropped {
            self.reload_dropped
                .schedule(RELOAD_DELAY, Self::load_dropped, cx);
        }
        cx.notify();
    }

    fn load_dropped(&mut self, cx: &mut Context<Self>) {
        self.request_lists(Demand::Now, cx);
        if self.index.value(&()).is_some() || self.index.has_settled(&()) {
            self.request_index(Demand::Now, cx);
        }
        if let Some(book) = self.shown_book {
            self.look_up(book, Demand::Now, cx);
        }
    }

    // ----- reads -----

    pub fn lists(&self) -> &[UserList] {
        self.lists.value(&()).map_or(&[], |l| l.as_slice())
    }

    pub fn list(&self, id: ListId) -> Option<&UserList> {
        self.lists().iter().find(|l| l.id == id)
    }

    /// The list Hardcover keeps owned editions on, once it exists.
    pub fn owned_list(&self) -> Option<&UserList> {
        self.lists().iter().find(|l| l.is_owned())
    }

    /// The lists a book can be put on from its page: all but Owned.
    pub fn user_lists(&self) -> impl Iterator<Item = &UserList> {
        self.lists().iter().filter(|l| !l.is_owned())
    }

    pub fn status(&self) -> LoadStatus {
        self.lists.status(&())
    }

    /// Whether a page showing list `id` has nothing left to show: the lists
    /// no longer include it, or can't be read (a refused scope). While they
    /// load, or after a failed load, the page stays.
    pub fn is_gone(&self, id: ListId) -> bool {
        is_gone(
            &self.status(),
            self.list(id).is_some(),
            self.session.allows(READ_LISTS),
        )
    }

    /// The lists have loaded (or failed) at least once, or the last launch's
    /// are shown.
    pub fn has_loaded(&self) -> bool {
        self.lists.has_settled(&())
    }

    /// The lists are the user's (loaded, or the last launch's), so a missing
    /// Owned list means there is none yet.
    fn lists_known(&self) -> bool {
        self.lists.value(&()).is_some()
    }

    pub fn is_writing(&self) -> bool {
        self.writing
    }

    pub fn is_busy(&self, book: BookId) -> bool {
        self.busy_books.contains(&book)
    }

    /// Every row of `book` on the user's lists, or `None` while that is unknown.
    pub fn rows_for(&self, book: BookId) -> Option<Vec<ListMembership>> {
        if let Some(rows) = self.book_rows.value(&book) {
            return Some(rows.to_vec());
        }
        let index = self.index.value(&())?;
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
        if !self.lists_known() {
            return None;
        }
        let rows = self.rows_for(book)?;
        Some(match self.owned_list() {
            Some(owned) => rows.into_iter().filter(|r| r.list_id == owned.id).collect(),
            None => Vec::new(),
        })
    }

    /// Why `book`'s lists can't be told, when nothing on its way could tell
    /// them: the lists, the index or the book's own lookup failed. The book
    /// page says so beside its list controls, with "Try again"
    /// ([`Self::retry_book`]).
    pub fn book_failure(&self, book: BookId) -> Option<SharedString> {
        if self.rows_for(book).is_some() && self.lists_known() {
            return None;
        }
        let loading = self.lists.is_loading(&())
            || self.index.is_loading(&())
            || self.book_rows.is_loading(&book);
        if loading {
            return None;
        }
        self.lists
            .error(&())
            .or_else(|| self.index.error(&()))
            .or_else(|| self.book_rows.error(&book))
            .cloned()
    }

    // ----- loading -----

    /// Loads the lists where they are stale or failed (at start), keeping
    /// the ones shown until the new ones arrive.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.request_lists(Demand::Shown, cx);
    }

    /// "Try again" after the lists failed to load.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.request_lists(Demand::Now, cx);
    }

    fn request_lists(&mut self, demand: Demand, cx: &mut Context<Self>) {
        let call = |client: &Client| {
            client.lists().map(|mut lists| {
                sort(&mut lists);
                lists
            })
        };
        if self.lists.request((), demand, call, Self::bind_lists(), cx) {
            cx.notify();
        }
    }

    /// Loads the lists again when list `list`'s books, just fetched,
    /// disagree with its count: they changed elsewhere (on the website, in
    /// another copy of the app) since the lists loaded. `sent` is the rows
    /// Hardcover sent, all of them once `complete`. Not while a write is
    /// under way: its own result is about to change the count.
    pub fn check_count(&mut self, list: ListId, sent: u32, complete: bool, cx: &mut Context<Self>) {
        if self.writing || !self.session.allows(READ_LISTS) {
            return;
        }
        let Some(count) = self.list(list).map(|l| l.books_count) else {
            return;
        };
        if count_disagrees(count, sent, complete) {
            self.request_lists(Demand::Now, cx);
        }
    }

    fn request_index(&mut self, demand: Demand, cx: &mut Context<Self>) {
        let call = |client: &Client| client.list_memberships();
        if self.index.request((), demand, call, Self::bind_index(), cx) {
            cx.notify();
        }
    }

    /// Reloads what is stale or failed: the lists, and the index if a book
    /// page has needed it (Refresh, which made them stale, or a new grant).
    /// The old index answers until the new one arrives.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_with(Demand::Shown, cx);
    }

    /// Reloads the lists, the index and the book shown, however fresh: a
    /// write changed them in ways its result doesn't say. List pages load
    /// their books again too (`ListsEvent::Reloaded`).
    fn reload_with(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if !self.session.allows(READ_LISTS) {
            return;
        }
        self.request_lists(demand, cx);
        if self.plan_reload() {
            self.request_index(demand, cx);
        }
        // Without an index on its way, the books waiting on it are looked
        // up now.
        if !self.index.is_loading(&()) {
            for book in self.take_lookups() {
                self.look_up(book, demand, cx);
            }
        }
        cx.emit(ListsEvent::Reloaded);
    }

    /// Marks the books looked up alone to be looked up again (their rows
    /// answer meanwhile), queueing the one shown. Whether the index should
    /// load again: only if it was loaded or wanted, so a Refresh with no
    /// book page opened costs no index load.
    fn plan_reload(&mut self) -> bool {
        self.book_rows.invalidate_all_but(|_| false);
        if let Some(book) = self.shown_book {
            self.pending_books.insert(book);
        }
        self.index.value(&()).is_some()
            || self.index.has_settled(&())
            || self.index.is_loading(&())
            || !self.pending_books.is_empty()
    }

    /// Makes sure `book`'s lists become known for its page: loads the index
    /// if needed, and looks the book up alone if the index was cut short.
    pub fn ensure_book(&mut self, book: BookId, cx: &mut Context<Self>) {
        self.shown_book = Some(book);
        self.request_index(Demand::Active, cx);
        if self.index.is_loading(&()) || self.index.value(&()).is_none() {
            // Decided when the index lands.
            self.pending_books.insert(book);
            return;
        }
        self.look_up(book, Demand::Active, cx);
    }

    /// "Try again" beside a book's list controls ([`Self::book_failure`]):
    /// asks again for whatever failed.
    pub fn retry_book(&mut self, book: BookId, cx: &mut Context<Self>) {
        if self.lists.error(&()).is_some() {
            self.request_lists(Demand::Now, cx);
        }
        if self.index.error(&()).is_some() {
            self.pending_books.insert(book);
            self.request_index(Demand::Now, cx);
        }
        if self.book_rows.error(&book).is_some() {
            self.look_up(book, Demand::Now, cx);
        }
    }

    /// The waiting books that need a lookup of their own, now that an index
    /// load has finished. Without any index they keep waiting (Refresh
    /// retries); a complete index answers for them all. One looked up
    /// already is asked for again only if its rows are stale (`look_up`'s
    /// demand decides).
    #[must_use]
    fn take_lookups(&mut self) -> Vec<BookId> {
        let Some(index) = self.index.value(&()) else {
            return Vec::new();
        };
        let complete = index.complete;
        let books = std::mem::take(&mut self.pending_books);
        if complete {
            return Vec::new();
        }
        books
            .into_iter()
            .filter(|b| !self.book_rows.is_loading(b))
            .collect()
    }

    /// Looks `book` up on its own when the index was cut short and doesn't say.
    fn look_up(&mut self, book: BookId, demand: Demand, cx: &mut Context<Self>) {
        let needed = self.index.value(&()).is_some_and(|m| !m.complete);
        if !needed {
            return;
        }
        let call = move |client: &Client| client.book_list_memberships(book);
        if self
            .book_rows
            .request(book, demand, call, Self::bind_book_rows(), cx)
        {
            cx.notify();
        }
    }

    // ----- local changes -----

    /// Changes the lists in place, as a write did, keeping their order.
    fn edit_lists(&mut self, edit: impl FnOnce(&mut Vec<UserList>), cx: &App) {
        let mut lists = self.lists().to_vec();
        edit(&mut lists);
        sort(&mut lists);
        self.lists.set((), lists, cx);
    }

    /// Records `row` in the index and the book's own rows, where missing.
    /// True if neither held it: the book wasn't known to be on the list.
    fn record_row(&mut self, row: ListMembership) -> bool {
        let mut known = false;
        let mut add = |rows: &mut Vec<ListMembership>| {
            if rows.iter().any(|r| r.id == row.id) {
                known = true;
            } else {
                rows.push(row);
            }
        };
        self.index.update(&(), |index| add(&mut index.rows));
        self.book_rows.update(&row.book_id, |rows| add(rows));
        !known
    }

    /// Records a new row and counts it on its list. Without an index it is
    /// counted anyway: the list's count is all there is.
    fn insert_row(&mut self, row: ListMembership) {
        if self.record_row(row) {
            self.lists.update(&(), |lists| {
                if let Some(list) = lists.iter_mut().find(|l| l.id == row.list_id) {
                    list.books_count += 1;
                }
            });
        }
    }

    fn remove_row(&mut self, list: ListId, row: ListBookId) {
        self.index
            .update(&(), |index| index.rows.retain(|r| r.id != row));
        self.book_rows
            .update_all(|_, rows| rows.retain(|r| r.id != row));
        self.lists.update(&(), |lists| {
            if let Some(list) = lists.iter_mut().find(|l| l.id == list) {
                list.books_count = list.books_count.saturating_sub(1);
            }
        });
    }

    /// When the lists are known and `id` isn't one of them, it is the Owned
    /// list that marking an edition owned just created: stands in for it
    /// until the lists reload, so the switch shows on (and can't post the
    /// edition again). True if it did.
    fn add_owned_placeholder(&mut self, id: ListId) -> bool {
        if !self.lists_known() || self.list(id).is_some() {
            return false;
        }
        let mut owned = UserList::new(id, "Owned");
        owned.slug = Some(OWNED_LIST_SLUG.into());
        self.lists.update(&(), |lists| {
            lists.push(owned);
            sort(lists);
        });
        true
    }

    /// A write is starting or has ended: whatever a load in flight brings
    /// is older than it, and dropped.
    fn note_write(&mut self, book: Option<BookId>) {
        self.lists.note_write(&());
        self.index.note_write(&());
        if let Some(book) = book {
            self.book_rows.note_write(&book);
        }
    }

    // ----- list writes -----

    /// Runs one list write. Resolves to `None` if it was dropped because
    /// the session ended (nothing to show), else to `Err` with the message
    /// to show; the lists are unchanged then.
    fn write_list<T: Send + 'static, R: 'static>(
        &mut self,
        run: impl FnOnce(&Client) -> Result<T, Error> + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Context<Self>) -> R + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<R, SharedString>>> {
        if !self.session.is_alive() {
            return Task::ready(None);
        }
        if self.writing {
            return Task::ready(Some(Err(
                "Another change is still being saved. Try again in a moment.".into(),
            )));
        }
        if !self.session.has_scope(WRITE_LISTS) {
            return Task::ready(Some(Err(needs_permission(WRITE_LISTS).into())));
        }
        self.writing = true;
        self.note_write(None);
        cx.notify();
        let call = self.session.interactive(run);
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.writing = false;
                // A load that started while the write ran may predate it too.
                this.note_write(None);
                cx.notify();
                // Dropped: the session ended (or the call panicked).
                Some(match result? {
                    Ok(value) => Ok(apply(this, value, cx)),
                    Err(e) => Err(this.session.error_message(&e, Some(WRITE_LISTS), cx).into()),
                })
            })
            .ok()
            .flatten()
        })
    }

    /// Creates a list; `then_add` also puts that book on it. `Ok(Some(_))`
    /// says why the book isn't on the new list: the list itself was created.
    pub fn create(
        &mut self,
        draft: ListDraft,
        then_add: Option<BookId>,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<Option<SharedString>, SharedString>>> {
        self.write_list(
            move |client| {
                let list = client.create_list(&draft)?;
                // The list stands even if adding the book fails; say so.
                let added = then_add.map(|book| client.add_to_list(list.id, book, None, None));
                Ok((list, added))
            },
            |this, (mut list, added), cx| {
                let id = list.id;
                let mut failure = None;
                match added {
                    Some(Ok(row)) => {
                        list.books_count = list.books_count.max(1);
                        this.edit_lists(|lists| lists.push(list), cx);
                        // Counted above: the new list's count came from the server.
                        this.record_row(row);
                    }
                    Some(Err(e)) => {
                        this.edit_lists(|lists| lists.push(list), cx);
                        failure = Some(e);
                    }
                    None => this.edit_lists(|lists| lists.push(list), cx),
                }
                cx.emit(ListsEvent::BookAdded(id));
                failure.map(|e| this.session.error_message(&e, Some(WRITE_LISTS), cx).into())
            },
            cx,
        )
    }

    /// Saves what the draft changes about one of the model's lists. Only
    /// those fields are written; the rest is kept as Hardcover has it at the
    /// time of the write, so changes made elsewhere since it was loaded stay.
    pub fn update(
        &mut self,
        id: ListId,
        draft: ListDraft,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        let Some(list) = self.list(id) else {
            return Task::ready(Some(Err("This list no longer exists.".into())));
        };
        let edit = ListEdit::between(list, &draft);
        self.write_list(
            move |client| client.update_list(id, &edit),
            |this, saved, cx| {
                this.edit_lists(
                    |lists| match lists.iter_mut().find(|l| l.id == saved.id) {
                        Some(l) => *l = saved,
                        None => lists.push(saved),
                    },
                    cx,
                );
            },
            cx,
        )
    }

    pub fn delete(
        &mut self,
        id: ListId,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        self.write_list(
            move |client| client.delete_list(id),
            move |this, (), cx| {
                this.edit_lists(|lists| lists.retain(|l| l.id != id), cx);
                this.index
                    .update(&(), |index| index.rows.retain(|r| r.list_id != id));
                this.book_rows
                    .update_all(|_, rows| rows.retain(|r| r.list_id != id));
                cx.emit(ListsEvent::Deleted(id));
            },
            cx,
        )
    }

    // ----- book writes -----

    /// Runs one change to `book`'s lists, resolving as
    /// [`Self::write_list`] does. `apply` may still fail the write, e.g.
    /// when only part of it went through.
    fn write_book<T: Send + 'static>(
        &mut self,
        book: BookId,
        scope: &'static str,
        run: impl FnOnce(&Client) -> Result<T, Error> + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Context<Self>) -> Result<(), SharedString> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        if !self.session.is_alive() {
            return Task::ready(None);
        }
        if self.busy_books.contains(&book) {
            return Task::ready(Some(Err(STILL_SAVING.into())));
        }
        if !self.session.has_scope(scope) {
            return Task::ready(Some(Err(needs_permission(scope).into())));
        }
        self.busy_books.insert(book);
        self.note_write(Some(book));
        cx.notify();
        let call = self.session.interactive(run);
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.busy_books.remove(&book);
                this.note_write(Some(book));
                cx.notify();
                // Dropped: the session ended (or the call panicked).
                Some(match result? {
                    Ok(value) => apply(this, value, cx),
                    Err(e) => Err(this.session.error_message(&e, Some(scope), cx).into()),
                })
            })
            .ok()
            .flatten()
        })
    }

    /// Puts `book` on `list`, as `edition` and at `position` if given (Undo
    /// puts a removed book back as the edition it was, at its place on a
    /// ranked list).
    pub fn add(
        &mut self,
        list: ListId,
        book: BookId,
        edition: Option<EditionId>,
        position: Option<i32>,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        self.write_book(
            book,
            WRITE_LISTS,
            move |client| client.add_to_list(list, book, edition, position),
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
    ) -> Task<Option<Result<(), SharedString>>> {
        let Some(rows) = self.rows_for(book) else {
            return Task::ready(Some(Err("Your lists are still loading.".into())));
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
    ) -> Task<Option<Result<(), SharedString>>> {
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
                    Some(e) => Err(this.session.error_message(&e, Some(WRITE_LISTS), cx).into()),
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
    ) -> Task<Option<Result<(), SharedString>>> {
        if !owned {
            let (Some(list), Some(rows)) = (self.owned_list().map(|l| l.id), self.owned_rows(book))
            else {
                return Task::ready(Some(Err("Your Owned list is still loading.".into())));
            };
            return self.remove_rows(list, book, rows.iter().map(|r| r.id).collect(), cx);
        }
        let Some(edition) = edition else {
            return Task::ready(Some(Err(
                "Choose an edition first. Hardcover marks editions as owned, not books.".into(),
            )));
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
                            this.reload_dropped
                                .schedule(RELOAD_DELAY, Self::load_dropped, cx);
                        }
                    }
                    // `Reloaded` also has the Owned list's page load again.
                    None => this.reload_with(Demand::Now, cx),
                }
                Ok(())
            },
            cx,
        )
    }
}

/// Whether a list's count can't be right, given its books as fetched:
/// `sent` rows, all of them when `complete`, else at least that many.
fn count_disagrees(count: u32, sent: u32, complete: bool) -> bool {
    if complete {
        sent != count
    } else {
        sent > count
    }
}

/// Whether a list's page has nothing left to show ([`ListsModel::is_gone`]).
fn is_gone(status: &LoadStatus, listed: bool, readable: bool) -> bool {
    match status {
        _ if !readable => true,
        LoadStatus::Idle => true,
        LoadStatus::Loaded => !listed,
        LoadStatus::Loading | LoadStatus::Failed(_) => false,
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

    fn session() -> SessionScope {
        SessionScope::for_tests(hardcover_core::demo::client(), None)
    }

    fn draft(name: &str) -> ListDraft {
        ListDraft {
            name: name.to_owned(),
            description: String::new(),
            privacy: None,
            ranked: false,
        }
    }

    /// A list write that lands after signing out is dropped: nothing to
    /// show, rather than an error about a closed window.
    #[gpui_kit::test]
    fn a_write_landing_after_sign_out_is_dropped(cx: &mut gpui_kit::TestAppContext) {
        let session = session();
        let model = cx.new(|_| ListsModel::new(session.clone()));
        // The call runs inline; its result waits to be picked up.
        let task = model.update(cx, |m, cx| m.create(draft("Picks"), None, cx));
        session.end();
        let task = cx.foreground_executor().spawn(task);
        cx.run_until_parked();
        assert_eq!(gpui_kit::block_on(task), None);
        assert!(model.read_with(cx, |m, _| m.lists().is_empty()));

        // And nothing is sent once it ended.
        let task = model.update(cx, |m, cx| m.create(draft("More"), None, cx));
        assert_eq!(gpui_kit::block_on(task), None);
    }

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

    fn index(rows: Vec<ListMembership>, complete: bool) -> ListMemberships {
        ListMemberships { rows, complete }
    }

    #[test]
    fn membership_is_unknown_until_the_index_says() {
        let mut m = ListsModel::new(session());
        m.lists.hydrate(
            (),
            vec![list(1, "Owned", "owned"), list(2, "Picks", "picks")],
        );
        let book = BookId::new(10);
        assert_eq!(m.is_on(ListId::new(2), book), None);

        m.index.hydrate(
            (),
            index(
                vec![row(100, 1, 10), row(101, 1, 10), row(102, 2, 11)],
                true,
            ),
        );
        assert_eq!(m.is_on(ListId::new(2), book), Some(false));
        assert_eq!(m.owned_rows(book).map(|r| r.len()), Some(2));
        assert_eq!(m.owned_rows(BookId::new(11)).map(|r| r.len()), Some(0));

        // A cut-short index can't say "not on it"; a lookup of the book can.
        m.index.update(&(), |i| i.complete = false);
        assert_eq!(m.is_on(ListId::new(2), book), None);
        m.book_rows.hydrate(book, vec![row(103, 2, 10)]);
        assert_eq!(m.is_on(ListId::new(2), book), Some(true));
    }

    #[test]
    fn a_count_disagrees_with_books_that_differ() {
        // All the books loaded: the count must match them.
        assert!(count_disagrees(144, 26, true));
        assert!(count_disagrees(3, 4, true));
        assert!(!count_disagrees(26, 26, true));
        // More to come: only more than the count can't be right.
        assert!(!count_disagrees(144, 100, false));
        assert!(count_disagrees(80, 100, false));
    }

    #[test]
    fn local_rows_keep_counts_without_counting_twice() {
        let mut m = ListsModel::new(session());
        m.lists.hydrate((), vec![list(2, "Picks", "picks")]);
        m.index.hydrate((), index(Vec::new(), true));
        let count = |m: &ListsModel| m.lists()[0].books_count;
        m.insert_row(row(100, 2, 10));
        // Adding a book already on the list returns the same row.
        m.insert_row(row(100, 2, 10));
        assert_eq!(count(&m), 1);
        assert_eq!(m.is_on(ListId::new(2), BookId::new(10)), Some(true));
        m.remove_row(ListId::new(2), ListBookId::new(100));
        assert_eq!(count(&m), 0);
        assert_eq!(m.is_on(ListId::new(2), BookId::new(10)), Some(false));
        // Never below zero.
        m.remove_row(ListId::new(2), ListBookId::new(100));
        assert_eq!(count(&m), 0);
    }

    #[test]
    fn a_row_known_from_the_book_lookup_is_not_counted_again() {
        let mut m = ListsModel::new(session());
        let mut picks = list(2, "Picks", "picks");
        picks.books_count = 5;
        m.lists.hydrate((), vec![picks]);
        // The index was cut short before this book; its own lookup has it.
        m.index.hydrate((), index(Vec::new(), false));
        m.book_rows.hydrate(BookId::new(10), vec![row(100, 2, 10)]);
        // Adding it again returns the same row.
        m.insert_row(row(100, 2, 10));
        assert_eq!(m.lists()[0].books_count, 5);
        assert_eq!(m.index.value(&()).unwrap().rows, [row(100, 2, 10)]);
        // A row neither holds is new.
        m.insert_row(row(101, 2, 11));
        assert_eq!(m.lists()[0].books_count, 6);
    }

    #[test]
    fn books_asked_about_early_are_looked_up_once_a_cut_short_index_lands() {
        let mut m = ListsModel::new(session());
        let (a, b) = (BookId::new(10), BookId::new(11));
        m.pending_books.extend([a, b]);
        // No index yet (still loading, or it failed): they keep waiting.
        assert_eq!(m.take_lookups(), []);
        assert_eq!(m.pending_books.len(), 2);

        m.index.hydrate((), index(Vec::new(), false));
        let mut books = m.take_lookups();
        books.sort();
        assert_eq!(books, [a, b]);
        assert!(m.pending_books.is_empty());

        // A complete index answers for every book.
        m.pending_books.insert(a);
        m.index.update(&(), |i| i.complete = true);
        assert_eq!(m.take_lookups(), []);
        assert!(m.pending_books.is_empty());
    }

    #[test]
    fn reload_skips_an_index_nobody_needed_and_looks_the_shown_book_up_again() {
        let mut m = ListsModel::new(session());
        assert!(!m.plan_reload());

        let book = BookId::new(10);
        m.shown_book = Some(book);
        m.index.hydrate((), index(Vec::new(), false));
        m.book_rows.hydrate(book, vec![row(100, 2, 10)]);
        assert!(m.plan_reload());
        // Still answering until it is looked up again.
        assert!(m.book_rows.value(&book).is_some());
        assert!(m.pending_books.contains(&book));
    }

    #[test]
    fn the_first_owned_edition_shows_owned_before_the_lists_reload() {
        let mut m = ListsModel::new(session());
        m.lists.hydrate((), vec![list(2, "Picks", "picks")]);
        m.index.hydrate((), index(Vec::new(), true));
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
        assert_eq!(m.lists().len(), 2);
    }

    #[test]
    fn the_first_owned_edition_adds_nothing_while_the_lists_are_unknown() {
        let mut m = ListsModel::new(session());
        assert!(!m.add_owned_placeholder(ListId::new(7)));
        assert_eq!(m.lists(), []);
    }

    #[test]
    fn a_list_page_gives_way_when_the_list_is_gone_or_unreadable() {
        let failed = LoadStatus::Failed("offline".into());
        assert!(is_gone(&LoadStatus::Idle, false, true));
        assert!(!is_gone(&LoadStatus::Loading, false, true));
        assert!(!is_gone(&failed, false, true));
        assert!(is_gone(&LoadStatus::Loaded, false, true));
        assert!(!is_gone(&LoadStatus::Loaded, true, true));
        // A refused scope.
        assert!(is_gone(&LoadStatus::Loaded, true, false));
    }

    /// D14: after the lists fail to load, the Owned switch says so (with a
    /// way to try again) instead of checking for ever.
    #[gpui_kit::test]
    fn a_failed_load_is_said_beside_the_book_and_tried_again(cx: &mut gpui_kit::TestAppContext) {
        let faults = hardcover_core::demo::Faults::parse("500").unwrap();
        let session = SessionScope::for_tests(hardcover_core::demo::client_with(&faults), None);
        let model = cx.new(|_| ListsModel::new(session));
        let book = BookId::new(10);
        model.update(cx, |m, cx| {
            m.load(cx);
            m.ensure_book(book, cx);
        });
        cx.run_until_parked();
        let failure = model.read_with(cx, |m, _| {
            assert_eq!(m.owned_rows(book), None);
            m.book_failure(book)
        });
        assert!(failure.is_some(), "said, not checking for ever");

        model.update(cx, |m, cx| m.retry_book(book, cx));
        model.read_with(cx, |m, _| {
            assert_eq!(m.lists.started(), 2);
            assert_eq!(m.index.started(), 2);
        });
    }

    /// A reload a write asks for (the Owned list changed in ways its result
    /// doesn't say) goes out however fresh the lists; one for a new grant
    /// only fetches what is stale.
    #[gpui_kit::test]
    fn a_write_reloads_however_fresh(cx: &mut gpui_kit::TestAppContext) {
        let model = cx.new(|_| ListsModel::new(session()));
        model.update(cx, |m, cx| {
            m.load(cx);
            m.ensure_book(BookId::new(10), cx);
        });
        cx.run_until_parked();
        let asked = |cx: &mut gpui_kit::TestAppContext| {
            model.read_with(cx, |m, _| (m.lists.started(), m.index.started()))
        };
        let before = asked(cx);
        model.update(cx, |m, cx| m.reload(cx));
        assert_eq!(asked(cx), before, "fresh: nothing");
        model.update(cx, |m, cx| m.reload_with(Demand::Now, cx));
        assert_eq!(asked(cx), (before.0 + 1, before.1 + 1));
    }

    /// B7: offline, the lists and the index of the last launch show.
    #[gpui_kit::test]
    fn the_last_lists_show_offline(cx: &mut gpui_kit::TestAppContext) {
        use hardcover_core::store::LibraryCache;
        use std::sync::Arc;
        let cache = Arc::new(LibraryCache::open_in_memory().unwrap());
        let launch = |client, cx: &mut gpui_kit::TestAppContext| {
            let session = SessionScope::for_tests(client, Some(cache.clone()));
            cx.update(|cx| session.install(cx));
            let library = cx.new(|_| LibraryModel::new(session.clone()));
            let model = cx.update(|cx| ListsModel::init(session, &library, cx));
            model.update(cx, |m, cx| {
                m.load(cx);
                m.ensure_book(BookId::new(1), cx);
            });
            cx.run_until_parked();
            model
        };
        let online = launch(hardcover_core::demo::client(), cx);
        let (lists, index) =
            online.read_with(cx, |m, _| (m.lists().to_vec(), m.index.value(&()).cloned()));
        assert!(!lists.is_empty());
        assert!(index.is_some());

        let faults = hardcover_core::demo::Faults::parse("offline").unwrap();
        let offline = launch(hardcover_core::demo::client_with(&faults), cx);
        offline.read_with(cx, |m, _| {
            assert_eq!(m.lists(), lists.as_slice());
            assert_eq!(m.index.value(&()).cloned(), index);
            assert!(matches!(m.status(), LoadStatus::Failed(_)));
        });
    }
}
