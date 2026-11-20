//! Plain data records returned by [`crate::api::Client`] and stored by [`crate::store`].
//!
//! Records have public fields but are `#[non_exhaustive]`: build them with their
//! `new` constructors and then set the optional fields you need.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Calendar date (`date` in the schema), e.g. a read's start date.
pub type Date = chrono::NaiveDate;
/// Point in time (`timestamptz` in the schema), always UTC.
pub type Timestamp = chrono::DateTime<chrono::Utc>;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            pub const fn new(id: i64) -> Self {
                Self(id)
            }
            pub const fn get(self) -> i64 {
                self.0
            }
        }

        impl From<i64> for $name {
            fn from(id: i64) -> Self {
                Self(id)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> i64 {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// `books.id`
    BookId
);
id_type!(
    /// `user_books.id`: one book in one user's library.
    UserBookId
);
id_type!(
    /// `user_book_reads.id`: one read-through of a library entry.
    ReadId
);
id_type!(
    /// `editions.id`
    EditionId
);
id_type!(
    /// `users.id`
    UserId
);
id_type!(
    /// `series.id`
    SeriesId
);
id_type!(
    /// `authors.id`
    AuthorId
);
id_type!(
    /// `goals.id`: one reading goal.
    GoalId
);
id_type!(
    /// `lists.id`: one of a user's lists.
    ListId
);
id_type!(
    /// `list_books.id`: one book on one list.
    ListBookId
);

/// A library entry's status (`user_books.status_id`).
///
/// Ids verified against the live `user_book_statuses` table (2026-11):
/// 1 want-to-read, 2 currently-reading, 3 read, 4 paused, 5 did-not-finish, 6 ignored.
///
/// [`Ignored`](Self::Ignored) is a real status (a book the user hid from
/// recommendations). [`crate::api::Client::library`] returns those entries too,
/// so the app can list them apart from the reading shelves. Rows with an id this
/// enum does not know are skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReadingStatus {
    WantToRead,
    CurrentlyReading,
    Read,
    Paused,
    DidNotFinish,
    Ignored,
}

impl ReadingStatus {
    /// The five statuses that are shelves in the library, in Hardcover's order.
    pub const SHELVES: [ReadingStatus; 5] = [
        Self::WantToRead,
        Self::CurrentlyReading,
        Self::Read,
        Self::Paused,
        Self::DidNotFinish,
    ];

    /// Every status, in Hardcover's order: the shelves, then Ignored.
    pub const ALL: [ReadingStatus; 6] = [
        Self::WantToRead,
        Self::CurrentlyReading,
        Self::Read,
        Self::Paused,
        Self::DidNotFinish,
        Self::Ignored,
    ];

    /// `status_id` used by the API.
    pub const fn id(self) -> i64 {
        match self {
            Self::WantToRead => 1,
            Self::CurrentlyReading => 2,
            Self::Read => 3,
            Self::Paused => 4,
            Self::DidNotFinish => 5,
            Self::Ignored => 6,
        }
    }

    pub const fn from_id(id: i64) -> Option<Self> {
        Some(match id {
            1 => Self::WantToRead,
            2 => Self::CurrentlyReading,
            3 => Self::Read,
            4 => Self::Paused,
            5 => Self::DidNotFinish,
            6 => Self::Ignored,
            _ => return None,
        })
    }

    /// Hardcover's English label (`user_book_statuses.status`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::WantToRead => "Want to Read",
            Self::CurrentlyReading => "Currently Reading",
            Self::Read => "Read",
            Self::Paused => "Paused",
            Self::DidNotFinish => "Did Not Finish",
            Self::Ignored => "Ignored",
        }
    }

    /// Hardcover's slug (`user_book_statuses.slug`).
    pub const fn slug(self) -> &'static str {
        match self {
            Self::WantToRead => "want-to-read",
            Self::CurrentlyReading => "currently-reading",
            Self::Read => "read",
            Self::Paused => "paused",
            Self::DidNotFinish => "did-not-finish",
            Self::Ignored => "ignored",
        }
    }

    pub const fn is_shelf(self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

impl fmt::Display for ReadingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.label())
    }
}

/// Who can see a library entry and the activity it generates
/// (`privacy_settings`, ids verified live: 1 Public, 2 Followers only, 3 Private).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Privacy {
    Public,
    FollowersOnly,
    Private,
}

impl Privacy {
    pub const ALL: [Privacy; 3] = [Self::Public, Self::FollowersOnly, Self::Private];

    pub const fn id(self) -> i64 {
        match self {
            Self::Public => 1,
            Self::FollowersOnly => 2,
            Self::Private => 3,
        }
    }

    pub const fn from_id(id: i64) -> Option<Self> {
        Some(match id {
            1 => Self::Public,
            2 => Self::FollowersOnly,
            3 => Self::Private,
            _ => return None,
        })
    }

    /// Hardcover's label (`privacy_settings.setting`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Public => "Public",
            Self::FollowersOnly => "Followers only",
            Self::Private => "Private",
        }
    }
}

/// A star rating from 0.5 to 5.0 in half steps.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Rating(f32);

impl Rating {
    pub const MIN: f32 = 0.5;
    pub const MAX: f32 = 5.0;

    /// `None` unless `value` is within 0.5..=5.0 and a multiple of 0.5.
    pub fn new(value: f32) -> Option<Self> {
        let doubled = value * 2.0;
        ((Self::MIN..=Self::MAX).contains(&value) && doubled == doubled.round())
            .then_some(Self(value))
    }

