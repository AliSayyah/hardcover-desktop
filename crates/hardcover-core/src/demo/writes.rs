//! The demo's mutations: each changes the client's [`State`] and answers in
//! the shape Hardcover answers it.
//!
//! The behaviour follows what CLAUDE.md records as verified: a resent status
//! changes nothing; a rating without a status moves the book to Read; moving
//! to Currently Reading starts a read and moving to Read finishes the open
//! one, on the change's `user_date` if it has one (else today); progress
//! that reaches the end finishes the read and marks the book
//! Read; only an empty Slate document deletes a review, and `reviewed_at`
//! stays; adding a book already on a list returns its row; an empty list
//! description clears it; `delete_goal` answers with `id: null`.
//!
//! Assumed where it is unverified: adding a book that is already in the
//! library is refused; moving to Read with no open read adds a read finished
//! today; a new read without an edition takes the entry's; a goal counts
//! the reads finished within its dates, both included; a book put on a
//! ranked list at a `position` takes it, and no other row moves; a book
//! left out of a vibe ("Never recommend…") goes from its books at once,
//! where Hardcover drops it only at the next build (see `vibes`).

use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use super::state::{Entry, Goal, Length, List, ListRow, Read, Review, State, read_length};
use super::{CATALOGUE, list, review_fields, slug, user_book};

const WANT_TO_READ: i64 = 1;
const CURRENTLY_READING: i64 = 2;
const READ: i64 = 3;

/// Every mutation the client sends.
pub(super) fn is_write(op: &str) -> bool {
    matches!(
        op,
        "InsertUserBook"
            | "UpdateUserBook"
            | "DeleteUserBook"
            | "InsertUserBookRead"
            | "UpdateUserBookRead"
            | "DeleteUserBookRead"
            | "EditionOwned"
            | "UpdateUserBookReview"
            | "DeleteUserBookReview"
            | "InsertGoal"
            | "UpdateGoal"
            | "DeleteGoal"
            | "InsertList"
            | "UpdateList"
            | "DeleteList"
            | "InsertListBook"
            | "DeleteListBook"
            | "EnsureSystemVibes"
            | "ExcludeFromVibe"
    )
}

/// The `data` of a mutation's response, or a GraphQL error's message.
pub(super) fn apply(state: &mut State, op: &str, vars: &Value) -> Result<Value, String> {
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    match op {
        "InsertUserBook" => insert_user_book(state, &vars["object"]),
        "UpdateUserBook" => update_user_book(state, int("id"), &vars["object"]),
        "DeleteUserBook" => Ok(delete_user_book(state, int("id"))),
        "InsertUserBookRead" => {
            write_read(state, ReadTarget::New(int("userBookId")), &vars["read"])
                .map(|payload| json!({ "insert_user_book_read": payload }))
        }
        "UpdateUserBookRead" => write_read(state, ReadTarget::Existing(int("id")), &vars["read"])
            .map(|payload| json!({ "update_user_book_read": payload })),
        "DeleteUserBookRead" => Ok(delete_read(state, int("id"))),
        "UpdateUserBookReview" => update_review(state, vars),
        "DeleteUserBookReview" => delete_review(state, vars),
        "EditionOwned" => edition_owned(state, int("id")),
        "InsertList" => Ok(insert_list(state, &vars["object"])),
        "UpdateList" => update_list(state, int("id"), &vars["object"]),
        "DeleteList" => Ok(delete_list(state, int("id"))),
        "InsertListBook" => insert_list_book(state, &vars["object"]),
        "DeleteListBook" => Ok(delete_list_book(state, int("id"))),
        "InsertGoal" => insert_goal(state, &vars["object"]),
        "UpdateGoal" => update_goal(state, int("id"), &vars["object"]),
        "DeleteGoal" => Ok(delete_goal(state, int("id"))),
        "EnsureSystemVibes" => Ok(super::vibes::ensure_system_vibes(state)),
        "ExcludeFromVibe" => super::vibes::update_vibe(state, int("id"), &vars["object"]),
        other => Err(format!("The demo has no answer for {other}.")),
    }
}

// ----- input helpers -----

pub(super) fn object(value: &Value) -> Result<&Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| "expected an input object".to_owned())
}

