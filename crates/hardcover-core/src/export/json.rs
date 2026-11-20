//! The full JSON backup.
//!
//! Its shape is spelled out here, apart from the models' own serde forms, so a
//! change to a model can't silently change version 1. Change the shape only
//! with a new [`BACKUP_VERSION`]. Dates are `YYYY-MM-DD`, times RFC 3339 in
//! UTC; absent values are `null`.

use serde::Serialize;
use serde_json::Value;

use super::Backup;
use crate::api::{
    Date, EditionSummary, Goal, LibraryEntry, ListBook, Privacy, ReadSession, Review, SeriesRef,
    Timestamp, UserList,
};

/// The backup's `format` field.
pub const BACKUP_FORMAT: &str = "hardcover-desktop-backup";
/// The backup's `version` field.
pub const BACKUP_VERSION: u32 = 1;

/// The backup, pretty-printed, ending in a newline. Library entries go by
/// entry id; lists by id, each with its books in the list's order; goals as
/// Hardcover lists them (the latest end first).
pub fn backup_json(backup: &Backup) -> String {
    let mut entries: Vec<&LibraryEntry> = backup.entries.iter().collect();
    entries.sort_by_key(|e| e.user_book_id);
    let lists = backup.lists.as_ref().map(|lists| {
        let mut lists: Vec<_> = lists.iter().collect();
        lists.sort_by_key(|l| l.list.id);
        lists.into_iter().map(|l| list(&l.list, &l.books)).collect()
    });
    let document = Document {
        format: BACKUP_FORMAT,
        version: BACKUP_VERSION,
        exported_at: backup.exported_at,
        generator: concat!("hardcover-desktop ", env!("CARGO_PKG_VERSION")),
        user: backup.user.as_ref().map(|u| UserOut {
            id: u.id.get(),
            username: &u.username,
            name: u.name.as_deref(),
        }),
        omitted: backup.omitted(),
        library: entries
            .into_iter()
            .map(|e| entry(e, backup.reviews.get(&e.user_book_id)))
            .collect(),
        lists,
        goals: backup
            .goals
            .as_ref()
            .map(|goals| goals.iter().map(goal).collect()),
    };
    let mut json = serde_json::to_string_pretty(&document).expect("the backup serializes");
    json.push('\n');
    json
}

#[derive(Serialize)]
struct Document<'a> {
    format: &'static str,
    version: u32,
    exported_at: Timestamp,
    /// The app and version that wrote the file.
    generator: &'static str,
    user: Option<UserOut<'a>>,
    /// Parts left out for lack of permission: "lists", "goals".
    omitted: Vec<&'static str>,
    library: Vec<EntryOut<'a>>,
    /// `null` when omitted.
    lists: Option<Vec<ListOut<'a>>>,
    /// `null` when omitted.
    goals: Option<Vec<GoalOut<'a>>>,
}

#[derive(Serialize)]
struct UserOut<'a> {
    id: i64,
    username: &'a str,
    name: Option<&'a str>,
}

#[derive(Serialize)]
struct EntryOut<'a> {
    user_book_id: i64,
    /// Hardcover's status slug: "want-to-read", "currently-reading", "read",
    /// "paused", "did-not-finish" or "ignored".
    status: &'static str,
    /// 0.5 to 5 in half stars.
    rating: Option<f32>,
    date_added: Option<Date>,
    updated_at: Option<Timestamp>,
    /// "public", "followers-only" or "private".
    privacy: Option<&'static str>,
    private_notes: Option<&'a str>,
    book: BookOut<'a>,
    edition: Option<EditionOut<'a>>,
    /// Every read, the current one first.
    reads: Vec<ReadOut>,
    /// `null` when the entry has no review. A review kept only in the
    /// website's editor format has `null` text.
    review: Option<ReviewOut<'a>>,
}

#[derive(Serialize)]
struct BookOut<'a> {
    id: i64,
    title: &'a str,
    subtitle: Option<&'a str>,
    authors: &'a [String],
    slug: Option<&'a str>,
    url: Option<String>,
    pages: Option<u32>,
    release_year: Option<i32>,
    series: Vec<SeriesOut<'a>>,
}

#[derive(Serialize)]
struct SeriesOut<'a> {
    id: Option<i64>,
    name: &'a str,
    position: Option<f32>,
}

#[derive(Serialize)]
struct EditionOut<'a> {
    id: i64,
    title: Option<&'a str>,
    /// Free text such as "Paperback".
    format: Option<&'a str>,
    /// Hardcover's kind of edition, such as "Audiobook".
    reading_format: Option<&'a str>,
    pages: Option<u32>,
    audio_seconds: Option<u32>,
    isbn_13: Option<&'a str>,
    publisher: Option<&'a str>,
    language: Option<&'a str>,
    release_date: Option<Date>,
}

#[derive(Serialize)]
struct ReadOut {
    id: i64,
    started_at: Option<Date>,
    finished_at: Option<Date>,
    progress_pages: Option<u32>,
    progress_seconds: Option<u32>,
    progress_percent: Option<f32>,
    edition_id: Option<i64>,
}

