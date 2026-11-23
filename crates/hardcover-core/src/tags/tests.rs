//! The rule against `fixtures/expected-lines.json`, worked out independently
//! from Hardcover's public catalogue counts in
//! `fixtures/catalogue-tag-counts.json` (read live, read-only, 2026-11-23).
//! Both are public catalogue data, no user's.

use serde_json::Value;

use super::*;

pub(crate) const CATALOGUE_COUNTS: &str = include_str!("fixtures/catalogue-tag-counts.json");
const EXPECTED: &str = include_str!("fixtures/expected-lines.json");

/// A book of the fixture.
pub(crate) struct FixtureBook {
    pub id: i64,
    pub title: String,
    pub votes: BookVotes,
    /// `cached_tags`' genres as `(name, votes)`.
    pub genres: Vec<(String, u32)>,
}

/// Every fixture book.
pub(crate) fn fixture_books() -> Vec<FixtureBook> {
    let data: Value = serde_json::from_str(CATALOGUE_COUNTS).unwrap();
    data["books"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let votes =
                BookVotes::from_counts(b["counts"].as_object().unwrap().iter().map(|(tag, n)| {
                    (
                        TagId::new(tag.parse().unwrap()),
                        u32::try_from(n.as_u64().unwrap()).unwrap(),
                    )
                }));
            let genres = b["genres"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| {
                    (
                        g["tag"].as_str().unwrap().to_owned(),
                        u32::try_from(g["count"].as_u64().unwrap()).unwrap(),
                    )
                })
                .collect();
            FixtureBook {
                id: b["id"].as_i64().unwrap(),
                title: b["title"].as_str().unwrap().to_owned(),
                votes,
                genres,
            }
        })
        .collect()
}

fn fixture(id: i64) -> BookVotes {
    fixture_books()
        .into_iter()
        .find(|b| b.id == id)
        .unwrap_or_else(|| panic!("no fixture book {id}"))
        .votes
}

fn slugs(moods: &[Mood]) -> Vec<&'static str> {
    moods.iter().map(|m| m.slug()).collect()
}

fn votes(pairs: &[(TagId, u32)]) -> BookVotes {
    BookVotes::from_counts(pairs.iter().copied())
}

const EDUCATED: i64 = 44_186;
const IT: i64 = 373_525;
const EDGEDANCER: i64 = 427_544;
const LORD_OF_THE_RINGS: i64 = 377_938;
const ULYSSES: i64 = 427_513;

#[test]
fn the_rule_gives_the_expected_line_for_every_fixture_book() {
    let expected: Value = serde_json::from_str(EXPECTED).unwrap();
    let expected = expected["expected"].as_array().unwrap();
    let books = fixture_books();
    assert_eq!(books.len(), 36);
    assert_eq!(expected.len(), books.len());
    for want in expected {
        let id = want["id"].as_i64().unwrap();
        let FixtureBook { title, votes, .. } = books.iter().find(|b| b.id == id).unwrap();
        assert_eq!(
            u64::from(votes.mood_votes()),
            want["mood_votes"].as_u64().unwrap(),
            "{title}: mood votes"
        );
        let line = mood_line(votes);
        match want["line"].as_array() {
            None => assert_eq!(line, None, "{title}"),
            Some(moods) => {
                let line = line.unwrap_or_else(|| panic!("{title}: no line"));
                let want_moods: Vec<&str> = moods.iter().map(|m| m.as_str().unwrap()).collect();
                assert_eq!(slugs(&line.moods), want_moods, "{title}: moods");
                assert_eq!(
                    line.fallback,
                    want["fallback"].as_bool().unwrap(),
                    "{title}: fallback"
                );
                assert_eq!(
                    line.pace.map(|p| p.name().to_lowercase()),
                    want["pace"].as_str().map(str::to_owned),
                    "{title}: pace"
                );
                for mood in Mood::ALL {
                    assert_eq!(
                        has_mood(votes, mood),
                        line.moods.contains(&mood),
                        "{title}: {mood:?}"
                    );
                }
            }
        }
        let want_rarely: Vec<&str> = want["rarely"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap())
            .collect();
        assert_eq!(slugs(&rarely(votes)), want_rarely, "{title}: rarely");
        assert_eq!(slugs(&breakdown(votes).rarely), want_rarely, "{title}");
    }
}

