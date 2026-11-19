//! Display text derived from core models: status labels, progress, ratings,
//! relative times and user-facing error copy.
//!
//! Plain functions without GPUI so they can be unit tested directly.

use chrono::{DateTime, Datelike, TimeZone};
use hardcover_core::auth::{DEFAULT_SCOPES, scopes};
use hardcover_core::{
    AuthorHit, Date, EditionSummary, Error, Goal, GoalMetric, Length, LibraryEntry, Rating,
    ReadingStatus, SeriesHit,
};

/// Sentence-case label for a status, as the app shows it.
pub fn status_label(status: ReadingStatus) -> &'static str {
    match status {
        ReadingStatus::WantToRead => "Want to read",
        ReadingStatus::CurrentlyReading => "Currently reading",
        ReadingStatus::Read => "Read",
        ReadingStatus::Paused => "Paused",
        ReadingStatus::DidNotFinish => "Did not finish",
        ReadingStatus::Ignored => "Ignored",
    }
}

/// The library shelves in sidebar order, Ignored last. Ctrl+1…6 follow this order.
pub const SIDEBAR_SHELVES: [ReadingStatus; 6] = [
    ReadingStatus::CurrentlyReading,
    ReadingStatus::WantToRead,
    ReadingStatus::Read,
    ReadingStatus::Paused,
    ReadingStatus::DidNotFinish,
    ReadingStatus::Ignored,
];

/// Reading progress of an entry's open read, ready for display.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressSummary {
    /// Current page, when the read records pages.
    pub page: Option<u32>,
    /// Page count of the edition being read.
    pub total_pages: Option<u32>,
    /// Position in an audiobook, when the read records time.
    pub seconds: Option<u32>,
    /// Running time of the audiobook being listened to.
    pub total_seconds: Option<u32>,
    /// 0..=100, from the server or derived from pages or time.
    pub percent: Option<f32>,
}

impl ProgressSummary {
    /// True when progress is counted in time: the edition is an audiobook.
    pub fn is_audio(&self) -> bool {
        self.total_seconds.is_some()
    }

    /// "Page 120 of 350", "Page 120", "2h 5m of 10h 30m", "42%", or "Not started".
    pub fn text(&self) -> String {
        match (self.page, self.total_pages, self.seconds, self.percent) {
            (Some(page), Some(total), _, _) => format!("Page {page} of {total}"),
            (Some(page), None, _, _) => format!("Page {page}"),
            (None, _, Some(secs), _) => match self.total_seconds {
                Some(total) => format!("{} of {}", duration_text(secs), duration_text(total)),
                None => duration_text(secs),
            },
            (None, _, None, Some(pct)) => format!("{}%", pct.round() as u32),
            (None, _, None, None) => "Not started".to_owned(),
        }
    }

    /// Compact form for a list row's trailing lane: "42%" or "p. 120".
    pub fn short_text(&self) -> Option<String> {
        match (self.percent, self.page) {
            (Some(pct), _) => Some(format!("{}%", pct.round() as u32)),
            (None, Some(page)) => Some(format!("p. {page}")),
            (None, None) => None,
        }
    }
}

/// Progress of the entry's unfinished read, if it has one, in the unit of
/// [`LibraryEntry::length`]: pages, or time for an audiobook.
pub fn progress_summary(entry: &LibraryEntry) -> ProgressSummary {
    let (total_pages, total_seconds) = match entry.length() {
        Some(Length::Pages(p)) => (Some(p), None),
        Some(Length::Seconds(s)) => (None, Some(s)),
        None => (None, None),
    };
    let read = entry.open_read();
    let page = read
        .and_then(|r| r.progress_pages)
        .filter(|p| *p > 0 && total_seconds.is_none());
    let seconds = read
        .and_then(|r| r.progress_seconds)
        .filter(|s| *s > 0 && page.is_none());
    let fraction = |done: u32, total: u32| (f64::from(done) / f64::from(total) * 100.0) as f32;
    let percent = read
        .and_then(|r| r.progress_percent)
        .or_else(|| match (page, total_pages, seconds, total_seconds) {
            (Some(p), Some(t), _, _) => Some(fraction(p, t)),
            (_, _, Some(s), Some(t)) => Some(fraction(s, t)),
            _ => None,
        })
        .map(|p| p.clamp(0.0, 100.0));
    ProgressSummary {
        page,
        total_pages,
        seconds,
        total_seconds,
        percent,
    }
}

