//! The icon on a genre's chip, chosen from the genre's name
//! (`hardcover-design`, Chips → TagChip).
//!
//! Hardcover's genres are free text with many spellings ("Science Fiction",
//! "Sci-fi", "SF"; "Children's"; "Literature & Fiction"), so a name is
//! normalised to lowercase words first: `&` reads "and", apostrophes go, and
//! any other punctuation separates words. A name that equals one of a
//! genre's names takes its icon; otherwise the first genre in [`GENRES`]
//! with a name that appears in it as whole words, which is why the more
//! specific genres come first ("Non-Fiction" before "Fiction", "Science
//! Fiction" before "Science", "Historical Fiction" before "History"). A name
//! that matches nothing (a Persian genre, "General") gets no icon.
//!
//! Every icon here is served by `crate::icons::AppAssets`: the kit's default
//! set or `icons::ExtraIcons`. A test loads each one, because an icon
//! missing from both draws nothing.

use gpui_kit::assets::IconName;

/// A genre that gets an icon: an example of its name as Hardcover writes
/// it (for the gallery), its icon and every name it goes by, normalised.
pub struct Genre {
    // Read by the gallery (demo build) and the tests.
    #[cfg_attr(not(any(test, feature = "demo")), expect(dead_code))]
    pub example: &'static str,
    pub icon: IconName,
    names: &'static [&'static str],
}