/// Refuses a field the demo doesn't write, so a new one isn't silently lost.
pub(super) fn only(object: &Map<String, Value>, known: &[&str]) -> Result<(), String> {
    match object.keys().find(|k| !known.contains(&k.as_str())) {
        Some(k) => Err(format!("The demo doesn't write `{k}`.")),
        None => Ok(()),
    }
}

fn date(value: &Value) -> Result<Option<NaiveDate>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => s
            .get(..10)
            .and_then(|d| d.parse().ok())
            .map(Some)
            .ok_or_else(|| format!("{s:?} is not a date")),
        other => Err(format!("{other} is not a date")),
    }
}

fn count(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// A text column: blank is none.
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

// ----- library -----

/// The payload of `insert_user_book` and `update_user_book`.
fn user_book_payload(entry: &Entry) -> Value {
    json!({ "error": null, "id": entry.id, "user_book": user_book(entry) })
}

fn refused(message: &str) -> Value {
    json!({ "error": message, "id": null, "user_book": null })
}

fn insert_user_book(state: &mut State, object: &Value) -> Result<Value, String> {
    let object = self::object(object)?;
    only(
        object,
        &["book_id", "status_id", "privacy_setting_id", "user_date"],
    )?;
    let day = user_date(object)?;
    let book = object
        .get("book_id")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let status = object
        .get("status_id")
        .and_then(Value::as_i64)
        .unwrap_or(WANT_TO_READ);
    if CATALOGUE.book(book).is_none() && CATALOGUE.upcoming(book).is_none() {
        return Ok(json!({ "insert_user_book": refused("Book not found.") }));
    }
    if state.library.iter().any(|e| e.book == book) {
        return Ok(json!({
            "insert_user_book": refused("This book is already in your library."),
        }));
    }
    let id = state.new_id();
    let now = state.clock.now();
    let mut entry = Entry {
        id,
        book,
        status: 0,
        rating: None,
        date_added: now.date_naive(),
        updated_at: now.to_utc(),
        privacy: object
            .get("privacy_setting_id")
            .and_then(Value::as_i64)
            .unwrap_or(1),
        private_notes: None,
        edition: None,
        reads: Vec::new(),
        review: Review::default(),
    };
    let read_id = state.new_id();
    set_status(&mut entry, status, day.unwrap_or(now.date_naive()), read_id);
    let payload = user_book_payload(&entry);
    state.library.push(entry);
    Ok(json!({ "insert_user_book": payload }))
}

fn update_user_book(state: &mut State, id: i64, object: &Value) -> Result<Value, String> {
    let object = self::object(object)?;
    only(
        object,
        &[
            "status_id",
            "rating",
            "edition_id",
            "private_notes",
            "privacy_setting_id",
            "user_date",
        ],
    )?;
    let day = user_date(object)?;
    let read_id = state.new_id();
    let now = state.clock.now();
    let Some(entry) = state.entry_mut(id) else {
        return Ok(json!({ "update_user_book": refused("Library entry not found.") }));
    };
    if let Some(status) = object.get("status_id").and_then(Value::as_i64) {
        set_status(entry, status, day.unwrap_or(now.date_naive()), read_id);
    }
    if let Some(rating) = object.get("rating") {
        entry.rating = rating.as_f64();
        // A bare rating means "I read this".
        if entry.rating.is_some() && !object.contains_key("status_id") {
            set_status(entry, READ, now.date_naive(), read_id);
        }
    }
    if let Some(edition) = object.get("edition_id") {
        entry.edition = edition.as_i64();
    }
    if let Some(notes) = object.get("private_notes") {
        entry.private_notes = text(notes);
    }
    if let Some(privacy) = object.get("privacy_setting_id").and_then(Value::as_i64) {
        entry.privacy = privacy;
    }
    entry.updated_at = now.to_utc();
    Ok(json!({ "update_user_book": user_book_payload(entry) }))
}

/// A status change's `user_date`: the day of the read it starts or finishes.
fn user_date(object: &Map<String, Value>) -> Result<Option<NaiveDate>, String> {
    match object.get("user_date") {
        None | Some(Value::Null) => Ok(None),
        Some(day) => day
            .as_str()
            .and_then(|d| d.parse().ok())
            .map(Some)
            .ok_or_else(|| format!("`user_date` isn't a date: {day}")),
    }
}

/// Moves `entry` to `status`. Currently Reading starts a read on `today`;
/// Read finishes the open read on it, at its end, or adds one finished then.
/// The same status changes nothing.
fn set_status(entry: &mut Entry, status: i64, today: NaiveDate, new_read: i64) {
    if entry.status == status {
        return;
    }
    entry.status = status;
    match status {
        CURRENTLY_READING => entry.reads.push(Read {
            id: new_read,
            started: Some(today),
            finished: None,
            progress_pages: None,
            progress_seconds: None,
            edition: entry.edition,
        }),
        READ => match open_read(entry) {
            Some(ix) => finish(entry, ix, today),
            None => entry.reads.push(Read {
                id: new_read,
                started: None,
                finished: Some(today),
                progress_pages: None,
                progress_seconds: None,
                edition: entry.edition,
            }),
        },
        _ => {}
    }
}

/// The newest unfinished read.
fn open_read(entry: &Entry) -> Option<usize> {
    entry
        .reads
        .iter()
        .enumerate()
        .filter(|(_, r)| r.finished.is_none())
        .max_by_key(|(_, r)| r.id)
        .map(|(ix, _)| ix)
}

/// Finishes read `ix` today, its progress at the end.
fn finish(entry: &mut Entry, ix: usize, today: NaiveDate) {
    let length = read_length(entry, &entry.reads[ix]);
    let read = &mut entry.reads[ix];
    read.finished = Some(today);
    match length {
        Some(Length::Pages(n)) => read.progress_pages = Some(n),
        Some(Length::Seconds(n)) => read.progress_seconds = Some(n),
        None => {}
    }
}

fn delete_user_book(state: &mut State, id: i64) -> Value {
    let Some(ix) = state.library.iter().position(|e| e.id == id) else {
        return json!({ "delete_user_book": null });
    };
    let entry = state.library.remove(ix);
    json!({ "delete_user_book": {
        "id": entry.id,
        "book_id": entry.book,
        "user_id": CATALOGUE.user.id,
    }})
}

enum ReadTarget {
    /// A new read of this entry.
    New(i64),
    /// This read.
    Existing(i64),
}

/// `insert_user_book_read` or `update_user_book_read`: the written read with
/// the whole entry nested, as the documents ask.
fn write_read(state: &mut State, target: ReadTarget, fields: &Value) -> Result<Value, String> {
    let fields = object(fields)?;
    only(
        fields,
        &[
            "started_at",
            "finished_at",
            "progress_pages",
            "progress_seconds",
            "edition_id",
        ],
    )?;
    let new_id = state.new_id();
    let now = state.clock.now();
    let found = match target {
        ReadTarget::New(entry) => state.entry_mut(entry).map(|e| {
            let edition = e.edition;
            e.reads.push(Read {
                id: new_id,
                started: None,
                finished: None,
                progress_pages: None,
                progress_seconds: None,
                edition,
            });
            (e, new_id)
        }),
        ReadTarget::Existing(read) => state
            .library
            .iter_mut()
            .find(|e| e.reads.iter().any(|r| r.id == read))
            .map(|e| (e, read)),
    };
    let Some((entry, read_id)) = found else {
        return Ok(json!({ "error": "Read not found.", "id": null, "user_book_read": null }));
    };
    let ix = entry
        .reads
        .iter()
        .position(|r| r.id == read_id)
        .expect("the read was just found");
    {
        let read = &mut entry.reads[ix];
        for (key, value) in fields {
            match key.as_str() {
                "started_at" => read.started = date(value)?,
                "finished_at" => read.finished = date(value)?,
                "progress_pages" => read.progress_pages = count(value),
                "progress_seconds" => read.progress_seconds = count(value),
                "edition_id" => read.edition = value.as_i64(),
                _ => unreachable!("checked by `only`"),
            }
        }
    }
    // Progress at the end finishes the read and marks the book Read.
    let read = &entry.reads[ix];
    let at_end = read.finished.is_none()
        && match read_length(entry, read) {
            Some(Length::Pages(n)) => read.progress_pages.is_some_and(|p| p >= n),
            Some(Length::Seconds(n)) => read.progress_seconds.is_some_and(|s| s >= n),
            None => false,
        };
    if at_end {
        entry.reads[ix].finished = Some(now.date_naive());
        entry.status = READ;
    }
    entry.updated_at = now.to_utc();
    let read = &entry.reads[ix];
    let mut written = super::read(entry, read);
    written["user_book_id"] = json!(entry.id);
    written["user_book"] = user_book(entry).unwrap_or(Value::Null);
    Ok(json!({ "error": null, "id": read.id, "user_book_read": written }))
}

fn delete_read(state: &mut State, id: i64) -> Value {
    let now = state.clock.now().to_utc();
    let deleted = state.library.iter_mut().find_map(|e| {
        let ix = e.reads.iter().position(|r| r.id == id)?;
        e.reads.remove(ix);
        e.updated_at = now;
        Some(id)
    });
    match deleted {
        Some(id) => json!({ "delete_user_book_read": { "error": null, "id": id } }),
        None => json!({ "delete_user_book_read": { "error": "Read not found.", "id": null } }),
    }
}

// ----- reviews -----

/// The payload of the review mutations: the entry and its review.
fn reviewed_payload(entry: &Entry) -> Value {
    let mut user_book = user_book(entry).unwrap_or(Value::Null);
    if let (Some(map), Value::Object(review)) = (user_book.as_object_mut(), review_fields(entry)) {
        map.extend(review);
    }
    json!({ "update_user_book": { "error": null, "id": entry.id, "user_book": user_book } })
}

fn update_review(state: &mut State, vars: &Value) -> Result<Value, String> {
    let read_id = state.new_id();
    let now = state.clock.now();
    let Some(entry) = state.entry_mut(vars["id"].as_i64().unwrap_or_default()) else {
        return Ok(json!({ "update_user_book": refused("Library entry not found.") }));
    };
    if let Some(status) = vars["statusId"].as_i64() {
        set_status(entry, status, now.date_naive(), read_id);
    }
    // Hardcover ignores null or empty Markdown and keeps the review.
    if let Some(markdown) = text(&vars["markdown"]) {
        entry.review.raw = Some(markdown.clone());
        entry.review.markdown = Some(markdown);
        entry.review.spoilers = vars["spoilers"].as_bool().unwrap_or(false);
        entry.review.reviewed_at = date(&vars["reviewedAt"])?
            .or(entry.review.reviewed_at)
            .or(Some(now.date_naive()));
    }
    entry.updated_at = now.to_utc();
    Ok(reviewed_payload(entry))
}

/// Only an empty Slate document deletes the review; null or `{}` is ignored.
fn delete_review(state: &mut State, vars: &Value) -> Result<Value, String> {
    let read_id = state.new_id();
    let now = state.clock.now();
    let Some(entry) = state.entry_mut(vars["id"].as_i64().unwrap_or_default()) else {
        return Ok(json!({ "update_user_book": refused("Library entry not found.") }));
    };
    if let Some(status) = vars["statusId"].as_i64() {
        set_status(entry, status, now.date_naive(), read_id);
    }
    if is_empty_slate(&vars["slate"]) {
        entry.review.markdown = None;
        entry.review.raw = Some(String::new());
        entry.review.spoilers = false;
    }
    entry.updated_at = now.to_utc();
    Ok(reviewed_payload(entry))
}

/// A Slate document whose every text node is empty.
fn is_empty_slate(slate: &Value) -> bool {
    fn texts(node: &Value, out: &mut Vec<String>) {
        if let Some(text) = node.get("text").and_then(Value::as_str) {
            out.push(text.to_owned());
        }
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            texts(child, out);
        }
    }
    let Some(document) = slate
        .get("document")
        .filter(|d| d.get("children").is_some())
    else {
        return false;
    };
    let mut found = Vec::new();
    texts(document, &mut found);
    found.iter().all(|t| t.trim().is_empty())
}