/// "10h 5m", "45m", "0m" for a position or running time.
pub fn duration_text(seconds: u32) -> String {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// "2:05" (hours and minutes), or "2:05:30" when the seconds are not zero, for
/// an audiobook position field. Exact, so submitting the field unchanged writes
/// the same position back.
pub fn position_text(seconds: u32) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if s == 0 {
        format!("{h}:{m:02}")
    } else {
        format!("{h}:{m:02}:{s:02}")
    }
}

/// A position typed as "h:mm" or "h:mm:ss", in seconds.
pub fn parse_position(text: &str) -> Option<u32> {
    let parts: Vec<u32> = text
        .trim()
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    match parts[..] {
        [h, m] if m < 60 => Some(h * 3600 + m * 60),
        [h, m, s] if m < 60 && s < 60 => Some(h * 3600 + m * 60 + s),
        _ => None,
    }
}

/// "Nov 3, 2024", as the date pickers show dates.
pub fn date_text(date: Date) -> String {
    date.format("%b %-d, %Y").to_string()
}

/// One line about an edition: "Paperback · 412 pages · Penguin, 2004".
/// `None` when nothing is known beyond its id.
pub fn edition_text(edition: &EditionSummary) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(format) = edition.format.as_ref().or(edition.reading_format.as_ref()) {
        parts.push(format.clone());
    }
    match edition.length() {
        Some(Length::Pages(p)) => parts.push(format!("{p} pages")),
        Some(Length::Seconds(s)) => parts.push(duration_text(s)),
        None => {}
    }
    let year = edition.release_date.map(|d| d.year().to_string());
    match (&edition.publisher, year) {
        (Some(p), Some(y)) => parts.push(format!("{p}, {y}")),
        (Some(p), None) => parts.push(p.clone()),
        (None, Some(y)) => parts.push(y),
        (None, None) => {}
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// "Jan 3, 2019 – Feb 1, 2019", "Started Jan 3, 2019", "Finished Feb 1, 2019"
/// or "No dates" for a read.
pub fn read_dates_text(started: Option<Date>, finished: Option<Date>) -> String {
    match (started, finished) {
        (Some(s), Some(f)) => format!("{} – {}", date_text(s), date_text(f)),
        (Some(s), None) => format!("Started {}", date_text(s)),
        (None, Some(f)) => format!("Finished {}", date_text(f)),
        (None, None) => "No dates".to_owned(),
    }
}

/// "4", "3.5".
pub fn rating_text(rating: Rating) -> String {
    let v = rating.value();
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Hardcover's community average, "4.12".
pub fn average_rating_text(avg: f32) -> String {
    format!("{avg:.2}")
}

/// Joined author list, or a placeholder.
pub fn authors_text(authors: &[String]) -> String {
    if authors.is_empty() {
        "Unknown author".to_owned()
    } else {
        authors.join(", ")
    }
}

/// "#3", "#2.5" for a series position.
pub fn series_position_text(position: f32) -> String {
    if position.fract() == 0.0 {
        format!("#{position:.0}")
    } else {
        format!("#{position}")
    }
}

/// A count with thousands separated: "950", "12,480".
pub fn count_text(count: u32) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1 book", "1,200 books".
pub fn books_count_text(count: u32) -> String {
    match count {
        1 => "1 book".to_owned(),
        n => format!("{} books", count_text(n)),
    }
}

/// An author result's secondary line: their book count and best-known titles,
/// "12 books · Dune, Dune Messiah".
pub fn author_hit_details(author: &AuthorHit) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(count) = author.books_count.filter(|n| *n > 0) {
        parts.push(books_count_text(count));
    }
    if !author.top_books.is_empty() {
        parts.push(
            author
                .top_books
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    parts.join(" · ")
}

/// A series result's secondary line: "by Frank Herbert · 6 books", or its
/// first titles when Hardcover gives no count.
pub fn series_hit_details(series: &SeriesHit) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(author) = &series.author {
        parts.push(format!("by {author}"));
    }
    match series
        .primary_books_count
        .or(series.books_count)
        .filter(|n| *n > 0)
    {
        Some(count) => parts.push(books_count_text(count)),
        None if !series.books.is_empty() => parts.push(
            series
                .books
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        ),
        None => {}
    }
    parts.join(" · ")
}

/// "just now", "5 min ago", "3 hours ago", "yesterday", "4 days ago",
/// "on Nov 3" or "on Nov 3, 2024".
pub fn relative_time<Tz: TimeZone>(now: &DateTime<Tz>, then: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let secs = (now.clone() - then.clone()).num_seconds().max(0);
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    match () {
        _ if mins < 1 => "just now".to_owned(),
        _ if mins < 60 => format!("{mins} min ago"),
        _ if hours == 1 => "1 hour ago".to_owned(),
        _ if hours < 24 => format!("{hours} hours ago"),
        _ if days == 1 => "yesterday".to_owned(),
        _ if days < 7 => format!("{days} days ago"),
        _ if then.year() == now.year() => format!("on {}", then.format("%b %-d")),
        _ => format!("on {}", then.format("%b %-d, %Y")),
    }
}

/// What to tell the user when a request fails, with a recovery hint.
pub fn error_message(error: &Error) -> String {
    match error {
        Error::RateLimited { .. } => {
            "Hardcover's rate limit was reached. Try again in a minute.".to_owned()
        }
        Error::Network(_) | Error::Timeout => {
            "Couldn't reach Hardcover. Check your connection and try again.".to_owned()
        }
        Error::Unauthorized => "Your session has expired. Sign out and sign in again.".to_owned(),
        // Where the permissions notice can offer to sign in again, the caller
        // says so instead (`Services::error_message`).
        Error::InsufficientScope { .. } => {
            "Hardcover didn't grant this app permission for this. Sign out and sign in again to grant it."
                .to_owned()
        }
        Error::InvalidInput(msg) => sentence(msg),
        Error::Storage(msg) => format!("Couldn't save data on this computer: {msg}."),
        Error::Auth(msg) => format!("Sign-in failed: {msg}."),
        Error::Cancelled => "Sign-in was cancelled.".to_owned(),
        Error::Api(msg) => format!("Hardcover reported an error: {msg}."),
        Error::Decode(_) => "Hardcover sent an unexpected response. Try again later.".to_owned(),
        _ => format!("{error}."),
    }
}

/// What the user loses without a scope, in the words of the permissions notice.
fn scope_feature(scope: &str) -> &'static str {
    match scope {
        scopes::READ_GOALS | scopes::WRITE_GOALS => "reading goals",
        scopes::WRITE_REVIEWS => "reviews",
        scopes::READ_LISTS | scopes::WRITE_LISTS => "lists",
        _ => "your library",
    }
}

/// The scope an `insufficient_scope` error names, as one the app requests.
/// Hardcover may name a narrower part (`read:goals:private`) or several scopes;
/// the first one the app requests wins.
pub fn requested_scope(named: Option<&str>) -> Option<&'static str> {
    named?.split_whitespace().find_map(|name| {
        DEFAULT_SCOPES.into_iter().find(|s| {
            name == *s
                || name
                    .strip_prefix(s)
                    .is_some_and(|rest| rest.starts_with(':'))
        })
    })
}