#[test]
fn no_mood_stands_out_so_the_line_names_the_most_voted() {
    // Each mood a thousandth of its site votes: every lift is about 1.
    let even = BookVotes::from_counts(
        Mood::ALL
            .into_iter()
            .map(|m| (m.id(), m.site_votes() / 1_000)),
    );
    let line = mood_line(&even).unwrap();
    assert!(line.fallback);
    // Adventurous and Emotional both have 75: the lower id first.
    assert_eq!(line.moods, [Mood::Adventurous, Mood::Emotional, Mood::Dark]);
    assert_eq!(line.pace, None);
    assert!(rarely(&even).is_empty());
}

#[test]
fn the_line_needs_fifty_mood_votes() {
    let ulysses = fixture(ULYSSES);
    assert_eq!(ulysses.mood_votes(), 14);
    assert_eq!(mood_line(&ulysses), None);
    assert!(!has_mood(&ulysses, Mood::Challenging));
    assert!(rarely(&ulysses).is_empty());

    let at = |n| votes(&[(Mood::Relaxing.id(), n)]);
    assert_eq!(mood_line(&at(49)), None);
    let line = mood_line(&at(50)).unwrap();
    assert_eq!(line.moods, [Mood::Relaxing]);
    assert!(!line.fallback);
}

#[test]
fn a_standout_needs_both_the_share_and_the_lift() {
    // 9 of 100 votes for Relaxing: a lift of about 9, but under 10%.
    let mut pairs = vec![(Mood::Relaxing.id(), 9), (Mood::Adventurous.id(), 91)];
    let line = mood_line(&votes(&pairs)).unwrap();
    assert!(!line.moods.contains(&Mood::Relaxing));
    pairs[0].1 = 10;
    pairs[1].1 = 90;
    let line = mood_line(&votes(&pairs)).unwrap();
    // Relaxing: 10% of the votes, 1.7% of the site's, a lift of about 5.8.
    // Adventurous: 90% against 12.5%, a lift of about 7.2, so it comes first.
    assert_eq!(line.moods, [Mood::Adventurous, Mood::Relaxing]);
}

#[test]
fn the_pace_is_named_only_when_readers_agree() {
    // Edgedancer: 3 slow, 19 medium, 48 fast.
    assert_eq!(
        mood_line(&fixture(EDGEDANCER)).unwrap().pace,
        Some(Pace::Fast)
    );
    // It: 15 slow of 28, just over half.
    assert_eq!(mood_line(&fixture(IT)).unwrap().pace, Some(Pace::Slow));
    // The Lord of the Rings: 12 slow, but only 15 pace votes.
    assert_eq!(mood_line(&fixture(LORD_OF_THE_RINGS)).unwrap().pace, None);

    let base = (Mood::Relaxing.id(), 60);
    let pace = |slow, medium, fast| {
        mood_line(&votes(&[
            base,
            (Pace::Slow.id(), slow),
            (Pace::Medium.id(), medium),
            (Pace::Fast.id(), fast),
        ]))
        .unwrap()
        .pace
    };
    assert_eq!(pace(10, 0, 9), None, "19 pace votes");
    assert_eq!(
        pace(10, 0, 10),
        Some(Pace::Fast),
        "a tie goes to the lower id, Fast (188)"
    );
    assert_eq!(pace(11, 0, 10), Some(Pace::Slow));
    assert_eq!(pace(10, 9, 1), Some(Pace::Slow), "exactly half");
    assert_eq!(pace(10, 8, 3), None, "leads with under half: 10 * 2 < 21");
    assert_eq!(
        pace(10, 10, 0),
        None,
        "a tie goes to the lower id, Medium (182)"
    );
    assert_eq!(pace(0, 30, 0), None, "Medium is never named");
}