// ----- lists -----

fn list_book(list: i64, row: &ListRow) -> Value {
    json!({
        "id": row.id,
        "list_id": list,
        "book_id": row.book,
        "edition_id": row.edition,
    })
}

/// Puts an edition on the Owned list, creating the list the first time.
fn edition_owned(state: &mut State, edition: i64) -> Result<Value, String> {
    let Some((book, _)) = CATALOGUE.edition(edition) else {
        return Err(format!("Edition {edition} not found."));
    };
    let book = book.id;
    if !state.lists.iter().any(List::is_owned) {
        let id = state.new_id();
        state.lists.push(new_list(id, "Owned", None, false, 1));
    }
    let row_id = state.new_id();
    let owned = state
        .lists
        .iter_mut()
        .find(|l| l.is_owned())
        .expect("the Owned list was just made");
    if let Some(row) = owned.rows.iter().find(|r| r.edition == Some(edition)) {
        return Ok(
            json!({ "edition_owned": { "id": row.id, "list_book": list_book(owned.id, row) } }),
        );
    }
    owned.rows.push(ListRow {
        id: row_id,
        book,
        edition: Some(edition),
        position: None,
    });
    let row = owned.rows.last().expect("just pushed");
    Ok(json!({ "edition_owned": { "id": row.id, "list_book": list_book(owned.id, row) } }))
}

