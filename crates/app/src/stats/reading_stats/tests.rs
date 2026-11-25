use super::*;
use hardcover_core::{
    BookId, BookSummary, EditionId, Rating, ReadId, SeriesId, SeriesRef, UserBookId, UserId,
};

fn date(s: &str) -> Date {
    Date::parse_from_str(s, "%Y-%m-%d").unwrap()
}

const TODAY: &str = "2026-11-19";

fn all(entries: &[LibraryEntry]) -> ReadingStats {
    compute(entries, date(TODAY), Period::All)
}

fn year(entries: &[LibraryEntry], y: i32) -> ReadingStats {
    compute(entries, date(TODAY), Period::Year(y))
}

fn entry(id: i64, status: ReadingStatus, pages: Option<u32>) -> LibraryEntry {
    let mut book = BookSummary::new(BookId::new(100 + id), format!("Book {id}"));
    book.pages = pages;
    LibraryEntry::new(UserBookId::new(id), UserId::new(1), book, status)
}

fn read(id: i64, started: Option<&str>, finished: Option<&str>) -> ReadSession {
    let mut r = ReadSession::new(ReadId::new(id));
    r.started_at = started.map(date);
    r.finished_at = finished.map(date);
    r
}

fn edition(pages: Option<u32>, seconds: Option<u32>) -> EditionSummary {
    let mut e = EditionSummary::new(EditionId::new(7));
    e.pages = pages;
    e.audio_seconds = seconds;
    e
}

fn format_edition(format: i64) -> EditionSummary {
    let mut e = edition(Some(300), None);
    e.reading_format_id = Some(format);
    e
}

/// A Read book with one read finished on `finished`.
fn finished(id: i64, finished: &str) -> LibraryEntry {
    let mut e = entry(id, ReadingStatus::Read, Some(300));
    e.reads = vec![read(id * 10, None, Some(finished))];
    e
}

fn years_of(stats: &ReadingStats) -> Vec<(i32, u32)> {
    stats
        .over_time
        .recent_years(usize::MAX)
        .0
        .iter()
        .map(|y| (y.year, y.books))
        .collect()
}

// ----- reads -----

#[test]
fn an_empty_library_has_no_stats() {
    let stats = all(&[]);
    assert!(stats.is_empty());
    assert!(stats.is_few());
    assert_eq!(years_of(&stats), []);
    assert_eq!(stats.reads, 0);
    assert_eq!(stats.ratings.average, None);
    assert!(stats.pace.is_none());
    assert!(
        stats
            .shelves
            .iter()
            .all(|(_, n, added)| *n == 0 && added.is_none())
    );
}

#[test]
fn rereads_count_in_the_year_they_finished() {
    let mut dune = entry(1, ReadingStatus::Read, Some(600));
    dune.reads = vec![
        read(10, Some("2026-01-01"), Some("2026-01-10")),
        read(11, Some("2024-03-01"), Some("2024-03-03")),
        read(12, Some("2026-05-01"), None),
    ];
    let stats = all(std::slice::from_ref(&dune));
    assert_eq!(stats.reads, 2);
    assert_eq!(stats.books.len(), 1, "one book, read twice");
    assert_eq!(
        years_of(&stats),
        vec![(2024, 1), (2025, 0), (2026, 1)],
        "gaps filled"
    );
    let this_year = year(&[dune], 2026);
    assert_eq!((this_year.reads, this_year.pages), (1, 600));
}

#[test]
fn an_open_read_is_the_abandoned_one_for_a_dnf_book() {
    let mut e = entry(1, ReadingStatus::DidNotFinish, Some(100));
    e.reads = vec![
        read(2, None, None),
        read(1, Some("2020-01-01"), Some("2020-01-20")),
    ];
    assert_eq!(all(&[e]).reads, 1);
}

