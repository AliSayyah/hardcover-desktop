//! Export tests. Every book, user, list and review here is made up.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use chrono::TimeZone as _;
use serde_json::{Value, json};

use super::goodreads::{COLUMNS, isbn_10, last_first, shelf_name, whole_stars};
use super::*;
use crate::api::{
    BookId, BookSummary, EditionId, EditionSummary, GoalId, GoalMetric, ListBookId, ListId,
    Privacy, Rating, ReadId, ReadSession, ReadingStatus, SeriesRef, UserId,
};

fn day(y: i32, m: u32, d: u32) -> Date {
    Date::from_ymd_opt(y, m, d).unwrap()
}

fn at() -> Timestamp {
    chrono::Utc
        .with_ymd_and_hms(2026, 11, 20, 8, 30, 0)
        .unwrap()
}

fn book(id: i64, title: &str, authors: &[&str]) -> BookSummary {
    let mut b = BookSummary::new(BookId::new(id), title);
    b.authors = authors.iter().map(|a| (*a).to_owned()).collect();
    b
}

fn entry(id: i64, book: BookSummary, status: ReadingStatus) -> LibraryEntry {
    let mut e = LibraryEntry::new(UserBookId::new(id), UserId::new(7), book, status);
    e.date_added = Some(day(2025, 1, id as u32 % 28 + 1));
    e
}

fn read(id: i64, started: Option<Date>, finished: Option<Date>) -> ReadSession {
    let mut r = ReadSession::new(ReadId::new(id));
    r.started_at = started;
    r.finished_at = finished;
    r
}

fn backup(entries: Vec<LibraryEntry>) -> Backup {
    Backup::new(at(), Some(User::new(UserId::new(7), "reader")), entries)
}

/// A strict RFC 4180 reader, enough to check what the writer produces.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => field.push(c),
            }
        } else {
            match c {
                '"' => {
                    assert!(field.is_empty(), "a quote inside an unquoted field");
                    quoted = true;
                }
                ',' => row.push(std::mem::take(&mut field)),
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                '\r' => panic!("a bare carriage return"),
                c => field.push(c),
            }
        }
    }
    assert!(!quoted, "an unterminated quote");
    assert!(field.is_empty() && row.is_empty(), "no newline at the end");
    rows
}

/// The CSV as (header → value) maps, one per data row.
fn csv_rows(backup: &Backup) -> Vec<std::collections::HashMap<String, String>> {
    let rows = parse_csv(&goodreads_csv(backup));
    let header = &rows[0];
    rows[1..]
        .iter()
        .map(|r| {
            assert_eq!(r.len(), header.len());
            header.iter().cloned().zip(r.iter().cloned()).collect()
        })
        .collect()
}

#[test]
fn the_header_is_goodreads_columns_in_order() {
    let csv = goodreads_csv(&backup(vec![]));
    assert_eq!(
        csv,
        "Book Id,Title,Author,Author l-f,Additional Authors,ISBN,ISBN13,My Rating,\
         Average Rating,Publisher,Binding,Number of Pages,Year Published,\
         Original Publication Year,Date Read,Date Added,Bookshelves,\
         Bookshelves with positions,Exclusive Shelf,My Review,Spoiler,Private Notes,\
         Read Count,Owned Copies\n"
    );
    assert_eq!(COLUMNS.len(), 24);
}