/// A request refused for lack of `scope`, while the permissions notice (or the
/// account menu) offers to sign in again.
pub fn scope_refused_message(scope: &str) -> String {
    format!(
        "Hardcover didn't grant this app permission for {}. Sign in again to grant it.",
        scope_feature(scope)
    )
}

/// `a`, `a and b`, `a, b and c`.
fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The features that `missing` scopes cover, in a fixed order.
pub fn missing_features(missing: &[&str]) -> Vec<&'static str> {
    const ORDER: [&str; 4] = ["your library", "reading goals", "reviews", "lists"];
    ORDER
        .into_iter()
        .filter(|f| missing.iter().any(|s| scope_feature(s) == *f))
        .collect()
}

/// The permissions notice: which features need the user to sign in again.
pub fn permissions_message(missing: &[&str]) -> String {
    format!(
        "Hardcover Desktop needs new permissions for {}. Sign in again to grant them.",
        join_and(&missing_features(missing))
    )
}

/// Why a feature is unavailable, for a disabled control's tooltip.
pub fn needs_permission(scope: &str) -> String {
    format!(
        "Hardcover Desktop has no permission for {}. Sign in again from the sidebar to grant it.",
        scope_feature(scope)
    )
}

// ----- reading goals -----

/// The unit a goal counts, singular or plural for `n`.
pub fn goal_unit(metric: &GoalMetric, n: f64) -> &str {
    let one = (n - 1.0).abs() < f64::EPSILON;
    match metric {
        GoalMetric::Books if one => "book",
        GoalMetric::Books => "books",
        GoalMetric::Pages if one => "page",
        GoalMetric::Pages => "pages",
        GoalMetric::Hours if one => "hour",
        GoalMetric::Hours => "hours",
        GoalMetric::Other(name) => name,
    }
}

