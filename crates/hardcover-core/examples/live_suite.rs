//! The weekly check of the live Hardcover API, run against a dedicated test
//! account (`.github/workflows/live-api.yml`).
//!
//! `api_check` catches a changed schema; this catches what a schema can't
//! say: `jsonb` columns that changed shape (`cached_tags`, `cached_image`,
//! `cached_book_ids`), fields that turned empty, and the behaviour the app
//! relies on (CLAUDE.md, "Hardcover API quirks"). Every call goes through
//! [`Client`], so the app's own documents and decoding are what is checked.
//!
//! ```sh
//! HARDCOVER_TOKEN=<test account's token> HARDCOVER_LIVE_ACCOUNT=<its username> \
//!   cargo run -p hardcover-core --example live_suite [-- --summary <file>]
//! ```
//!
//! It writes: it adds books (as Private), rates, reviews, makes a list, a goal
//! and a follow, then undoes all of it. It also puts an edition on the
//! account's Owned list (which posts to its feed) and excludes a book from its
//! Recommendations, which can't be undone. So it refuses to run unless the
//! token's account is `HARDCOVER_LIVE_ACCOUNT`, and refuses an account that
//! holds more than a run of this suite could have left behind
//! ([`Baseline::guard`]): never point it at a real reader's account. A run
//! starts by clearing what an earlier, interrupted run left.
//!
//! Each check is independent: a failure is reported and the run goes on, and
//! the cleanup always runs. The exit code says what kind of failure it was:
//!
//! - 0: every check passed.
//! - 1: the app is broken against the API: a check failed (an API error, a
//!   response that didn't decode, a behaviour that changed).
//! - 2: refused or not set up (the guard, a missing variable).
//! - 3: inconclusive: only the network failed (timeouts, 5xx, rate limits).
//! - 4: the token was rejected (expired or revoked): replace it.
//!
//! `--summary <file>` appends a Markdown report (for `$GITHUB_STEP_SUMMARY`).
//! Only the test account's data and public catalogue data are printed.

use std::fmt::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Datelike;
use hardcover_core::auth::StaticToken;
use hardcover_core::tags::{BROWSE_GENRES, Mood, mood_line};
use hardcover_core::{
    BookId, Client, Coverage, Date, EditionId, Error, GoalId, GoalMetric, ListDraft, ListEdit,
    ListId, NewGoal, OWNED_LIST_SLUG, Privacy, ProgressUnit, ProgressUpdate, Rating, ReadingStatus,
    SeriesId, TrendingPeriod, User, UserBookId, VibeKind,
};

/// Dune: a long-lived, much-read book in a series, with many editions.
const DUNE: BookId = BookId::new(312_460);
/// Project Hail Mary: has an audiobook edition. The library writes use it.
const HAIL_MARY: BookId = BookId::new(427_578);
/// The Way of Kings: many community reviews; Stormlight's first book.
const WAY_OF_KINGS: BookId = BookId::new(386_446);

/// The most a run of this suite leaves behind if it is killed: the guard
/// refuses an account holding more (a reader's real account).
const MAX_LEFTOVER_ENTRIES: usize = 3;
const MAX_LEFTOVER_LISTS: usize = 3;
const MAX_LEFTOVER_GOALS: usize = 3;
const MAX_LEFTOVER_FOLLOWS: usize = 3;
const MAX_OWNED_BOOKS: u32 = 3;

/// How long a read-back may lag a write (Hardcover can answer a read right
/// after a write with the old row).
const SETTLE: Duration = Duration::from_secs(20);

const LIST_NAME: &str = "Hardcover Desktop API check";
const GOAL_DESCRIPTION: &str = "Hardcover Desktop API check";
const REVIEW_TEXT: &str = "Written by Hardcover Desktop's weekly API check, and deleted at once.";

fn main() -> ExitCode {
    let summary_path = summary_path();
    let Some(token) = StaticToken::from_env() else {
        eprintln!("refused: set HARDCOVER_TOKEN to the test account's token");
        return ExitCode::from(2);
    };
    let Ok(account) = std::env::var("HARDCOVER_LIVE_ACCOUNT") else {
        eprintln!("refused: set HARDCOVER_LIVE_ACCOUNT to the test account's username");
        return ExitCode::from(2);
    };
    let client = Client::new(Arc::new(token));
    let mut suite = Suite::default();
    let code = match run(&client, &account, &mut suite) {
        Ok(()) => suite.exit_code(),
        Err(refusal) => {
            suite.refused = Some(refusal);
            2
        }
    };
    let report = suite.report();
    print!("{report}");
    if let Some(path) = summary_path {
        append(&path, &report);
    }
    ExitCode::from(code)
}

fn summary_path() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--summary" {
            return args.next();
        }
    }
    None
}

fn append(path: &str, text: &str) {
    use std::io::Write as _;
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()));
    if let Err(e) = written {
        eprintln!("could not write the summary to {path}: {e}");
    }
}

/// Runs every section. `Err` only for a refusal (the guard): nothing ran.
fn run(c: &Client, account: &str, suite: &mut Suite) -> Result<(), String> {
    suite.last = Some(Instant::now());
    let me = match c.me() {
        Ok(me) => me,
        Err(e) => {
            suite.record::<()>("setup", "who the token belongs to", Err(Fail::Api(e)));
            return Ok(());
        }
    };
    if me.username != account {
        return Err(format!(
            "the token belongs to @{}, not the test account @{account}",
            me.username
        ));
    }
    let baseline = match Baseline::read(c, &me) {
        Ok(b) => b,
        Err(e) => {
            suite.record::<()>("setup", "read the account's state", Err(e));
            return Ok(());
        }
    };
    baseline.guard()?;
    suite.record("setup", "clear what an earlier run left", reset(c, &me));

    catalogue(c, &me, suite);
    vibes(c, suite);
    library(c, &me, suite);
    lists(c, suite);
    goals(c, suite);
    follows(c, suite);

    suite.record("cleanup", "clear everything this run made", reset(c, &me));
    suite.record(
        "cleanup",
        "the account is back to empty",
        settle(|| {
            let b = Baseline::read(c, &me)?;
            Ok(b.is_empty().then_some(()).ok_or_else(|| b.describe()))
        }),
    );
    Ok(())
}