#[test]
fn a_full_row_matches_goodreads_bytes() {
    let mut b = book(
        1001,
        "The Lantern Keeper",
        &["Ada Q. Fernwood", "Bo Tallis", "Cy Rook"],
    );
    b.average_rating = Some(3.912);
    b.pages = Some(300);
    b.release_year = Some(1987);
    let mut e = entry(11, b, ReadingStatus::Read);
    let mut ed = EditionSummary::new(EditionId::new(55));
    ed.isbn_13 = Some("978-0-306-40615-7".into());
    ed.publisher = Some("Quill, Ink & Sons".into());
    ed.format = Some("Paperback".into());
    ed.pages = Some(312);
    ed.release_date = Some(day(2001, 5, 4));
    e.edition = Some(ed);
    e.rating = Rating::new(4.0);
    e.date_added = Some(day(2024, 1, 2));
    e.reads = vec![read(1, Some(day(2024, 2, 1)), Some(day(2024, 3, 9)))];
    let csv = goodreads_csv(&backup(vec![e]));
    let row = csv.lines().nth(1).unwrap();
    assert_eq!(
        row,
        "1001,The Lantern Keeper,Ada Q. Fernwood,\"Fernwood, Ada Q.\",\"Bo Tallis, Cy Rook\",\
         \"=\"\"0306406152\"\"\",\"=\"\"9780306406157\"\"\",4,3.91,\"Quill, Ink & Sons\",\
         Paperback,312,2001,1987,2024/03/09,2024/01/02,,,read,,,,1,0"
    );
}

#[test]
fn missing_values_are_empty_and_an_empty_isbn_is_quoted_like_goodreads() {
    let e = entry(12, book(1002, "Untitled Notes", &[]), ReadingStatus::Read);
    let csv = goodreads_csv(&backup(vec![e]));
    assert_eq!(
        csv.lines().nth(1).unwrap(),
        "1002,Untitled Notes,,,,\"=\"\"\"\"\",\"=\"\"\"\"\",0,,,,,,,,2025/01/13,,,read,,,,0,0"
    );
}

#[test]
fn reviews_with_commas_quotes_and_newlines_survive() {
    let mut e = entry(
        13,
        book(1003, "A \"Quoted\", Title", &["Pat Doe"]),
        ReadingStatus::Read,
    );
    e.private_notes = Some("line one\r\nline two, with \"quotes\"".into());
    let review = "Loved it, mostly.\n\n\"Odd\" ending, though;\nworth it.";
    let mut b = backup(vec![e]);
    b.reviews.insert(
        UserBookId::new(13),
        Review::new(review, true, Some(day(2024, 4, 1))),
    );
    let rows = csv_rows(&b);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["Title"], "A \"Quoted\", Title");
    assert_eq!(rows[0]["My Review"], review);
    assert_eq!(rows[0]["Spoiler"], "true");
    assert_eq!(
        rows[0]["Private Notes"],
        "line one\r\nline two, with \"quotes\""
    );
}

#[test]
fn persian_text_survives() {
    let title = "باغِ آینه‌ها، جلد دوم";
    let author = "نویسنده‌ی نمونه";
    let review = "کتابی آرام و خوش‌خوان؛ پایانش را دوست داشتم.";
    let mut e = entry(14, book(1004, title, &[author]), ReadingStatus::WantToRead);
    e.private_notes = Some("یادداشت: «فصل سوم» را دوباره بخوانم".into());
    let mut list = UserList::new(ListId::new(3), "کتاب‌های خوب، ۲۰۲۶");
    list.slug = Some("ktab-hay-khob".into());
    let mut b = backup(vec![e]);
    b.reviews
        .insert(UserBookId::new(14), Review::new(review, false, None));
    b.lists = Some(vec![ListContents {
        list,
        books: vec![ListBook::new(
            ListBookId::new(30),
            book(1004, title, &[author]),
        )],
    }]);
    let rows = csv_rows(&b);
    assert_eq!(rows[0]["Title"], title);
    assert_eq!(rows[0]["Author"], author);
    assert_eq!(rows[0]["My Review"], review);
    assert_eq!(rows[0]["Spoiler"], "");
    assert_eq!(
        rows[0]["Private Notes"],
        "یادداشت: «فصل سوم» را دوباره بخوانم"
    );
    // The Persian comma goes with the rest of the punctuation; the
    // zero-width non-joiner inside a word stays.
    assert_eq!(rows[0]["Bookshelves"], "to-read, کتاب‌های-خوب-۲۰۲۶");
    assert_eq!(
        rows[0]["Bookshelves with positions"],
        "to-read (#1), کتاب‌های-خوب-۲۰۲۶ (#1)"
    );
    // And in the JSON, as written.
    let json: Value = serde_json::from_str(&backup_json(&b)).unwrap();
    assert_eq!(json["library"][0]["book"]["title"], title);
    assert_eq!(json["library"][0]["review"]["markdown"], review);
    assert_eq!(json["lists"][0]["name"], "کتاب‌های خوب، ۲۰۲۶");
    assert!(
        backup_json(&b).contains(title),
        "not escaped as \\u sequences"
    );
}