#[test]
fn a_finish_dated_after_today_counts_nowhere() {
    let mut e = entry(1, ReadingStatus::Read, None);
    e.reads = vec![
        read(1, None, Some("2025-03-01")),
        read(2, None, Some("2062-03-01")),
        // Later this year: not yet.
        read(3, None, Some("2026-12-20")),
    ];
    let stats = all(&[e]);
    assert_eq!(years_of(&stats).last(), Some(&(2026, 0)));
    assert_eq!(years_of(&stats).first(), Some(&(2025, 1)));
    assert_eq!(stats.reads, 1);
    assert_eq!(stats.read_without_date, 0);

    // A Read book finished only in a later year counts nowhere, and it
    // has a finish date, so it isn't one without.
    let mut later = entry(2, ReadingStatus::Read, Some(300));
    later.reads = vec![read(3, Some("2062-02-01"), Some("2062-03-01"))];
    let stats = all(&[later]);
    assert_eq!(stats.reads, 0);
    assert_eq!(stats.read_without_date, 0);
    assert!(years_of(&stats).is_empty());
}

#[test]
fn a_finished_reread_on_another_shelf_still_counts() {
    let mut e = entry(1, ReadingStatus::CurrentlyReading, Some(100));
    e.reads = vec![
        read(2, Some("2026-10-01"), None),
        read(1, None, Some("2025-02-01")),
    ];
    let stats = all(&[e]);
    assert_eq!(stats.reads, 1);
    assert_eq!(stats.read_without_date, 0);
}

#[test]
fn pages_come_from_the_read_then_the_entry_then_the_book() {
    let mut a = entry(1, ReadingStatus::Read, Some(300));
    let mut from_read = read(1, None, Some("2026-02-01"));
    from_read.edition = Some(edition(Some(250), None));
    a.edition = Some(edition(Some(400), None));
    a.reads = vec![from_read, read(2, None, Some("2026-03-01"))];

    let mut b = entry(2, ReadingStatus::Read, Some(320));
    b.reads = vec![read(3, None, Some("2026-04-01"))];

    // An audiobook read adds no pages, even though the book has some; its
    // running time is time listened.
    let mut c = entry(3, ReadingStatus::Read, Some(500));
    let mut listened = read(4, None, Some("2026-05-01"));
    let mut audio = edition(None, Some(36_000));
    audio.reading_format_id = Some(2);
    listened.edition = Some(audio);
    c.reads = vec![listened];

    let stats = year(&[a, b, c], 2026);
    assert_eq!(stats.reads, 4);
    assert_eq!(stats.pages, 250 + 400 + 320);
    assert_eq!(stats.seconds_listened, 36_000);
    assert_eq!(stats.hours_listened(), 10);
}

#[test]
fn days_to_finish_needs_both_dates_in_order() {
    let mut e = entry(1, ReadingStatus::Read, None);
    e.reads = vec![
        // 10 days, counting the first and the last.
        read(1, Some("2026-01-01"), Some("2026-01-10")),
        // The same day: one day.
        read(2, Some("2026-02-01"), Some("2026-02-01")),
        // Counted for its year, not for the average.
        read(3, None, Some("2001-06-01")),
        read(4, Some("2026-03-05"), Some("2026-03-01")),
    ];
    let others = [finished(2, "2026-04-01"), finished(3, "2026-04-02")];
    let mut entries = vec![e];
    entries.extend(others);
    let stats = all(&entries);
    assert_eq!(stats.reads, 6);
    let pace = stats.pace.unwrap();
    assert_eq!(pace.to_finish, Some(5.5));
    assert_eq!(pace.quickest.as_ref().map(|q| q.value), Some(1));
    assert_eq!(pace.longest.as_ref().map(|q| q.value), Some(10));
}

#[test]
fn the_abandoned_read_of_a_did_not_finish_book_does_not_count() {
    let mut e = entry(1, ReadingStatus::DidNotFinish, Some(200));
    e.reads = vec![
        read(1, Some("2020-01-01"), Some("2020-01-20")),
        // Hardcover may date the day the reader gave up.
        read(2, Some("2026-04-01"), Some("2026-04-25")),
    ];
    assert_eq!(all(std::slice::from_ref(&e)).reads, 1);
    assert_eq!(year(&[e], 2026).reads, 0);
}