const fn genre(example: &'static str, icon: IconName, names: &'static [&'static str]) -> Genre {
    Genre {
        example,
        icon,
        names,
    }
}

/// The genres with icons, the most specific first: when no name matches a
/// genre's name exactly, the first genre whose name appears in it wins.
pub const GENRES: &[Genre] = &[
    genre(
        "Nonfiction",
        IconName::Newspaper,
        &["nonfiction", "non fiction"],
    ),
    genre(
        "Science Fiction",
        IconName::Rocket,
        &["science fiction", "sci fi", "scifi", "sf", "cyberpunk"],
    ),
    genre("Space Opera", IconName::Orbit, &["space opera", "space"]),
    genre(
        "Magical Realism",
        IconName::Sparkles,
        &["magical realism", "magic realism"],
    ),
    genre(
        "Graphic Novels",
        IconName::BookImage,
        &[
            "graphic novels",
            "graphic novel",
            "comics",
            "comic",
            "comic books",
            "manga",
        ],
    ),
    genre(
        "Short Stories",
        IconName::BookCopy,
        &[
            "short stories",
            "short story",
            "short story collections",
            "anthology",
            "anthologies",
        ],
    ),
    genre(
        "Self-Help",
        IconName::Sprout,
        &[
            "self help",
            "personal development",
            "self improvement",
            "productivity",
        ],
    ),
    genre(
        "Literary Fiction",
        IconName::BookMarked,
        &[
            "literary fiction",
            "literary",
            "literature and fiction",
            "literature",
        ],
    ),
    genre(
        "Dystopian",
        IconName::ScanEye,
        &["dystopian", "dystopia", "post apocalyptic", "apocalyptic"],
    ),
    genre(
        "Paranormal",
        IconName::Ghost,
        &["paranormal", "supernatural", "ghost stories", "ghosts"],
    ),
    genre("Horror", IconName::Skull, &["horror"]),
    genre(
        "Thriller",
        IconName::Siren,
        &["thriller", "thrillers", "suspense"],
    ),
    genre(
        "Mystery",
        IconName::Search,
        &["mystery", "mysteries", "detective", "whodunit"],
    ),
    genre(
        "Crime",
        IconName::FingerprintPattern,
        &["crime", "true crime", "noir"],
    ),
    genre(
        "Romance",
        IconName::Heart,
        &["romance", "romantic", "romantasy"],
    ),
    genre("Fantasy", IconName::WandSparkles, &["fantasy"]),
    genre(
        "Mythology",
        IconName::Castle,
        &[
            "mythology",
            "myths",
            "folklore",
            "fairy tales",
            "fairytales",
        ],
    ),
    genre(
        "War",
        IconName::Swords,
        &["war", "military", "military fiction", "military history"],
    ),
    genre(
        "Historical Fiction",
        IconName::Hourglass,
        &["historical fiction", "historical"],
    ),
    genre(
        "Classics",
        IconName::Landmark,
        &["classics", "classic", "classic literature"],
    ),
    genre(
        "Poetry",
        IconName::Feather,
        &["poetry", "poems", "poem", "verse"],
    ),
    genre(
        "Plays",
        IconName::Theater,
        &["plays", "drama", "theatre", "theater"],
    ),
    genre(
        "Biography",
        IconName::BookUser,
        &[
            "biography",
            "biographies",
            "memoir",
            "memoirs",
            "autobiography",
            "autobiographies",
        ],
    ),
    genre("Philosophy", IconName::Lightbulb, &["philosophy"]),
    genre(
        "Psychology",
        IconName::Brain,
        &["psychology", "psychological"],
    ),
    genre(
        "Religion",
        IconName::Church,
        &[
            "religion",
            "religious",
            "spirituality",
            "spiritual",
            "theology",
            "faith",
            "christian",
            "christianity",
        ],
    ),
    genre(
        "Politics",
        IconName::Vote,
        &["politics", "political", "political science", "government"],
    ),
    genre(
        "Business",
        IconName::Briefcase,
        &[
            "business",
            "economics",
            "finance",
            "entrepreneurship",
            "management",
        ],
    ),
    genre("History", IconName::ScrollText, &["history"]),
    genre(
        "Science",
        IconName::FlaskConical,
        &[
            "science",
            "popular science",
            "physics",
            "chemistry",
            "biology",
            "astronomy",
        ],
    ),
    genre(
        "Humor",
        IconName::FaceGrinning,
        &["humor", "humour", "comedy", "funny", "satire"],
    ),
    genre(
        "Adventure",
        IconName::Compass,
        &["adventure", "action and adventure", "action"],
    ),
    genre("Travel", IconName::Plane, &["travel", "travel writing"]),
    genre(
        "Cooking",
        IconName::ChefHat,
        &[
            "cooking",
            "cookbooks",
            "cookbook",
            "food",
            "food and drink",
            "culinary",
        ],
    ),
    genre(
        "Sports",
        IconName::Trophy,
        &["sports", "sport", "martial arts", "fitness"],
    ),
    genre(
        "Art",
        IconName::Palette,
        &["art", "arts", "design", "photography"],
    ),
    genre("Music", IconName::Music, &["music"]),
    genre(
        "LGBTQ",
        IconName::Rainbow,
        &[
            "lgbtq", "lgbt", "lgbtqia", "lgbtq+", "queer", "gay", "lesbian",
        ],
    ),
    genre(
        "Young Adult",
        IconName::Backpack,
        &["young adult", "ya", "teen", "teens"],
    ),
    genre(
        "Children's",
        IconName::Blocks,
        &[
            "childrens",
            "children",
            "kids",
            "middle grade",
            "picture books",
            "juvenile",
        ],
    ),
    genre(
        "Contemporary",
        IconName::Coffee,
        &["contemporary", "contemporary fiction"],
    ),
    genre(
        "Fiction",
        IconName::BookText,
        &["fiction", "general fiction", "novels"],
    ),
];

/// The icon for a genre's chip, or none when the name matches no genre.
pub fn genre_icon(name: &str) -> Option<IconName> {
    let words = normalise(name);
    let words: Vec<&str> = words.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let phrases = || {
        GENRES.iter().flat_map(|genre| {
            genre
                .names
                .iter()
                .map(move |name| (genre.icon, name.split_whitespace().collect::<Vec<_>>()))
        })
    };
    phrases()
        .find(|(_, phrase)| *phrase == words)
        .or_else(|| phrases().find(|(_, phrase)| words.windows(phrase.len()).any(|w| w == phrase)))
        .map(|(icon, _)| icon)
}

/// Lowercase words separated by single spaces: `&` reads "and",
/// apostrophes are dropped ("Children's" is "childrens"), and any other
/// character that is not a letter or a digit separates words.
fn normalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        match c {
            '&' => out.push_str(" and "),
            '\'' | '\u{2018}' | '\u{2019}' => {}
            c if c.is_alphanumeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::AppAssets;
    use gpui_kit::AssetSource as _;

    fn icon(name: &str) -> Option<IconName> {
        genre_icon(name)
    }

    #[test]
    fn every_icon_loads_through_the_app_assets() {
        for genre in GENRES {
            let bytes = AppAssets
                .load(&genre.icon.path())
                .unwrap()
                .unwrap_or_else(|| panic!("{:?} is not served by AppAssets", genre.icon));
            assert!(bytes.starts_with(b"<svg"), "{:?}", genre.icon);
        }
    }

    #[test]
    fn each_example_maps_to_its_own_icon() {
        for genre in GENRES {
            assert_eq!(icon(genre.example), Some(genre.icon), "{}", genre.example);
        }
    }

    #[test]
    fn names_are_already_normalised() {
        for genre in GENRES {
            for name in genre.names {
                // "lgbtq+" is the one name that normalises to another
                // ("lgbtq"); it is harmless and documents the spelling.
                if *name != "lgbtq+" {
                    assert_eq!(normalise(name).trim(), *name);
                }
            }
        }
    }

    #[test]
    fn spellings_and_variants_match() {
        let cases = [
            ("Science Fiction", IconName::Rocket),
            ("Sci-Fi", IconName::Rocket),
            ("Sci-fi", IconName::Rocket),
            ("SF", IconName::Rocket),
            ("sci fi", IconName::Rocket),
            ("Space", IconName::Orbit),
            ("Space Opera", IconName::Orbit),
            ("Young Adult", IconName::Backpack),
            ("YA", IconName::Backpack),
            ("Children's", IconName::Blocks),
            ("Children\u{2019}s", IconName::Blocks),
            ("Middle Grade", IconName::Blocks),
            ("Historical Fiction", IconName::Hourglass),
            ("Literary Fiction", IconName::BookMarked),
            ("Literary", IconName::BookMarked),
            ("Literature & Fiction", IconName::BookMarked),
            ("Nonfiction", IconName::Newspaper),
            ("Non-Fiction", IconName::Newspaper),
            ("Graphic Novels", IconName::BookImage),
            ("Comics", IconName::BookImage),
            ("LGBTQ", IconName::Rainbow),
            ("LGBT", IconName::Rainbow),
            ("LGBTQ+", IconName::Rainbow),
            ("Self Help", IconName::Sprout),
            ("Self-Help", IconName::Sprout),
            ("Biography", IconName::BookUser),
            ("Memoir", IconName::BookUser),
            ("Horror", IconName::Skull),
            ("Thriller", IconName::Siren),
            ("Mystery", IconName::Search),
            ("Crime", IconName::FingerprintPattern),
            ("Romance", IconName::Heart),
            ("Fantasy", IconName::WandSparkles),
            ("Magical Realism", IconName::Sparkles),
            ("Dystopian", IconName::ScanEye),
            ("Classics", IconName::Landmark),
            ("Classic Literature", IconName::Landmark),
            ("Poetry", IconName::Feather),
            ("Philosophy", IconName::Lightbulb),
            ("History", IconName::ScrollText),
            ("Science", IconName::FlaskConical),
            ("Business", IconName::Briefcase),
            ("Psychology", IconName::Brain),
            ("Religion", IconName::Church),
            ("religious", IconName::Church),
            ("Humor", IconName::FaceGrinning),
            ("Humour", IconName::FaceGrinning),
            ("Adventure", IconName::Compass),
            ("Action & Adventure", IconName::Compass),
            ("War", IconName::Swords),
            ("Travel", IconName::Plane),
            ("Cooking", IconName::ChefHat),
            ("Food", IconName::ChefHat),
            ("Art", IconName::Palette),
            ("Music", IconName::Music),
            ("Sports", IconName::Trophy),
            ("Politics", IconName::Vote),
            ("Short Stories", IconName::BookCopy),
            ("Plays", IconName::Theater),
            ("Drama", IconName::Theater),
            ("Paranormal", IconName::Ghost),
            ("Contemporary", IconName::Coffee),
            ("Fiction", IconName::BookText),
            ("  FANTASY ", IconName::WandSparkles),
        ];
        for (name, expected) in cases {
            assert_eq!(icon(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn the_more_specific_genre_wins() {
        let cases = [
            // Whole names first, then the earlier genre in the table.
            ("Non-Fiction", IconName::Newspaper),
            ("Science Fiction & Fantasy", IconName::Rocket),
            ("High Fantasy", IconName::WandSparkles),
            ("Urban Fantasy", IconName::WandSparkles),
            ("Romantasy", IconName::Heart),
            ("Historical Romance", IconName::Heart),
            ("Historical Fantasy", IconName::WandSparkles),
            ("Political Science", IconName::Vote),
            ("Military History", IconName::Swords),
            ("Martial Arts", IconName::Trophy),
            ("Psychological Thriller", IconName::Siren),
            ("Contemporary Romance", IconName::Heart),
            ("Women's Fiction", IconName::BookText),
            ("Popular Science", IconName::FlaskConical),
        ];
        for (name, expected) in cases {
            assert_eq!(icon(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn only_whole_words_match() {
        // "art" in "Smart", "war" in "Star Wars", "sf" and "ya" inside
        // other words, and "verse" in "Multiverse" are not genres.
        for name in [
            "Smart Thinking",
            "Star Wars",
            "Sfumato",
            "Yarn",
            "Multiverse",
            "Spacetime",
        ] {
            assert_eq!(icon(name), None, "{name}");
        }
    }

    #[test]
    fn unmatched_names_get_no_icon() {
        for name in ["General", "Women", "", "   ", "رمان", "ادبیات فارسی"] {
            assert_eq!(icon(name), None, "{name}");
        }
    }

    /// Every genre in the demo catalogue that should get an icon gets one,
    /// and the rest are known to have none.
    #[test]
    fn the_demo_catalogues_genres() {
        let catalogue: serde_json::Value = serde_json::from_str(include_str!(
            "../../../hardcover-core/src/demo/catalogue.json"
        ))
        .unwrap();
        let mut genres: Vec<&str> = catalogue["books"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|book| book["genres"].as_array().into_iter().flatten())
            .filter_map(|genre| genre.as_str())
            .collect();
        genres.sort_unstable();
        genres.dedup();
        assert!(!genres.is_empty());
        let without: Vec<&str> = genres
            .iter()
            .copied()
            .filter(|genre| icon(genre).is_none())
            .collect();
        assert_eq!(without, ["General", "Women"]);
    }
}