#[test]
fn half_stars_round_up_to_whole_ones() {
    let cases = [
        (None, 0),
        (Some(0.5), 1),
        (Some(1.0), 1),
        (Some(1.5), 2),
        (Some(2.5), 3),
        (Some(3.0), 3),
        (Some(3.5), 4),
        (Some(4.5), 5),
        (Some(5.0), 5),
    ];
    for (rating, stars) in cases {
        assert_eq!(whole_stars(rating), stars, "{rating:?}");
    }
    let mut e = entry(15, book(1005, "Halfway", &[]), ReadingStatus::Read);
    e.rating = Rating::new(3.5);
    assert_eq!(csv_rows(&backup(vec![e]))[0]["My Rating"], "4");
}

#[test]
fn each_status_has_its_shelf_and_ignored_books_are_left_out() {
    let entries: Vec<LibraryEntry> = ReadingStatus::ALL
        .iter()
        .enumerate()
        .map(|(ix, status)| {
            let id = ix as i64 + 1;
            entry(id, book(2000 + id, &format!("Book {id}"), &[]), *status)
        })
        .collect();
    let rows = csv_rows(&backup(entries));
    let shelves: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r["Exclusive Shelf"].as_str(), r["Bookshelves"].as_str()))
        .collect();
    assert_eq!(
        shelves,
        [
            ("to-read", "to-read"),
            ("currently-reading", "currently-reading"),
            ("read", ""),
            ("paused", "paused"),
            ("did-not-finish", "did-not-finish"),
        ]
    );
    assert!(rows.iter().all(|r| r["Book Id"] != "2006"));
}

#[test]
fn status_shelf_positions_follow_the_order_added() {
    let mut first = entry(
        21,
        book(3001, "Later id, added first", &[]),
        ReadingStatus::WantToRead,
    );
    first.date_added = Some(day(2020, 1, 1));
    let mut second = entry(
        20,
        book(3002, "Earlier id, added later", &[]),
        ReadingStatus::WantToRead,
    );
    second.date_added = Some(day(2021, 1, 1));
    let rows = csv_rows(&backup(vec![first, second]));
    // Rows go by entry id; positions by date added.
    assert_eq!(rows[0]["Book Id"], "3002");
    assert_eq!(rows[0]["Bookshelves with positions"], "to-read (#2)");
    assert_eq!(rows[1]["Bookshelves with positions"], "to-read (#1)");
}

#[test]
fn multiple_reads_count_the_finished_ones_and_date_the_latest() {
    let mut e = entry(
        16,
        book(1006, "Reread", &[]),
        ReadingStatus::CurrentlyReading,
    );
    e.reads = vec![
        read(40, Some(day(2026, 10, 1)), None),
        read(41, Some(day(2019, 1, 1)), Some(day(2019, 2, 1))),
        read(42, Some(day(2023, 6, 1)), Some(day(2023, 7, 15))),
        read(43, None, Some(day(2021, 3, 3))),
    ];
    let rows = csv_rows(&backup(vec![e.clone()]));
    assert_eq!(rows[0]["Read Count"], "3");
    assert_eq!(rows[0]["Date Read"], "2023/07/15");
    assert_eq!(rows[0]["Exclusive Shelf"], "currently-reading");

    let json: Value = serde_json::from_str(&backup_json(&backup(vec![e]))).unwrap();
    let reads = json["library"][0]["reads"].as_array().unwrap();
    let ids: Vec<i64> = reads.iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, [40, 41, 42, 43]);
    assert_eq!(reads[0]["finished_at"], Value::Null);
    assert_eq!(reads[2]["finished_at"], "2023-07-15");
}