fn new_list(id: i64, name: &str, description: Option<String>, ranked: bool, privacy: i64) -> List {
    List {
        id,
        name: name.to_owned(),
        slug: slug(name),
        description,
        privacy,
        ranked,
        default_view: "list".to_owned(),
        featured_profile: false,
        url: None,
        rows: Vec::new(),
    }
}

fn list_payload(key: &str, l: &List) -> Value {
    json!({ key: { "errors": null, "id": l.id, "list": list(l) } })
}

const LIST_FIELDS: [&str; 7] = [
    "name",
    "description",
    "ranked",
    "privacy_setting_id",
    "default_view",
    "featured_profile",
    "url",
];

fn insert_list(state: &mut State, object: &Value) -> Value {
    let Ok(object) = self::object(object) else {
        return json!({ "insert_list": { "errors": ["expected a list"], "id": null, "list": null } });
    };
    if let Err(e) = only(object, &LIST_FIELDS) {
        return json!({ "insert_list": { "errors": [e], "id": null, "list": null } });
    }
    let Some(name) = object.get("name").and_then(text) else {
        return json!({ "insert_list": { "errors": ["Name can't be blank"], "id": null, "list": null } });
    };
    let id = state.new_id();
    let mut l = new_list(id, &name, None, false, 1);
    edit_list(&mut l, object);
    let payload = list_payload("insert_list", &l);
    state.lists.push(l);
    payload
}

