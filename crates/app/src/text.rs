//! Text as the reader types it against text as Hardcover stores it: one
//! folding for every search over the library (search's library matches, a
//! shelf's filter, the Series filter), and the digits of a number field.
//!
//! Persian keyboards type ی and ک, while many catalogue titles hold the
//! Arabic ي and ك; a Persian title may write its numbers ۱۹۸۴ and the reader
//! 1984; "Les Misérables" is typed "miserables". [`fold`] makes each pair the
//! same text, and [`search_words`] and [`indexed_words`] cut it into the words
//! a query matches. Plain functions, so every pair is pinned by a test.

use std::borrow::Cow;

use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::canonical_combining_class;

/// The zero-width non-joiner, which Persian writes inside a word
/// ("می‌خواهم"), and which a reader may type, leave out or replace with a
/// space.
const ZWNJ: char = '\u{200c}';

/// `text` folded for matching: lowercase, without accents, harakat or other
/// marks (decomposed, then every combining mark dropped), compatibility
/// forms decomposed ("ﬁ" is "fi"), Arabic letter forms as Persian writes
/// them (ي ى → ی, ك → ک, ة → ه), the tatweel and zero-width joiner dropped,
/// Persian and Arabic-Indic digits as ASCII, and the result composed again
/// (NFC). The zero-width non-joiner is kept: [`search_words`] and
/// [`indexed_words`] decide what it means.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.nfkd() {
        if canonical_combining_class(c) != 0 {
            continue;
        }
        match c {
            // Arabic yeh and alef maksura; yeh with hamza is yeh plus a mark.
            '\u{064a}' | '\u{0649}' => out.push('ی'),
            '\u{0643}' => out.push('ک'),
            // Teh marbuta, and the ae that heh with yeh above (ۀ) leaves.
            '\u{0629}' | '\u{06d5}' => out.push('ه'),
            // Tatweel (kashida) and the zero-width joiner only stretch or join.
            '\u{0640}' | '\u{200d}' => {}
            c => match ascii_digit(c) {
                Some(d) => out.push(d),
                None => out.extend(c.to_lowercase()),
            },
        }
    }
    out.nfc().collect()
}

/// The words of a query: runs of letters and digits in [`fold`]ed text. An
/// apostrophe inside a word is dropped ("ender's" is "enders"), and so is a
/// zero-width non-joiner, so a word typed with one or without one is the
/// same word.
pub fn search_words(text: &str) -> Vec<String> {
    split_words(&fold(text).replace(ZWNJ, ""))
}

/// The words a query is matched against, from a title, a name or a series:
/// as [`search_words`] cuts them, and where a word holds a zero-width
/// non-joiner, each of its parts too. So "می‌خواهم" is found by "میخواهم",
/// by "می‌خواهم" and by "می خواهم". The words come in order, a word's parts
/// after it.
pub fn indexed_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in split_words_keeping(&fold(text), ZWNJ) {
        let parts: Vec<&str> = word.split(ZWNJ).filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        out.push(parts.concat());
        if parts.len() > 1 {
            out.extend(parts.iter().map(|p| (*p).to_owned()));
        }
    }
    out
}

/// Runs of letters and digits; apostrophes inside a word are dropped.
fn split_words(text: &str) -> Vec<String> {
    split_words_keeping(text, '\'')
}

/// Runs of letters, digits and `keep`; apostrophes inside a word are
/// dropped.
fn split_words_keeping(text: &str, keep: char) -> Vec<String> {
    text.replace(['\'', '’'], "")
        .split(|c: char| !(c.is_alphanumeric() || c == keep))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `text` with Persian and Arabic-Indic digits as ASCII ones, the Arabic
/// decimal separator `٫` as `.` and the Arabic percent sign `٪` as `%`: what
/// a number field reads, so it can be typed on a Persian keyboard. Borrowed
/// when there is nothing to change.
pub fn ascii_numerals(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| match c {
                '\u{066b}' => '.',
                '\u{066a}' => '%',
                c => ascii_digit(c).unwrap_or(c),
            })
            .collect(),
    )
}

