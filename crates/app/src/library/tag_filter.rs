//! A shelf's genre and mood filter (`hardcover-design`, The filter bar),
//! pure and tested: what a book has, what a pick keeps, the chips each row
//! offers with their counts, how many fit, and the words of its empty state
//! and foot.
//!
//! - **What a book has** ([`BookFacts`]): its genres as Hardcover tags them,
//!   spellings merged ([`hardcover_core::tags::BookGenres`]), and the moods
//!   of its mood line ([`has_mood`], the book header's own test, so a book
//!   the filter keeps always shows the mood).
//! - **A pick** ([`TagPick`]): one genre and one mood at most; a book must
//!   have both. Pressing the selected chip clears it; pressing another of
//!   the same row replaces it.
//! - **The chips** ([`facets`]): each row offers what the shelf's books
//!   have, the most books first across the whole shelf (ties by name for
//!   genres, in Hardcover's order for moods), so the rows don't reorder as
//!   the reader picks. A chip's count is what a press would leave: with a
//!   mood picked, a genre counts that mood's books, and the other way round.
//!   An unselected chip at 0 is disabled; a selected one never is, and stays
//!   offered when the shelf no longer has it, so it can be cleared.
//! - **Fitting a row** ([`fit_row`]): as many as fit on one line, then
//!   "More" with the rest; the selected chip that wouldn't fit in its place
//!   takes the first.

use std::collections::HashMap;

use hardcover_core::BookTags;
use hardcover_core::tags::{Mood, has_mood};

/// One of a book's genres: its key ([`hardcover_core::tags::genre_key`])
/// and the name it is tagged with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenreName {
    pub key: String,
    pub name: String,
}

/// What the filter knows of one book.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookFacts {
    pub genres: Vec<GenreName>,
    /// The moods on its mood line, in [`Mood::ALL`]'s order.
    pub moods: Vec<Mood>,
}

impl BookFacts {
    pub fn of(tags: &BookTags) -> Self {
        Self {
            genres: tags
                .genres
                .iter()
                .map(|g| GenreName {
                    key: g.key.clone(),
                    name: g.name.clone(),
                })
                .collect(),
            moods: Mood::ALL
                .into_iter()
                .filter(|m| has_mood(&tags.moods, *m))
                .collect(),
        }
    }

    pub fn has_genre(&self, key: &str) -> bool {
        self.genres.iter().any(|g| g.key == key)
    }

    pub fn has_mood(&self, mood: Mood) -> bool {
        self.moods.contains(&mood)
    }

    /// No chip finds it: no genre and no mood line.
    pub fn is_unfindable(&self) -> bool {
        self.genres.is_empty() && self.moods.is_empty()
    }
}

/// A picked genre: its key, and the name its chip shows, kept so the chip
/// stays drawn after the shelf loses its last book of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenrePick {
    pub key: String,
    pub name: String,
}

/// The chips picked on one shelf.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TagPick {
    pub genre: Option<GenrePick>,
    pub mood: Option<Mood>,
}

impl TagPick {
    pub fn is_empty(&self) -> bool {
        self.genre.is_none() && self.mood.is_none()
    }

    /// Whether a book with `facts` (or none known) stays.
    pub fn keeps(&self, facts: Option<&BookFacts>) -> bool {
        if self.is_empty() {
            return true;
        }
        facts.is_some_and(|f| self.keeps_genre(f) && self.keeps_mood(f))
    }

    fn keeps_genre(&self, facts: &BookFacts) -> bool {
        self.genre.as_ref().is_none_or(|g| facts.has_genre(&g.key))
    }

    fn keeps_mood(&self, facts: &BookFacts) -> bool {
        self.mood.is_none_or(|m| facts.has_mood(m))
    }

    /// A press on a genre chip: picks it, or clears it when it is picked.
    pub fn press_genre(&mut self, key: &str, name: &str) {
        if self.genre.as_ref().is_some_and(|g| g.key == key) {
            self.genre = None;
        } else {
            self.genre = Some(GenrePick {
                key: key.to_owned(),
                name: name.to_owned(),
            });
        }
    }

