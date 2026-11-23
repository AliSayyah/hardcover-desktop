use chrono::{DateTime, Datelike, NaiveDate};

use super::*;
use crate::api::DOCUMENTS;
use crate::{
    BookId, EditionId, GoalMetric, ListDraft, ListEdit, NewGoal, Privacy, ProgressUpdate, Rating,
    ReadingStatus, UserBookId,
};

const PROJECT_HAIL_MARY: i64 = 427_578;
const ATOMIC_HABITS: i64 = 503;
const CRUEL_PRINCE: i64 = 98_065;

fn at(timestamp: &str) -> Clock {
    Clock::Fixed(DateTime::parse_from_rfc3339(timestamp).unwrap())
}

/// A client whose today is 2026-11-21, in Tehran.
fn pinned() -> Client {
    client_at(&Faults::default(), at("2026-11-21T10:00:00+03:30"))
}

fn day(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn library(client: &Client) -> Vec<crate::LibraryEntry> {
    client.library(client.me().unwrap().id).unwrap().entries
}

fn entry(client: &Client, id: i64) -> crate::LibraryEntry {
    client.library_entry(UserBookId::new(id)).unwrap().unwrap()
}

fn answer_fresh(op: &str, vars: &Value) -> Value {
    answer(&mut State::new(&CATALOGUE, Clock::System), op, vars)
}

#[test]
fn signs_in_as_the_demo_reader() {
    let me = client().me().unwrap();
    assert_eq!(me.username, "demo_reader");
    assert!(me.avatar.is_none());
}

#[test]
fn the_library_fills_every_shelf() {
    let client = client();
    let library = library(&client);
    assert_eq!(library.len(), CATALOGUE.library.len());
    for status in ReadingStatus::ALL {
        assert!(
            library.iter().any(|e| e.status == status),
            "nothing on {status:?}"
        );
    }
    let reading = library
        .iter()
        .find(|e| e.status == ReadingStatus::CurrentlyReading)
        .unwrap();
    assert!(reading.open_read().unwrap().progress_pages.is_some());
    assert!(reading.book.cover.is_some());
    assert!(
        library.iter().any(|e| e.reads.len() > 1),
        "no entry shows a read history"
    );
    assert!(library.iter().any(|e| e.private_notes.is_some()));
}

#[test]
fn every_date_is_counted_from_today() {
    let client = pinned();
    let library = library(&client);
    let today = day("2026-11-21");
    let reading: Vec<_> = library
        .iter()
        .filter(|e| e.status == ReadingStatus::CurrentlyReading)
        .collect();
    // Updated within the week, except one left idle on purpose.
    let idle: Vec<_> = reading
        .iter()
        .filter(|e| (today - e.updated_at.unwrap().date_naive()).num_days() > 7)
        .collect();
    assert_eq!(idle.len(), 1, "one idle book");
    assert_eq!(idle[0].user_book_id, UserBookId::new(ATOMIC_HABITS));
    assert!(reading.len() >= 3);

    // Educated was finished today, The Martian nine days ago.
    let educated = entry(&client, 527);
    assert_eq!(educated.reads[0].finished_at, Some(today));
    let martian = entry(&client, 523);
    assert_eq!(martian.reads[0].finished_at, Some(day("2026-11-12")));

    // The same catalogue on another day moves with it.
    let later = client_at(&Faults::default(), at("2031-03-02T08:00:00+00:00"));
    assert_eq!(
        entry(&later, 527).reads[0].finished_at,
        Some(day("2031-03-02"))
    );
}

#[test]
fn a_series_has_gone_untouched_for_over_a_year() {
    let client = pinned();
    let library = library(&client);
    let today = day("2026-11-21");
    let in_middle_earth: Vec<_> = library
        .iter()
        .filter(|e| e.series.iter().any(|s| s.name == "Middle Earth"))
        .collect();
    assert_eq!(in_middle_earth.len(), 2, "The Hobbit and Fellowship");
    let last = in_middle_earth
        .iter()
        .flat_map(|e| {
            e.reads
                .iter()
                .flat_map(|r| [r.started_at, r.finished_at])
                .chain([e.updated_at.map(|t| t.date_naive())])
        })
        .flatten()
        .max()
        .unwrap();
    assert!((today - last).num_days() > 365, "{last}");
}

#[test]
fn the_goal_is_this_years_dated_like_hardcovers() {
    let client = pinned();
    let goals = client.goals().unwrap();
    assert_eq!(goals.len(), 1);
    let goal = &goals[0];
    assert_eq!(goal.metric, GoalMetric::Books);
    assert_eq!(goal.target, 12);
    assert_eq!(goal.start, day("2026-01-01"));
    assert_eq!(goal.end, day("2027-01-01"));
    assert_eq!(goal.description.as_deref(), Some("2026 Reading Goal"));
    let finished_this_year = CATALOGUE
        .library
        .iter()
        .flat_map(|e| &e.reads)
        .filter_map(|r| r.finished)
        .filter(|f| (-324..=0).contains(f))
        .count();
    assert!(finished_this_year > 0);
    assert!((goal.progress - finished_this_year as f64).abs() < f64::EPSILON);

    // On New Year's Day the goal is the new year's, with the book finished
    // that day already counted.
    let jan_1 = client_at(&Faults::default(), at("2027-01-01T09:00:00+03:30"));
    let goal = &jan_1.goals().unwrap()[0];
    assert_eq!(goal.start, day("2027-01-01"));
    assert_eq!(goal.end, day("2028-01-01"));
    assert!(goal.progress >= 1.0);
}

#[test]
fn books_have_their_editions_or_a_stand_in() {
    let client = client();
    for book in &CATALOGUE.books {
        let editions = client.book_editions(BookId::new(book.id), 0).unwrap().items;
        if book.editions.is_empty() {
            assert_eq!(editions.len(), 1, "{}", book.title);
            assert_eq!(editions[0].id, EditionId::new(book.id));
            assert_eq!(editions[0].cover.as_ref().unwrap().url, book.cover.url);
        } else {
            assert_eq!(editions.len(), book.editions.len(), "{}", book.title);
        }
    }
    // A book with a page edition and an audiobook edition.
    let phm = client
        .book_editions(BookId::new(PROJECT_HAIL_MARY), 0)
        .unwrap()
        .items;
    assert!(phm.iter().any(|e| e.pages.is_some()));
    assert!(
        phm.iter()
            .any(|e| e.audio_seconds.is_some() && e.reading_format_id == Some(2))
    );
}

#[test]
fn audiobooks_count_time() {
    let client = client();
    let habits = entry(&client, ATOMIC_HABITS);
    assert!(habits.is_audio());
    assert_eq!(habits.length(), Some(crate::api::Length::Seconds(20_117)));
    assert!(habits.open_read().unwrap().progress_seconds.is_some());
    // A book with no page count, its audiobook's length known, read on paper.
    let cruel = library(&client)
        .into_iter()
        .find(|e| e.book.id == BookId::new(CRUEL_PRINCE))
        .unwrap();
    assert_eq!(cruel.book.pages, None);
    assert!(cruel.book_audio_edition.is_some());
    assert!(!cruel.is_audio());
    assert_eq!(cruel.length(), Some(crate::api::Length::Pages(370)));
    // One of its audiobook editions lists no length.
    let editions = client
        .book_editions(BookId::new(CRUEL_PRINCE), 0)
        .unwrap()
        .items;
    assert!(
        editions
            .iter()
            .any(|e| e.reading_format_id == Some(2) && e.audio_seconds.is_none())
    );
}

#[test]
fn the_library_covers_the_awkward_cases() {
    let client = client();
    let library = library(&client);
    let by_book = |id: i64| {
        library
            .iter()
            .find(|e| e.book.id == BookId::new(id))
            .unwrap()
    };
    // A fractional position.
    assert_eq!(by_book(427_544).series[0].position, Some(2.5));
    // A book in two series.
    assert_eq!(by_book(139_773).series.len(), 2);
    // Read, with no date.
    let gatsby = by_book(377_193);
    assert_eq!(gatsby.status, ReadingStatus::Read);
    assert!(gatsby.reads.is_empty());
    // Finished lately, not rated.
    let today = chrono::Local::now().date_naive();
    let to_rate = library
        .iter()
        .filter(|e| e.status == ReadingStatus::Read && e.rating.is_none())
        .filter(|e| {
            e.reads
                .iter()
                .filter_map(|r| r.finished_at)
                .any(|f| (today - f).num_days() <= 60)
        })
        .count();
    assert_eq!(to_rate, 2);
    // Paused with an open read.
    let paused = library
        .iter()
        .find(|e| e.status == ReadingStatus::Paused)
        .unwrap();
    assert!(paused.open_read().is_some());
}

#[test]
fn a_compilation_is_on_its_series_page_only() {
    let client = client();
    let id = crate::SeriesId::new(CATALOGUE.series_id("Middle Earth"));
    // The series page sends it, marked (the client leaves it out).
    let body = answer_fresh("SeriesPage", &json!({ "id": id.get() }));
    let rows = body["data"]["series_by_pk"]["book_series"]
        .as_array()
        .unwrap();
    let compilations: Vec<&Value> = rows.iter().filter(|r| r["compilation"] == true).collect();
    assert_eq!(compilations.len(), 1);
    assert_eq!(compilations[0]["book"]["title"], "The Lord of the Rings");
    let page = client.series(id).unwrap();
    assert!(
        page.books
            .iter()
            .all(|b| b.book.title != "The Lord of the Rings")
    );
    let overview = client.tracked_series(&[id]).series;
    let titles: Vec<&str> = overview[0]
        .books
        .iter()
        .map(|b| b.book.title.as_str())
        .collect();
    assert_eq!(titles, ["The Hobbit", "The Fellowship of the Ring"]);
}

#[test]
fn only_catalogue_covers_and_default_avatars_are_downloaded() {
    let transport = CoverTransport {
        http: None,
        faults: Faults::default(),
    };
    let response = transport
        .send(HttpRequest::new(Method::Get, "https://example.com/x.jpg"))
        .unwrap();
    assert_eq!(response.status, 404);
    let downloads = &*DOWNLOADS;
    for url in downloads.iter() {
        assert!(url.starts_with("https://assets.hardcover.app/"), "{url}");
    }
    assert!(downloads.contains("https://assets.hardcover.app/static/avatars/profile1.png"));
    // Every avatar is one of Hardcover's defaults, never a user's picture.
    for url in reviews::avatars() {
        assert!(url.contains("/static/avatars/"), "{url}");
    }
}

#[test]
fn library_pages_honour_the_cursor_and_limit() {
    let mut state = State::new(&CATALOGUE, Clock::System);
    let mut page = |after: i64| {
        let body = answer(
            &mut state,
            "LibraryPage",
            &json!({ "userId": 1, "afterId": after, "limit": 5 }),
        );
        body["data"]["user_books"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_i64().unwrap())
            .collect::<Vec<_>>()
    };
    let mut seen: Vec<i64> = Vec::new();
    let mut after = 0;
    loop {
        let ids = page(after);
        assert!(ids.len() <= 5);
        seen.extend(&ids);
        match ids.last() {
            Some(last) => after = *last,
            None => break,
        }
    }
    assert_eq!(seen.len(), CATALOGUE.library.len());
    assert!(seen.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn every_book_has_details_and_a_hardcover_cover() {
    let client = client();
    for book in &CATALOGUE.books {
        let details = client.book_details(BookId::new(book.id)).unwrap();
        assert_eq!(details.summary.title, book.title);
        assert!(details.description.is_some());
        let cover = details.summary.cover.expect("a cover");
        assert!(
            cover.url.starts_with("https://assets.hardcover.app/"),
            "{}",
            cover.url
        );
    }
}

#[test]
fn entries_can_be_read_back() {
    let client = client();
    let me = client.me().unwrap();
    let first = &CATALOGUE.library[0];
    let entry = client.library_entry(UserBookId::new(first.id)).unwrap();
    assert_eq!(entry.unwrap().user_book_id, UserBookId::new(first.id));
    let by_book = client
        .library_entry_for_book(me.id, BookId::new(first.book))
        .unwrap();
    assert!(by_book.is_some());
    assert!(
        client
            .library_entry_for_book(me.id, BookId::new(427_314))
            .unwrap()
            .is_none(),
        "Children of Dune is only in search"
    );
}

#[test]
fn search_matches_titles_and_authors() {
    let client = client();
    let herbert = client.search_books("frank herbert", 1).unwrap();
    assert_eq!(herbert.books.len(), 4);
    let dune = client.search_books("Dune", 1).unwrap();
    assert_eq!(dune.books.len(), 4);
    assert_eq!(dune.books[0].title, "Dune", "the most read first");
    assert_eq!(client.search_books("zzz", 1).unwrap().books, []);
}

#[test]
fn the_reader_has_reviewed_a_few_books() {
    let client = pinned();
    let dune = client.review(UserBookId::new(521)).unwrap();
    assert!(dune.text().unwrap().contains("Dune Messiah"));
    assert_eq!(dune.reviewed_at, Some(day("2026-10-13")));
    assert!(!dune.has_spoilers);
    let reviewed = CATALOGUE.library.iter().filter(|e| e.review.is_some());
    assert_eq!(reviewed.count(), 2);
    assert!(!client.review(UserBookId::new(501)).unwrap().exists());
}

// ----- writes -----

#[test]
fn a_status_change_starts_and_finishes_reads() {
    let client = pinned();
    let today = day("2026-11-21");
    // Want to Read → Currently Reading starts a read today.
    let messiah = client
        .set_status(UserBookId::new(511), ReadingStatus::CurrentlyReading)
        .unwrap();
    assert_eq!(messiah.status, ReadingStatus::CurrentlyReading);
    let read = messiah.open_read().unwrap();
    assert_eq!(read.started_at, Some(today));
    // Resending the status changes nothing.
    let again = client
        .set_status(UserBookId::new(511), ReadingStatus::CurrentlyReading)
        .unwrap();
    assert_eq!(again.reads.len(), 1);
    // Read finishes it today, at the end.
    let done = client
        .set_status(UserBookId::new(511), ReadingStatus::Read)
        .unwrap();
    assert_eq!(done.reads.len(), 1);
    assert_eq!(done.reads[0].finished_at, Some(today));
    assert_eq!(done.reads[0].progress_pages, Some(304));
    // The next sync sees it.
    let synced = library(&client)
        .into_iter()
        .find(|e| e.user_book_id == UserBookId::new(511))
        .unwrap();
    assert_eq!(synced, done);
    // A new client starts over.
    assert_eq!(entry(&pinned(), 511).status, ReadingStatus::WantToRead);
}

/// The client sends this computer's date with the status (`user_date`), and
/// the read takes it, not the server's own today.
#[test]
fn a_status_change_dates_its_read_with_the_day_sent() {
    let client = Client::with_transport(
        Arc::new(DemoToken),
        Arc::new(ApiTransport {
            state: Mutex::new(State::new(&CATALOGUE, at("2026-11-21T10:00:00+03:30"))),
            faults: Faults::default(),
        }),
        TokenBucket::new(1_000, 1_000.0),
    );
    // Late on the 20th here, already the 21st where the server counts.
    client.pin_today(day("2026-11-20"));
    let started = client
        .set_status(UserBookId::new(511), ReadingStatus::CurrentlyReading)
        .unwrap();
    assert_eq!(
        started.open_read().unwrap().started_at,
        Some(day("2026-11-20"))
    );
    // Children of Dune, not in the library.
    let added = client
        .add_to_library(BookId::new(427_314), ReadingStatus::Read)
        .unwrap();
    assert_eq!(added.reads[0].finished_at, Some(day("2026-11-20")));
}

#[test]
fn a_rating_alone_means_read_and_a_kept_status_stays() {
    let client = client();
    let rating = Rating::new(4.0);
    let kept = client
        .set_rating(UserBookId::new(512), rating, ReadingStatus::WantToRead)
        .unwrap();
    assert_eq!(kept.status, ReadingStatus::WantToRead);
    assert_eq!(kept.rating, rating);
    let mut state = State::new(&CATALOGUE, Clock::System);
    let body = answer(
        &mut state,
        "UpdateUserBook",
        &json!({ "id": 512, "object": { "rating": 3.5 } }),
    );
    assert_eq!(
        body["data"]["update_user_book"]["user_book"]["status_id"],
        3
    );
}

#[test]
fn progress_to_the_end_finishes_the_read() {
    let client = pinned();
    let id = UserBookId::new(502);
    let read = entry(&client, 502).open_read().unwrap().id;
    let halfway = client
        .update_progress(id, Some(read), ProgressUpdate::Pages(500))
        .unwrap()
        .entry;
    assert_eq!(halfway.open_read().unwrap().progress_pages, Some(500));
    let percent = halfway.open_read().unwrap().progress_percent.unwrap();
    assert!((percent - 49.65).abs() < 0.1, "{percent}");
    let done = client
        .update_progress(id, Some(read), ProgressUpdate::Pages(1007))
        .unwrap()
        .entry;
    assert_eq!(done.status, ReadingStatus::Read);
    assert!(done.open_read().is_none());
    // An audiobook by percentage, in seconds.
    let habits = client
        .update_progress(
            UserBookId::new(ATOMIC_HABITS),
            None,
            ProgressUpdate::Percent(50.0),
        )
        .unwrap()
        .entry;
    let read = habits.open_read().unwrap();
    assert_eq!(read.progress_pages, None);
    let half = crate::api::Length::Seconds(20_117).position_at(50.0);
    assert_eq!(
        ProgressUpdate::Seconds(read.progress_seconds.unwrap()),
        half
    );
}

#[test]
fn reads_can_be_added_dated_and_deleted() {
    let client = client();
    let id = UserBookId::new(525);
    // Children of Dune, not in the library.
    let added = client
        .add_read(id, Some(day("2015-03-01")), day("2015-03-20"), None)
        .unwrap();
    assert_eq!(added.reads.len(), 2);
    let new = added
        .reads
        .iter()
        .find(|r| r.finished_at == Some(day("2015-03-20")))
        .unwrap()
        .id;
    let dated = client
        .set_read_dates(new, Some(day("2015-03-02")), Some(day("2015-03-21")))
        .unwrap();
    assert!(
        dated
            .reads
            .iter()
            .any(|r| r.started_at == Some(day("2015-03-02")))
    );
    let deleted = client.delete_read(id, new).unwrap();
    assert_eq!(deleted.reads.len(), 1);
    assert_eq!(entry(&client, 525).reads.len(), 1);
}

#[test]
fn an_edition_change_moves_the_read_to_the_audiobook() {
    let client = client();
    let change = client
        .set_edition(UserBookId::new(501), EditionId::new(31_878_554))
        .unwrap();
    assert!(change.read_left_behind.is_none());
    let entry = change.entry;
    assert!(entry.is_audio());
    let read = entry.open_read().unwrap();
    assert_eq!(read.progress_pages, None);
    assert!(read.progress_seconds.is_some_and(|s| s > 30_000));
}

#[test]
fn reviews_round_trip_and_an_empty_slate_deletes_one() {
    let client = pinned();
    let id = UserBookId::new(523);
    let written = client
        .set_review(
            id,
            "Funny and tense.",
            true,
            Some(day("2026-11-20")),
            ReadingStatus::Read,
        )
        .unwrap();
    assert_eq!(written.review.text(), Some("Funny and tense."));
    assert_eq!(written.entry.unwrap().status, ReadingStatus::Read);
    let read_back = client.review(id).unwrap();
    assert!(read_back.has_spoilers && read_back.exists());
    assert_eq!(read_back.reviewed_at, Some(day("2026-11-20")));

    let deleted = client.delete_review(id, ReadingStatus::Read).unwrap();
    assert!(!deleted.review.exists());
    let read_back = client.review(id).unwrap();
    assert!(!read_back.exists());
}

#[test]
fn notes_privacy_and_shelving_are_written() {
    let client = client();
    let noted = client
        .set_private_notes(UserBookId::new(502), "  Page 400 is the best.  ")
        .unwrap();
    assert_eq!(
        noted.private_notes.as_deref(),
        Some("Page 400 is the best.")
    );
    let private = client
        .set_privacy(UserBookId::new(502), Privacy::Private)
        .unwrap();
    assert_eq!(private.privacy, Some(Privacy::Private));

    // Children of Dune, not in the library.
    let added = client
        .add_to_library(BookId::new(427_314), ReadingStatus::WantToRead)
        .unwrap();
    assert_eq!(library(&client).len(), CATALOGUE.library.len() + 1);
    // Already shelved: refused.
    assert!(
        client
            .add_to_library(BookId::new(427_314), ReadingStatus::Read)
            .is_err()
    );
    client.remove_from_library(added.user_book_id).unwrap();
    assert_eq!(library(&client).len(), CATALOGUE.library.len());
    // A book still to come can be added from Coming soon.
    let upcoming = CATALOGUE.upcoming[0].id;
    let wanted = client
        .add_to_library(BookId::new(upcoming), ReadingStatus::WantToRead)
        .unwrap();
    assert!(!wanted.series.is_empty());
}

#[test]
fn lists_are_written() {
    let client = client();
    let draft = ListDraft {
        name: "Beach reads".into(),
        description: "Light ones.".into(),
        privacy: Some(Privacy::FollowersOnly),
        ranked: true,
    };
    let list = client.create_list(&draft).unwrap();
    assert_eq!(list.slug.as_deref(), Some("beach-reads"));
    let first = client
        .add_to_list(list.id, BookId::new(427_578), None, None)
        .unwrap();
    let second = client
        .add_to_list(list.id, BookId::new(312_460), None, None)
        .unwrap();
    // Adding it again returns its row.
    assert_eq!(
        client
            .add_to_list(list.id, BookId::new(427_578), None, None)
            .unwrap(),
        first
    );
    let books = client.list_books(list.id, 0).unwrap().items;
    assert_eq!(
        books.iter().map(|b| b.position).collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
    client.remove_from_list(first.id).unwrap();
    assert_eq!(client.list_books(list.id, 0).unwrap().items.len(), 1);
    assert!(client.list_memberships().unwrap().rows.contains(&second));
    // Undo puts it back at its place, not after the last.
    let back = client
        .add_to_list(list.id, BookId::new(427_578), None, Some(1))
        .unwrap();
    let books = client.list_books(list.id, 0).unwrap().items;
    let place = books.iter().find(|b| b.id == back.id).map(|b| b.position);
    assert_eq!(place, Some(Some(1)));
    client.remove_from_list(back.id).unwrap();

    let lists = client.lists().unwrap();
    let list = lists.iter().find(|l| l.id == list.id).unwrap();
    let renamed = client
        .update_list(
            list.id,
            &ListEdit::between(
                list,
                &ListDraft {
                    name: "Summer".into(),
                    description: String::new(),
                    privacy: None,
                    ranked: true,
                },
            ),
        )
        .unwrap();
    assert_eq!(renamed.name, "Summer");
    assert_eq!(renamed.description, None, "an empty description clears it");
    assert_eq!(renamed.privacy_id, Some(2), "kept as read");
    client.delete_list(list.id).unwrap();
    assert!(client.lists().unwrap().iter().all(|l| l.id != list.id));
}

#[test]
fn an_edition_goes_on_the_owned_list_once() {
    let client = client();
    let owned = client
        .lists()
        .unwrap()
        .into_iter()
        .find(|l| l.is_owned())
        .unwrap();
    let row = client
        .mark_owned(EditionId::new(31_878_554))
        .unwrap()
        .unwrap();
    assert_eq!(row.list_id, owned.id);
    assert_eq!(row.book_id, BookId::new(PROJECT_HAIL_MARY));
    assert_eq!(
        client
            .mark_owned(EditionId::new(31_878_554))
            .unwrap()
            .unwrap(),
        row
    );
    let after = client
        .lists()
        .unwrap()
        .into_iter()
        .find(|l| l.is_owned())
        .unwrap();
    assert_eq!(after.books_count, owned.books_count + 1);
}

#[test]
fn goals_are_written() {
    let client = pinned();
    let goal = client
        .create_goal(&NewGoal {
            metric: GoalMetric::Pages,
            target: 5_000,
            start: day("2026-11-01"),
            end: day("2026-11-30"),
            description: "November pages".into(),
        })
        .unwrap();
    // The pages of the books finished this month: The Martian and Educated.
    assert!(
        (goal.progress - (385.0 + 352.0)).abs() < f64::EPSILON,
        "{}",
        goal.progress
    );
    let edited = client.update_goal(goal.id, |g| g.target = 1_000).unwrap();
    assert_eq!(edited.target, 1_000);
    assert_eq!(
        edited.description.as_deref(),
        Some("November pages"),
        "the rest kept as read"
    );
    assert_eq!(client.goals().unwrap().len(), 2);
    client.delete_goal(goal.id).unwrap();
    assert_eq!(client.goals().unwrap().len(), 1);
    assert!(client.delete_goal(goal.id).is_err());
}

#[test]
fn every_write_is_answered() {
    for (name, doc) in DOCUMENTS {
        let op = operation(doc);
        if doc.trim_start().starts_with("mutation") {
            assert!(
                writes::is_write(op),
                "{name} ({op}) is not handled as a write"
            );
        }
    }
}

// ----- faults -----

#[test]
fn faults_parse() {
    let faults =
        Faults::parse("latency=800ms, 429, error-rate=0.2, scope=read:goals, writes").unwrap();
    assert_eq!(faults.latency(), std::time::Duration::from_millis(800));
    assert_eq!(
        faults.to_string(),
        "429,error-rate=0.2,writes,latency=800ms,scope=read:goals"
    );
    assert_eq!(
        Faults::parse("latency=1.5s").unwrap().latency().as_millis(),
        1_500
    );
    assert!(Faults::parse("").unwrap().is_empty());
    // A rate alone means server errors.
    assert_eq!(
        Faults::parse("error-rate=0.5").unwrap().to_string(),
        "500,error-rate=0.5"
    );
    for bad in [
        "404",
        "latency=fast",
        "error-rate=2",
        "scope=read:everything",
        "offline=yes",
    ] {
        assert!(Faults::parse(bad).is_err(), "{bad}");
    }
}

fn faulty(spec: &str) -> Client {
    client_with(&Faults::parse(spec).unwrap())
}

#[test]
fn faults_fail_requests_as_hardcover_would() {
    assert!(matches!(faulty("offline").me(), Err(Error::Network(_))));
    assert!(matches!(faulty("timeout").me(), Err(Error::Timeout)));
    assert!(matches!(faulty("401").me(), Err(Error::Unauthorized)));
    assert!(matches!(
        faulty("429").me(),
        Err(Error::RateLimited { retry_after: Some(d) }) if d.as_secs() == 60
    ));
    assert!(matches!(faulty("500").me(), Err(Error::Network(m)) if m.contains("500")));
    // A rate of nothing fails nothing.
    assert!(faulty("500,error-rate=0").me().is_ok());
}

#[test]
fn write_faults_leave_reads_working() {
    let client = faulty("500,writes");
    assert!(client.me().is_ok());
    assert!(
        client
            .set_status(UserBookId::new(511), ReadingStatus::Read)
            .is_err()
    );
    assert_eq!(entry(&client, 511).status, ReadingStatus::WantToRead);
}

#[test]
fn a_missing_scope_is_refused_and_left_out_of_the_grant() {
    use crate::auth::scopes::{READ_GOALS, READ_LIBRARY, WRITE_GOALS};
    let faults = Faults::parse("scope=read:goals").unwrap();
    let client = client_with(&faults);
    assert!(matches!(
        client.goals(),
        Err(Error::InsufficientScope { scope: Some(s) }) if s == READ_GOALS
    ));
    assert!(client.me().is_ok());
    let granted = faults.granted_scopes();
    assert!(!granted.contains(READ_GOALS));
    assert!(granted.contains(WRITE_GOALS) && granted.contains(READ_LIBRARY));
    // Refused by the server only: still in the grant.
    let refused = Faults::parse("refuse=read:goals").unwrap();
    assert!(refused.granted_scopes().contains(READ_GOALS));
    assert!(matches!(
        client_with(&refused).goals(),
        Err(Error::InsufficientScope { .. })
    ));
    assert!(Faults::default().granted_scopes().contains(READ_GOALS));
}

#[test]
fn latency_delays_every_request() {
    let client = faulty("latency=50ms");
    let start = std::time::Instant::now();
    client.me().unwrap();
    assert!(start.elapsed() >= std::time::Duration::from_millis(50));
}

#[test]
fn offline_fails_covers_too() {
    let transport = CoverTransport {
        http: None,
        faults: Faults::parse("offline").unwrap(),
    };
    let url = &CATALOGUE.books[0].cover.url;
    assert!(matches!(
        transport.send(HttpRequest::new(Method::Get, url)),
        Err(Error::Network(_))
    ));
    let writes_only = CoverTransport {
        http: None,
        faults: Faults::parse("offline,writes").unwrap(),
    };
    assert!(writes_only.send(HttpRequest::new(Method::Get, url)).is_ok());
}

/// The scopes each document needs, by the committed scope map: those whose
/// loss alone leaves the document ungranted.
#[test]
fn required_scopes_follow_the_scope_map() {
    use crate::api::sdl;
    use crate::auth::DEFAULT_SCOPES;
    let caps: Value = serde_json::from_str(include_str!(
        "../../../../schema/hardcover-capabilities.json"
    ))
    .unwrap();
    let schema = sdl::parse(include_str!("../../../../schema/hardcover.graphql")).unwrap();
    for (name, doc) in DOCUMENTS {
        let usage = sdl::usage(&schema, name, doc).unwrap();
        let docs = [(*name, usage)];
        let needed: Vec<&str> = DEFAULT_SCOPES
            .into_iter()
            .filter(|scope| {
                let rest: Vec<&str> = DEFAULT_SCOPES.into_iter().filter(|s| s != scope).collect();
                !sdl::check_scopes(&caps, &rest, &docs)
                    .unwrap()
                    .problems
                    .is_empty()
            })
            .collect();
        let op = operation(doc);
        assert_eq!(
            faults::required_scopes(op),
            needed.as_slice(),
            "{name} ({op})"
        );
    }
}

fn operation(doc: &str) -> &str {
    doc.split_whitespace()
        .nth(1)
        .and_then(|s| s.split(['(', '{']).next())
        .unwrap()
}

// ----- the rest of the catalogue -----

#[test]
fn series_and_author_pages_link_up() {
    let client = client();
    let dune = client.book_details(BookId::new(312_460)).unwrap();
    let series_id = dune.series[0].id.expect("a series id");
    let series = client.series(series_id).unwrap();
    assert_eq!(series.name, "Dune");
    let titles: Vec<&str> = series.books.iter().map(|b| b.book.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "Dune",
            "Dune Messiah",
            "Children of Dune",
            "God Emperor of Dune"
        ]
    );
    let author = series.author.expect("an author");
    assert_eq!(author.name, "Frank Herbert");
    assert_eq!(dune.contributors[0].author_id, Some(author.id));

    let page = client.author(author.id).unwrap();
    assert_eq!(page.books.len(), 4);
    assert_eq!(page.books[0].book.title, "Dune", "the most read first");
    assert!(page.bio.is_none() && page.photo.is_none());
    assert!(client.author(crate::AuthorId::new(1)).is_err());
}

#[test]
fn library_entries_carry_their_series() {
    let client = client();
    let library = library(&client);
    let messiah = library
        .iter()
        .find(|e| e.book.title == "Dune Messiah")
        .expect("Dune Messiah is shelved");
    let details = client.book_details(messiah.book.id).unwrap();
    assert_eq!(messiah.series, details.series, "the same as the book page");
    assert_eq!(messiah.series[0].name, "Dune");
    assert_eq!(messiah.series[0].position, Some(2.0));
    assert!(messiah.series[0].id.is_some());
    let habits = library.iter().find(|e| e.book.id == BookId::new(428_023));
    assert_eq!(habits.expect("shelved").series, []);
    // Featured first.
    let fellowship = library
        .iter()
        .find(|e| e.book.id == BookId::new(139_773))
        .unwrap();
    assert_eq!(fellowship.series[0].name, "The Lord of the Rings");
}

#[test]
fn similar_books_come_from_the_catalogue() {
    let client = client();
    let dune = client.book_details(BookId::new(312_460)).unwrap();
    assert_ne!(dune.similar_book_ids, []);
    assert!(!dune.similar_book_ids.contains(&BookId::new(312_460)));
    let books = client.books_by_ids(&dune.similar_book_ids).unwrap();
    assert_eq!(books.len(), dune.similar_book_ids.len());
    assert_eq!(books[0].id, dune.similar_book_ids[0]);
    assert_eq!(dune.ratings_distribution, []);
}

#[test]
fn author_and_series_hits_open_their_pages() {
    let client = client();
    let authors = client.search_authors("herbert", 1).unwrap();
    assert_eq!(authors.hits.len(), 1);
    let herbert = &authors.hits[0];
    assert_eq!(herbert.name, "Frank Herbert");
    assert_eq!(herbert.top_books[0], "Dune");
    assert_eq!(herbert.books_count, None);
    assert_eq!(client.author(herbert.id).unwrap().name, "Frank Herbert");

    let series = client.search_series("dune", 1).unwrap();
    assert_eq!(series.hits.len(), 1);
    let dune = &series.hits[0];
    assert_eq!(dune.author.as_deref(), Some("Frank Herbert"));
    assert_eq!(dune.books.first().map(String::as_str), Some("Dune"));
    assert_eq!(client.series(dune.id).unwrap().name, "Dune");

    // Every author and series in the catalogue can be found and opened.
    for (id, name) in CATALOGUE.authors() {
        let hits = client.search_authors(name, 1).unwrap().hits;
        let hit = hits.iter().find(|a| a.id.get() == id).expect(name);
        assert_eq!(client.author(hit.id).unwrap().name, name);
    }
    for (id, name) in CATALOGUE.series() {
        let hits = client.search_series(name, 1).unwrap().hits;
        let hit = hits.iter().find(|s| s.id.get() == id).expect(name);
        assert_eq!(client.series(hit.id).unwrap().name, name);
    }
    assert_eq!(client.search_series("no such series", 1).unwrap().hits, []);
}

#[test]
fn trending_resolves_every_id_for_every_period() {
    use crate::TrendingPeriod::*;
    let client = client();
    let mut firsts = Vec::new();
    for period in [Week, Month, ThreeMonths, Year, AllTime] {
        let books = client.trending_books(period).unwrap();
        assert_eq!(
            books.len(),
            crate::api::TRENDING_LIMIT.min(CATALOGUE.books.len())
        );
        firsts.push(books[0].id);
    }
    // Periods rank differently, so switching shows a change.
    assert_ne!(firsts[0], firsts[1]);
    assert_ne!(firsts[1], firsts[3]);
}

#[test]
fn the_reader_has_lists_and_an_owned_list() {
    let client = client();
    let lists = client.lists().unwrap();
    assert!(lists.len() >= 3);
    let owned: Vec<_> = lists.iter().filter(|l| l.is_owned()).collect();
    assert_eq!(owned.len(), 1);
    let ranked = lists.iter().find(|l| l.ranked).expect("a ranked list");
    let books = client.list_books(ranked.id, 0).unwrap().items;
    assert_eq!(books.len(), ranked.books_count as usize);
    let positions: Vec<Option<i32>> = books.iter().map(|b| b.position).collect();
    assert_eq!(positions[..2], [Some(1), Some(2)]);
    let past_the_end = client.list_books(ranked.id, 100).unwrap();
    assert!(past_the_end.items.is_empty() && !past_the_end.has_more);

    let all = client.list_memberships().unwrap();
    assert!(all.complete);
    let total: u32 = lists.iter().map(|l| l.books_count).sum();
    assert_eq!(all.rows.len(), total as usize);
    // Owned rows name an edition; other lists name none.
    for row in &all.rows {
        let on_owned = row.list_id == owned[0].id;
        assert_eq!(row.edition_id.is_some(), on_owned, "{row:?}");
    }
    // An audiobook edition on the Owned list.
    assert!(
        all.rows
            .iter()
            .any(|r| r.edition_id == Some(EditionId::new(30_858_044)))
    );
    let dune = client.book_list_memberships(BookId::new(312_460)).unwrap();
    assert!(dune.iter().any(|r| r.list_id == owned[0].id));
}

#[test]
fn upcoming_books_follow_the_library_series() {
    let catalogue = &*CATALOGUE;
    for u in &catalogue.upcoming {
        assert!(catalogue.book(u.id).is_none(), "{} is a catalogue id", u.id);
        assert_ne!(
            catalogue.series_id(&u.series),
            0,
            "{} has no series",
            u.title
        );
        assert!(u.days > 0, "{} is already out", u.title);
    }
    assert!(
        catalogue.upcoming.iter().any(|u| u.days <= 14),
        "Home shows one soon"
    );

    let client = client();
    let library = library(&client);
    let series: Vec<crate::SeriesId> = library
        .iter()
        .flat_map(|e| e.series.iter().filter_map(|s| s.id))
        .collect();
    let today = today();
    let books = client.upcoming_in_series(&series, today).unwrap();
    assert_eq!(books.len(), catalogue.upcoming.len());
    assert!(
        books
            .windows(2)
            .all(|w| w[0].release_date <= w[1].release_date)
    );
    assert!(
        books
            .iter()
            .all(|b| b.release_date > today && b.book.cover.is_none())
    );
    assert!(books.iter().all(|b| !b.series.is_empty()));
    // Only the series asked for; none, no books.
    let first = books[0].series[0].id.unwrap();
    let one = client.upcoming_in_series(&[first], today).unwrap();
    assert!(one.iter().all(|b| b.series[0].id == Some(first)));
    assert!(one.len() < books.len());

    // Its page opens, with no description, and its series lists it.
    let details = client.book_details(books[0].book.id).unwrap();
    assert_eq!(details.release_date, Some(books[0].release_date));
    assert!(details.description.is_none());
    let page = client.series(first).unwrap();
    assert!(page.books.iter().any(|b| b.book.id == books[0].book.id));
}

#[test]
fn the_series_overview_lists_catalogue_and_upcoming_books() {
    let catalogue = &*CATALOGUE;
    let client = client();
    let ids: Vec<crate::SeriesId> = catalogue
        .series()
        .into_iter()
        .map(|(id, _)| crate::SeriesId::new(id))
        .collect();
    let load = client.tracked_series(&ids);
    assert!(load.error.is_none() && load.missing.is_empty());
    let series = load.series;
    assert_eq!(series.len(), ids.len());
    let named = |name: &str| series.iter().find(|s| s.name == name).unwrap();

    let dune: Vec<f32> = named("Dune").books.iter().map(|b| b.position).collect();
    assert_eq!(dune, [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(named("Dune").author.as_ref().unwrap().name, "Frank Herbert");
    // A novella still to come, with its date.
    let mistborn = named("The Mistborn Saga: The Original Trilogy");
    let novella = mistborn
        .books
        .iter()
        .find(|b| b.position.total_cmp(&3.5).is_eq())
        .unwrap();
    assert!(novella.release_date.unwrap() > today());
    // An upcoming book without a position is filtered, as the query does.
    assert!(
        named("Reidverse")
            .books
            .iter()
            .all(|b| b.book.title != "Untitled Reidverse Novel")
    );
    assert!(
        series
            .iter()
            .all(|s| { s.books.windows(2).all(|w| w[0].position <= w[1].position) })
    );
    // Only the series asked for.
    let one = client.tracked_series(&ids[..1]).series;
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, ids[0]);
}

#[test]
fn every_document_has_an_answer() {
    for (name, doc) in DOCUMENTS {
        let op = operation(doc);
        let body = answer_fresh(op, &json!({}));
        let unanswered = body["errors"][0]["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("The demo has no answer"));
        assert!(!unanswered, "{name} ({op}) has no demo answer");
    }
}

#[test]
fn the_clock_gives_the_local_date() {
    let clock = at("2026-12-31T23:30:00-05:00");
    assert_eq!(clock.today(), day("2026-12-31"));
    assert_eq!(clock.today().year(), 2026);
}

// ----- vibes -----

#[test]
fn the_reader_has_built_system_vibes_over_books_they_havent_read() {
    let client = pinned();
    let vibes = client.system_vibes().unwrap();
    let kinds: Vec<crate::VibeKind> = vibes.iter().map(|v| v.kind).collect();
    assert_eq!(
        kinds,
        [crate::VibeKind::Recommendations, crate::VibeKind::TopPicks]
    );
    // Built a day before the pinned "now", as Hardcover dates it (UTC).
    let built = vibes[0].built_at.unwrap();
    assert_eq!(built.to_rfc3339(), "2026-11-20T06:30:00+00:00");
    // Recommendations leave out what the reader has read, and keep some
    // of their Want to read.
    let read: Vec<BookId> = library(&client)
        .iter()
        .filter(|e| e.status == ReadingStatus::Read)
        .map(|e| e.book.id)
        .collect();
    let recommended = &vibes[0].book_ids;
    assert!(recommended.len() >= 10);
    assert!(recommended.iter().all(|b| !read.contains(b)));
    let books = client.vibe_books(recommended, 0).unwrap();
    assert_eq!(books.items.len(), recommended.len(), "all in the catalogue");
    assert_eq!(
        books.items.iter().map(|b| b.id).collect::<Vec<_>>(),
        *recommended,
        "in the vibe's order"
    );
}

#[test]
fn featured_vibes_come_newest_first_and_are_others() {
    let client = pinned();
    let me = client.me().unwrap().id;
    let featured = client.featured_vibes().unwrap();
    assert!(featured.len() >= 2);
    assert!(featured.iter().all(|v| v.featured && v.user_id != Some(me)));
    assert!(featured.iter().all(crate::Vibe::is_built));
    assert_eq!(featured[0].title, "Worlds to get lost in");
    // Someone else's vibe can't be changed.
    let id = featured[0].id;
    assert!(
        client
            .exclude_from_vibe(id, featured[0].book_ids[0])
            .is_err()
    );
    assert_eq!(
        client.vibe(id).unwrap().unwrap().book_ids,
        featured[0].book_ids
    );
}

#[test]
fn never_recommend_takes_the_book_out_of_the_vibe() {
    let client = pinned();
    let vibe = client.system_vibes().unwrap().remove(0);
    let book = vibe.book_ids[2];
    let written = client.exclude_from_vibe(vibe.id, book).unwrap();
    assert!(!written.book_ids.contains(&book));
    assert_eq!(written.book_ids.len(), vibe.book_ids.len() - 1);
    // The rest of the vibe is as it was: the write resent it.
    assert_eq!(written.title, vibe.title);
    assert_eq!(written.description, vibe.description);
    assert_eq!(written.privacy_id, vibe.privacy_id);
    assert_eq!(written.built_at, vibe.built_at, "no rebuild");
    let read_back = client.vibe(vibe.id).unwrap().unwrap();
    assert_eq!(read_back.book_ids, written.book_ids);
}

#[test]
fn the_vibe_write_refuses_what_the_demo_doesnt_write() {
    let mut state = State::new(&CATALOGUE, Clock::System);
    let vars = json!({ "id": 501, "object": { "title": "Recs", "dynamic_signals": [] } });
    let body = answer(&mut state, "ExcludeFromVibe", &vars);
    assert!(
        body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("dynamic_signals")
    );
    let vars = json!({ "id": 501, "object": {
        "vibe_exclusions_attributes": [{"excluded_type": "Book", "excluded_id": 1, "_destroy": true}]
    }});
    let body = answer(&mut state, "ExcludeFromVibe", &vars);
    assert!(
        body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("_destroy")
    );
}

#[test]
fn unbuilt_and_missing_system_vibes() {
    let faults = Faults::parse("vibes=unbuilt").unwrap();
    assert_eq!(faults.to_string(), "vibes=unbuilt");
    let client = client_with(&faults);
    let vibes = client.system_vibes().unwrap();
    assert_eq!(vibes.len(), 2);
    assert!(vibes.iter().all(|v| !v.is_built() && v.built_at.is_none()));
    assert!(!client.featured_vibes().unwrap().is_empty(), "others' stay");

    let client = client_with(&Faults::parse("vibes=none").unwrap());
    assert!(client.system_vibes().unwrap().is_empty());
    let ids = client.ensure_system_vibes().unwrap();
    assert_eq!(ids.len(), 2);
    let vibes = client.system_vibes().unwrap();
    assert_eq!(vibes.iter().map(|v| v.id).collect::<Vec<_>>(), ids);
    assert!(vibes.iter().all(|v| !v.is_built()));
    assert_eq!(client.ensure_system_vibes().unwrap(), ids, "made once");
    assert!(Faults::parse("vibes=sometimes").is_err());
}

// ----- tags -----

#[test]
fn every_demo_book_has_its_real_votes_and_line() {
    use crate::tags::{mood_line, rarely};
    let expected: Value =
        serde_json::from_str(include_str!("../tags/fixtures/expected-lines.json")).unwrap();
    let counts: Value =
        serde_json::from_str(include_str!("../tags/fixtures/catalogue-tag-counts.json")).unwrap();
    let client = client();
    for book in &CATALOGUE.books {
        let details = client.book_details(BookId::new(book.id)).unwrap();
        let want = expected["expected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == book.id)
            .unwrap_or_else(|| panic!("{} is not in the fixture", book.title));
        let real = counts["books"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == book.id)
            .unwrap();
        assert_eq!(real["in_demo"], true, "{}", book.title);
        for (tag, n) in real["counts"].as_object().unwrap() {
            let tag = crate::TagId::new(tag.parse().unwrap());
            assert_eq!(
                u64::from(details.votes.get(tag)),
                n.as_u64().unwrap(),
                "{}: {tag}",
                book.title
            );
        }
        let line = mood_line(&details.votes);
        let slugs = |moods: &[crate::tags::Mood]| -> Vec<String> {
            moods.iter().map(|m| m.slug().to_owned()).collect()
        };
        match want["line"].as_array() {
            None => assert_eq!(line, None, "{}", book.title),
            Some(moods) => {
                let line = line.unwrap();
                let want: Vec<String> = moods
                    .iter()
                    .map(|m| m.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(slugs(&line.moods), want, "{}", book.title);
            }
        }
        let want_rarely: Vec<String> = want["rarely"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            slugs(&rarely(&details.votes)),
            want_rarely,
            "{}",
            book.title
        );
        // Every genre the catalogue names, with its real votes.
        assert_eq!(details.genres.len(), book.genres.len(), "{}", book.title);
    }
}

#[test]
fn the_demo_shows_the_awkward_mood_lines() {
    use crate::tags::{Pace, mood_line};
    let client = client();
    let line = |id: i64| mood_line(&client.book_details(BookId::new(id)).unwrap().votes);
    // Ulysses: 14 mood votes, no line.
    assert_eq!(line(427_513), None);
    // Edgedancer is fast, The Fellowship of the Ring slow; most name no pace.
    assert_eq!(line(427_544).unwrap().pace, Some(Pace::Fast));
    assert_eq!(line(139_773).unwrap().pace, Some(Pace::Slow));
    assert_eq!(line(PROJECT_HAIL_MARY).unwrap().pace, None);
    // An upcoming book has no tags.
    let upcoming = CATALOGUE.upcoming[0].id;
    let details = client.book_details(BookId::new(upcoming)).unwrap();
    assert!(details.votes.is_empty());
}

/// Two Want to Read books share a genre and a mood, so the shelf's filter
/// has something to combine.
#[test]
fn want_to_read_books_share_a_genre_and_a_mood() {
    use crate::tags::{Mood, has_mood};
    let client = client();
    let ids: Vec<BookId> = library(&client)
        .into_iter()
        .filter(|e| e.status == ReadingStatus::WantToRead)
        .map(|e| e.book.id)
        .collect();
    let load = client.book_tags(&ids);
    assert!(load.error.is_none() && load.missing.is_empty());
    assert_eq!(load.books.len(), ids.len());
    let both = |genre: &str, mood: Mood| {
        load.books
            .iter()
            .filter(|b| b.genres.has(genre) && has_mood(&b.moods, mood))
            .count()
    };
    // Dune Messiah, The Midnight Library, Klara and the Sun.
    assert_eq!(both("science fiction", Mood::Reflective), 3);
    // Fourth Wing, Circe, Edgedancer, The Fellowship of the Ring.
    assert_eq!(both("fantasy", Mood::Adventurous), 4);
    // Klara's "Sci-fi" 3 adds to its "Science Fiction" 9; Edgedancer's
    // genres with a vote or two are left out.
    let klara = load.books.iter().find(|b| b.book.get() == 266_607).unwrap();
    assert_eq!(klara.genres.as_slice()[0].votes, 12);
    let edgedancer = load.books.iter().find(|b| b.book.get() == 427_544).unwrap();
    let names: Vec<&str> = edgedancer.genres.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Fantasy"]);
}

#[test]
fn a_tag_page_ranks_the_catalogue_by_votes() {
    use crate::TagId;
    use crate::tags::Mood;
    let client = client();
    let relaxing = client.books_by_tag(Mood::Relaxing.id(), None, 0).unwrap();
    assert!(!relaxing.has_more);
    let votes: Vec<u32> = relaxing.items.iter().map(|b| b.votes).collect();
    assert!(votes.windows(2).all(|w| w[0] >= w[1]), "{votes:?}");
    let top = CATALOGUE
        .books
        .iter()
        .max_by_key(|b| b.votes.get(&Mood::Relaxing.id().get()))
        .unwrap();
    assert_eq!(relaxing.items[0].book.id.get(), top.id);
    assert_eq!(
        relaxing.items.len(),
        CATALOGUE
            .books
            .iter()
            .filter(|b| b.votes.contains_key(&Mood::Relaxing.id().get()))
            .count()
    );

    // Paged: an offset starts further down, and pages join up.
    let later = client.books_by_tag(Mood::Relaxing.id(), None, 2).unwrap();
    assert_eq!(later.items, relaxing.items[2..]);
    assert_eq!(
        later.next_offset,
        u32::try_from(relaxing.items.len()).unwrap()
    );
    let page = |offset: i64| {
        answer_fresh(
            "BooksByTag",
            &json!({ "tag": Mood::Relaxing.id().get(), "limit": 3, "offset": offset }),
        )["data"]["taggable_counts"]
            .as_array()
            .unwrap()
            .clone()
    };
    let (first, second) = (page(0), page(3));
    assert_eq!(first.len(), 3);
    assert_eq!(first[0]["book"]["id"], top.id);
    assert_eq!(second[0]["book"]["id"], relaxing.items[3].book.id.get());

    // A Browse genre: Fantasy (Hardcover's tag 2), the most voted first.
    let fantasy = client.books_by_tag(TagId::new(2), None, 0).unwrap();
    assert_eq!(fantasy.items[0].book.title, "Mistborn: The Final Empire");
    assert_eq!(fantasy.items[0].votes, 65);
    assert!(fantasy.items.iter().all(|b| b.votes > 0));

    // Narrowed: adventurous books with at least 3 Fantasy votes.
    let narrowed = client
        .books_by_tag(Mood::Adventurous.id(), Some(TagId::new(2)), 0)
        .unwrap();
    let want: Vec<i64> = {
        let mut books: Vec<&Book> = CATALOGUE
            .books
            .iter()
            .filter(|b| b.votes.contains_key(&Mood::Adventurous.id().get()))
            .filter(|b| b.genres.iter().any(|g| g.name == "Fantasy" && g.votes >= 3))
            .collect();
        books.sort_by_key(|b| {
            (
                std::cmp::Reverse(b.votes[&Mood::Adventurous.id().get()]),
                b.id,
            )
        });
        books.into_iter().map(|b| b.id).collect()
    };
    let got: Vec<i64> = narrowed.items.iter().map(|b| b.book.id.get()).collect();
    assert_eq!(got, want);
    let adventurous = client
        .books_by_tag(Mood::Adventurous.id(), None, 0)
        .unwrap();
    assert!(got.len() >= 3 && got.len() < adventurous.items.len());
    // Young Adult (3): The Cruel Prince has 2 votes for it and Edgedancer 1,
    // under the threshold; Mistborn has 26.
    let young_adult = client
        .books_by_tag(Mood::Adventurous.id(), Some(TagId::new(3)), 0)
        .unwrap();
    let titles: Vec<&str> = young_adult
        .items
        .iter()
        .map(|b| b.book.title.as_str())
        .collect();
    assert_eq!(titles, ["Mistborn: The Final Empire"]);

    // A genre the Browse list hasn't got has a demo id of its own.
    let sci_fi = CATALOGUE.genre_id("Sci-fi");
    assert!(sci_fi > 90_000);
    let page = client.books_by_tag(TagId::new(sci_fi), None, 0).unwrap();
    let titles: Vec<&str> = page.items.iter().map(|b| b.book.title.as_str()).collect();
    assert_eq!(titles, ["Klara and the Sun"]);
    assert_eq!(CATALOGUE.genre_id("Science Fiction"), 6);
    assert!(
        client
            .books_by_tag(TagId::new(424_242), None, 0)
            .unwrap()
            .items
            .is_empty()
    );
}