#[test]
fn lists_become_shelves_and_the_owned_list_owned_copies() {
    let a = book(1007, "On Two Lists", &[]);
    let b = book(1008, "Owned Only", &[]);
    let mut owned = UserList::new(ListId::new(1), "Owned");
    owned.slug = Some(crate::api::OWNED_LIST_SLUG.into());
    let favourites = UserList::new(ListId::new(2), "Favourites, All Time!");
    let mut summer = UserList::new(ListId::new(3), "Summer");
    summer.slug = Some("summer".into());
    let mut on_owned = ListBook::new(ListBookId::new(10), b.clone());
    on_owned.edition_id = Some(EditionId::new(99));
    let mut b_ = backup(vec![
        entry(17, a.clone(), ReadingStatus::Read),
        entry(18, b.clone(), ReadingStatus::Read),
    ]);
    b_.lists = Some(vec![
        ListContents {
            list: summer,
            books: vec![
                ListBook::new(ListBookId::new(20), book(9999, "Not in the library", &[])),
                ListBook::new(ListBookId::new(21), a.clone()),
            ],
        },
        ListContents {
            list: owned,
            books: vec![on_owned],
        },
        ListContents {
            list: favourites,
            books: vec![
                ListBook::new(ListBookId::new(30), a.clone()),
                // A second edition of the same book: still one shelf.
                ListBook::new(ListBookId::new(31), a.clone()),
            ],
        },
    ]);
    let rows = csv_rows(&b_);
    assert_eq!(rows[0]["Bookshelves"], "favourites-all-time, summer");
    assert_eq!(
        rows[0]["Bookshelves with positions"],
        "favourites-all-time (#1), summer (#2)"
    );
    assert_eq!(rows[0]["Owned Copies"], "0");
    assert_eq!(rows[1]["Bookshelves"], "");
    assert_eq!(rows[1]["Owned Copies"], "1");
}

#[test]
fn shelf_names_never_hold_a_comma() {
    let named = |name: &str, slug: Option<&str>| {
        let mut l = UserList::new(ListId::new(5), name);
        l.slug = slug.map(Into::into);
        shelf_name(&l)
    };
    assert_eq!(
        named("Sci-Fi, Classics & More", None),
        "sci-fi-classics-more"
    );
    assert_eq!(named("  --  ", Some("my-list")), "my-list");
    assert_eq!(named(",,,", None), "list-5");
}

#[test]
fn isbn_10_and_author_names() {
    assert_eq!(isbn_10("9780306406157").as_deref(), Some("0306406152"));
    // A check digit of 10 is written X.
    assert_eq!(isbn_10("9780804429573").as_deref(), Some("080442957X"));
    assert_eq!(isbn_10("9791234567896"), None);
    assert_eq!(isbn_10("978030640615"), None);
    assert_eq!(isbn_10("97803064061x7"), None);
    assert_eq!(last_first("Aaron A. Reed"), "Reed, Aaron A.");
    assert_eq!(last_first("Plato"), "Plato");
    assert_eq!(last_first(""), "");
}

#[test]
fn file_names_carry_the_date_and_extension() {
    assert_eq!(
        Format::GoodreadsCsv.file_name(day(2026, 3, 9)),
        "hardcover-library-2026-03-09.csv"
    );
    assert_eq!(
        Format::Json.file_name(day(2026, 3, 9)),
        "hardcover-library-2026-03-09.json"
    );
}