#[test]
fn the_breakdown_lists_what_readers_voted() {
    // Educated (hand-counted from the fixture): 279 mood votes, Adventurous
    // has 4 (under 5); Dark and Inspiring both 33, Dark's id is lower.
    let b = breakdown(&fixture(EDUCATED));
    assert_eq!(b.mood_votes, 279);
    assert_eq!(
        b.moods,
        [
            (Mood::Emotional, 53),
            (Mood::Reflective, 42),
            (Mood::Dark, 33),
            (Mood::Inspiring, 33),
            (Mood::Sad, 31),
            (Mood::Challenging, 30),
            (Mood::Tense, 22),
            (Mood::Hopeful, 20),
            (Mood::Informative, 11),
        ]
    );
    let pace = b.pace.unwrap();
    assert_eq!(
        pace.parts,
        [(Pace::Slow, 9), (Pace::Medium, 43), (Pace::Fast, 10)]
    );
    assert_eq!(pace.total, 62);
    assert_eq!(pace.leader, Pace::Medium);
    assert_eq!(b.drive, None, "no drive votes");
    assert!(b.characters.is_empty());

    // Edgedancer: drive 11 plot, 20 mix, 28 character; Loveable 49 of 62
    // and Strong 45 of 61 are past two thirds, Diverse 30 of 59 isn't.
    let b = breakdown(&fixture(EDGEDANCER));
    let drive = b.drive.unwrap();
    assert_eq!(
        drive.parts,
        [(Drive::Plot, 11), (Drive::Mix, 20), (Drive::Character, 28)]
    );
    assert_eq!(drive.leader, Drive::Character);
    assert_eq!(
        b.characters,
        [
            CharacterLean {
                side: CharacterTrait::Loveable,
                votes: 49,
                pair_votes: 62
            },
            CharacterLean {
                side: CharacterTrait::StrongDevelopment,
                votes: 45,
                pair_votes: 61
            },
        ]
    );

    // It: Loveable 15 of 23 is just under two thirds; Diverse 16 of 24 is
    // exactly two thirds; Strong 19 of 23 is past it.
    let b = breakdown(&fixture(IT));
    let sides: Vec<CharacterTrait> = b.characters.iter().map(|c| c.side).collect();
    assert_eq!(
        sides,
        [CharacterTrait::StrongDevelopment, CharacterTrait::Diverse]
    );

    // The Lord of the Rings: 13 Loveable and none Unloveable, but 13 votes
    // in the pair; 15 pace votes, 13 drive votes.
    let b = breakdown(&fixture(LORD_OF_THE_RINGS));
    assert!(b.characters.is_empty());
    assert_eq!(b.pace, None);
    assert_eq!(b.drive, None);

    // The negative side counts as well.
    let b = breakdown(&votes(&[
        (CharacterTrait::Loveable.id(), 5),
        (CharacterTrait::Unloveable.id(), 15),
    ]));
    assert_eq!(b.characters[0].side, CharacterTrait::Unloveable);
}

#[test]
fn votes_keep_only_the_known_tags() {
    let v = votes(&[
        (TagId::new(2), 40),
        (Mood::Dark.id(), 3),
        (Mood::Dark.id(), 4),
        (Pace::Fast.id(), 0),
    ]);
    assert_eq!(v.get(TagId::new(2)), 0, "a genre isn't a vote");
    assert_eq!(v.mood(Mood::Dark), 7);
    assert_eq!(v.mood_votes(), 7);
    assert!(BookVotes::default().is_empty());
    // A round trip through JSON, as a cache would keep them.
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(serde_json::from_str::<BookVotes>(&json).unwrap(), v);
}

#[test]
fn the_tags_are_hardcovers() {
    let ids: Vec<i64> = MOOD_IDS.iter().map(|t| t.get()).collect();
    assert_eq!(
        ids,
        [
            154, 191, 148, 207, 206, 180, 192, 181, 193, 187, 190, 240, 203, 241
        ]
    );
    let mut all: Vec<i64> = VOTE_TAG_IDS.iter().map(|t| t.get()).collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 26, "no id twice");
    for mood in Mood::ALL {
        assert_eq!(Mood::from_id(mood.id()), Some(mood));
        assert_eq!(Mood::from_slug(mood.slug()), Some(mood));
        assert_eq!(mood.slug(), mood.name().to_lowercase());
    }
    // The site totals agree with the fixture's.
    let data: Value = serde_json::from_str(CATALOGUE_COUNTS).unwrap();
    for mood in Mood::ALL {
        assert_eq!(
            data["site_mood_totals"][mood.id().get().to_string()].as_u64(),
            Some(u64::from(mood.site_votes())),
            "{mood:?}"
        );
    }
    // Every count in the fixture is for one of the tags asked for.
    for book in fixture_books() {
        assert!(!book.votes.is_empty(), "{}", book.title);
    }
    for b in data["books"].as_array().unwrap() {
        for tag in b["counts"].as_object().unwrap().keys() {
            let tag = TagId::new(tag.parse().unwrap());
            assert!(VOTE_TAG_IDS.contains(&tag), "{tag}");
        }
    }
    assert_eq!(Pace::Fast.id().get(), 188);
    assert_eq!(Drive::Mix.id().get(), 183);
    for pair in TraitPair::ALL {
        for side in pair.sides() {
            assert_eq!(side.pair(), pair);
        }
    }
}

// ----- genres -----