#[derive(Serialize)]
struct ReviewOut<'a> {
    /// The review as Markdown.
    markdown: Option<&'a str>,
    /// Hardcover's plain-text form.
    plain: Option<&'a str>,
    has_spoilers: bool,
    reviewed_at: Option<Date>,
}

#[derive(Serialize)]
struct ListOut<'a> {
    id: i64,
    name: &'a str,
    slug: Option<&'a str>,
    description: Option<&'a str>,
    /// Hardcover's `privacy_setting_id`: 1 public, 2 followers only, 3 private.
    privacy_setting_id: Option<i64>,
    ranked: bool,
    /// The list Hardcover keeps owned editions on.
    owned: bool,
    books: Vec<ListBookOut<'a>>,
}

#[derive(Serialize)]
struct ListBookOut<'a> {
    list_book_id: i64,
    book_id: i64,
    title: &'a str,
    authors: &'a [String],
    edition_id: Option<i64>,
    position: Option<i32>,
}

#[derive(Serialize)]
struct GoalOut<'a> {
    id: i64,
    /// "book", "page" or "hour".
    metric: &'a str,
    target: u32,
    /// Books, pages or hours so far.
    progress: f64,
    start: Date,
    end: Date,
    description: Option<&'a str>,
    archived: bool,
    completed_at: Option<Timestamp>,
    privacy_setting_id: Option<i64>,
    /// As Hardcover stores them, such as `{"readingFormatId": 2}`; `{}` for none.
    conditions: Value,
}

fn privacy_slug(privacy: Privacy) -> &'static str {
    match privacy {
        Privacy::Public => "public",
        Privacy::FollowersOnly => "followers-only",
        Privacy::Private => "private",
    }
}

fn entry<'a>(e: &'a LibraryEntry, review: Option<&'a Review>) -> EntryOut<'a> {
    let book = &e.book;
    EntryOut {
        user_book_id: e.user_book_id.get(),
        status: e.status.slug(),
        rating: e.rating.map(|r| r.value()),
        date_added: e.date_added,
        updated_at: e.updated_at,
        privacy: e.privacy.map(privacy_slug),
        private_notes: e.private_notes.as_deref(),
        book: BookOut {
            id: book.id.get(),
            title: &book.title,
            subtitle: book.subtitle.as_deref(),
            authors: &book.authors,
            slug: book.slug.as_deref(),
            url: book.web_url(),
            pages: book.pages,
            release_year: book.release_year,
            series: e.series.iter().map(series).collect(),
        },
        edition: e.edition.as_ref().map(edition),
        reads: e.reads.iter().map(read).collect(),
        review: review.filter(|r| r.exists()).map(|r| ReviewOut {
            markdown: r.markdown.as_deref(),
            plain: r.plain.as_deref(),
            has_spoilers: r.has_spoilers,
            reviewed_at: r.reviewed_at,
        }),
    }
}

fn series(s: &SeriesRef) -> SeriesOut<'_> {
    SeriesOut {
        id: s.id.map(|id| id.get()),
        name: &s.name,
        position: s.position,
    }
}

fn edition(e: &EditionSummary) -> EditionOut<'_> {
    EditionOut {
        id: e.id.get(),
        title: e.title.as_deref(),
        format: e.format.as_deref(),
        reading_format: e.reading_format.as_deref(),
        pages: e.pages,
        audio_seconds: e.audio_seconds,
        isbn_13: e.isbn_13.as_deref(),
        publisher: e.publisher.as_deref(),
        language: e.language.as_deref(),
        release_date: e.release_date,
    }
}

fn read(r: &ReadSession) -> ReadOut {
    ReadOut {
        id: r.id.get(),
        started_at: r.started_at,
        finished_at: r.finished_at,
        progress_pages: r.progress_pages,
        progress_seconds: r.progress_seconds,
        progress_percent: r.progress_percent,
        edition_id: r.edition_id.map(|id| id.get()),
    }
}

fn list<'a>(l: &'a UserList, books: &'a [ListBook]) -> ListOut<'a> {
    ListOut {
        id: l.id.get(),
        name: &l.name,
        slug: l.slug.as_deref(),
        description: l.description.as_deref(),
        privacy_setting_id: l.privacy_id,
        ranked: l.ranked,
        owned: l.is_owned(),
        books: books
            .iter()
            .map(|b| ListBookOut {
                list_book_id: b.id.get(),
                book_id: b.book.id.get(),
                title: &b.book.title,
                authors: &b.book.authors,
                edition_id: b.edition_id.map(|id| id.get()),
                position: b.position,
            })
            .collect(),
    }
}

fn goal(g: &Goal) -> GoalOut<'_> {
    GoalOut {
        id: g.id.get(),
        metric: g.metric.api_name(),
        target: g.target,
        progress: g.progress,
        start: g.start,
        end: g.end,
        description: g.description.as_deref(),
        archived: g.archived,
        completed_at: g.completed_at,
        privacy_setting_id: g.privacy_id,
        conditions: match &g.conditions.0 {
            Value::Null => Value::Object(Default::default()),
            other => other.clone(),
        },
    }
}