    /// Lenient conversion for values coming from the API: rounds to the nearest half
    /// star and treats anything below a quarter star as "no rating".
    pub fn from_api(value: f64) -> Option<Self> {
        if !value.is_finite() || value < 0.25 {
            return None;
        }
        let rounded =
            ((value * 2.0).round() / 2.0).clamp(f64::from(Self::MIN), f64::from(Self::MAX));
        Some(Self(rounded as f32))
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Rating {
    type Error = String;
    fn try_from(v: f32) -> Result<Self, Self::Error> {
        Self::new(v).ok_or_else(|| format!("invalid rating {v}: expected 0.5..=5.0 in 0.5 steps"))
    }
}

impl From<Rating> for f32 {
    fn from(r: Rating) -> f32 {
        r.0
    }
}

impl fmt::Display for Rating {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.fract() == 0.0 {
            write!(f, "{:.0}", self.0)
        } else {
            write!(f, "{:.1}", self.0)
        }
    }
}

/// An image hosted by Hardcover (book cover, edition cover or avatar).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cover {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl Cover {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            width: None,
            height: None,
        }
    }
}

/// The signed-in user (`me`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub name: Option<String>,
    pub avatar: Option<Cover>,
}

impl User {
    pub fn new(id: UserId, username: impl Into<String>) -> Self {
        Self {
            id,
            username: username.into(),
            name: None,
            avatar: None,
        }
    }
}

/// What lists and search results show for a book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BookSummary {
    pub id: BookId,
    pub title: String,
    pub subtitle: Option<String>,
    /// Primary authors (contributions with no role or the "Author" role), in credit order.
    pub authors: Vec<String>,
    pub cover: Option<Cover>,
    /// Page count of the default physical edition.
    pub pages: Option<u32>,
    pub release_year: Option<i32>,
    /// URL slug: `https://hardcover.app/books/<slug>`.
    pub slug: Option<String>,
    /// Hardcover average rating (0..=5).
    pub average_rating: Option<f32>,
    /// Number of users who have the book in their library.
    pub users_count: Option<u32>,
}

impl BookSummary {
    pub fn new(id: BookId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            subtitle: None,
            authors: Vec::new(),
            cover: None,
            pages: None,
            release_year: None,
            slug: None,
            average_rating: None,
            users_count: None,
        }
    }

    /// `https://hardcover.app/books/<slug>`, when the book has a slug.
    pub fn web_url(&self) -> Option<String> {
        self.slug
            .as_deref()
            .map(|s| format!("https://hardcover.app/books/{s}"))
    }
}

/// One edition of a book: the one a library entry or a read is tied to, or a
/// choice in the edition picker. A read's edition carries only the id, title,
/// lengths and format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EditionSummary {
    pub id: EditionId,
    pub title: Option<String>,
    pub pages: Option<u32>,
    /// Length of an audiobook edition.
    pub audio_seconds: Option<u32>,
    pub cover: Option<Cover>,
    /// `editions.edition_format`, free text such as "Paperback", "Hardcover", "Kindle Edition".
    pub format: Option<String>,
    /// `reading_formats.format`, Hardcover's broad kind of edition (such as "Audiobook").
    pub reading_format: Option<String>,
    pub release_date: Option<Date>,
    pub isbn_13: Option<String>,
    pub publisher: Option<String>,
    pub language: Option<String>,
}

impl EditionSummary {
    pub fn new(id: EditionId) -> Self {
        Self {
            id,
            title: None,
            pages: None,
            audio_seconds: None,
            cover: None,
            format: None,
            reading_format: None,
            release_date: None,
            isbn_13: None,
            publisher: None,
            language: None,
        }
    }

    /// The edition's length: pages, or for an audiobook without a page count,
    /// its running time.
    pub fn length(&self) -> Option<Length> {
        match (self.pages, self.audio_seconds) {
            (Some(p), _) if p > 0 => Some(Length::Pages(p)),
            (_, Some(s)) if s > 0 => Some(Length::Seconds(s)),
            _ => None,
        }
    }
}

/// How long an edition is, in the unit its progress is recorded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Length {
    Pages(u32),
    Seconds(u32),
}

/// One read-through of a book (`user_book_reads`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReadSession {
    pub id: ReadId,
    pub started_at: Option<Date>,
    pub finished_at: Option<Date>,
    pub progress_pages: Option<u32>,
    pub progress_seconds: Option<u32>,
    /// `user_book_reads.progress`, a percentage 0..=100 computed by the server.
    pub progress_percent: Option<f32>,
    pub edition_id: Option<EditionId>,
    /// The read's edition, when it has one.
    #[serde(default)]
    pub edition: Option<EditionSummary>,
}

impl ReadSession {
    pub fn new(id: ReadId) -> Self {
        Self {
            id,
            started_at: None,
            finished_at: None,
            progress_pages: None,
            progress_seconds: None,
            progress_percent: None,
            edition_id: None,
            edition: None,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished_at.is_some()
    }

    /// The date that places the read in time: when it finished, or else when it started.
    pub fn date(&self) -> Option<Date> {
        self.finished_at.or(self.started_at)
    }
}

/// Order of reads in [`LibraryEntry::reads`]: open reads first (newest first),
/// then finished ones by date, most recent first. Ids break ties, newest first.
///
/// Ids alone are not enough: a past read added today gets the highest id.
pub fn read_order(a: &ReadSession, b: &ReadSession) -> std::cmp::Ordering {
    a.is_finished()
        .cmp(&b.is_finished())
        .then_with(|| b.date().cmp(&a.date()))
        .then_with(|| b.id.cmp(&a.id))
}

/// One book in the user's library (`user_books`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LibraryEntry {
    pub user_book_id: UserBookId,
    /// Owner of the entry. Lets the cache file entries per user.
    pub user_id: UserId,
    pub book: BookSummary,
    pub status: ReadingStatus,
    pub rating: Option<Rating>,
    pub edition: Option<EditionSummary>,
    /// Every read of the book, in [`read_order`]: the current one first.
    #[serde(default)]
    pub reads: Vec<ReadSession>,
    pub date_added: Option<Date>,
    pub updated_at: Option<Timestamp>,
    /// `user_books.private_notes`, visible only to the owner.
    #[serde(default)]
    pub private_notes: Option<String>,
    /// Who can see the entry. `None` in caches written before it was stored.
    #[serde(default)]
    pub privacy: Option<Privacy>,
    /// The book's series, featured first. Empty in caches written before it
    /// was stored, until the next sync.
    #[serde(default)]
    pub series: Vec<SeriesRef>,
}