fn genres_of(id: i64) -> BookGenres {
    let book = fixture_books().into_iter().find(|b| b.id == id).unwrap();
    BookGenres::from_tags(book.genres)
}

fn named(genres: &BookGenres) -> Vec<(&str, u32)> {
    genres.iter().map(|g| (g.name.as_str(), g.votes)).collect()
}

#[test]
fn genre_keys_merge_spellings() {
    for (name, key) in [
        ("Science Fiction", "science fiction"),
        ("science-fiction", "science fiction"),
        ("Sci-fi", "science fiction"),
        ("Sci-Fi", "science fiction"),
        ("SciFi", "science fiction"),
        ("Sci Fi & Fantasy", "science fiction and fantasy"),
        ("Science Fiction & Fantasy", "science fiction and fantasy"),
        ("Science Fiction And Fantasy", "science fiction and fantasy"),
        ("science fiction fantasy", "science fiction fantasy"),
        ("Non-Fiction", "nonfiction"),
        ("Nonfiction", "nonfiction"),
        ("Self-Help", "self help"),
        ("Self help", "self help"),
        ("Children's", "childrens"),
        ("\"Fiction\"", "fiction"),
        ("  English", "english"),
        ("Fiction / Literary", "fiction literary"),
        ("Science", "science"),
        ("Fiction", "fiction"),
        ("داستان علمی", "داستان علمی"),
        ("!!!", ""),
    ] {
        assert_eq!(genre_key(name), key, "{name}");
    }
}

#[test]
fn a_books_genres_merge_by_key() {
    // Klara and the Sun: "Science Fiction" 9 and "Sci-fi" 3 are one genre.
    assert_eq!(
        named(&genres_of(266_607)),
        [
            ("Science Fiction", 12),
            ("Dystopian", 5),
            ("Literary Fiction", 3)
        ]
    );
    // Edgedancer: "Science Fiction & Fantasy" 2 and "Science Fiction And
    // Fantasy" 1 reach 3 together; the quoted and stray tags stay under it.
    let edgedancer = genres_of(EDGEDANCER);
    assert_eq!(
        named(&edgedancer),
        [("Fantasy", 11), ("Science Fiction & Fantasy", 3)]
    );
    assert!(edgedancer.has("science fiction and fantasy"));
    assert!(!edgedancer.has("science fiction"), "one vote");
    // Atomic Habits: "Self-Help" 10 and "Self help" 4.
    let habits = genres_of(428_023);
    assert_eq!(named(&habits)[0], ("Self-Help", 14));
    // God Emperor of Dune: "science fiction fantasy" stays apart from
    // "Science Fiction & Fantasy".
    let god_emperor = genres_of(427_363);
    let keys: Vec<&str> = god_emperor.iter().map(|g| g.key.as_str()).collect();
    assert!(keys.contains(&"science fiction and fantasy"));
    assert!(keys.contains(&"science fiction fantasy"));
    assert!(keys.contains(&"science fiction"));
}

#[test]
fn the_name_is_the_most_voted_spelling() {
    let genres = BookGenres::from_tags([("sci-fi", 2), ("Science Fiction", 5), ("SciFi", 5)]);
    assert_eq!(named(&genres), [("Science Fiction", 12)]);
    let genres = BookGenres::from_tags([("Fantasy", 2), ("Horror", 2), ("fantasy", 1)]);
    assert_eq!(named(&genres), [("Fantasy", 3)], "Horror has 2");
    // Ties between genres keep their order.
    let genres = BookGenres::from_tags([("Horror", 4), ("Fantasy", 4), ("Romance", 9)]);
    assert_eq!(
        named(&genres),
        [("Romance", 9), ("Horror", 4), ("Fantasy", 4)]
    );
    assert!(BookGenres::from_tags([("!!!", 9)]).is_empty());
}

#[test]
fn browse_genres_find_their_spellings() {
    let ids: Vec<i64> = BROWSE_GENRES.iter().map(|g| g.id.get()).collect();
    assert_eq!(
        ids,
        [2, 6, 3, 11, 7, 18, 52, 28, 26, 1, 16, 4, 38, 14, 8, 29]
    );
    assert_eq!(browse_genre("Sci-fi").map(|g| g.id.get()), Some(6));
    assert_eq!(browse_genre("Non-Fiction").map(|g| g.id.get()), Some(14));
    assert_eq!(browse_genre("fantasy").map(|g| g.name), Some("Fantasy"));
    assert_eq!(browse_genre("Science Fiction & Fantasy"), None);
}