    /// A press on a mood chip: picks it, or clears it when it is picked.
    pub fn press_mood(&mut self, mood: Mood) {
        self.mood = if self.mood == Some(mood) {
            None
        } else {
            Some(mood)
        };
    }
}

/// One chip of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip<T> {
    pub value: T,
    pub name: String,
    /// The books a press would leave (with the other row's pick).
    pub count: usize,
    pub selected: bool,
}

impl<T> Chip<T> {
    /// A press would leave nothing. A selected chip is never disabled.
    pub fn disabled(&self) -> bool {
        self.count == 0 && !self.selected
    }
}

/// The two rows' chips, and how many books no chip finds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facets {
    /// Valued by genre key.
    pub genres: Vec<Chip<String>>,
    pub moods: Vec<Chip<Mood>>,
    /// Books whose tags are known and have no genre and no mood line.
    pub unfindable: usize,
}

/// The chips for a shelf whose books have `books` (`None` for a book whose
/// tags aren't known), under `pick`. See the module doc.
pub fn facets<'a>(
    books: impl IntoIterator<Item = Option<&'a BookFacts>>,
    pick: &TagPick,
) -> Facets {
    struct GenreTally<'a> {
        shelf: usize,
        count: usize,
        /// How many books spell it each way.
        names: HashMap<&'a str, usize>,
    }
    let mut genres: HashMap<&'a str, GenreTally<'a>> = HashMap::new();
    let mut moods: HashMap<Mood, (usize, usize)> = HashMap::new();
    let mut unfindable = 0;
    for facts in books.into_iter().flatten() {
        if facts.is_unfindable() {
            unfindable += 1;
            continue;
        }
        let (by_mood, by_genre) = (pick.keeps_mood(facts), pick.keeps_genre(facts));
        for genre in &facts.genres {
            let tally = genres.entry(&genre.key).or_insert_with(|| GenreTally {
                shelf: 0,
                count: 0,
                names: HashMap::new(),
            });
            tally.shelf += 1;
            tally.count += usize::from(by_mood);
            *tally.names.entry(&genre.name).or_default() += 1;
        }
        for mood in &facts.moods {
            let (shelf, count) = moods.entry(*mood).or_default();
            *shelf += 1;
            *count += usize::from(by_genre);
        }
    }

    let mut genre_chips: Vec<(usize, Chip<String>)> = genres
        .into_iter()
        .map(|(key, tally)| {
            // The spelling most of the shelf's books use; the first by name
            // on a tie, so it never depends on the map's order.
            let name = tally
                .names
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map_or(key, |(name, _)| *name)
                .to_owned();
            let selected = pick.genre.as_ref().is_some_and(|g| g.key == key);
            let chip = Chip {
                value: key.to_owned(),
                name,
                count: tally.count,
                selected,
            };
            (tally.shelf, chip)
        })
        .collect();
    genre_chips.sort_by(|(a, x), (b, y)| {
        b.cmp(a)
            .then_with(|| x.name.to_lowercase().cmp(&y.name.to_lowercase()))
            .then_with(|| x.name.cmp(&y.name))
    });
    let mut genre_chips: Vec<Chip<String>> = genre_chips.into_iter().map(|(_, c)| c).collect();
    if let Some(picked) = &pick.genre
        && !genre_chips.iter().any(|c| c.value == picked.key)
    {
        genre_chips.push(Chip {
            value: picked.key.clone(),
            name: picked.name.clone(),
            count: 0,
            selected: true,
        });
    }

    let order = |m: &Mood| Mood::ALL.iter().position(|x| x == m).unwrap_or(usize::MAX);
    let mut mood_chips: Vec<(usize, Chip<Mood>)> = moods
        .into_iter()
        .map(|(mood, (shelf, count))| {
            let chip = Chip {
                value: mood,
                name: mood.name().to_owned(),
                count,
                selected: pick.mood == Some(mood),
            };
            (shelf, chip)
        })
        .collect();
    mood_chips
        .sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| order(&x.value).cmp(&order(&y.value))));
    let mut mood_chips: Vec<Chip<Mood>> = mood_chips.into_iter().map(|(_, c)| c).collect();
    if let Some(mood) = pick.mood
        && !mood_chips.iter().any(|c| c.value == mood)
    {
        mood_chips.push(Chip {
            value: mood,
            name: mood.name().to_owned(),
            count: 0,
            selected: true,
        });
    }

    Facets {
        genres: genre_chips,
        moods: mood_chips,
        unfindable,
    }
}