// ----- checks -----

fn catalogue(c: &Client, me: &User, s: &mut Suite) {
    const S: &str = "catalogue";
    s.record(S, "search books", {
        let page = c.search_books("dune", 1);
        page.map_err(Fail::from).and_then(|page| {
            ensure(page.total.unwrap_or(0) > 100, "Dune matches over 100 books")?;
            ensure(
                page.books.iter().take(5).any(|b| b.id == DUNE),
                "Dune (312460) is among the first five results",
            )?;
            ensure(
                page.books.iter().all(|b| !b.title.is_empty()),
                "every result has a title",
            )?;
            ensure(
                page.books
                    .iter()
                    .any(|b| !b.authors.is_empty() && b.cover.is_some()),
                "results have authors and covers",
            )
        })
    });
    s.record(S, "search authors", {
        c.search_authors("brandon sanderson", 1)
            .map_err(Fail::from)
            .and_then(|hits| {
                ensure(
                    hits.hits
                        .first()
                        .is_some_and(|a| a.name.contains("Sanderson") && !a.top_books.is_empty()),
                    "the first author is Sanderson, with books",
                )
            })
    });
    s.record(S, "search series", {
        c.search_series("stormlight", 1)
            .map_err(Fail::from)
            .and_then(|hits| {
                ensure(
                    hits.hits.iter().any(|h| h.name.contains("Stormlight")),
                    "The Stormlight Archive is found",
                )
            })
    });

    let dune = s.record(S, "book details (Dune)", {
        c.book_details(DUNE).map_err(Fail::from).and_then(|d| {
            ensure(d.summary.title == "Dune", "the title is Dune")?;
            ensure(
                d.summary.authors.iter().any(|a| a == "Frank Herbert"),
                "Frank Herbert is its author",
            )?;
            ensure(d.summary.cover.is_some(), "it has a cover")?;
            ensure(
                d.description.as_deref().is_some_and(|t| t.len() > 100),
                "it has a description",
            )?;
            ensure(
                d.series
                    .iter()
                    .any(|r| r.id.is_some() && r.name.contains("Dune")),
                "it is in the Dune series, with the series' id",
            )?;
            ensure(
                d.contributors.iter().any(|a| a.author_id.is_some()),
                "its contributors have author ids",
            )?;
            ensure(!d.genres.is_empty(), "it has genres (cached_tags)")?;
            ensure(
                d.votes.mood_votes() >= 50,
                "it has mood votes (taggable_counts)",
            )?;
            ensure(mood_line(&d.votes).is_some(), "the votes give a mood line")?;
            ensure(
                d.editions_count.unwrap_or(0) > 20,
                "it counts over 20 editions",
            )?;
            ensure(!d.similar_book_ids.is_empty(), "it has similar books")?;
            ensure(
                !d.ratings_distribution.is_empty() && d.summary.average_rating.is_some(),
                "it has ratings",
            )?;
            Ok(d)
        })
    });
    s.record(S, "book details of a book that doesn't exist", {
        match c.book_details(BookId::new(i64::from(i32::MAX))) {
            Err(Error::Api(_)) => Ok(()),
            Err(e) => Err(Fail::Api(e)),
            Ok(_) => Err(Fail::Assert("a missing book should be an error".into())),
        }
    });

    let edition = s.record(S, "editions, two pages", {
        c.book_editions(DUNE, 0)
            .map_err(Fail::from)
            .and_then(|first| {
                ensure(
                    first.has_more && !first.items.is_empty(),
                    "Dune has more than a page of editions",
                )?;
                ensure(
                    first.items.iter().any(|e| e.reading_format_id.is_some()),
                    "editions have a reading format",
                )?;
                let second = c.book_editions(DUNE, first.next_offset)?;
                ensure(!second.items.is_empty(), "the second page has editions")?;
                ensure(
                    second
                        .items
                        .iter()
                        .all(|e| first.items.iter().all(|f| f.id != e.id)),
                    "the pages don't overlap",
                )?;
                Ok(first.items[0].id)
            })
    });

    let dune_series = dune
        .as_ref()
        .and_then(|d| d.series.iter().find(|r| r.name.contains("Dune")))
        .and_then(|r| r.id);
    let herbert = dune.as_ref().and_then(|d| {
        d.contributors
            .iter()
            .find(|a| a.name == "Frank Herbert")
            .and_then(|a| a.author_id)
    });
    if let Some(series) = dune_series {
        s.record(S, "series page (Dune)", {
            c.series(series).map_err(Fail::from).and_then(|d| {
                ensure(d.books.len() >= 6, "the Dune series has its books")?;
                ensure(
                    d.books
                        .iter()
                        .any(|b| b.book.id == DUNE && b.position == Some(1.0)),
                    "Dune is book 1",
                )?;
                ensure(
                    d.author.as_ref().is_some_and(|a| a.name == "Frank Herbert"),
                    "the series' author is Frank Herbert",
                )
            })
        });
        s.record(S, "tracked series (Dune, Stormlight)", {
            let stormlight = c
                .book_details(WAY_OF_KINGS)
                .map_err(Fail::from)
                .map(|d| d.series.iter().find_map(|r| r.id));
            stormlight.and_then(|stormlight| {
                let stormlight = stormlight.ok_or("The Way of Kings has a series id")?;
                let load = c.tracked_series(&[series, stormlight]);
                if let Some(e) = load.error {
                    return Err(Fail::Api(e));
                }
                ensure(load.missing.is_empty(), "no batch failed")?;
                ensure(load.series.len() == 2, "both series come back")?;
                ensure(
                    load.series.iter().all(|t| t.books.len() >= 4),
                    "each with its books",
                )
            })
        });
        s.record(S, "upcoming books in series", {
            let today = today();
            c.upcoming_in_series(&[series], today)
                .map_err(Fail::from)
                .and_then(|books| {
                    ensure(
                        books.iter().all(|b| b.release_date > today),
                        "only books after today",
                    )
                })
        });
    } else {
        s.skip(S, "series page (Dune)", "the book details failed");
    }
    if let Some(author) = herbert {
        s.record(S, "author page (Frank Herbert)", {
            c.author(author).map_err(Fail::from).and_then(|a| {
                ensure(a.name == "Frank Herbert", "the name")?;
                ensure(
                    a.books.iter().any(|b| b.book.id == DUNE),
                    "Dune is among the books",
                )?;
                ensure(
                    a.series.iter().any(|r| Some(r.id) == dune_series),
                    "the Dune series is among the series",
                )
            })
        });
    } else {
        s.skip(S, "author page (Frank Herbert)", "the book details failed");
    }

    s.record(S, "books by ids, in order", {
        c.books_by_ids(&[WAY_OF_KINGS, DUNE, HAIL_MARY])
            .map_err(Fail::from)
            .and_then(|books| {
                let ids: Vec<BookId> = books.iter().map(|b| b.id).collect();
                ensure(
                    ids == [WAY_OF_KINGS, DUNE, HAIL_MARY],
                    format!("all three, in the order asked: got {ids:?}"),
                )
            })
    });
    for (period, name) in [
        (TrendingPeriod::Week, "week"),
        (TrendingPeriod::Month, "month"),
        (TrendingPeriod::ThreeMonths, "3 months"),
        (TrendingPeriod::Year, "year"),
        (TrendingPeriod::AllTime, "all time"),
    ] {
        s.record(S, &format!("trending ({name})"), {
            c.trending_books(period)
                .map_err(Fail::from)
                .and_then(|books| {
                    ensure(books.len() >= 10, "at least 10 books")?;
                    ensure(
                        books.iter().any(|b| !b.series.is_empty()),
                        "with their series",
                    )
                })
        });
    }

    let mood = Mood::ALL[0];
    let fantasy = BROWSE_GENRES
        .iter()
        .find(|g| g.name == "Fantasy")
        .map(|g| g.id);
    s.record(S, &format!("a mood's books ({})", mood.name()), {
        c.books_by_tag(mood.id(), None, 0)
            .map_err(Fail::from)
            .and_then(|page| {
                ensure(
                    page.items.len() >= 10 && page.has_more,
                    "a full page, and more",
                )?;
                ensure(
                    page.items.windows(2).all(|w| w[0].votes >= w[1].votes),
                    "the most votes first",
                )?;
                let next = c.books_by_tag(mood.id(), None, page.next_offset)?;
                ensure(
                    next.items
                        .iter()
                        .all(|b| page.items.iter().all(|a| a.book.id != b.book.id)),
                    "the next page doesn't repeat a book",
                )
            })
    });
    if let Some(fantasy) = fantasy {
        s.record(
            S,
            &format!("a mood's books within Fantasy ({})", mood.name()),
            {
                c.books_by_tag(mood.id(), Some(fantasy), 0)
                    .map_err(Fail::from)
                    .and_then(|page| ensure(page.items.len() >= 10, "a full page"))
            },
        );
        s.record(S, "a genre's books (Fantasy)", {
            c.books_by_tag(fantasy, None, 0)
                .map_err(Fail::from)
                .and_then(|page| {
                    ensure(page.items.len() >= 10, "a full page")?;
                    ensure(
                        page.items.iter().any(|b| b.book.id == WAY_OF_KINGS)
                            || page.items.iter().all(|b| b.votes > 0),
                        "with votes",
                    )
                })
        });
    } else {
        s.skip(S, "a genre's books (Fantasy)", "Fantasy left BROWSE_GENRES");
    }
    s.record(S, "book tags (shelf filter)", {
        let load = c.book_tags(&[DUNE, HAIL_MARY, WAY_OF_KINGS]);
        if let Some(e) = load.error {
            Err(Fail::Api(e))
        } else {
            ensure(load.missing.is_empty(), "no batch failed")
                .and_then(|()| ensure(load.books.len() == 3, "all three books"))
                .and_then(|()| {
                    ensure(
                        load.books
                            .iter()
                            .all(|t| !t.genres.is_empty() && t.moods.mood_votes() > 0),
                        "each with genres and mood votes",
                    )
                })
        }
    });

    s.record(S, "other readers' reviews", {
        c.book_reviews(WAY_OF_KINGS, me.id, 0)
            .map_err(Fail::from)
            .and_then(|page| {
                ensure(page.has_more, "more than a page")?;
                ensure(!page.reviews.is_empty(), "the page has reviews")?;
                ensure(
                    page.reviews.iter().all(|r| !r.text.trim().is_empty()),
                    "each with text",
                )?;
                ensure(
                    page.reviews.iter().any(|r| r.rating.is_some()),
                    "with ratings",
                )?;
                let count = c.book_reviews_count(WAY_OF_KINGS, me.id)?;
                ensure(count > 20, format!("the count is over 20 (got {count})"))
            })
    });
    s.owned_edition = edition;
}