impl LibraryEntry {
    pub fn new(
        user_book_id: UserBookId,
        user_id: UserId,
        book: BookSummary,
        status: ReadingStatus,
    ) -> Self {
        Self {
            user_book_id,
            user_id,
            book,
            status,
            rating: None,
            edition: None,
            reads: Vec::new(),
            date_added: None,
            updated_at: None,
            private_notes: None,
            privacy: None,
            series: Vec::new(),
        }
    }

    /// The read progress and the date fields act on: the newest unfinished
    /// read, or else the most recently finished one.
    pub fn current_read(&self) -> Option<&ReadSession> {
        self.reads.iter().min_by(|a, b| read_order(a, b))
    }

    /// The current read if it is unfinished.
    pub fn open_read(&self) -> Option<&ReadSession> {
        self.current_read().filter(|r| !r.is_finished())
    }

    pub fn read(&self, id: ReadId) -> Option<&ReadSession> {
        self.reads.iter().find(|r| r.id == id)
    }

    /// Adds `read`, or replaces the read with its id, keeping [`read_order`].
    pub fn upsert_read(&mut self, read: ReadSession) {
        self.reads.retain(|r| r.id != read.id);
        self.reads.push(read);
        self.reads.sort_by(read_order);
    }

    /// The cover to show: the edition's, falling back to the book's.
    pub fn cover(&self) -> Option<&Cover> {
        self.edition
            .as_ref()
            .and_then(|e| e.cover.as_ref())
            .or(self.book.cover.as_ref())
    }

    /// Length of what is being read: the open read's edition, else the entry's
    /// edition, else the book's default page count. The first of those with a
    /// known length decides the unit, as in [`crate::api::Client::update_progress`].
    pub fn length(&self) -> Option<Length> {
        self.open_read()
            .and_then(|r| r.edition.as_ref())
            .and_then(EditionSummary::length)
            .or_else(|| self.edition.as_ref().and_then(EditionSummary::length))
            .or_else(|| self.book.pages.filter(|p| *p > 0).map(Length::Pages))
    }

    /// Page count of what is being read; `None` for an audiobook.
    pub fn total_pages(&self) -> Option<u32> {
        match self.length() {
            Some(Length::Pages(p)) => Some(p),
            _ => None,
        }
    }
}

/// A series the book belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SeriesRef {
    /// `None` only if Hardcover left it out; the series page needs it.
    #[serde(default)]
    pub id: Option<SeriesId>,
    pub name: String,
    pub slug: Option<String>,
    /// Position in the series; may be fractional (1.5 for a novella).
    pub position: Option<f32>,
    /// Hardcover's featured series for this book.
    pub featured: bool,
}

impl SeriesRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: None,
            name: name.into(),
            slug: None,
            position: None,
            featured: false,
        }
    }
}

/// Someone credited on a book (`cached_contributors`), in credit order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Contributor {
    /// `None` when the cached entry has no author id; it then cannot link.
    pub author_id: Option<AuthorId>,
    pub name: String,
    /// The role, e.g. "Translator", "Narrator", "Illustrator". `None` (or
    /// "Author") for a primary author.
    pub role: Option<String>,
}

impl Contributor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            author_id: None,
            name: name.into(),
            role: None,
        }
    }

    /// No role, or the "Author" role.
    pub fn is_author(&self) -> bool {
        self.role
            .as_deref()
            .is_none_or(|r| r.eq_ignore_ascii_case("author"))
    }
}

/// How many readers gave one star rating (`ratings_distribution`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RatingBucket {
    /// 0.5 to 5.0.
    pub rating: f32,
    pub count: u32,
}

impl RatingBucket {
    pub fn new(rating: f32, count: u32) -> Self {
        Self { rating, count }
    }
}

/// Everything the detail page shows for a book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BookDetails {
    pub summary: BookSummary,
    pub description: Option<String>,
    pub release_date: Option<Date>,
    /// Featured series first.
    pub series: Vec<SeriesRef>,
    /// Top community genres (`cached_tags.Genre`), most-voted first.
    pub genres: Vec<String>,
    /// Top community moods (`cached_tags.Mood`).
    pub moods: Vec<String>,
    /// Top community tags (`cached_tags.Tag`).
    pub tags: Vec<String>,
    /// Top content warnings (`cached_tags["Content Warning"]`).
    pub content_warnings: Vec<String>,
    pub editions_count: Option<u32>,
    pub ratings_count: Option<u32>,
    /// Everyone credited, with roles, in credit order.
    #[serde(default)]
    pub contributors: Vec<Contributor>,
    /// Ratings per star level, highest first; empty when Hardcover has none.
    #[serde(default)]
    pub ratings_distribution: Vec<RatingBucket>,
    /// Other titles the book is known by (`alternative_titles`).
    #[serde(default)]
    pub alternative_titles: Vec<String>,
    /// Hardcover's similar books, most similar first (`cached_similar_book_ids`).
    #[serde(default)]
    pub similar_book_ids: Vec<BookId>,
}

impl BookDetails {
    pub fn new(summary: BookSummary) -> Self {
        Self {
            summary,
            description: None,
            release_date: None,
            series: Vec::new(),
            genres: Vec::new(),
            moods: Vec::new(),
            tags: Vec::new(),
            content_warnings: Vec::new(),
            editions_count: None,
            ratings_count: None,
            contributors: Vec::new(),
            ratings_distribution: Vec::new(),
            alternative_titles: Vec::new(),
            similar_book_ids: Vec::new(),
        }
    }
}