#[test]
fn the_json_backup_has_version_one_shape() {
    let mut b = book(1009, "Glass Orchard", &["Mira Vell"]);
    b.subtitle = Some("A Novel".into());
    b.slug = Some("glass-orchard".into());
    b.pages = Some(288);
    b.release_year = Some(2011);
    let mut e = entry(19, b, ReadingStatus::Read);
    e.rating = Rating::new(4.5);
    e.privacy = Some(Privacy::FollowersOnly);
    e.private_notes = Some("Lent to a friend.".into());
    e.updated_at = Some(at());
    let mut series = SeriesRef::new("Orchard Cycle");
    series.id = Some(crate::api::SeriesId::new(400));
    series.position = Some(1.0);
    e.series = vec![series];
    let mut ed = EditionSummary::new(EditionId::new(77));
    ed.format = Some("Hardcover".into());
    ed.isbn_13 = Some("9780306406157".into());
    e.edition = Some(ed);
    let mut r = read(5, Some(day(2025, 2, 1)), Some(day(2025, 2, 20)));
    r.progress_pages = Some(288);
    r.edition_id = Some(EditionId::new(77));
    e.reads = vec![r];
    let ignored = entry(20, book(1010, "Not For Me", &[]), ReadingStatus::Ignored);

    let mut list = UserList::new(ListId::new(8), "Comfort reads");
    list.slug = Some("comfort-reads".into());
    list.privacy_id = Some(1);
    list.ranked = true;
    let mut row = ListBook::new(
        ListBookId::new(81),
        book(1009, "Glass Orchard", &["Mira Vell"]),
    );
    row.position = Some(1);

    let mut goal = Goal::new(
        GoalId::new(2),
        GoalMetric::Books,
        30,
        day(2026, 1, 1),
        day(2026, 12, 31),
    );
    goal.progress = 12.0;
    goal.privacy_id = Some(3);

    let mut backup = backup(vec![ignored, e]);
    // A review only the website's editor can read: kept, with no text.
    backup.reviews.insert(
        UserBookId::new(20),
        Review {
            has_review: true,
            ..Review::default()
        },
    );
    backup.reviews.insert(
        UserBookId::new(19),
        Review::new("Quiet and *strange*.", true, Some(day(2025, 2, 21))),
    );
    backup.lists = Some(vec![ListContents {
        list,
        books: vec![row],
    }]);
    backup.goals = Some(vec![goal]);

    let text = backup_json(&backup);
    assert!(text.starts_with("{\n  \"format\": \"hardcover-desktop-backup\",\n  \"version\": 1,"));
    assert!(text.ends_with("}\n"));
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let expected = json!({
        "format": "hardcover-desktop-backup",
        "version": 1,
        "exported_at": "2026-11-20T08:30:00Z",
        "generator": concat!("hardcover-desktop ", env!("CARGO_PKG_VERSION")),
        "user": { "id": 7, "username": "reader", "name": null },
        "omitted": [],
        "library": [
            {
                "user_book_id": 19,
                "status": "read",
                "rating": 4.5,
                "date_added": "2025-01-20",
                "updated_at": "2026-11-20T08:30:00Z",
                "privacy": "followers-only",
                "private_notes": "Lent to a friend.",
                "book": {
                    "id": 1009,
                    "title": "Glass Orchard",
                    "subtitle": "A Novel",
                    "authors": ["Mira Vell"],
                    "slug": "glass-orchard",
                    "url": "https://hardcover.app/books/glass-orchard",
                    "pages": 288,
                    "release_year": 2011,
                    "series": [{ "id": 400, "name": "Orchard Cycle", "position": 1.0 }],
                },
                "edition": {
                    "id": 77,
                    "title": null,
                    "format": "Hardcover",
                    "reading_format": null,
                    "pages": null,
                    "audio_seconds": null,
                    "isbn_13": "9780306406157",
                    "publisher": null,
                    "language": null,
                    "release_date": null,
                },
                "reads": [{
                    "id": 5,
                    "started_at": "2025-02-01",
                    "finished_at": "2025-02-20",
                    "progress_pages": 288,
                    "progress_seconds": null,
                    "progress_percent": null,
                    "edition_id": 77,
                }],
                "review": {
                    "markdown": "Quiet and *strange*.",
                    "plain": null,
                    "has_spoilers": true,
                    "reviewed_at": "2025-02-21",
                },
            },
            {
                "user_book_id": 20,
                "status": "ignored",
                "rating": null,
                "date_added": "2025-01-21",
                "updated_at": null,
                "privacy": null,
                "private_notes": null,
                "book": {
                    "id": 1010,
                    "title": "Not For Me",
                    "subtitle": null,
                    "authors": [],
                    "slug": null,
                    "url": null,
                    "pages": null,
                    "release_year": null,
                    "series": [],
                },
                "edition": null,
                "reads": [],
                "review": { "markdown": null, "plain": null, "has_spoilers": false, "reviewed_at": null },
            },
        ],
        "lists": [{
            "id": 8,
            "name": "Comfort reads",
            "slug": "comfort-reads",
            "description": null,
            "privacy_setting_id": 1,
            "ranked": true,
            "owned": false,
            "books": [{
                "list_book_id": 81,
                "book_id": 1009,
                "title": "Glass Orchard",
                "authors": ["Mira Vell"],
                "edition_id": null,
                "position": 1,
            }],
        }],
        "goals": [{
            "id": 2,
            "metric": "book",
            "target": 30,
            "progress": 12.0,
            "start": "2026-01-01",
            "end": "2026-12-31",
            "description": null,
            "archived": false,
            "completed_at": null,
            "privacy_setting_id": 3,
            "conditions": {},
        }],
    });
    assert_eq!(parsed, expected);
}