fn vibes(c: &Client, s: &mut Suite) {
    const S: &str = "vibes";
    let featured = s.record(S, "featured vibes", {
        c.featured_vibes().map_err(Fail::from).and_then(|v| {
            ensure(!v.is_empty(), "Hardcover features some")?;
            ensure(v.iter().all(|v| v.featured), "each is featured")?;
            let built = v
                .iter()
                .find(|v| !v.book_ids.is_empty())
                .cloned()
                .ok_or("one has books (cached_book_ids)")?;
            ensure(
                built.built_at.is_some(),
                "a vibe with books has its build time",
            )?;
            Ok(built)
        })
    });
    if let Some(vibe) = featured {
        s.record(S, "a vibe by id, and its books", {
            c.vibe(vibe.id).map_err(Fail::from).and_then(|v| {
                let v = v.ok_or("the featured vibe is found by id")?;
                ensure(v.book_ids == vibe.book_ids, "with the same books")?;
                let page = c.vibe_books(&v.book_ids, 0)?;
                ensure(!page.items.is_empty(), "its books resolve")?;
                ensure(page.items[0].id == v.book_ids[0], "in the vibe's order")
            })
        });
    }
    let recommendations = s.record(S, "system vibes exist", {
        c.ensure_system_vibes().map_err(Fail::from).and_then(|ids| {
            ensure(ids.len() >= 2, "ensure_system_vibes returns their ids")?;
            let found = c.system_vibes()?;
            ensure(
                found.iter().any(|v| v.kind == VibeKind::TopPicks),
                "Top Picks is read back",
            )?;
            found
                .iter()
                .find(|v| v.kind == VibeKind::Recommendations)
                .map(|v| v.id)
                .ok_or_else(|| Fail::from("Recommendations is read back"))
        })
    });
    // Exclusions can't be undone, so every run excludes the same book: the
    // first run's call is a new exclusion, every later one a repeat, which
    // the client must take as done.
    if let Some(vibe) = recommendations {
        s.record(S, "never recommend a book, twice", {
            c.exclude_from_vibe(vibe, DUNE)
                .and_then(|_| c.exclude_from_vibe(vibe, DUNE))
                .map(drop)
                .map_err(Fail::from)
        });
    } else {
        s.skip(
            S,
            "never recommend a book, twice",
            "no Recommendations vibe",
        );
    }
}

