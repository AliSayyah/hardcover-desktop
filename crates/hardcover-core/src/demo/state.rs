//! The demo reader's data as it stands in this run: their library, reads,
//! reviews, lists and goals. Seeded from `catalogue.json` when the client is
//! built, with every day offset turned into a date from that day; writes
//! change it in memory, and nothing outlives the client.

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Utc};
use serde_json::{Value, json};

use super::{Book, CATALOGUE, Catalogue, DemoEdition, ListItem, SeedEntry, SeedGoal, SeedList};

/// Where the demo's "now" comes from. Tests pin it, so dates don't move with
/// the calendar.
#[derive(Clone, Copy)]
pub(super) enum Clock {
    System,
    #[cfg(test)]
    Fixed(DateTime<FixedOffset>),
}

impl Clock {
    pub(super) fn now(self) -> DateTime<FixedOffset> {
        match self {
            Clock::System => chrono::Local::now().fixed_offset(),
            #[cfg(test)]
            Clock::Fixed(now) => now,
        }
    }

    /// Today, in the clock's time zone.
    pub(super) fn today(self) -> NaiveDate {
        self.now().date_naive()
    }
}

/// Ids given to rows the demo creates, far above the catalogue's own.
const FIRST_NEW_ID: i64 = 1_000_000;

pub(super) struct State {
    pub(super) clock: Clock,
    pub(super) library: Vec<Entry>,
    pub(super) lists: Vec<List>,
    pub(super) goals: Vec<Goal>,
    pub(super) vibes: Vec<super::vibes::Vibe>,
    pub(super) follows: Vec<super::follows::Follow>,
    next_id: i64,
}

/// A `user_books` row.
pub(super) struct Entry {
    pub(super) id: i64,
    pub(super) book: i64,
    pub(super) status: i64,
    pub(super) rating: Option<f64>,
    pub(super) date_added: NaiveDate,
    pub(super) updated_at: DateTime<Utc>,
    pub(super) privacy: i64,
    pub(super) private_notes: Option<String>,
    pub(super) edition: Option<i64>,
    pub(super) reads: Vec<Read>,
    pub(super) review: Review,
}

/// A `user_book_reads` row.
#[derive(Clone)]
pub(super) struct Read {
    pub(super) id: i64,
    pub(super) started: Option<NaiveDate>,
    pub(super) finished: Option<NaiveDate>,
    pub(super) progress_pages: Option<u32>,
    pub(super) progress_seconds: Option<u32>,
    pub(super) edition: Option<i64>,
}

/// An entry's review columns.
#[derive(Default)]
pub(super) struct Review {
    pub(super) markdown: Option<String>,
    /// Hardcover's plain-text copy, filled when Markdown is saved.
    pub(super) raw: Option<String>,
    pub(super) spoilers: bool,
    /// Kept when the review is deleted, as on Hardcover.
    pub(super) reviewed_at: Option<NaiveDate>,
}

impl Review {
    pub(super) fn exists(&self) -> bool {
        self.markdown.is_some() || self.raw.as_deref().is_some_and(|r| !r.is_empty())
    }
}

/// A `lists` row with its `list_books`.
pub(super) struct List {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) slug: String,
    pub(super) description: Option<String>,
    pub(super) privacy: i64,
    pub(super) ranked: bool,
    pub(super) default_view: String,
    pub(super) featured_profile: bool,
    pub(super) url: Option<String>,
    pub(super) rows: Vec<ListRow>,
}

impl List {
    pub(super) fn is_owned(&self) -> bool {
        self.slug == crate::api::OWNED_LIST_SLUG
    }
}

/// A `list_books` row.
pub(super) struct ListRow {
    pub(super) id: i64,
    pub(super) book: i64,
    pub(super) edition: Option<i64>,
    pub(super) position: Option<i64>,
}

/// A `goals` row.
pub(super) struct Goal {
    pub(super) id: i64,
    pub(super) metric: String,
    pub(super) goal: i64,
    pub(super) description: String,
    pub(super) start: NaiveDate,
    pub(super) end: NaiveDate,
    pub(super) conditions: Value,
    pub(super) archived: bool,
    pub(super) privacy: i64,
}

/// What progress on a read is counted in, and the length in that unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Length {
    Pages(u32),
    Seconds(u32),
}

impl State {
    pub(super) fn new(catalogue: &Catalogue, clock: Clock) -> Self {
        let now = clock.now();
        let today = now.date_naive();
        let day = |offset: i64| today + Duration::days(offset);
        let library = catalogue
            .library
            .iter()
            .map(|e| seed_entry(e, now, &day))
            .collect();
        let lists = catalogue
            .lists
            .iter()
            .map(|l| seed_list(catalogue, l))
            .collect();
        let goals = catalogue
            .goals
            .iter()
            .map(|g| seed_goal(g, today.year()))
            .collect();
        let vibes = super::vibes::seed(
            &catalogue.vibes,
            &catalogue.user,
            now.with_timezone(&Utc),
            super::vibes::SystemVibes::Built,
        );
        let follows = super::follows::seed(catalogue, now.with_timezone(&Utc));
        Self {
            clock,
            library,
            lists,
            goals,
            vibes,
            follows,
            next_id: FIRST_NEW_ID,
        }
    }

    /// The reader's own vibes as `system` says they start.
    pub(super) fn with_system_vibes(mut self, system: super::vibes::SystemVibes) -> Self {
        if system != super::vibes::SystemVibes::Built {
            self.vibes = super::vibes::seed(
                &super::CATALOGUE.vibes,
                &super::CATALOGUE.user,
                self.clock.now().with_timezone(&Utc),
                system,
            );
        }
        self
    }