fn update_list(state: &mut State, id: i64, object: &Value) -> Result<Value, String> {
    let object = self::object(object)?;
    if let Err(e) = only(object, &LIST_FIELDS) {
        return Ok(json!({ "update_list": { "errors": [e], "id": null, "list": null } }));
    }
    let Some(l) = state.lists.iter_mut().find(|l| l.id == id) else {
        return Ok(
            json!({ "update_list": { "errors": ["List not found"], "id": null, "list": null } }),
        );
    };
    edit_list(l, object);
    Ok(list_payload("update_list", l))
}

/// Sets the fields `object` has. An empty description clears it.
fn edit_list(l: &mut List, object: &Map<String, Value>) {
    for (key, value) in object {
        match key.as_str() {
            "name" => {
                if let Some(name) = text(value) {
                    l.name = name;
                }
            }
            "description" => l.description = text(value),
            "ranked" => l.ranked = value.as_bool().unwrap_or(l.ranked),
            "privacy_setting_id" => l.privacy = value.as_i64().unwrap_or(l.privacy),
            "default_view" => {
                if let Some(view) = value.as_str() {
                    l.default_view = view.to_owned();
                }
            }
            "featured_profile" => l.featured_profile = value.as_bool().unwrap_or(false),
            "url" => l.url = text(value),
            _ => {}
        }
    }
    // A list turned ranked numbers its books in their order.
    if l.ranked {
        let mut next = l.rows.iter().filter_map(|r| r.position).max().unwrap_or(0);
        for row in l.rows.iter_mut().filter(|r| r.position.is_none()) {
            next += 1;
            row.position = Some(next);
        }
    }
}

fn delete_list(state: &mut State, id: i64) -> Value {
    let before = state.lists.len();
    state.lists.retain(|l| l.id != id);
    json!({ "delete_list": { "success": state.lists.len() < before } })
}