fn library(c: &Client, me: &User, s: &mut Suite) {
    const S: &str = "library";
    let today = today();
    let Some(added) = s.record(S, "add a book (Private, Want to read)", {
        c.add_to_library_with_privacy(HAIL_MARY, ReadingStatus::WantToRead, Privacy::Private)
            .map_err(Fail::from)
            .and_then(|e| {
                ensure(e.book.id == HAIL_MARY, "the entry is the book's")?;
                ensure(e.status == ReadingStatus::WantToRead, "on Want to read")?;
                ensure(e.privacy == Some(Privacy::Private), "private")?;
                ensure(e.user_id == me.id, "the account's")?;
                Ok(e)
            })
    }) else {
        s.skip(S, "the rest of the library", "the book couldn't be added");
        return;
    };
    let id = added.user_book_id;

    s.record(S, "sync the library", {
        settle(|| {
            let sync = c.library(me.id)?;
            let Coverage::Complete { unreadable } = &sync.coverage else {
                return Ok(Err("the sync paged to the end".into()));
            };
            if !unreadable.is_empty() {
                return Ok(Err(format!("unreadable rows: {unreadable:?}")));
            }
            Ok(sync
                .entries
                .iter()
                .any(|e| e.user_book_id == id)
                .then_some(())
                .ok_or_else(|| "the new entry is in the sync".to_owned()))
        })
    });
    let audio = s.record(S, "read an entry back", {
        c.library_entry(id).map_err(Fail::from).and_then(|e| {
            let e = e.ok_or("the entry is found by id")?;
            ensure(
                e.book.cover.is_some() && !e.book.authors.is_empty(),
                "with its book",
            )?;
            ensure(
                !e.series.is_empty() || e.book_audio_edition.is_some(),
                "with series or audio",
            )?;
            let by_book = c.library_entry_for_book(me.id, HAIL_MARY)?;
            ensure(by_book.is_some_and(|b| b.user_book_id == id), "and by book")?;
            e.book_audio_edition
                .ok_or_else(|| Fail::from("the book names its audiobook edition"))
        })
    });
    s.record(S, "adding a book again keeps its entry", {
        c.add_to_library_with_privacy(HAIL_MARY, ReadingStatus::Ignored, Privacy::Private)
            .map_err(Fail::from)
            .and_then(|e| {
                ensure(e.user_book_id == id, "the same entry")?;
                ensure(e.status == ReadingStatus::Ignored, "with the new status")?;
                c.set_status(id, ReadingStatus::WantToRead)?;
                Ok(())
            })
    });
    s.record(S, "start reading: a read from today", {
        c.set_status(id, ReadingStatus::CurrentlyReading)
            .map_err(Fail::from)
            .and_then(|e| {
                ensure(
                    e.status == ReadingStatus::CurrentlyReading,
                    "Currently reading",
                )?;
                settle(|| {
                    let e = c.library_entry(id)?.ok_or("the entry")?;
                    Ok(match e.open_read() {
                        Some(r) if r.started_at == Some(today) => Ok(()),
                        r => Err(format!("an open read started today: {r:?}")),
                    })
                })
            })
    });
    s.record(S, "progress in pages", {
        c.update_progress(id, None, ProgressUpdate::Pages(100))
            .map_err(Fail::from)
            .and_then(|change| {
                if let Some(e) = change.progress_left_out {
                    return Err(Fail::Api(e));
                }
                let read = change.entry.open_read().ok_or("an open read")?;
                ensure(read.progress_pages == Some(100), "at page 100")
            })
    });
    s.record(S, "a rating keeps the status", {
        let four = Rating::from_api(4.0).expect("4 is a rating");
        c.set_rating(id, Some(four), ReadingStatus::CurrentlyReading)
            .map_err(Fail::from)
            .and_then(|e| {
                ensure(e.rating == Some(four), "rated 4")?;
                ensure(e.status == ReadingStatus::CurrentlyReading, "still reading")?;
                let e = c.set_rating(id, None, ReadingStatus::CurrentlyReading)?;
                ensure(e.rating.is_none(), "the rating clears")?;
                ensure(e.status == ReadingStatus::CurrentlyReading, "still reading")
            })
    });
    s.record(S, "write and delete a review", {
        c.set_review(id, REVIEW_TEXT, true, None, ReadingStatus::CurrentlyReading)
            .map_err(Fail::from)
            .and_then(|written| {
                ensure(
                    written.review.markdown.as_deref() == Some(REVIEW_TEXT),
                    "the review is saved as Markdown",
                )?;
                ensure(written.review.has_spoilers, "with its spoiler flag")?;
                if let Some(e) = &written.entry {
                    ensure(
                        e.status == ReadingStatus::CurrentlyReading,
                        "the status stays",
                    )?;
                }
                let read = c.review(id)?;
                ensure(read.has_review, "it reads back")?;
                ensure(
                    read.plain
                        .as_deref()
                        .is_some_and(|t| t.contains("weekly API check")),
                    "with its plain text (review_raw)",
                )?;
                c.delete_review(id, ReadingStatus::CurrentlyReading)?;
                settle(|| {
                    let r = c.review(id)?;
                    Ok((!r.has_review && r.markdown.is_none())
                        .then_some(())
                        .ok_or_else(|| format!("the review is gone: {r:?}")))
                })?;
                let e = c.library_entry(id)?.ok_or("the entry")?;
                ensure(
                    e.status == ReadingStatus::CurrentlyReading,
                    "deleting it keeps the status",
                )
            })
    });
    s.record(S, "private notes", {
        c.set_private_notes(id, "API check note")
            .map_err(Fail::from)
            .and_then(|e| {
                ensure(
                    e.private_notes.as_deref() == Some("API check note"),
                    "saved",
                )?;
                let e = c.set_private_notes(id, "")?;
                ensure(
                    e.private_notes.as_deref().is_none_or(str::is_empty),
                    "blank clears them",
                )
            })
    });
    s.record(S, "privacy", {
        c.set_privacy(id, Privacy::FollowersOnly)
            .map_err(Fail::from)
            .and_then(|e| {
                ensure(e.privacy == Some(Privacy::FollowersOnly), "followers only")?;
                let e = c.set_privacy(id, Privacy::Private)?;
                ensure(e.privacy == Some(Privacy::Private), "back to private")
            })
    });
    if let Some(audio) = audio {
        s.record(S, "move the read to the audiobook edition", {
            c.set_edition(id, audio)
                .map_err(Fail::from)
                .and_then(|change| {
                    if let Some(e) = change.read_left_behind {
                        return Err(Fail::Api(e));
                    }
                    let e = change.entry;
                    let edition = e.edition.as_ref().ok_or("the entry has the edition")?;
                    ensure(edition.id == audio, "the audiobook edition")?;
                    ensure(
                        edition.reading_format_id == Some(hardcover_core::AUDIOBOOK_FORMAT_ID),
                        "which is an audiobook (reading_format_id 2)",
                    )?;
                    ensure(edition.audio_seconds.is_some(), "with its running time")?;
                    ensure(
                        e.measure().is_some_and(|m| m.unit == ProgressUnit::Time),
                        "progress now counts time",
                    )?;
                    let change = c.update_progress(id, None, ProgressUpdate::Seconds(3_600))?;
                    if let Some(e) = change.progress_left_out {
                        return Err(Fail::Api(e));
                    }
                    let read = change.entry.open_read().ok_or("an open read")?;
                    ensure(read.progress_seconds == Some(3_600), "at one hour")
                })
        });
    } else {
        s.skip(
            S,
            "move the read to the audiobook edition",
            "no audiobook edition",
        );
    }
    s.record(S, "a past read: add, date, delete", {
        let start = Date::from_ymd_opt(2020, 1, 1).expect("a date");
        let end = Date::from_ymd_opt(2020, 2, 1).expect("a date");
        let new_end = Date::from_ymd_opt(2020, 2, 2).expect("a date");
        c.add_read(id, Some(start), end, None)
            .map_err(Fail::from)
            .and_then(|e| {
                let past = e
                    .reads
                    .iter()
                    .find(|r| r.finished_at == Some(end))
                    .ok_or("the past read is listed")?;
                ensure(past.started_at == Some(start), "with its start")?;
                ensure(
                    e.open_read().is_some(),
                    "the open read stays the current one",
                )?;
                let e = c.set_read_dates(past.id, Some(start), Some(new_end))?;
                ensure(
                    e.read(past.id)
                        .is_some_and(|r| r.finished_at == Some(new_end)),
                    "its dates change",
                )?;
                let e = c.delete_read(id, past.id)?;
                ensure(e.read(past.id).is_none(), "and it goes")
            })
    });
    s.record(S, "finish: the read ends today", {
        c.set_status(id, ReadingStatus::Read)
            .map_err(Fail::from)
            .and_then(|e| {
                ensure(e.status == ReadingStatus::Read, "Read")?;
                settle(|| {
                    let e = c.library_entry(id)?.ok_or("the entry")?;
                    Ok(match e.current_read() {
                        Some(r) if r.finished_at == Some(today) => Ok(()),
                        r => Err(format!("the read finished today: {r:?}")),
                    })
                })
            })
    });
    s.record(S, "remove the book", {
        c.remove_from_library(id)
            .map_err(Fail::from)
            .and_then(|()| {
                settle(|| {
                    Ok(c.library_entry(id)?
                        .is_none()
                        .then_some(())
                        .ok_or_else(|| "the entry is gone".to_owned()))
                })
            })
    });
}