/// A series' author or another linked author.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuthorRef {
    pub id: AuthorId,
    pub name: String,
}

impl AuthorRef {
    pub fn new(id: AuthorId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

/// One book on a series page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SeriesBook {
    pub book: BookSummary,
    /// `None` for a book the series lists without a number.
    pub position: Option<f32>,
}

impl SeriesBook {
    pub fn new(book: BookSummary) -> Self {
        Self {
            book,
            position: None,
        }
    }

    /// A whole-numbered entry: the series' main books, as opposed to
    /// novellas in between (1.5) and unnumbered extras.
    pub fn is_main(&self) -> bool {
        self.position.is_some_and(|p| p.fract() == 0.0)
    }
}

/// A series page (`series_by_pk`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SeriesDetails {
    pub id: SeriesId,
    pub name: String,
    pub slug: Option<String>,
    pub author: Option<AuthorRef>,
    pub description: Option<String>,
    pub is_completed: Option<bool>,
    /// Hardcover's count of the main books (without companions).
    pub primary_books_count: Option<u32>,
    /// In reading order: numbered books by position (one book per position),
    /// then unnumbered ones. Merged duplicates, partial books and
    /// compilations are left out.
    pub books: Vec<SeriesBook>,
}

impl SeriesDetails {
    pub fn new(id: SeriesId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            slug: None,
            author: None,
            description: None,
            is_completed: None,
            primary_books_count: None,
            books: Vec::new(),
        }
    }

    /// `https://hardcover.app/series/<slug>`, when the series has a slug.
    pub fn web_url(&self) -> Option<String> {
        self.slug
            .as_deref()
            .map(|s| format!("https://hardcover.app/series/{s}"))
    }
}

/// One book on an author page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuthorBook {
    pub book: BookSummary,
    /// The author's role when they did not write it, e.g. "Translator".
    pub role: Option<String>,
}

impl AuthorBook {
    pub fn new(book: BookSummary) -> Self {
        Self { book, role: None }
    }
}

/// An author page (`authors_by_pk`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuthorDetails {
    pub id: AuthorId,
    pub name: String,
    pub slug: Option<String>,
    pub bio: Option<String>,
    pub photo: Option<Cover>,
    pub born_year: Option<i32>,
    pub death_year: Option<i32>,
    /// Hardcover's count of the author's books.
    pub books_count: Option<u32>,
    /// Their most-read books (at most [`crate::api::AUTHOR_BOOKS_LIMIT`]),
    /// most-read first, one row per book.
    pub books: Vec<AuthorBook>,
}

impl AuthorDetails {
    pub fn new(id: AuthorId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            slug: None,
            bio: None,
            photo: None,
            born_year: None,
            death_year: None,
            books_count: None,
            books: Vec::new(),
        }
    }

    /// `https://hardcover.app/authors/<slug>`, when the author has a slug.
    pub fn web_url(&self) -> Option<String> {
        self.slug
            .as_deref()
            .map(|s| format!("https://hardcover.app/authors/{s}"))
    }
}

/// One page of book search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchPage {
    pub books: Vec<BookSummary>,
    /// 1-based.
    pub page: u32,
    pub per_page: u32,
    /// Total matches (`found`), when Typesense reports it.
    pub total: Option<u32>,
}

impl SearchPage {
    pub fn new(books: Vec<BookSummary>, page: u32, per_page: u32) -> Self {
        Self {
            books,
            page,
            per_page,
            total: None,
        }
    }

    pub fn has_more(&self) -> bool {
        match self.total {
            Some(total) => u64::from(self.page) * u64::from(self.per_page) < u64::from(total),
            None => self.books.len() as u32 >= self.per_page,
        }
    }
}

/// What a reading goal counts (`goals.metric`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GoalMetric {
    /// Books finished.
    Books,
    /// Pages read.
    Pages,
    /// Hours read or listened. The API's `progress` is in seconds;
    /// [`Goal::progress`] is converted to hours.
    Hours,
    /// A metric this version does not know, kept as Hardcover named it.
    Other(String),
}

impl GoalMetric {
    pub fn from_api(name: &str) -> Self {
        match name {
            "book" => Self::Books,
            "page" => Self::Pages,
            "hour" => Self::Hours,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn api_name(&self) -> &str {
        match self {
            Self::Books => "book",
            Self::Pages => "page",
            Self::Hours => "hour",
            Self::Other(name) => name,
        }
    }
}

/// Which books count toward a goal (`goals.conditions`): a format, categories
/// or author facts. Kept exactly as Hardcover sent it, because changing a goal
/// resends the whole goal, conditions included.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GoalConditions(pub(crate) serde_json::Value);

/// `GoalConditionInput`'s fields. The stored JSON uses the same names (the
/// official SDK sends it back unchanged).
const GOAL_CONDITION_FIELDS: [&str; 5] = [
    "authorBipoc",
    "authorGenderIds",
    "authorLgbtqia",
    "bookCategoryIds",
    "readingFormatId",
];

impl GoalConditions {
    /// True if every book counts.
    pub fn is_empty(&self) -> bool {
        match &self.0 {
            serde_json::Value::Object(map) => map
                .values()
                .all(|v| v.is_null() || v.as_array().is_some_and(|a| a.is_empty())),
            _ => true,
        }
    }

    /// The conditions as a `GoalConditionInput`, or `None` if they hold a
    /// field this version can't send back (it would be lost on update).
    pub fn to_input(&self) -> Option<serde_json::Value> {
        match &self.0 {
            serde_json::Value::Object(map) => {
                let known = map
                    .iter()
                    .filter(|(_, v)| !v.is_null())
                    .all(|(k, _)| GOAL_CONDITION_FIELDS.contains(&k.as_str()));
                known.then(|| {
                    serde_json::Value::Object(
                        map.iter()
                            .filter(|(_, v)| !v.is_null())
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    )
                })
            }
            serde_json::Value::Null => Some(serde_json::json!({})),
            _ => None,
        }
    }
}

/// A reading goal: read `target` books (or pages, or hours) between `start`
/// and `end`. Hardcover calculates the progress from the user's reading.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Goal {
    pub id: GoalId,
    pub metric: GoalMetric,
    /// In the metric's unit (hours for hour goals).
    pub target: u32,
    /// In the metric's unit (hours for hour goals), as Hardcover calculated it.
    pub progress: f64,
    pub start: Date,
    pub end: Date,
    pub description: Option<String>,
    pub archived: bool,
    pub completed_at: Option<Timestamp>,
    /// `privacy_setting_id` as Hardcover sent it, known or not: changing a
    /// goal sends it back unchanged. See [`Goal::privacy`].
    pub privacy_id: Option<i64>,
    pub conditions: GoalConditions,
}

impl Goal {
    pub fn new(id: GoalId, metric: GoalMetric, target: u32, start: Date, end: Date) -> Self {
        Self {
            id,
            metric,
            target,
            progress: 0.0,
            start,
            end,
            description: None,
            archived: false,
            completed_at: None,
            privacy_id: None,
            conditions: GoalConditions::default(),
        }
    }