/// Adds a book to a list; one already there returns its row. On a ranked
/// list it goes at the `position` given, else after the last; the other
/// rows keep theirs (unverified on Hardcover).
fn insert_list_book(state: &mut State, object: &Value) -> Result<Value, String> {
    let object = self::object(object)?;
    only(object, &["list_id", "book_id", "edition_id", "position"])?;
    let asked = object.get("position").and_then(Value::as_i64);
    let list_id = object
        .get("list_id")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let book = object
        .get("book_id")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let edition = object.get("edition_id").and_then(Value::as_i64);
    if CATALOGUE.book(book).is_none() {
        return Err(format!("Book {book} not found."));
    }
    let row_id = state.new_id();
    let Some(l) = state.lists.iter_mut().find(|l| l.id == list_id) else {
        return Err(format!("List {list_id} not found."));
    };
    if let Some(row) = l
        .rows
        .iter()
        .find(|r| r.book == book && (edition.is_none() || r.edition == edition))
    {
        return Ok(
            json!({ "insert_list_book": { "id": row.id, "list_book": list_book(l.id, row) } }),
        );
    }
    let position = l.ranked.then(|| {
        asked.unwrap_or_else(|| l.rows.iter().filter_map(|r| r.position).max().unwrap_or(0) + 1)
    });
    l.rows.push(ListRow {
        id: row_id,
        book,
        edition,
        position,
    });
    let row = l.rows.last().expect("just pushed");
    Ok(json!({ "insert_list_book": { "id": row.id, "list_book": list_book(l.id, row) } }))
}

fn delete_list_book(state: &mut State, id: i64) -> Value {
    let found = state.lists.iter_mut().find_map(|l| {
        let ix = l.rows.iter().position(|r| r.id == id)?;
        l.rows.remove(ix);
        Some(l.id)
    });
    match found {
        Some(list) => json!({ "delete_list_book": { "id": id, "list_id": list } }),
        None => json!({ "delete_list_book": { "id": null, "list_id": null } }),
    }
}

// ----- goals -----

const GOAL_FIELDS: [&str; 8] = [
    "metric",
    "goal",
    "description",
    "start_date",
    "end_date",
    "conditions",
    "archived",
    "privacy_setting_id",
];

/// A whole `GoalInput`: fields left out are not kept.
fn goal_from(id: i64, object: &Map<String, Value>) -> Result<Goal, String> {
    only(object, &GOAL_FIELDS)?;
    let metric = object
        .get("metric")
        .and_then(Value::as_str)
        .filter(|m| matches!(*m, "book" | "page" | "hour"))
        .ok_or("a goal needs a metric: book, page or hour")?;
    let goal = object
        .get("goal")
        .and_then(Value::as_i64)
        .filter(|g| *g > 0)
        .ok_or("a goal needs a target above zero")?;
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .ok_or("a goal needs a description")?;
    let start = date(object.get("start_date").unwrap_or(&Value::Null))?
        .ok_or("a goal needs a start date")?;
    let end =
        date(object.get("end_date").unwrap_or(&Value::Null))?.ok_or("a goal needs an end date")?;
    Ok(Goal {
        id,
        metric: metric.to_owned(),
        goal,
        description: description.to_owned(),
        start,
        end,
        conditions: object.get("conditions").cloned().unwrap_or(json!({})),
        archived: object
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        privacy: object
            .get("privacy_setting_id")
            .and_then(Value::as_i64)
            .unwrap_or(1),
    })
}

fn goal_errors(key: &str, message: &str) -> Value {
    json!({ key: { "errors": [message], "id": null, "goal": null } })
}

fn insert_goal(state: &mut State, object: &Value) -> Result<Value, String> {
    let id = state.new_id();
    let goal = match goal_from(id, self::object(object)?) {
        Ok(goal) => goal,
        Err(e) => return Ok(goal_errors("insert_goal", &e)),
    };
    let payload =
        json!({ "insert_goal": { "errors": null, "id": id, "goal": state.goal_json(&goal) } });
    state.goals.push(goal);
    Ok(payload)
}

fn update_goal(state: &mut State, id: i64, object: &Value) -> Result<Value, String> {
    let Some(ix) = state.goals.iter().position(|g| g.id == id) else {
        return Ok(goal_errors("update_goal", "Goal not found"));
    };
    let goal = match goal_from(id, self::object(object)?) {
        Ok(goal) => goal,
        Err(e) => return Ok(goal_errors("update_goal", &e)),
    };
    state.goals[ix] = goal;
    let payload = state.goal_json(&state.goals[ix]);
    Ok(json!({ "update_goal": { "errors": null, "id": id, "goal": payload } }))
}

/// Hardcover answers a delete with `id: null`, whatever happened.
fn delete_goal(state: &mut State, id: i64) -> Value {
    let before = state.goals.len();
    state.goals.retain(|g| g.id != id);
    if state.goals.len() == before {
        return goal_errors("delete_goal", "Goal not found");
    }
    json!({ "delete_goal": { "errors": null, "id": null } })
}