fn lists(c: &Client, s: &mut Suite) {
    const S: &str = "lists";
    let Some(list) = s.record(S, "create a ranked list", {
        let draft = ListDraft {
            name: LIST_NAME.into(),
            description: "Made by the weekly API check.".into(),
            privacy: Some(Privacy::Private),
            ranked: true,
        };
        c.create_list(&draft).map_err(Fail::from).and_then(|l| {
            ensure(l.name == LIST_NAME, "with its name")?;
            ensure(l.ranked, "ranked")?;
            ensure(l.privacy() == Some(Privacy::Private), "private")?;
            Ok(l.id)
        })
    }) else {
        s.skip(S, "the rest of the lists", "the list couldn't be made");
        owned(c, s);
        return;
    };
    s.record(S, "the list is read back", {
        settle(|| {
            Ok(c.lists()?
                .iter()
                .any(|l| l.id == list)
                .then_some(())
                .ok_or_else(|| "the new list is listed".to_owned()))
        })
    });
    s.record(S, "edit its description only", {
        let mut edit = ListEdit::default();
        edit.description = Some("Edited by the weekly API check.".into());
        c.update_list(list, &edit)
            .map_err(Fail::from)
            .and_then(|l| {
                ensure(
                    l.description.as_deref() == Some("Edited by the weekly API check."),
                    "the description changes",
                )?;
                ensure(l.name == LIST_NAME && l.ranked, "the rest stays")
            })
    });
    let rows = s.record(S, "add books, ranked", {
        c.add_to_list(list, DUNE, None, Some(1))
            .map_err(Fail::from)
            .and_then(|first| {
                let again = c.add_to_list(list, DUNE, None, Some(1))?;
                ensure(again.id == first.id, "adding it again returns the same row")?;
                let second = c.add_to_list(list, WAY_OF_KINGS, None, Some(2))?;
                Ok((first.id, second.id))
            })
    });
    if let Some((dune_row, _)) = rows {
        s.record(S, "its books, in order", {
            settle(|| {
                let page = c.list_books(list, 0)?;
                let ids: Vec<BookId> = page.items.iter().map(|b| b.book.id).collect();
                Ok((ids == [DUNE, WAY_OF_KINGS])
                    .then_some(())
                    .ok_or_else(|| format!("Dune, then The Way of Kings: got {ids:?}")))
            })
        });
        s.record(S, "memberships", {
            c.list_memberships().map_err(Fail::from).and_then(|m| {
                ensure(m.complete, "the index is complete")?;
                ensure(
                    m.rows
                        .iter()
                        .any(|r| r.list_id == list && r.book_id == DUNE),
                    "Dune is on the list",
                )?;
                let book = c.book_list_memberships(DUNE)?;
                ensure(
                    book.iter().any(|r| r.list_id == list),
                    "and the book's own lookup says so",
                )
            })
        });
        s.record(S, "take a book off", {
            c.remove_from_list(dune_row)
                .map_err(Fail::from)
                .and_then(|()| {
                    settle(|| {
                        let page = c.list_books(list, 0)?;
                        let ids: Vec<BookId> = page.items.iter().map(|b| b.book.id).collect();
                        Ok((ids == [WAY_OF_KINGS])
                            .then_some(())
                            .ok_or_else(|| format!("only The Way of Kings left: got {ids:?}")))
                    })
                })
        });
    } else {
        s.skip(S, "the list's books", "the books couldn't be added");
    }
    s.record(S, "an empty description clears it", {
        let mut edit = ListEdit::default();
        edit.description = Some(String::new());
        c.update_list(list, &edit)
            .map_err(Fail::from)
            .and_then(|l| {
                ensure(
                    l.description.as_deref().is_none_or(str::is_empty),
                    "the description is gone",
                )
            })
    });
    s.record(S, "delete the list", {
        c.delete_list(list).map_err(Fail::from).and_then(|()| {
            settle(|| {
                Ok((!c.lists()?.iter().any(|l| l.id == list))
                    .then_some(())
                    .ok_or_else(|| "the list is gone".to_owned()))
            })
        })
    });
    owned(c, s);
}