    /// Who can see the goal, if Hardcover sent a setting this version knows.
    pub fn privacy(&self) -> Option<Privacy> {
        self.privacy_id.and_then(Privacy::from_id)
    }

    /// Progress as a fraction of the target; above 1 once it is passed.
    pub fn fraction(&self) -> f64 {
        if self.target == 0 {
            return 0.0;
        }
        self.progress / f64::from(self.target)
    }

    /// True if `day` falls within the goal's dates.
    pub fn is_current(&self, day: Date) -> bool {
        (self.start..=self.end).contains(&day)
    }

    /// Where steady progress would be by the end of `day`, in the metric's unit.
    pub fn expected_by(&self, day: Date) -> f64 {
        let total = (self.end - self.start).num_days() + 1;
        if total <= 0 {
            return f64::from(self.target);
        }
        let elapsed = ((day - self.start).num_days() + 1).clamp(0, total);
        f64::from(self.target) * elapsed as f64 / total as f64
    }

    /// Ahead (positive) or behind (negative) of steady progress on `day`.
    pub fn pace(&self, day: Date) -> f64 {
        self.progress - self.expected_by(day)
    }
}

/// A goal to create, for [`crate::api::Client::create_goal`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewGoal {
    pub metric: GoalMetric,
    pub target: u32,
    pub start: Date,
    pub end: Date,
    /// Hardcover requires one; the website writes e.g. "30 books in 2026".
    pub description: String,
}

/// An author in search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuthorHit {
    pub id: AuthorId,
    pub name: String,
    pub slug: Option<String>,
    pub photo: Option<Cover>,
    /// Books Hardcover credits to the author.
    pub books_count: Option<u32>,
    /// Titles of the author's most popular books, at most five.
    pub top_books: Vec<String>,
}

impl AuthorHit {
    pub fn new(id: AuthorId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            slug: None,
            photo: None,
            books_count: None,
            top_books: Vec::new(),
        }
    }
}

/// A series in search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SeriesHit {
    pub id: SeriesId,
    pub name: String,
    pub slug: Option<String>,
    /// The series' primary author.
    pub author: Option<String>,
    pub books_count: Option<u32>,
    /// Books with a whole-number position (1, 2, 3), as the series page counts them.
    pub primary_books_count: Option<u32>,
    /// Titles of books in the series, as many as the search returns.
    pub books: Vec<String>,
}

impl SeriesHit {
    pub fn new(id: SeriesId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            slug: None,
            author: None,
            books_count: None,
            primary_books_count: None,
            books: Vec::new(),
        }
    }
}

/// One page of search results of any kind. Book search returns a
/// [`SearchPage`], which converts into this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchHits<T> {
    pub hits: Vec<T>,
    /// 1-based.
    pub page: u32,
    pub per_page: u32,
    /// Total matches (`found`), when Typesense reports it.
    pub total: Option<u32>,
}

impl<T> SearchHits<T> {
    pub fn new(hits: Vec<T>, page: u32, per_page: u32) -> Self {
        Self {
            hits,
            page,
            per_page,
            total: None,
        }
    }

    pub fn has_more(&self) -> bool {
        match self.total {
            Some(total) => u64::from(self.page) * u64::from(self.per_page) < u64::from(total),
            None => self.hits.len() as u32 >= self.per_page,
        }
    }
}

impl From<SearchPage> for SearchHits<BookSummary> {
    fn from(page: SearchPage) -> Self {
        Self {
            hits: page.books,
            page: page.page,
            per_page: page.per_page,
            total: page.total,
        }
    }
}

/// How far back [`crate::api::Client::trending_books`] looks: Hardcover ranks
/// the books the most readers finished in this window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrendingPeriod {
    Week,
    Month,
    ThreeMonths,
    Year,
    AllTime,
}

impl TrendingPeriod {
    /// `TrendingDuration` in the schema.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
            Self::ThreeMonths => "three_month",
            Self::Year => "one_year",
            Self::AllTime => "all",
        }
    }
}

/// The slug of the list Hardcover keeps owned editions on. The server creates
/// it on the first [`crate::api::Client::mark_owned`]; the official SDK finds
/// it the same way (the user's `lists` row with this slug).
pub const OWNED_LIST_SLUG: &str = "owned";