/// Which chips of a row show, and in what order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowFit {
    /// Indices into the row's chips, in the order drawn.
    pub shown: Vec<usize>,
    /// The others, in their order, for "More". Empty: no "More".
    pub more: Vec<usize>,
}

/// Fits a row of chips `widths` wide, `gap` apart, into `available`, with
/// "More" (`more` wide, a gap after the last chip) when they don't all fit.
/// The selected chip (`selected`) that wouldn't fit in its place goes first;
/// the first chip shown is kept even when it is too wide (it is cut).
pub fn fit_row(
    widths: &[f32],
    selected: Option<usize>,
    available: f32,
    gap: f32,
    more: f32,
) -> RowFit {
    let n = widths.len();
    let total: f32 = widths.iter().sum::<f32>() + gap * n.saturating_sub(1) as f32;
    if n == 0 || total <= available {
        return RowFit {
            shown: (0..n).collect(),
            more: Vec::new(),
        };
    }
    // Chips in `order`, while each fits before "More".
    let take = |order: &mut dyn Iterator<Item = usize>| {
        let mut used = more;
        let mut shown = Vec::new();
        for ix in order {
            let next = used + widths[ix] + gap;
            if next > available && !shown.is_empty() {
                break;
            }
            used = next;
            shown.push(ix);
        }
        shown
    };
    let mut shown = take(&mut (0..n));
    if let Some(sel) = selected
        && sel < n
        && !shown.contains(&sel)
    {
        shown = take(&mut std::iter::once(sel).chain((0..n).filter(|ix| *ix != sel)));
    }
    let more = (0..n).filter(|ix| !shown.contains(ix)).collect();
    RowFit { shown, more }
}

/// The empty state's sentence when the chips (and maybe the text filter)
/// leave nothing on `shelf`.
pub fn no_matches(shelf: &str, pick: &TagPick, query: &str) -> String {
    let query = query.trim();
    if !query.is_empty() {
        return if pick.is_empty() {
            format!("No books in {shelf} match “{query}”.")
        } else {
            format!("No books in {shelf} match “{query}” with these filters.")
        };
    }
    match (&pick.genre, pick.mood) {
        (Some(genre), Some(mood)) => {
            format!(
                "No books in {shelf} are both {} and {}.",
                genre.name,
                mood.name()
            )
        }
        (Some(genre), None) => format!("No books in {shelf} match {}.", genre.name),
        (None, Some(mood)) => format!("No books in {shelf} match {}.", mood.name()),
        (None, None) => format!("No books in {shelf} match."),
    }
}