#[test]
fn parts_the_session_cant_read_are_null_and_named() {
    let text = backup_json(&backup(vec![]));
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["omitted"], json!(["lists", "goals"]));
    assert_eq!(parsed["lists"], Value::Null);
    assert_eq!(parsed["goals"], Value::Null);
    assert_eq!(parsed["library"], json!([]));
}

#[test]
fn files_are_written_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.csv");
    write_atomic(&path, b"first").unwrap();
    write_atomic(&path, "دوم".as_bytes()).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "دوم");
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from("library.csv")]);
    // A missing directory fails without leaving anything behind.
    let missing = dir.path().join("nope").join("library.csv");
    assert!(write_atomic(&missing, b"x").is_err());
}

// ----- gathering -----

fn demo_backup(client: &Client) -> Backup {
    let user = client.me().unwrap();
    let entries = client.library(user.id).unwrap();
    Backup::new(at(), Some(user), entries)
}

/// The whole export against the demo, as the app runs it. With
/// `EXPORT_SAMPLE_DIR` set, the two files are left there to inspect.
#[test]
fn the_demo_library_exports_whole() {
    let client = crate::demo::client();
    let mut seen = Vec::new();
    let all = Parts {
        lists: true,
        goals: true,
    };
    let backup = gather(
        &client,
        demo_backup(&client),
        all,
        &AtomicBool::new(false),
        |p| seen.push(p),
    )
    .unwrap();
    assert_eq!(backup.reviews.len(), 2);
    let lists = backup.lists.as_ref().unwrap();
    assert!(
        lists
            .iter()
            .any(|l| l.list.is_owned() && !l.books.is_empty())
    );
    assert!(
        lists
            .iter()
            .all(|l| l.books.len() as u32 == l.list.books_count)
    );
    assert!(!backup.goals.as_ref().unwrap().is_empty());
    assert_eq!(seen.first(), Some(&Progress::Reviews(0)));
    assert!(seen.contains(&Progress::Reviews(2)));
    assert!(seen.contains(&Progress::Lists {
        done: lists.len(),
        total: lists.len()
    }));
    assert_eq!(seen.last(), Some(&Progress::Goals));

    let dir = match std::env::var_os("EXPORT_SAMPLE_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => tempfile::tempdir().unwrap().keep(),
    };
    for format in Format::ALL {
        let path = dir.join(format.file_name(day(2026, 11, 20)));
        save(format, &backup, &path).unwrap();
    }
    let csv = std::fs::read_to_string(dir.join("hardcover-library-2026-11-20.csv")).unwrap();
    let rows = parse_csv(&csv);
    let shelved = backup
        .entries
        .iter()
        .filter(|e| e.status != ReadingStatus::Ignored)
        .count();
    assert_eq!(rows.len(), shelved + 1);
    assert!(
        rows.iter()
            .any(|r| r[19].contains('\n') || !r[19].is_empty())
    );
    let json: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("hardcover-library-2026-11-20.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        json["library"].as_array().unwrap().len(),
        backup.entries.len()
    );
    assert_eq!(json["omitted"], json!([]));
}