/// A Persian (۰–۹) or Arabic-Indic (٠–٩) digit as its ASCII digit.
fn ascii_digit(c: char) -> Option<char> {
    let zero = match c {
        '\u{06f0}'..='\u{06f9}' => 0x06f0,
        '\u{0660}'..='\u{0669}' => 0x0660,
        _ => return None,
    };
    char::from_digit(u32::from(c) - zero, 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arabic_letter_forms_fold_to_persian_ones() {
        // Kelidar with Arabic kaf and yeh, typed on a Persian keyboard.
        assert_eq!(fold("كليدر"), fold("کلیدر"));
        assert_eq!(fold("کلیدر"), "کلیدر");
        // Alef maksura, teh marbuta, heh with yeh above.
        assert_eq!(fold("موسى"), "موسی");
        assert_eq!(fold("مدرسة"), "مدرسه");
        assert_eq!(fold("خانۀ"), fold("خانه"));
        // Yeh with hamza is yeh; alef with madda or hamza is alef.
        assert_eq!(fold("ئ"), "ی");
        assert_eq!(fold("آسمان"), "اسمان");
        assert_eq!(fold("أحمد"), "احمد");
    }

    #[test]
    fn marks_tatweel_and_joiners_are_dropped() {
        // Harakat and the shadda.
        assert_eq!(fold("كِتَابٌ"), fold("کتاب"));
        assert_eq!(fold("مُحَمَّد"), "محمد");
        // A stretched word.
        assert_eq!(fold("کتـــاب"), "کتاب");
        assert_eq!(fold("a\u{200d}b"), "ab");
    }

    #[test]
    fn latin_accents_and_compatibility_forms_fold() {
        assert_eq!(fold("Les Misérables"), "les miserables");
        assert_eq!(fold("Ångström"), "angstrom");
        assert_eq!(fold("ﬁre"), "fire");
        // Composed again: Hangul stays syllables.
        assert_eq!(fold("한강"), "한강");
    }

    #[test]
    fn persian_and_arabic_digits_fold_to_ascii() {
        assert_eq!(fold("۱۹۸۴"), "1984");
        assert_eq!(fold("١٩٨٤"), "1984");
        assert_eq!(search_words("۱۹۸۴"), search_words("1984"));
    }

    #[test]
    fn a_word_with_a_non_joiner_matches_with_or_without_it() {
        let title = indexed_words("نمی\u{200c}خوریم امروز");
        assert_eq!(title, ["نمیخوریم", "نمی", "خوریم", "امروز"]);
        // Typed without it, with it, or with a space.
        assert_eq!(search_words("نمیخوریم"), ["نمیخوریم"]);
        assert_eq!(search_words("نمی\u{200c}خوریم"), ["نمیخوریم"]);
        assert_eq!(search_words("نمی خوریم"), ["نمی", "خوریم"]);
        // A title without one is found by a query with one.
        assert_eq!(indexed_words("نمیخوریم"), ["نمیخوریم"]);
    }

    #[test]
    fn words_drop_apostrophes_and_split_on_punctuation() {
        assert_eq!(search_words("Ender’s Game"), ["enders", "game"]);
        assert_eq!(indexed_words("Dune: Messiah"), ["dune", "messiah"]);
        assert_eq!(search_words("  "), [] as [String; 0]);
    }

    #[test]
    fn a_number_field_reads_persian_digits_and_separators() {
        assert_eq!(ascii_numerals("۱۲۰"), "120");
        assert_eq!(ascii_numerals("۴۲٫۵٪"), "42.5%");
        assert_eq!(ascii_numerals("٢:٠٥"), "2:05");
        assert!(matches!(ascii_numerals("42"), Cow::Borrowed(_)));
    }
}