/// One of the signed-in user's lists (`lists`).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct UserList {
    pub id: ListId,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    /// `privacy_setting_id` as Hardcover sent it, known or not: changing a
    /// list sends it back unchanged. See [`UserList::privacy`].
    pub privacy_id: Option<i64>,
    /// The books have an order (their position) that the list shows.
    pub ranked: bool,
    pub books_count: u32,
    /// Kept only to send back on update: `update_list` takes a whole `ListInput`.
    pub(crate) default_view: Option<String>,
    pub(crate) featured_profile: Option<bool>,
    pub(crate) url: Option<String>,
}

impl UserList {
    pub fn new(id: ListId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            slug: None,
            description: None,
            privacy_id: None,
            ranked: false,
            books_count: 0,
            default_view: None,
            featured_profile: None,
            url: None,
        }
    }

    /// Who can see the list, if Hardcover sent a setting this version knows.
    pub fn privacy(&self) -> Option<Privacy> {
        self.privacy_id.and_then(Privacy::from_id)
    }

    /// The list Hardcover keeps owned editions on. It is managed through
    /// [`crate::api::Client::mark_owned`], not edited like other lists.
    pub fn is_owned(&self) -> bool {
        self.slug.as_deref() == Some(OWNED_LIST_SLUG)
    }

    /// `https://hardcover.app/@<username>/lists/<slug>`, when the list has a slug.
    pub fn web_url(&self, username: &str) -> Option<String> {
        self.slug
            .as_deref()
            .map(|s| format!("https://hardcover.app/@{username}/lists/{s}"))
    }
}

/// A list as the user enters it, for [`crate::api::Client::create_list`] and
/// [`crate::api::Client::update_list`].
#[derive(Debug, Clone, PartialEq)]
pub struct ListDraft {
    pub name: String,
    /// Blank clears it.
    pub description: String,
    /// `None` keeps the list's setting as read (one this version may not
    /// know), or gives a new list the account's default.
    pub privacy: Option<Privacy>,
    pub ranked: bool,
}

/// That a book is on one of the user's lists (`list_books`), without the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListMembership {
    pub id: ListBookId,
    pub list_id: ListId,
    pub book_id: BookId,
    /// The edition on the list, if one was chosen (always, on the Owned list).
    pub edition_id: Option<EditionId>,
}

/// Every book on the user's lists, from [`crate::api::Client::list_memberships`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListMemberships {
    pub rows: Vec<ListMembership>,
    /// False if there were more rows than the client reads in one go: a
    /// book missing from `rows` may still be on a list then.
    pub complete: bool,
}

/// One book on a list, for the list's page.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ListBook {
    pub id: ListBookId,
    pub book: BookSummary,
    pub edition_id: Option<EditionId>,
    /// Place on a ranked list; usually empty on others.
    pub position: Option<i32>,
}

impl ListBook {
    pub fn new(id: ListBookId, book: BookSummary) -> Self {
        Self {
            id,
            book,
            edition_id: None,
            position: None,
        }
    }
}