#[test]
fn read_books_without_a_finish_date_are_reported() {
    let mut undated = entry(2, ReadingStatus::Read, None);
    undated.reads = vec![read(2, Some("2026-01-01"), None)];
    let bare = entry(3, ReadingStatus::Read, None);
    let stats = all(&[finished(1, "2026-01-01"), undated, bare]);
    assert_eq!(stats.read_without_date, 2);
    assert_eq!(stats.reads, 1);
    assert_eq!(stats.shelf(ReadingStatus::Read), 3);
}

#[test]
fn the_chart_keeps_the_latest_years() {
    let mut e = entry(1, ReadingStatus::Read, None);
    e.reads = vec![
        read(1, None, Some("2001-06-01")),
        read(2, None, Some("2012-12-20")),
        read(3, None, Some("2019-02-10")),
        read(4, None, Some("2026-10-12")),
    ];
    let stats = all(&[e]);
    // Every year from the first finish is kept, 2001 to 2026.
    assert_eq!(years_of(&stats).len(), 26);
    // A chart with room for 12 shows 2015 to 2026, and counts the rest.
    let (years, earlier) = stats.over_time.recent_years(12);
    assert_eq!(years.len(), 12);
    assert_eq!(years.first().unwrap().year, 2015);
    assert_eq!(years.last().unwrap().year, 2026);
    assert_eq!((earlier.year, earlier.books), (2015, 2));
    let charted: u32 = years.iter().map(|y| y.books).sum();
    assert_eq!(charted + earlier.books, stats.reads);
    // A wider one reaches back to 2010; one wider than the data, all.
    let (years, earlier) = stats.over_time.recent_years(17);
    assert_eq!((years.first().unwrap().year, earlier.books), (2010, 1));
    let (years, earlier) = stats.over_time.recent_years(40);
    assert_eq!((years.len(), earlier.books), (26, 0));
}

#[test]
fn the_years_kept_stop_at_the_maximum() {
    let mut e = entry(1, ReadingStatus::Read, None);
    e.reads = vec![
        read(1, None, Some("1970-06-01")),
        read(2, None, Some("2026-10-12")),
    ];
    let stats = all(&[e]);
    let OverTime::Years { years, earlier } = &stats.over_time else {
        panic!("all time is per year");
    };
    assert_eq!(years.len(), usize::try_from(MAX_YEARS).unwrap());
    assert_eq!(years.first().unwrap().year, 1987);
    assert_eq!(earlier.books, 1);
}

#[test]
fn the_range_reaches_this_year_without_a_finish_in_it() {
    let stats = all(&[finished(1, "2024-06-01")]);
    let years: Vec<i32> = years_of(&stats).into_iter().map(|(y, _)| y).collect();
    assert_eq!(years, vec![2024, 2025, 2026]);
}

// ----- periods -----