/// An amount in a goal's unit, rounded down: whole books and pages, hours to
/// one decimal (`12.5`, or `12` when whole). Down, so an amount short of the
/// target never shows as the target.
fn goal_amount(metric: &GoalMetric, n: f64) -> String {
    match metric {
        GoalMetric::Hours => {
            // The epsilon keeps 2.3 (22.999… tenths in binary) from showing as 2.2.
            let tenths = (n * 10.0 + 1e-6).floor() as i64;
            match tenths % 10 {
                0 => format!("{}", tenths / 10),
                t => format!("{}.{}", tenths / 10, t.abs()),
            }
        }
        _ => format!("{}", n.floor() as i64),
    }
}

/// `2026` for a calendar year, otherwise the dates: `Mar 1 – Jun 30, 2026`.
pub fn goal_period(start: Date, end: Date) -> String {
    let whole_year = start.year() == end.year()
        && (start.month(), start.day()) == (1, 1)
        && (end.month(), end.day()) == (12, 31);
    if whole_year {
        start.year().to_string()
    } else if start.year() == end.year() {
        format!("{} – {}", start.format("%b %-d"), end.format("%b %-d, %Y"))
    } else {
        format!(
            "{} – {}",
            start.format("%b %-d, %Y"),
            end.format("%b %-d, %Y")
        )
    }
}

/// `9 / 12 books`.
pub fn goal_progress(goal: &Goal) -> String {
    format!(
        "{} / {} {}",
        goal_amount(&goal.metric, goal.progress),
        goal.target,
        goal_unit(&goal.metric, f64::from(goal.target))
    )
}