/// How far the reader is, for [`crate::api::Client::update_progress`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressUpdate {
    /// Current page.
    Pages(u32),
    /// Percentage 0..=100. Hardcover stores pages (or seconds), so this is converted
    /// with the read's edition length, like the official SDK does.
    Percent(f32),
    /// Position in an audiobook.
    Seconds(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_ids_round_trip() {
        for id in 1..=6 {
            assert_eq!(ReadingStatus::from_id(id).unwrap().id(), id);
        }
        assert_eq!(ReadingStatus::from_id(0), None);
        assert_eq!(ReadingStatus::from_id(7), None);
        assert_eq!(ReadingStatus::CurrentlyReading.id(), 2);
        assert!(ReadingStatus::SHELVES.iter().all(|s| s.is_shelf()));
        assert!(!ReadingStatus::Ignored.is_shelf());
        assert_eq!(ReadingStatus::ALL[..5], ReadingStatus::SHELVES);
        for p in Privacy::ALL {
            assert_eq!(Privacy::from_id(p.id()), Some(p));
        }
    }

    fn read(
        id: i64,
        started: Option<(i32, u32, u32)>,
        finished: Option<(i32, u32, u32)>,
    ) -> ReadSession {
        let date = |(y, m, d)| Date::from_ymd_opt(y, m, d).unwrap();
        let mut r = ReadSession::new(ReadId::new(id));
        r.started_at = started.map(date);
        r.finished_at = finished.map(date);
        r
    }

    #[test]
    fn current_read_is_the_open_one_not_the_highest_id() {
        let mut e = LibraryEntry::new(
            UserBookId::new(1),
            UserId::new(1),
            BookSummary::new(BookId::new(1), "Synthetic"),
            ReadingStatus::CurrentlyReading,
        );
        assert!(e.current_read().is_none());
        e.upsert_read(read(10, Some((2026, 11, 1)), None));
        // A re-read from years ago, added later, so it has the higher id.
        e.upsert_read(read(11, Some((2019, 3, 1)), Some((2019, 4, 1))));
        assert_eq!(e.current_read().unwrap().id, ReadId::new(10));
        assert_eq!(e.open_read().unwrap().id, ReadId::new(10));
        // Finishing it: still current, as the most recent finished read.
        e.upsert_read(read(10, Some((2026, 11, 1)), Some((2026, 11, 20))));
        assert_eq!(e.current_read().unwrap().id, ReadId::new(10));
        assert!(e.open_read().is_none());
        let ids: Vec<i64> = e.reads.iter().map(|r| r.id.get()).collect();
        assert_eq!(ids, [10, 11]);
        // Without dates, the newest id wins.
        e.upsert_read(read(12, None, Some((2027, 1, 1))));
        e.upsert_read(read(13, None, None));
        let ids: Vec<i64> = e.reads.iter().map(|r| r.id.get()).collect();
        assert_eq!(ids, [13, 12, 10, 11]);
    }

    #[test]
    fn length_follows_the_open_read_then_the_entry_then_the_book() {
        let mut book = BookSummary::new(BookId::new(1), "Synthetic");
        book.pages = Some(300);
        let mut e = LibraryEntry::new(
            UserBookId::new(1),
            UserId::new(1),
            book,
            ReadingStatus::CurrentlyReading,
        );
        assert_eq!(e.length(), Some(Length::Pages(300)));
        let mut audio = EditionSummary::new(EditionId::new(2));
        audio.audio_seconds = Some(36_000);
        e.edition = Some(audio);
        assert_eq!(e.length(), Some(Length::Seconds(36_000)));
        assert_eq!(e.total_pages(), None);
        // An open read still on a paper edition keeps counting pages.
        let mut paper = EditionSummary::new(EditionId::new(3));
        paper.pages = Some(410);
        let mut r = read(5, Some((2026, 1, 1)), None);
        r.edition_id = Some(paper.id);
        r.edition = Some(paper);
        e.upsert_read(r);
        assert_eq!(e.total_pages(), Some(410));
    }

    #[test]
    fn rating_validation() {
        assert!(Rating::new(0.0).is_none());
        assert!(Rating::new(0.5).is_some());
        assert!(Rating::new(3.5).is_some());
        assert!(Rating::new(3.25).is_none());
        assert!(Rating::new(5.0).is_some());
        assert!(Rating::new(5.5).is_none());
        assert!(Rating::new(f32::NAN).is_none());
        assert_eq!(Rating::from_api(4.0).unwrap().value(), 4.0);
        assert_eq!(Rating::from_api(3.8).unwrap().value(), 4.0);
        assert_eq!(Rating::from_api(7.0).unwrap().value(), 5.0);
        assert!(Rating::from_api(0.0).is_none());
        assert_eq!(Rating::new(4.0).unwrap().to_string(), "4");
        assert_eq!(Rating::new(4.5).unwrap().to_string(), "4.5");
    }

    #[test]
    fn rating_serde_validates() {
        let r: Rating = serde_json::from_str("4.5").unwrap();
        assert_eq!(r.value(), 4.5);
        assert!(serde_json::from_str::<Rating>("4.3").is_err());
        assert_eq!(serde_json::to_string(&r).unwrap(), "4.5");
    }

    #[test]
    fn ids_are_transparent_in_json() {
        assert_eq!(serde_json::to_string(&BookId::new(42)).unwrap(), "42");
        assert_eq!(
            serde_json::from_str::<UserBookId>("7").unwrap(),
            UserBookId::new(7)
        );
        assert_eq!(ReadId::new(9).to_string(), "9");
    }

    fn goal(target: u32, progress: f64) -> Goal {
        let d = |m, d| Date::from_ymd_opt(2026, m, d).unwrap();
        let mut g = Goal::new(
            GoalId::new(1),
            GoalMetric::Books,
            target,
            d(1, 1),
            d(12, 31),
        );
        g.progress = progress;
        g
    }

    #[test]
    fn goal_pace_follows_the_calendar() {
        let d = |m, d| Date::from_ymd_opt(2026, m, d).unwrap();
        let g = goal(365, 100.0);
        assert_eq!(g.expected_by(d(1, 1)), 1.0);
        assert_eq!(g.expected_by(d(4, 10)), 100.0);
        assert_eq!(g.pace(d(4, 10)), 0.0);
        assert_eq!(g.pace(d(1, 31)), 69.0);
        // Before the start nothing is expected; after the end, everything.
        assert_eq!(g.expected_by(Date::from_ymd_opt(2025, 12, 1).unwrap()), 0.0);
        assert_eq!(
            g.expected_by(Date::from_ymd_opt(2027, 2, 1).unwrap()),
            365.0
        );
        assert!(g.is_current(d(12, 31)));
        assert!(!g.is_current(Date::from_ymd_opt(2027, 1, 1).unwrap()));
        assert_eq!(goal(12, 9.0).fraction(), 0.75);
        assert_eq!(goal(0, 9.0).fraction(), 0.0);
    }

    #[test]
    fn goal_metrics_round_trip() {
        for name in ["book", "page", "hour", "minute"] {
            assert_eq!(GoalMetric::from_api(name).api_name(), name);
        }
        assert_eq!(
            GoalMetric::from_api("minute"),
            GoalMetric::Other("minute".into())
        );
    }

    #[test]
    fn goal_conditions_go_back_unchanged_or_not_at_all() {
        use serde_json::json;
        let empty = GoalConditions(json!({}));
        assert!(empty.is_empty());
        assert_eq!(empty.to_input(), Some(json!({})));
        assert_eq!(GoalConditions(json!(null)).to_input(), Some(json!({})));
        let audio = GoalConditions(json!({"readingFormatId": 2, "bookCategoryIds": []}));
        assert!(!audio.is_empty());
        assert_eq!(
            audio.to_input(),
            Some(json!({"readingFormatId": 2, "bookCategoryIds": []}))
        );
        // Nulls are dropped; an unknown field can't be sent back.
        assert_eq!(
            GoalConditions(json!({"authorBipoc": null})).to_input(),
            Some(json!({}))
        );
        assert_eq!(GoalConditions(json!({"genre": "poetry"})).to_input(), None);
    }

    #[test]
    fn search_page_has_more() {
        let mut p = SearchPage::new(vec![], 1, 25);
        p.total = Some(30);
        assert!(p.has_more());
        p.page = 2;
        assert!(!p.has_more());
    }
}

/// The user's review of a library entry (`user_books.review_*`).
///
/// Not part of [`LibraryEntry`]: review text can be long, so the library sync
/// leaves it out and the book page fetches it with [`crate::api::Client::review`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Review {
    /// The review as Markdown (`review_markdown`).
    pub markdown: Option<String>,
    /// The review as plain text (`review_raw`). Shown when Hardcover has no
    /// Markdown for a review.
    pub plain: Option<String>,
    pub has_spoilers: bool,
    /// The day the review is dated.
    pub reviewed_at: Option<Date>,
    /// Hardcover's `has_review`. True with no text is a review kept in a form
    /// this app can't read (only the website's own editor format).
    #[serde(default)]
    pub has_review: bool,
}