#[test]
fn a_year_counts_only_its_own_finishes_by_month() {
    let entries = [
        finished(1, "2026-01-05"),
        finished(2, "2026-01-20"),
        finished(3, "2026-03-02"),
        finished(4, "2025-12-31"),
    ];
    let stats = year(&entries, 2026);
    assert_eq!(stats.reads, 3);
    assert_eq!(stats.pages, 900);
    let OverTime::Months { year, months } = &stats.over_time else {
        panic!("a year is per month");
    };
    assert_eq!(*year, 2026);
    let books: Vec<u32> = months.iter().map(|m| m.books).collect();
    assert_eq!(books, [2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(months[0].pages, 600);
    assert_eq!(stats.over_time.recent_years(12).0, &[]);
}

#[test]
fn the_years_to_choose_are_those_with_a_finish_newest_first() {
    let mut e = finished(1, "2019-06-01");
    e.reads.push(read(99, None, Some("2030-01-01")));
    let entries = [
        e,
        finished(2, "2026-01-05"),
        finished(3, "2026-02-05"),
        finished(4, "2021-02-05"),
    ];
    assert_eq!(finish_years(&entries, date(TODAY)), vec![2026, 2021, 2019]);
}

#[test]
fn a_period_with_few_books_says_little() {
    let two = [finished(1, "2026-01-05"), finished(2, "2026-02-05")];
    let stats = year(&two, 2026);
    assert!(stats.is_few());
    assert!(stats.pace.is_none());
    let three = [
        finished(1, "2026-01-05"),
        finished(2, "2026-02-05"),
        finished(3, "2026-03-05"),
    ];
    assert!(!year(&three, 2026).is_few());
}

#[test]
fn the_pace_runs_from_the_first_read_or_january_to_today() {
    let mut first = entry(1, ReadingStatus::Read, Some(100));
    first.reads = vec![read(1, Some("2026-11-10"), Some("2026-11-12"))];
    let entries = [first, finished(2, "2026-11-15"), finished(3, "2026-11-19")];
    // From the first start, Nov 10, to today, Nov 19: ten days.
    let pace = all(&entries).pace.unwrap();
    assert_eq!(pace.days, 10);
    assert_eq!(pace.reads, 3);
    assert_eq!(pace.pages, 700);
    assert!((pace.days_per_book() - 10.0 / 3.0).abs() < 1e-9);
    assert!((pace.pages_per_day() - 70.0).abs() < 1e-9);
    assert_eq!(pace.forecast, None, "all time has no forecast");

    // This year: Jan 1 to Nov 19 is day 323; three books by then is 3.39
    // a year, so three by Dec 31.
    let pace = year(&entries, 2026).pace.unwrap();
    assert_eq!(pace.days, 323);
    assert_eq!(pace.forecast, Some(3));

    // A past year runs to Dec 31 and forecasts nothing.
    let past = [
        finished(1, "2025-01-05"),
        finished(2, "2025-02-05"),
        finished(3, "2025-03-05"),
    ];
    let pace = year(&past, 2025).pace.unwrap();
    assert_eq!((pace.days, pace.forecast), (365, None));
}

#[test]
fn the_forecast_scales_this_years_books_to_december() {
    let entries: Vec<LibraryEntry> = (1..=28)
        .map(|i| finished(i, &format!("2026-{:02}-01", (i % 11) + 1)))
        .collect();
    // 28 books by Nov 19 (day 323 of 365): 31.6, so 32.
    assert_eq!(year(&entries, 2026).pace.unwrap().forecast, Some(32));
}

// ----- ratings -----

#[test]
fn ratings_average_and_spread_by_half_star() {
    let rated = |id, value| {
        let mut e = entry(id, ReadingStatus::Read, None);
        e.rating = Rating::new(value);
        e
    };
    let stats = all(&[
        rated(1, 5.0),
        rated(2, 4.5),
        rated(3, 4.5),
        rated(4, 0.5),
        entry(5, ReadingStatus::WantToRead, None),
    ]);
    assert_eq!(stats.ratings.count, 4);
    assert_eq!(stats.ratings.average, Some(3.625));
    assert_eq!(stats.ratings.counts, [1, 0, 0, 0, 0, 0, 0, 0, 2, 1]);
    assert_eq!(stats.shelf(ReadingStatus::WantToRead), 1);
    assert_eq!(stats.library_books, 5);
}

#[test]
fn a_year_counts_the_ratings_of_its_books() {
    let mut a = finished(1, "2026-03-01");
    a.rating = Rating::new(4.0);
    let mut b = finished(2, "2025-03-01");
    b.rating = Rating::new(2.0);
    let mut unread = entry(3, ReadingStatus::WantToRead, None);
    unread.rating = Rating::new(5.0);
    let stats = year(&[a, b, unread], 2026);
    assert_eq!(stats.ratings.count, 1);
    assert_eq!(stats.ratings.average, Some(4.0));
}

#[test]
fn books_rated_far_from_hardcover_are_compared() {
    let rated = |id, yours, theirs: f32, count| {
        let mut e = finished(id, "2026-03-01");
        e.rating = Rating::new(yours);
        e.book.average_rating = Some(theirs);
        e.book.ratings_count = Some(count);
        e
    };
    let stats = all(&[
        rated(1, 5.0, 3.6, 900),
        rated(2, 5.0, 4.4, 900), // 0.6 above
        rated(3, 4.5, 4.2, 900), // too close
        rated(4, 2.0, 4.2, 900), // 2.2 below
        rated(5, 5.0, 2.0, 5),   // too few ratings on Hardcover
        rated(6, 3.0, 3.5, 900), // 0.5 below
    ]);
    let ids = |c: &[Compared]| c.iter().map(|c| c.book.id.get()).collect::<Vec<_>>();
    assert_eq!(ids(&stats.higher), vec![101, 102]);
    assert_eq!(ids(&stats.lower), vec![104, 106]);
}

// ----- what was read -----

#[test]
fn each_book_counts_once_as_its_latest_read() {
    let mut twice = entry(1, ReadingStatus::Read, Some(300));
    let mut audio_read = read(2, None, Some("2026-05-01"));
    let mut audio = edition(None, Some(30_000));
    audio.reading_format_id = Some(2);
    audio_read.edition = Some(audio);
    let mut paper_read = read(1, None, Some("2020-05-01"));
    paper_read.edition = Some(format_edition(1));
    twice.reads = vec![audio_read, paper_read];

    let mut ebook = finished(2, "2026-01-01");
    ebook.edition = Some(format_edition(4));
    let unknown = finished(3, "2026-01-02");

    let stats = all(&[twice, ebook, unknown]);
    assert_eq!(stats.books.len(), 3);
    assert_eq!(
        stats.formats,
        Formats {
            physical: 0,
            ebook: 1,
            audiobook: 1,
            unknown: 1
        },
        "the reread counts as its latest read, an audiobook"
    );
    // The latest finish first.
    assert_eq!(stats.books.first().map(|b| b.get()), Some(101));
}

#[test]
fn lengths_fall_into_buckets() {
    let book = |id, pages| {
        let mut e = finished(id, "2026-01-01");
        e.book.pages = pages;
        e
    };
    let stats = all(&[
        book(1, Some(100)),
        book(2, Some(101)),
        book(3, Some(1000)),
        book(4, Some(1104)),
        book(5, None),
    ]);
    assert_eq!(stats.lengths.buckets, [1, 1, 0, 0, 0, 0, 1, 1]);
    assert_eq!(stats.lengths.without, 1);
    assert_eq!(stats.lengths.average, Some(576));
    let longest = stats.lengths.longest.unwrap();
    assert_eq!((longest.book.id.get(), longest.value), (104, 1104));
}

#[test]
fn authors_rank_by_books_then_rating() {
    let by = |id, author: &str, rating| {
        let mut e = finished(id, &format!("2026-01-{id:02}"));
        e.book.authors = vec![author.to_owned()];
        e.rating = Rating::new(rating);
        e
    };
    let stats = all(&[
        by(1, "Andy Weir", 5.0),
        by(2, "Frank Herbert", 3.0),
        by(3, "Frank Herbert", 4.0),
        by(4, "Kazuo Ishiguro", 4.5),
        by(5, "Kazuo Ishiguro", 3.0),
        by(6, "Andy Weir", 4.0),
    ]);
    assert_eq!(stats.authors, 3);
    let names: Vec<&str> = stats.top_authors.iter().map(|a| a.name.as_str()).collect();
    // Two books each, so by average: 4.5, 3.75, 3.5.
    assert_eq!(names, ["Andy Weir", "Kazuo Ishiguro", "Frank Herbert"]);
    // An author read once isn't listed.
    let stats = all(&[
        by(1, "Andy Weir", 5.0),
        by(2, "Frank Herbert", 3.0),
        by(3, "Frank Herbert", 4.0),
    ]);
    let names: Vec<&str> = stats.top_authors.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Frank Herbert"]);
    let herbert = &stats.top_authors[0];
    assert_eq!((herbert.books, herbert.average), (2, Some(3.5)));
    // The latest finish first.
    assert_eq!(herbert.books_shown[0].id.get(), 103);
}

#[test]
fn series_need_two_books_and_skip_compilations() {
    let in_series = |id, name: &str, position, compilation| {
        let mut e = finished(id, "2026-01-01");
        let mut s = SeriesRef::new(name);
        s.id = Some(SeriesId::new(7));
        s.position = Some(position);
        s.compilation = compilation;
        e.series = vec![s];
        e
    };
    let stats = all(&[
        in_series(1, "Dune", 2.0, false),
        in_series(2, "Dune", 1.0, false),
        in_series(3, "Dune", 1.0, true),
        in_series(4, "Alone", 1.0, false),
    ]);
    assert_eq!(stats.series.len(), 1);
    let dune = &stats.series[0];
    assert_eq!(
        (dune.name.as_str(), dune.books, dune.pages),
        ("Dune", 2, 600)
    );
    assert_eq!(dune.lead.id.get(), 102, "the lowest position leads");
}

#[test]
fn a_reread_counts_its_times_up_to_the_period() {
    let mut e = entry(1, ReadingStatus::Read, Some(300));
    e.reads = vec![
        read(1, None, Some("2019-01-01")),
        read(2, None, Some("2024-01-01")),
        read(3, None, Some("2026-01-01")),
    ];
    let once = finished(2, "2026-02-01");
    let stats = all(&[e.clone(), once.clone()]);
    assert_eq!(stats.rereads.len(), 1);
    assert_eq!(stats.rereads[0].times, 3);
    let in_2024 = year(&[e.clone(), once], 2024);
    assert_eq!(in_2024.rereads[0].times, 2);
    assert!(
        year(&[e], 2019).rereads.is_empty(),
        "the first read isn't a reread"
    );
}

#[test]
fn the_wait_runs_from_adding_to_the_first_finish_since() {
    let added = |id, on: &str, done: &str| {
        let mut e = finished(id, done);
        e.date_added = Some(date(on));
        e
    };
    let mut before = finished(4, "2026-03-01");
    // Added only once it had been read: no wait.
    before.date_added = Some(date("2026-06-01"));
    let stats = all(&[
        added(1, "2026-01-01", "2026-01-01"),
        added(2, "2026-01-01", "2026-01-31"),
        added(3, "2025-01-01", "2026-01-01"),
        before,
    ]);
    let wait = stats.wait.unwrap();
    assert_eq!(wait.measured, 3);
    assert!((wait.average_days - (0.0 + 30.0 + 365.0) / 3.0).abs() < 1e-9);
    assert_eq!((wait.quickest.book.id.get(), wait.quickest.value), (101, 0));
    assert_eq!((wait.longest.book.id.get(), wait.longest.value), (103, 365));
}

#[test]
fn decades_put_old_and_odd_years_aside() {
    let released = |id, year| {
        let mut e = finished(id, "2026-01-01");
        e.book.release_year = year;
        e
    };
    let stats = all(&[
        released(1, Some(1937)),
        released(2, Some(1388)),
        released(3, Some(1965)),
        released(4, Some(1999)),
        released(5, Some(2021)),
        released(6, None),
        released(7, Some(2031)),
    ]);
    let d = &stats.decades;
    assert_eq!(d.earlier, 1);
    assert_eq!(d.no_year, 3);
    assert_eq!(
        d.decades,
        vec![
            (1960, 1),
            (1970, 0),
            (1980, 0),
            (1990, 1),
            (2000, 0),
            (2010, 0),
            (2020, 1)
        ]
    );
    assert_eq!(d.oldest.as_ref().map(|o| o.value), Some(1937));
}

#[test]
fn a_year_adds_what_joined_each_shelf() {
    let mut want = entry(1, ReadingStatus::WantToRead, None);
    want.date_added = Some(date("2026-04-01"));
    let mut old = entry(2, ReadingStatus::WantToRead, None);
    old.date_added = Some(date("2024-04-01"));
    let stats = year(&[want, old], 2026);
    assert!(
        stats
            .shelves
            .contains(&(ReadingStatus::WantToRead, 2, Some(1)))
    );
}

#[test]
fn the_past_years_reads_ignore_the_period() {
    let books = [
        finished(1, "2001-03-01"),
        finished(2, "2025-11-20"),
        finished(3, "2025-11-19"),
        finished(4, "2026-02-01"),
    ];
    // 365 days back from Nov 19, 2026 reaches Nov 20, 2025, not the 19th.
    assert_eq!(all(&books).past_year_reads, 2);
    assert_eq!(year(&books, 2001).past_year_reads, 2);
}