fn owned(c: &Client, s: &mut Suite) {
    const S: &str = "lists";
    let Some(edition) = s.owned_edition else {
        s.skip(S, "the Owned list", "no edition to put on it");
        return;
    };
    s.record(S, "the Owned list: on and off", {
        c.mark_owned(edition).map_err(Fail::from).and_then(|_| {
            let (owned, row) = settle(|| {
                let lists = c.lists()?;
                let Some(owned) = lists
                    .iter()
                    .find(|l| l.slug.as_deref() == Some(OWNED_LIST_SLUG))
                else {
                    return Ok(Err("the Owned list exists".to_owned()));
                };
                let page = c.list_books(owned.id, 0)?;
                Ok(page
                    .items
                    .iter()
                    .find(|b| b.edition_id == Some(edition))
                    .map(|b| (owned.id, b.id))
                    .ok_or_else(|| "the edition is on it".to_owned()))
            })?;
            c.remove_from_list(row)?;
            settle(|| {
                let page = c.list_books(owned, 0)?;
                Ok((!page.items.iter().any(|b| b.edition_id == Some(edition)))
                    .then_some(())
                    .ok_or_else(|| "the edition is off it".to_owned()))
            })
        })
    });
}

fn goals(c: &Client, s: &mut Suite) {
    const S: &str = "goals";
    let year = today().year();
    let Some(goal) = s.record(S, "create a goal", {
        let new = NewGoal {
            metric: GoalMetric::Books,
            target: 12,
            start: Date::from_ymd_opt(year, 1, 1).expect("a date"),
            end: Date::from_ymd_opt(year, 12, 31).expect("a date"),
            description: GOAL_DESCRIPTION.into(),
        };
        c.create_goal(&new).map_err(Fail::from).and_then(|g| {
            ensure(g.metric == GoalMetric::Books && g.target == 12, "12 books")?;
            Ok(g.id)
        })
    }) else {
        s.skip(S, "the rest of the goals", "the goal couldn't be made");
        return;
    };
    s.record(S, "the goal is read back", {
        settle(|| {
            Ok(match c.goals()?.iter().find(|g| g.id == goal) {
                Some(g) if g.description.as_deref() == Some(GOAL_DESCRIPTION) => Ok(()),
                g => Err(format!("the goal with its description: {g:?}")),
            })
        })
    });
    s.record(S, "change its target", {
        c.update_goal(goal, |g| g.target = 13)
            .map_err(Fail::from)
            .and_then(|g| {
                ensure(g.target == 13, "13 books")?;
                ensure(
                    g.description.as_deref() == Some(GOAL_DESCRIPTION),
                    "the description stays",
                )
            })
    });
    s.record(S, "delete the goal", {
        c.delete_goal(goal).map_err(Fail::from).and_then(|()| {
            settle(|| {
                Ok((!c.goals()?.iter().any(|g| g.id == goal))
                    .then_some(())
                    .ok_or_else(|| "the goal is gone".to_owned()))
            })
        })
    });
}