/// The foot's sentence: how many of the shelf's books no chip finds.
pub fn unfindable_line(count: usize) -> String {
    if count == 1 {
        "1 book on this shelf has no genre or mood votes yet, so no chip finds it.".to_owned()
    } else {
        format!(
            "{} books on this shelf have no genre or mood votes yet, so no chip finds them.",
            crate::format::count_text(count)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(genres: &[&str], moods: &[Mood]) -> BookFacts {
        BookFacts {
            genres: genres
                .iter()
                .map(|g| GenreName {
                    key: g.to_lowercase(),
                    name: (*g).to_owned(),
                })
                .collect(),
            moods: moods.to_vec(),
        }
    }

    /// A shelf of seven books and one with no votes.
    fn shelf() -> Vec<BookFacts> {
        use Mood::*;
        vec![
            book(&["Fantasy"], &[Relaxing, Lighthearted, Hopeful]),
            book(&["Fantasy"], &[Dark, Challenging, Tense]),
            book(
                &["Fantasy", "Classics"],
                &[Lighthearted, Adventurous, Funny],
            ),
            book(&["Mystery", "Thriller"], &[Mysterious, Dark, Tense]),
            book(&["Fantasy", "Mystery"], &[Mysterious, Reflective]),
            book(&["Fantasy", "Young Adult"], &[Tense, Adventurous, Dark]),
            book(&["Fantasy"], &[Reflective, Adventurous, Emotional]),
            book(&[], &[]),
        ]
    }

    fn pick(genre: Option<&str>, mood: Option<Mood>) -> TagPick {
        TagPick {
            genre: genre.map(|g| GenrePick {
                key: g.to_lowercase(),
                name: g.to_owned(),
            }),
            mood,
        }
    }

    fn of(books: &[BookFacts], pick: &TagPick) -> Facets {
        facets(books.iter().map(Some), pick)
    }

    fn genre<'a>(f: &'a Facets, name: &str) -> &'a Chip<String> {
        f.genres.iter().find(|c| c.name == name).unwrap()
    }

    fn mood(f: &Facets, m: Mood) -> &Chip<Mood> {
        f.moods.iter().find(|c| c.value == m).unwrap()
    }

    #[test]
    fn rows_list_the_shelfs_own_tags_most_books_first() {
        let f = of(&shelf(), &TagPick::default());
        let names: Vec<&str> = f.genres.iter().map(|c| c.name.as_str()).collect();
        // Ties by name.
        assert_eq!(
            names,
            ["Fantasy", "Mystery", "Classics", "Thriller", "Young Adult"]
        );
        let moods: Vec<Mood> = f.moods.iter().map(|c| c.value).collect();
        // Adventurous, Dark and Tense have 3 each: Hardcover's order.
        assert_eq!(&moods[..3], [Mood::Adventurous, Mood::Dark, Mood::Tense]);
        assert_eq!(genre(&f, "Fantasy").count, 6);
        assert_eq!(f.unfindable, 1);
    }

    /// With Fantasy picked, each mood counts the Fantasy books that have it,
    /// and the genres count every book (no mood picked).
    #[test]
    fn counts_follow_the_other_rows_pick() {
        let f = of(&shelf(), &pick(Some("Fantasy"), None));
        assert_eq!(mood(&f, Mood::Adventurous).count, 3);
        assert_eq!(mood(&f, Mood::Mysterious).count, 1);
        assert_eq!(
            genre(&f, "Mystery").count,
            2,
            "genres ignore their own pick"
        );
        assert!(genre(&f, "Fantasy").selected);

        let f = of(&shelf(), &pick(Some("Fantasy"), Some(Mood::Dark)));
        assert_eq!(genre(&f, "Fantasy").count, 2);
        assert_eq!(genre(&f, "Mystery").count, 1);
        assert_eq!(genre(&f, "Classics").count, 0);
        assert_eq!(mood(&f, Mood::Dark).count, 2);
    }

    #[test]
    fn the_order_doesnt_follow_the_pick() {
        let before = of(&shelf(), &TagPick::default());
        let after = of(&shelf(), &pick(None, Some(Mood::Mysterious)));
        let names = |f: &Facets| f.genres.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&before), names(&after));
    }

    #[test]
    fn a_chip_that_would_leave_nothing_is_disabled_unless_selected() {
        let f = of(&shelf(), &pick(Some("Fantasy"), Some(Mood::Dark)));
        assert!(genre(&f, "Classics").disabled());
        assert!(!genre(&f, "Fantasy").disabled());
        let thriller = genre(&f, "Thriller");
        assert_eq!(thriller.count, 1);
        assert!(!thriller.disabled());
    }

    #[test]
    fn a_pick_keeps_books_with_both() {
        let books = shelf();
        let p = pick(Some("Fantasy"), Some(Mood::Dark));
        let kept = books.iter().filter(|b| p.keeps(Some(b))).count();
        assert_eq!(kept, 2);
        assert!(!p.keeps(None), "a book with unknown tags isn't kept");
        assert!(TagPick::default().keeps(None), "no pick keeps everything");
    }

    #[test]
    fn pressing_the_selected_chip_clears_only_it() {
        let mut p = pick(Some("Fantasy"), Some(Mood::Dark));
        p.press_genre("fantasy", "Fantasy");
        assert_eq!(p, pick(None, Some(Mood::Dark)));
        p.press_mood(Mood::Tense);
        assert_eq!(p.mood, Some(Mood::Tense), "another chip replaces it");
        p.press_mood(Mood::Tense);
        assert!(p.is_empty());
    }

    /// The last Mystery book was started: the chip stays, selected and
    /// pressable, at 0, and the shelf shows no matches.
    #[test]
    fn a_pick_the_shelf_no_longer_has_stays_offered() {
        let books: Vec<BookFacts> = shelf()
            .into_iter()
            .filter(|b| !b.has_genre("mystery"))
            .collect();
        let p = pick(Some("Mystery"), Some(Mood::Adventurous));
        let f = of(&books, &p);
        let chip = genre(&f, "Mystery");
        assert_eq!(
            (chip.count, chip.selected, chip.disabled()),
            (0, true, false)
        );
        assert_eq!(books.iter().filter(|b| p.keeps(Some(b))).count(), 0);
        assert_eq!(
            no_matches("Want to read", &p, ""),
            "No books in Want to read are both Mystery and Adventurous."
        );
    }

    #[test]
    fn a_genres_name_is_the_one_most_books_use() {
        let mut books = vec![book(&["Sci-fi"], &[]), book(&["Science Fiction"], &[])];
        for b in &mut books {
            b.genres[0].key = "science fiction".into();
        }
        books.push(books[1].clone());
        let f = of(&books, &TagPick::default());
        assert_eq!(f.genres.len(), 1);
        assert_eq!(f.genres[0].name, "Science Fiction");
        assert_eq!(f.genres[0].count, 3);
    }

    #[test]
    fn a_row_fits_as_many_as_it_can_then_more() {
        let w = [50.0, 40.0, 60.0, 30.0];
        assert_eq!(
            fit_row(&w, None, 500.0, 6.0, 40.0).more,
            Vec::<usize>::new()
        );
        // 50 + 6 + 40 + 6 + More (40) = 142.
        let fit = fit_row(&w, None, 150.0, 6.0, 40.0);
        assert_eq!(fit.shown, [0, 1]);
        assert_eq!(fit.more, [2, 3]);
    }

    #[test]
    fn a_selected_chip_that_doesnt_fit_goes_first() {
        let w = [50.0, 40.0, 60.0, 30.0];
        let fit = fit_row(&w, Some(3), 150.0, 6.0, 40.0);
        assert_eq!(fit.shown, [3, 0]);
        assert_eq!(fit.more, [1, 2]);
        // One that fits keeps its place.
        assert_eq!(fit_row(&w, Some(1), 150.0, 6.0, 40.0).shown, [0, 1]);
    }

    #[test]
    fn a_narrow_row_keeps_one_chip() {
        let fit = fit_row(&[200.0, 40.0], None, 100.0, 6.0, 40.0);
        assert_eq!(fit.shown, [0]);
        let fit = fit_row(&[200.0, 40.0], Some(1), 60.0, 6.0, 40.0);
        assert_eq!(fit.shown, [1]);
    }

    #[test]
    fn the_empty_states_words() {
        let p = pick(Some("Mystery"), None);
        assert_eq!(
            no_matches("Read", &p, ""),
            "No books in Read match Mystery."
        );
        assert_eq!(
            no_matches("Read", &p, " dune "),
            "No books in Read match “dune” with these filters."
        );
        assert_eq!(
            unfindable_line(3),
            "3 books on this shelf have no genre or mood votes yet, so no chip finds them."
        );
    }
}