    pub(super) fn new_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    pub(super) fn entry(&self, id: i64) -> Option<&Entry> {
        self.library.iter().find(|e| e.id == id)
    }

    pub(super) fn entry_mut(&mut self, id: i64) -> Option<&mut Entry> {
        self.library.iter_mut().find(|e| e.id == id)
    }

    pub(super) fn list(&self, id: i64) -> Option<&List> {
        self.lists.iter().find(|l| l.id == id)
    }

    /// `GoalFields`. Progress is worked out from the library as Hardcover
    /// does: the reads finished within the goal's dates, both included.
    pub(super) fn goal_json(&self, goal: &Goal) -> Value {
        let finished = self.library.iter().flat_map(|e| {
            e.reads
                .iter()
                .filter(|r| {
                    r.finished
                        .is_some_and(|f| (goal.start..=goal.end).contains(&f))
                })
                .map(move |r| (e, r))
        });
        let progress: i64 = match goal.metric.as_str() {
            "page" => finished
                .filter_map(|(e, _)| CATALOGUE.book(e.book)?.pages)
                .map(i64::from)
                .sum(),
            // Hardcover counts an hour goal's progress in seconds.
            "hour" => finished
                .filter_map(|(e, r)| match read_length(e, r) {
                    Some(Length::Seconds(s)) => Some(i64::from(s)),
                    _ => None,
                })
                .sum(),
            _ => finished.fold(0_i64, |count, _| count + 1),
        };
        json!({
            "id": goal.id,
            "metric": goal.metric,
            "goal": goal.goal,
            "progress": progress,
            "description": goal.description,
            "start_date": goal.start.to_string(),
            "end_date": goal.end.to_string(),
            "conditions": goal.conditions,
            "archived": goal.archived,
            "completed_at": null,
            "privacy_setting_id": goal.privacy,
        })
    }
}

fn seed_entry(e: &SeedEntry, now: DateTime<FixedOffset>, day: &impl Fn(i64) -> NaiveDate) -> Entry {
    let reads = e
        .reads
        .iter()
        .map(|r| Read {
            id: r.id,
            started: r.started.map(day),
            finished: r.finished.map(day),
            progress_pages: r.progress_pages,
            progress_seconds: r.progress_seconds,
            edition: r.edition,
        })
        .collect();
    let review = e.review.as_ref().map_or_default(|r| Review {
        markdown: Some(r.markdown.clone()),
        raw: Some(r.markdown.clone()),
        spoilers: r.spoilers,
        reviewed_at: Some(day(r.reviewed)),
    });
    Entry {
        id: e.id,
        book: e.book,
        status: e.status,
        rating: e.rating,
        date_added: day(e.added),
        updated_at: (now + Duration::days(e.updated)).to_utc(),
        privacy: e.privacy.unwrap_or(1),
        private_notes: e.private_notes.clone(),
        edition: e.edition,
        reads,
        review,
    }
}

/// Row ids are the list's id times 1,000 plus the place. The Owned list holds
/// editions: the one named, else the book's first.
fn seed_list(catalogue: &Catalogue, l: &SeedList) -> List {
    let owned = l.slug == crate::api::OWNED_LIST_SLUG;
    let rows = (1_i64..)
        .zip(&l.books)
        .map(|(place, item)| {
            let (book, edition) = match *item {
                ListItem::Book(book) => (book, None),
                ListItem::Edition { book, edition } => (book, Some(edition)),
            };
            let edition = edition.or_else(|| {
                owned
                    .then(|| catalogue.book(book).map(Book::default_edition))
                    .flatten()
            });
            ListRow {
                id: l.id * 1_000 + place,
                book,
                edition,
                position: l.ranked.then_some(place),
            }
        })
        .collect();
    List {
        id: l.id,
        name: l.name.clone(),
        slug: l.slug.clone(),
        description: l.description.clone(),
        privacy: l.privacy,
        ranked: l.ranked,
        default_view: "list".to_owned(),
        featured_profile: false,
        url: None,
        rows,
    }
}

/// A goal for the calendar year `year`, dated as Hardcover dates its own year
/// goals: Jan 1 to Jan 1 of the next year.
fn seed_goal(g: &SeedGoal, year: i32) -> Goal {
    let jan_1 = |year| NaiveDate::from_ymd_opt(year, 1, 1).expect("Jan 1 exists");
    Goal {
        id: g.id,
        metric: g.metric.clone(),
        goal: g.goal,
        description: format!("{year} Reading Goal"),
        start: jan_1(year),
        end: jan_1(year + 1),
        conditions: json!({}),
        archived: false,
        privacy: 1,
    }
}

/// The edition a read counts against: its own, else its entry's.
pub(super) fn read_edition(
    entry: &Entry,
    read: &Read,
) -> Option<(&'static Book, Option<&'static DemoEdition>)> {
    let id = read.edition.or(entry.edition)?;
    CATALOGUE.edition(id)
}

/// What a read's progress is counted in, as the app's `measure_of` decides:
/// an audiobook edition counts time, by its own running time only; any other
/// edition counts pages, its own or else the book's.
pub(super) fn read_length(entry: &Entry, read: &Read) -> Option<Length> {
    let book_pages = || {
        CATALOGUE
            .book(entry.book)
            .and_then(|b| b.pages)
            .map(Length::Pages)
    };
    match read_edition(entry, read) {
        Some((_, Some(e))) if e.is_audiobook() => e.audio_seconds.map(Length::Seconds),
        Some((_, Some(e))) => e.pages.map(Length::Pages).or_else(book_pages),
        _ => book_pages(),
    }
}