/// How the goal stands on `today`: `2 books behind schedule`, `On schedule`,
/// `Goal reached`. `None` for a metric this version does not know.
pub fn goal_pace(goal: &Goal, today: Date) -> Option<String> {
    if matches!(goal.metric, GoalMetric::Other(_)) {
        return None;
    }
    if goal.progress >= f64::from(goal.target) {
        return Some("Goal reached".to_owned());
    }
    if today < goal.start {
        return Some(format!("Starts {}", goal.start.format("%b %-d, %Y")));
    }
    if today > goal.end {
        let short = (f64::from(goal.target) - goal.progress).ceil();
        return Some(format!(
            "Ended {} {} short",
            goal_amount(&goal.metric, short),
            goal_unit(&goal.metric, short)
        ));
    }
    let pace = goal.pace(today).round();
    Some(if pace == 0.0 {
        "On schedule".to_owned()
    } else {
        let n = pace.abs();
        let side = if pace > 0.0 { "ahead of" } else { "behind" };
        format!(
            "{} {} {side} schedule",
            goal_amount(&goal.metric, n),
            goal_unit(&goal.metric, n)
        )
    })
}

/// The description Hardcover requires for a new goal: `12 books in 2026`, or
/// `500 pages by Jun 30, 2026` for other dates.
pub fn goal_description(metric: &GoalMetric, target: u32, start: Date, end: Date) -> String {
    let unit = goal_unit(metric, f64::from(target));
    let period = goal_period(start, end);
    if period.len() == 4 {
        format!("{target} {unit} in {period}")
    } else {
        format!("{target} {unit} by {}", end.format("%b %-d, %Y"))
    }
}

/// The description to save with a goal's new target. One the app would have
/// written follows the target, and so does a missing one, since Hardcover
/// requires one; the user's own stays.
pub fn retargeted_description(goal: &Goal, target: u32) -> String {
    let written = goal_description(&goal.metric, goal.target, goal.start, goal.end);
    match goal.description.as_deref() {
        Some(own) if !own.trim().is_empty() && own != written => own.to_owned(),
        _ => goal_description(&goal.metric, target, goal.start, goal.end),
    }
}