fn follows(c: &Client, s: &mut Suite) {
    const S: &str = "follows";
    let series = c
        .book_details(DUNE)
        .ok()
        .and_then(|d| d.series.iter().find(|r| r.name.contains("Dune"))?.id);
    let Some(series) = series else {
        s.skip(S, "follow a series", "Dune's series id is unknown");
        return;
    };
    s.record(S, "follow a series, twice", {
        c.follow_series(series).map_err(Fail::from).and_then(|f| {
            ensure(f.id == series, "the series")?;
            let again = c.follow_series(series)?;
            ensure(again.id == series, "following again is harmless")
        })
    });
    s.record(S, "the follow is read back", {
        settle(|| {
            Ok(match c.followed_series()?.iter().find(|f| f.id == series) {
                Some(f) if f.followed_at.is_some() => Ok(()),
                f => Err(format!("followed, with its time: {f:?}")),
            })
        })
    });
    s.record(S, "unfollow it", {
        c.unfollow_series(&[series])
            .map_err(Fail::from)
            .and_then(|()| {
                settle(|| {
                    Ok((!c.followed_series()?.iter().any(|f| f.id == series))
                        .then_some(())
                        .ok_or_else(|| "the follow is gone".to_owned()))
                })
            })
            .and_then(|()| {
                c.unfollow_series(&[series])
                    .map_err(|e| Fail::Assert(format!("unfollowing again should succeed: {e}")))
            })
    });
}

// ----- the account's state -----

/// What the test account holds: what [`reset`] clears.
struct Baseline {
    entries: Vec<UserBookId>,
    lists: Vec<ListId>,
    owned: Vec<hardcover_core::ListBookId>,
    goals: Vec<GoalId>,
    follows: Vec<SeriesId>,
}

impl Baseline {
    fn read(c: &Client, me: &User) -> Result<Self, Fail> {
        let sync = c.library(me.id)?;
        let mut entries: Vec<UserBookId> = sync.entries.iter().map(|e| e.user_book_id).collect();
        if let Coverage::Complete { unreadable } = sync.coverage {
            entries.extend(unreadable);
        }
        let mut lists = Vec::new();
        let mut owned = Vec::new();
        for list in c.lists()? {
            if list.slug.as_deref() == Some(OWNED_LIST_SLUG) {
                if list.books_count > MAX_OWNED_BOOKS {
                    return Err(Fail::Assert(format!(
                        "the Owned list holds {} books",
                        list.books_count
                    )));
                }
                owned.extend(c.list_books(list.id, 0)?.items.iter().map(|b| b.id));
            } else {
                lists.push(list.id);
            }
        }
        let goals = c.goals()?.iter().map(|g| g.id).collect();
        let follows = c
            .followed_series()?
            .into_iter()
            .flat_map(|f| f.followed_as)
            .collect();
        Ok(Self {
            entries,
            lists,
            owned,
            goals,
            follows,
        })
    }

    /// Refuses an account holding more than a run could have left.
    fn guard(&self) -> Result<(), String> {
        let over = self.entries.len() > MAX_LEFTOVER_ENTRIES
            || self.lists.len() > MAX_LEFTOVER_LISTS
            || self.goals.len() > MAX_LEFTOVER_GOALS
            || self.follows.len() > MAX_LEFTOVER_FOLLOWS;
        if over {
            return Err(format!(
                "the account holds more than a run of this suite leaves ({}); \
                 it only runs on an empty test account",
                self.describe()
            ));
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.lists.is_empty()
            && self.owned.is_empty()
            && self.goals.is_empty()
            && self.follows.is_empty()
    }

    fn describe(&self) -> String {
        format!(
            "{} library entries, {} lists, {} books on Owned, {} goals, {} followed series",
            self.entries.len(),
            self.lists.len(),
            self.owned.len(),
            self.goals.len(),
            self.follows.len()
        )
    }
}

/// Clears the test account: every entry, list, Owned row, goal and follow.
fn reset(c: &Client, me: &User) -> Result<(), Fail> {
    let b = Baseline::read(c, me)?;
    b.guard().map_err(Fail::Assert)?;
    for id in &b.entries {
        c.remove_from_library(*id)?;
    }
    for id in &b.lists {
        c.delete_list(*id)?;
    }
    for row in &b.owned {
        c.remove_from_list(*row)?;
    }
    for id in &b.goals {
        c.delete_goal(*id)?;
    }
    if !b.follows.is_empty() {
        c.unfollow_series(&b.follows)?;
    }
    Ok(())
}

// ----- the harness -----

/// Why a check failed.
#[derive(Debug)]
enum Fail {
    /// A call failed.
    Api(Error),
    /// A call answered, but not as the app expects.
    Assert(String),
}

impl From<Error> for Fail {
    fn from(e: Error) -> Self {
        Self::Api(e)
    }
}

impl From<&str> for Fail {
    fn from(s: &str) -> Self {
        Self::Assert(s.to_owned())
    }
}

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Self::Assert(s)
    }
}