#[test]
fn parts_not_asked_for_are_not_fetched() {
    let client = crate::demo::client();
    let none = Parts {
        lists: false,
        goals: false,
    };
    let backup = gather(
        &client,
        demo_backup(&client),
        none,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(backup.omitted(), ["lists", "goals"]);
    assert_eq!(backup.reviews.len(), 2);
}

#[test]
fn a_cancelled_export_stops() {
    let client = crate::demo::client();
    let cancel = AtomicBool::new(false);
    let all = Parts {
        lists: true,
        goals: true,
    };
    let mut requests = 0;
    let result = gather(&client, demo_backup(&client), all, &cancel, |p| {
        requests += 1;
        if matches!(p, Progress::Lists { done: 1, .. }) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    assert_eq!(result.unwrap_err(), Error::Cancelled);
    assert!(requests > 0);
}

#[test]
fn list_pages_go_on_past_a_short_page_until_the_count() {
    use crate::http::fake::FakeTransport;
    use crate::ratelimit::TokenBucket;

    struct Token;
    impl crate::api::TokenSource for Token {
        fn access_token(&self) -> Result<String> {
            Ok("tok".into())
        }
        fn on_unauthorized(&self, _: &str) -> Result<bool> {
            Ok(false)
        }
    }
    let http = Arc::new(FakeTransport::new());
    let client = Client::with_transport(
        Arc::new(Token),
        http.clone(),
        TokenBucket::new(1000, 1000.0),
    );
    let row = |id: i64, book: Option<i64>| {
        json!({
            "id": id, "position": null, "edition_id": null,
            "book": book.map(|b| json!({
                "id": b, "title": format!("Book {b}"), "subtitle": null, "slug": null,
                "pages": null, "release_year": null, "rating": null, "users_count": null,
                "cached_image": {}, "cached_contributors": [],
            })),
        })
    };
    // The first page has a row whose book didn't resolve: 99 books, but
    // the list holds 150, so a second page follows.
    let first: Vec<Value> = (0..100)
        .map(|i| row(i, (i != 50).then_some(1000 + i)))
        .collect();
    let second: Vec<Value> = (100..150).map(|i| row(i, Some(1000 + i))).collect();
    http.push(200, &json!({"data": {"list_books": first}}).to_string());
    http.push(200, &json!({"data": {"list_books": second}}).to_string());
    let mut list = UserList::new(ListId::new(4), "Long");
    list.books_count = 150;
    let books = super::list_books(&client, &list, &|| Ok(())).unwrap();
    assert_eq!(books.len(), 149);
    let offsets: Vec<Value> = http
        .requests()
        .iter()
        .map(|r| r.json.as_ref().unwrap()["variables"]["offset"].clone())
        .collect();
    assert_eq!(offsets, [json!(0), json!(100)]);
}