/// Upper-cases the first letter and ends with a period.
fn sentence(msg: &str) -> String {
    let msg = msg.trim().trim_end_matches('.');
    let mut chars = msg.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use hardcover_core::{
        BookId, BookSummary, EditionId, EditionSummary, GoalId, ReadId, ReadSession, UserBookId,
        UserId,
    };

    fn entry_with_read(
        pages: Option<u32>,
        percent: Option<f32>,
        total: Option<u32>,
    ) -> LibraryEntry {
        let mut book = BookSummary::new(BookId::new(1), "Book");
        book.pages = total;
        let mut e = LibraryEntry::new(
            UserBookId::new(1),
            UserId::new(1),
            book,
            ReadingStatus::CurrentlyReading,
        );
        let mut read = ReadSession::new(ReadId::new(9));
        read.progress_pages = pages;
        read.progress_percent = percent;
        e.upsert_read(read);
        e
    }

    #[test]
    fn permissions_message_names_the_missing_features() {
        let missing = [
            scopes::WRITE_REVIEWS,
            scopes::READ_GOALS,
            scopes::WRITE_GOALS,
            scopes::READ_LISTS,
            scopes::WRITE_LISTS,
        ];
        assert_eq!(
            permissions_message(&missing),
            "Hardcover Desktop needs new permissions for reading goals, reviews and lists. \
             Sign in again to grant them."
        );
        assert_eq!(
            permissions_message(&[scopes::WRITE_GOALS]),
            "Hardcover Desktop needs new permissions for reading goals. Sign in again to grant them."
        );
        assert_eq!(
            missing_features(&[scopes::WRITE_LISTS, scopes::READ_LIBRARY]),
            ["your library", "lists"]
        );
    }

    fn goal(metric: GoalMetric, target: u32, progress: f64) -> Goal {
        let d = |m, d| Date::from_ymd_opt(2026, m, d).unwrap();
        let mut g = Goal::new(GoalId::new(1), metric, target, d(1, 1), d(12, 31));
        g.progress = progress;
        g
    }

    #[test]
    fn a_new_target_keeps_the_users_own_description() {
        let mut g = goal(GoalMetric::Books, 12, 3.0);
        g.description = Some("12 books in 2026".into());
        assert_eq!(retargeted_description(&g, 20), "20 books in 2026");
        // Hardcover requires one: a missing or blank one is written.
        for missing in [None, Some(" ".to_owned())] {
            g.description = missing;
            assert_eq!(retargeted_description(&g, 20), "20 books in 2026");
        }
        g.description = Some("Read more sci-fi".into());
        assert_eq!(retargeted_description(&g, 20), "Read more sci-fi");
    }

    #[test]
    fn refused_scopes_map_to_requested_ones() {
        assert_eq!(
            requested_scope(Some("read:goals")),
            Some(scopes::READ_GOALS)
        );
        assert_eq!(
            requested_scope(Some("read:goals:private")),
            Some(scopes::READ_GOALS)
        );
        assert_eq!(
            requested_scope(Some("read:journal write:library")),
            Some(scopes::WRITE_LIBRARY)
        );
        // Not a part of read:goals, and not a scope the app requests.
        assert_eq!(requested_scope(Some("read:goalsx")), None);
        assert_eq!(requested_scope(Some("read:journal")), None);
        assert_eq!(requested_scope(None), None);
        assert_eq!(
            scope_refused_message(scopes::WRITE_LIBRARY),
            "Hardcover didn't grant this app permission for your library. Sign in again to grant it."
        );
    }

    #[test]
    fn goal_copy() {
        let d = |y, m, d| Date::from_ymd_opt(y, m, d).unwrap();
        let books = goal(GoalMetric::Books, 12, 9.0);
        assert_eq!(goal_progress(&books), "9 / 12 books");
        assert_eq!(goal_period(books.start, books.end), "2026");
        assert_eq!(
            goal_period(d(2026, 3, 1), d(2026, 6, 30)),
            "Mar 1 – Jun 30, 2026"
        );
        assert_eq!(
            goal_period(d(2026, 12, 1), d(2027, 1, 31)),
            "Dec 1, 2026 – Jan 31, 2027"
        );
        // 12 a year: 10.6 expected by Nov 19.
        assert_eq!(
            goal_pace(&books, d(2026, 11, 19)).unwrap(),
            "2 books behind schedule"
        );
        assert_eq!(
            goal_pace(&books, d(2026, 9, 1)).unwrap(),
            "1 book ahead of schedule"
        );
        assert_eq!(goal_pace(&books, d(2026, 10, 10)).unwrap(), "On schedule");
        assert_eq!(
            goal_pace(&books, d(2027, 1, 5)).unwrap(),
            "Ended 3 books short"
        );
        assert_eq!(
            goal_pace(&goal(GoalMetric::Books, 12, 12.0), d(2026, 2, 1)).unwrap(),
            "Goal reached"
        );
        let hours = goal(GoalMetric::Hours, 100, 12.5);
        assert_eq!(goal_progress(&hours), "12.5 / 100 hours");
        // Rounded down once, so an amount short of the target never shows as it.
        let hours = |progress| goal_progress(&goal(GoalMetric::Hours, 100, progress));
        assert_eq!(hours(99.96), "99.9 / 100 hours");
        assert_eq!(hours(100.0), "100 / 100 hours");
        assert_eq!(hours(2.3), "2.3 / 100 hours");
        assert_eq!(hours(0.04), "0 / 100 hours");
        assert_eq!(
            goal_pace(&goal(GoalMetric::Hours, 100, 99.96), d(2027, 1, 5)).unwrap(),
            "Ended 1 hour short"
        );
        assert!(goal_pace(&goal(GoalMetric::Other("x".into()), 5, 1.0), d(2026, 5, 1)).is_none());
        assert_eq!(
            goal_description(&GoalMetric::Books, 12, d(2026, 1, 1), d(2026, 12, 31)),
            "12 books in 2026"
        );
        assert_eq!(
            goal_description(&GoalMetric::Pages, 1, d(2026, 1, 1), d(2026, 6, 30)),
            "1 page by Jun 30, 2026"
        );
    }

    #[test]
    fn progress_with_pages_and_total() {
        let p = progress_summary(&entry_with_read(Some(120), None, Some(400)));
        assert_eq!(p.text(), "Page 120 of 400");
        assert_eq!(p.percent, Some(30.0));
        assert_eq!(p.short_text().as_deref(), Some("30%"));
    }

    #[test]
    fn progress_prefers_edition_pages_and_server_percent() {
        let mut e = entry_with_read(Some(50), Some(25.0), Some(400));
        let mut edition = EditionSummary::new(EditionId::new(3));
        edition.pages = Some(200);
        e.edition = Some(edition);
        let p = progress_summary(&e);
        assert_eq!(p.text(), "Page 50 of 200");
        assert_eq!(p.percent, Some(25.0));
    }

    #[test]
    fn progress_fallbacks() {
        assert_eq!(
            progress_summary(&entry_with_read(Some(12), None, None)).text(),
            "Page 12"
        );
        assert_eq!(
            progress_summary(&entry_with_read(Some(12), None, None))
                .short_text()
                .as_deref(),
            Some("p. 12")
        );
        assert_eq!(
            progress_summary(&entry_with_read(None, Some(41.6), None)).text(),
            "42%"
        );
        let none = progress_summary(&entry_with_read(None, None, Some(300)));
        assert_eq!(none.text(), "Not started");
        assert_eq!(none.short_text(), None);
    }

    #[test]
    fn finished_read_has_no_progress() {
        let mut e = entry_with_read(Some(300), Some(100.0), Some(300));
        e.reads[0].finished_at = chrono::NaiveDate::from_ymd_opt(2024, 1, 1);
        let p = progress_summary(&e);
        assert_eq!(p.page, None);
        assert_eq!(p.percent, None);
    }

    #[test]
    fn audiobook_progress_counts_time() {
        let mut e = entry_with_read(None, None, Some(400));
        e.reads[0].progress_seconds = Some(2 * 3600 + 5 * 60);
        // A stale page count from a paper edition is not shown against time.
        e.reads[0].progress_pages = Some(120);
        let mut audio = EditionSummary::new(EditionId::new(3));
        audio.audio_seconds = Some(10 * 3600);
        e.edition = Some(audio);
        let p = progress_summary(&e);
        assert!(p.is_audio());
        assert_eq!(p.page, None);
        assert_eq!(p.text(), "2h 5m of 10h 0m");
        assert_eq!(p.percent.map(|p| p.round()), Some(21.0));
    }

    #[test]
    fn positions_and_durations() {
        assert_eq!(duration_text(45 * 60 + 30), "45m");
        assert_eq!(duration_text(36_300), "10h 5m");
        assert_eq!(position_text(7_500), "2:05");
        assert_eq!(position_text(7_530), "2:05:30");
        assert_eq!(parse_position(&position_text(7_530)), Some(7_530));
        assert_eq!(parse_position("2:05"), Some(7_500));
        assert_eq!(parse_position(" 0:45:30 "), Some(2_730));
        assert_eq!(parse_position("2:75"), None);
        assert_eq!(parse_position("125"), None);
        assert_eq!(parse_position(""), None);
    }

    #[test]
    fn edition_and_read_lines() {
        let mut e = EditionSummary::new(EditionId::new(1));
        assert_eq!(edition_text(&e), None);
        e.format = Some("Paperback".into());
        e.pages = Some(412);
        e.publisher = Some("Synthetic House".into());
        e.release_date = chrono::NaiveDate::from_ymd_opt(2004, 5, 6);
        assert_eq!(
            edition_text(&e).unwrap(),
            "Paperback · 412 pages · Synthetic House, 2004"
        );
        let d = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d);
        assert_eq!(
            read_dates_text(d(2019, 1, 3), d(2019, 2, 1)),
            "Jan 3, 2019 – Feb 1, 2019"
        );
        assert_eq!(read_dates_text(d(2019, 1, 3), None), "Started Jan 3, 2019");
        assert_eq!(read_dates_text(None, None), "No dates");
    }

    #[test]
    fn ratings_and_positions() {
        assert_eq!(rating_text(Rating::new(4.0).unwrap()), "4");
        assert_eq!(rating_text(Rating::new(3.5).unwrap()), "3.5");
        assert_eq!(series_position_text(3.0), "#3");
        assert_eq!(series_position_text(2.5), "#2.5");
        assert_eq!(average_rating_text(4.123), "4.12");
        assert_eq!(authors_text(&[]), "Unknown author");
        assert_eq!(authors_text(&["A".into(), "B".into()]), "A, B");
    }

    #[test]
    fn relative_times() {
        let now = Utc.with_ymd_and_hms(2026, 11, 19, 12, 0, 0).unwrap();
        let ago = |d: Duration| relative_time(&now, &(now - d));
        assert_eq!(ago(Duration::seconds(20)), "just now");
        assert_eq!(ago(Duration::minutes(5)), "5 min ago");
        assert_eq!(ago(Duration::minutes(61)), "1 hour ago");
        assert_eq!(ago(Duration::hours(5)), "5 hours ago");
        assert_eq!(ago(Duration::hours(30)), "yesterday");
        assert_eq!(ago(Duration::days(4)), "4 days ago");
        assert_eq!(ago(Duration::days(16)), "on Nov 3");
        assert_eq!(ago(Duration::days(400)), "on Oct 15, 2025");
        // Clock skew never yields a negative time.
        assert_eq!(
            relative_time(&now, &(now + Duration::minutes(3))),
            "just now"
        );
    }

    #[test]
    fn error_copy() {
        assert_eq!(
            error_message(&Error::RateLimited { retry_after: None }),
            "Hardcover's rate limit was reached. Try again in a minute."
        );
        assert_eq!(
            error_message(&Error::Timeout),
            "Couldn't reach Hardcover. Check your connection and try again."
        );
        assert_eq!(
            error_message(&Error::InvalidInput(
                "finish date is before the start date".into()
            )),
            "Finish date is before the start date."
        );
    }

    #[test]
    fn labels_are_sentence_case() {
        for s in SIDEBAR_SHELVES {
            let label = status_label(s);
            assert!(label.chars().skip(1).all(|c| !c.is_uppercase()), "{label}");
        }
    }

    #[test]
    fn search_hit_details() {
        use hardcover_core::{AuthorId, SeriesId};
        let mut author = AuthorHit::new(AuthorId::new(1), "Jane Quill");
        assert_eq!(author_hit_details(&author), "");
        author.top_books = vec!["A".into(), "B".into(), "C".into(), "D".into()];
        assert_eq!(author_hit_details(&author), "A, B, C");
        author.books_count = Some(1);
        assert_eq!(author_hit_details(&author), "1 book · A, B, C");

        let mut series = SeriesHit::new(SeriesId::new(2), "Sand");
        assert_eq!(series_hit_details(&series), "");
        series.books = vec!["Sand and Stars".into()];
        series.author = Some("Jane Quill".into());
        assert_eq!(
            series_hit_details(&series),
            "by Jane Quill · Sand and Stars"
        );
        series.books_count = Some(9);
        assert_eq!(series_hit_details(&series), "by Jane Quill · 9 books");
        series.primary_books_count = Some(6);
        assert_eq!(series_hit_details(&series), "by Jane Quill · 6 books");
    }

    #[test]
    fn counts_group_thousands() {
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(950), "950");
        assert_eq!(count_text(1000), "1,000");
        assert_eq!(count_text(12_480), "12,480");
        assert_eq!(count_text(1_234_567), "1,234,567");
        assert_eq!(books_count_text(1), "1 book");
        assert_eq!(books_count_text(2500), "2,500 books");
    }
}