fn ensure(ok: bool, what: impl Into<String>) -> Result<(), Fail> {
    if ok {
        Ok(())
    } else {
        Err(Fail::Assert(format!("expected: {}", what.into())))
    }
}

/// Tries `probe` until it is satisfied (`Ok(Ok(v))`) or [`SETTLE`] has
/// passed; a call that fails ends it at once. `Ok(Err(why))` is "not yet".
fn settle<T>(mut probe: impl FnMut() -> Result<Result<T, String>, Fail>) -> Result<T, Fail> {
    let start = Instant::now();
    loop {
        match probe()? {
            Ok(v) => return Ok(v),
            Err(why) if start.elapsed() >= SETTLE => {
                return Err(Fail::Assert(format!(
                    "expected within {}s: {why}",
                    SETTLE.as_secs()
                )));
            }
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    }
}

fn today() -> Date {
    chrono::Local::now().date_naive()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Passed,
    Skipped,
    /// The network failed: inconclusive.
    Network,
    /// A check failed: the app is broken against the API.
    Broken,
    /// The token was rejected.
    Token,
}

struct Check {
    section: &'static str,
    name: String,
    kind: Kind,
    detail: Option<String>,
    took: Duration,
}

#[derive(Default)]
struct Suite {
    checks: Vec<Check>,
    refused: Option<String>,
    owned_edition: Option<EditionId>,
    last: Option<Instant>,
}

impl Suite {
    /// Records a check's result and returns its value if it passed. The
    /// check's time runs from the previous record (the body has already run).
    fn record<T>(
        &mut self,
        section: &'static str,
        name: &str,
        result: Result<T, Fail>,
    ) -> Option<T> {
        let now = Instant::now();
        let took = self.last.map_or(Duration::ZERO, |t| now - t);
        self.last = Some(now);
        let (kind, detail, value) = match result {
            Ok(v) => (Kind::Passed, None, Some(v)),
            Err(Fail::Assert(why)) => (Kind::Broken, Some(why), None),
            Err(Fail::Api(e)) => (kind_of(&e), Some(format!("{e} ({e:?})")), None),
        };
        let mark = match kind {
            Kind::Passed => "ok  ",
            Kind::Skipped => "skip",
            Kind::Network => "NET ",
            Kind::Broken => "FAIL",
            Kind::Token => "AUTH",
        };
        match &detail {
            Some(d) => eprintln!("{mark} {section}: {name}: {d}"),
            None => eprintln!("{mark} {section}: {name} ({:.1}s)", took.as_secs_f32()),
        }
        self.checks.push(Check {
            section,
            name: name.to_owned(),
            kind,
            detail,
            took,
        });
        value
    }

    fn skip(&mut self, section: &'static str, name: &str, why: &str) {
        eprintln!("skip {section}: {name}: {why}");
        self.checks.push(Check {
            section,
            name: name.to_owned(),
            kind: Kind::Skipped,
            detail: Some(why.to_owned()),
            took: Duration::ZERO,
        });
    }

    fn worst(&self) -> Kind {
        self.checks
            .iter()
            .map(|c| c.kind)
            .max()
            .unwrap_or(Kind::Passed)
    }

    fn exit_code(&self) -> u8 {
        match self.worst() {
            Kind::Passed | Kind::Skipped => 0,
            Kind::Broken => 1,
            Kind::Network => 3,
            Kind::Token => 4,
        }
    }

    fn report(&self) -> String {
        let mut out = String::from("\n## Live API suite\n\n");
        if let Some(why) = &self.refused {
            let _ = writeln!(out, "**Refused:** {why}\n");
            return out;
        }
        let count = |k: Kind| self.checks.iter().filter(|c| c.kind == k).count();
        let outcome = match self.worst() {
            Kind::Passed | Kind::Skipped => "every check passed",
            Kind::Broken => "**broken**: the app no longer matches the live API",
            Kind::Network => "**inconclusive**: only the network failed",
            Kind::Token => "**the token was rejected**: replace the test account's token",
        };
        let _ = writeln!(
            out,
            "{outcome}. {} passed, {} failed, {} network failures, {} skipped.\n",
            count(Kind::Passed),
            count(Kind::Broken) + count(Kind::Token),
            count(Kind::Network),
            count(Kind::Skipped)
        );
        let problems: Vec<&Check> = self
            .checks
            .iter()
            .filter(|c| c.kind != Kind::Passed)
            .collect();
        if !problems.is_empty() {
            out.push_str("| | Section | Check | Why |\n|---|---|---|---|\n");
            for c in problems {
                let mark = match c.kind {
                    Kind::Broken => "❌",
                    Kind::Token => "🔑",
                    Kind::Network => "🌐",
                    Kind::Passed | Kind::Skipped => "⏭",
                };
                let why = c.detail.as_deref().unwrap_or("").replace('|', "\\|");
                let _ = writeln!(out, "| {mark} | {} | {} | {why} |", c.section, c.name);
            }
            out.push('\n');
        }
        let total: Duration = self.checks.iter().map(|c| c.took).sum();
        let _ = writeln!(
            out,
            "{} checks in {:.0}s.",
            self.checks.len(),
            total.as_secs_f32()
        );
        out
    }
}

fn kind_of(e: &Error) -> Kind {
    match e {
        Error::Unauthorized => Kind::Token,
        Error::RateLimited { .. } | Error::Timeout | Error::OutcomeUnknown | Error::Network(_) => {
            Kind::Network
        }
        // A personal token holds every scope, so a refused scope is a change
        // on Hardcover's side, like any other API error.
        _ => Kind::Broken,
    }
}