impl Review {
    pub fn new(markdown: impl Into<String>, has_spoilers: bool, reviewed_at: Option<Date>) -> Self {
        Self {
            markdown: Some(markdown.into()),
            plain: None,
            has_spoilers,
            reviewed_at,
            has_review: true,
        }
    }

    /// True if the entry has a review, readable here or not.
    pub fn exists(&self) -> bool {
        self.has_review || self.text().is_some()
    }

    /// The text to show and to edit: the Markdown, else the plain text.
    pub fn text(&self) -> Option<&str> {
        self.markdown.as_deref().or(self.plain.as_deref())
    }
}

/// A library entry and its review, as a review write left them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReviewedEntry {
    /// `None` if the write's response had no entry and reading it back failed:
    /// the review was written, and the entry was left as it was.
    pub entry: Option<LibraryEntry>,
    pub review: Review,
}

/// What [`crate::api::Client::set_edition`] did.
#[derive(Debug)]
pub struct EditionChange {
    /// The entry as of the change.
    pub entry: LibraryEntry,
    /// Why the open read could not move to the new edition, if it couldn't.
    /// The edition change itself went through.
    pub read_left_behind: Option<crate::Error>,
}

/// Another reader's review of a book, for the book page's "Reviews" section.
/// Reading them needs `read:social`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CommunityReview {
    /// The reviewer's `user_books` row.
    pub id: UserBookId,
    pub reviewer: Reviewer,
    pub rating: Option<Rating>,
    /// `likes_count`.
    pub likes: u32,
    pub reviewed_at: Option<Date>,
    pub has_spoilers: bool,
    /// `review_raw`: plain text, paragraphs separated by line breaks. Never empty.
    pub text: String,
}

impl CommunityReview {
    pub fn new(id: UserBookId, reviewer: Reviewer, text: impl Into<String>) -> Self {
        Self {
            id,
            reviewer,
            rating: None,
            likes: 0,
            reviewed_at: None,
            has_spoilers: false,
            text: text.into(),
        }
    }

    /// The paragraphs of [`Self::text`], trimmed, without empty ones.
    pub fn paragraphs(&self) -> impl Iterator<Item = &str> {
        self.text
            .split('\n')
            .map(str::trim)
            .filter(|p| !p.is_empty())
    }
}

/// The author of a [`CommunityReview`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Reviewer {
    pub id: UserId,
    /// `None` when Hardcover sent none: then there is no profile to link to.
    pub username: Option<String>,
    /// `None` when blank.
    pub name: Option<String>,
    /// `cached_image`. Often a default avatar, sometimes an SVG.
    pub avatar: Option<Cover>,
    /// `cached_image.color`, the avatar's main colour, as RGB.
    pub color: Option<[u8; 3]>,
}

impl Reviewer {
    pub fn new(id: UserId) -> Self {
        Self {
            id,
            username: None,
            name: None,
            avatar: None,
            color: None,
        }
    }

    /// The name to show: the name, else `@username`, else "A reader".
    pub fn display_name(&self) -> String {
        match (&self.name, &self.username) {
            (Some(name), _) => name.clone(),
            (None, Some(username)) => format!("@{username}"),
            (None, None) => "A reader".to_owned(),
        }
    }

    /// The reviewer's profile on hardcover.app, if their username is known.
    pub fn profile_url(&self) -> Option<String> {
        self.username
            .as_deref()
            .map(|u| format!("https://hardcover.app/@{u}"))
    }

    /// The avatar, unless it is an SVG: the cover pipeline decodes only
    /// raster images.
    pub fn raster_avatar(&self) -> Option<&Cover> {
        self.avatar.as_ref().filter(|a| {
            let path = a.url.split(['?', '#']).next().unwrap_or_default();
            !path.to_ascii_lowercase().ends_with(".svg")
        })
    }
}

/// One page of [`CommunityReview`]s, from [`crate::api::Client::book_reviews`].
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ReviewsPage {
    /// The page's reviews. Rows with no text are left out, so this can be
    /// shorter than the page.
    pub reviews: Vec<CommunityReview>,
    /// The offset of the next page: this page's offset plus the rows it read.
    pub next_offset: u32,
    /// True if Hardcover has more rows after this page.
    pub has_more: bool,
}

#[cfg(test)]
mod community_review_tests {
    use super::*;

    #[test]
    fn a_reviewer_is_named_by_name_then_username() {
        let mut r = Reviewer::new(UserId::new(7));
        assert_eq!(r.display_name(), "A reader");
        assert_eq!(r.profile_url(), None);
        r.username = Some("quill".into());
        assert_eq!(r.display_name(), "@quill");
        assert_eq!(
            r.profile_url().as_deref(),
            Some("https://hardcover.app/@quill")
        );
        r.name = Some("Jane Quill".into());
        assert_eq!(r.display_name(), "Jane Quill");
    }

    #[test]
    fn svg_avatars_are_not_raster() {
        let mut r = Reviewer::new(UserId::new(7));
        r.avatar = Some(Cover::new("https://assets.example/users/7/a.SVG?v=2"));
        assert!(r.raster_avatar().is_none());
        r.avatar = Some(Cover::new("https://assets.example/users/7/a.png"));
        assert!(r.raster_avatar().is_some());
    }

    #[test]
    fn paragraphs_skip_blank_lines() {
        let r = CommunityReview::new(
            UserBookId::new(1),
            Reviewer::new(UserId::new(2)),
            "First.\n\n  Second.  \n\n\n",
        );
        assert_eq!(r.paragraphs().collect::<Vec<_>>(), ["First.", "Second."]);
    }
}
