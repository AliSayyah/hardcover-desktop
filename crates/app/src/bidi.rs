//! Text that may wrap and may be right-to-left, such as Persian or Arabic
//! titles and descriptions.
//!
//! GPUI's Linux text layout draws wrapped right-to-left text in the wrong
//! place: a Persian title that needs two lines lands mostly left of its own
//! box, behind the cover and off the pane. A right-to-left line that doesn't
//! wrap is drawn correctly (the list rows show that), so [`WrappingText`]
//! wraps such text itself: each word is laid out alone, and the words flow
//! right to left in a wrapping row, packed to the right as right-to-left text
//! reads. Left-to-right text takes GPUI's own path, unchanged.
//!
//! The words come from splitting on whitespace only, so a zero-width
//! non-joiner (نمی‌خوریم) stays inside its word. Consecutive left-to-right
//! words, such as "Harry Potter" in a Persian title, stay together as one
//! piece, so they read left to right. This is a small part of the Unicode
//! bidirectional algorithm, enough for titles and plain paragraphs; markdown
//! (reviews) and text fields keep GPUI's own layout. The pieces stay in
//! reading order in the element tree, only the layout reverses them, so
//! assistive technology reads them in order.

use gpui_kit::{
    App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use unicode_bidi::{Direction, get_base_direction};

/// A paragraph of text that wraps, in either direction. Font, size and color
/// come from the parent, as for a plain string child.
#[derive(IntoElement)]
pub struct WrappingText {
    text: SharedString,
    line_clamp: Option<usize>,
}

impl WrappingText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            line_clamp: None,
        }
    }

    /// Shows at most `lines` lines. Left-to-right text ends the last one with
    /// an ellipsis; right-to-left text is cut at the last line.
    pub fn line_clamp(mut self, lines: usize) -> Self {
        self.line_clamp = Some(lines);
        self
    }
}

impl RenderOnce for WrappingText {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        if !is_rtl(&self.text) {
            return div()
                .when_some(self.line_clamp, |this, lines| this.line_clamp(lines))
                .child(self.text);
        }
        // The parent's text style is in effect here, so this is the height of
        // one line of this text.
        let line_height = window.text_style().line_height_in_pixels(window.rem_size());
        let pieces = pieces(&self.text);
        let last = pieces.len().saturating_sub(1);
        div()
            .flex()
            .flex_row_reverse()
            .flex_wrap()
            .when_some(self.line_clamp, |this, lines| {
                this.max_h(line_height * lines as f32).overflow_hidden()
            })
            .children(pieces.into_iter().enumerate().map(|(i, piece)| {
                // The space goes on the piece's left, where the next piece
                // follows, whichever way the piece itself reads.
                div()
                    .flex()
                    .flex_row_reverse()
                    .child(piece)
                    .when(i < last, |this| this.child(" "))
            }))
    }
}

/// Whether `text` reads right to left: its first strong character does.
pub fn is_rtl(text: &str) -> bool {
    get_base_direction(text) == Direction::Rtl
}

/// The pieces right-to-left `text` is laid out in, in reading order: each
/// word alone, except that a left-to-right run stays together. The run takes
/// in the European numbers that follow its words ("Catch 22"; rule W7 of the
/// bidirectional algorithm), and other neutral words only between two of its
/// words (rule N1).
fn pieces(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let directions: Vec<Direction> = words.iter().map(|w| get_base_direction(*w)).collect();
    let mut pieces: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if directions[i] != Direction::Ltr {
            pieces.push(words[i].to_owned());
            i += 1;
            continue;
        }
        let mut end = i + 1;
        let mut j = end;
        while j < words.len() && directions[j] != Direction::Rtl {
            if directions[j] == Direction::Ltr || is_european_number(words[j]) {
                end = j + 1;
            }
            j += 1;
        }
        pieces.push(words[i..end].join(" "));
        i = end;
    }
    pieces
}

/// A word with no strong direction that holds ASCII digits, such as "22" or
/// "(2024)". Persian and Arabic digits are not European numbers.
fn is_european_number(word: &str) -> bool {
    get_base_direction(word) == Direction::Mixed && word.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_to_right_text_is_not_rtl() {
        assert!(!is_rtl("The Way of Kings"));
        // The first strong character decides, as for a paragraph.
        assert!(!is_rtl("Dune: کتاب"));
        assert!(!is_rtl("2024"));
    }

    #[test]
    fn persian_and_arabic_text_is_rtl() {
        assert!(is_rtl("کتاب خوب"));
        assert!(is_rtl("۱۹۸۴ کتاب"));
        // Digits aren't strong: the first letter decides.
        assert!(is_rtl("1984 کتاب"));
        assert!(is_rtl("كتاب جيد"));
    }

    #[test]
    fn a_persian_title_splits_into_words() {
        assert_eq!(pieces("کتاب   خوب،  و\nبد"), ["کتاب", "خوب،", "و", "بد"]);
    }

    #[test]
    fn a_zero_width_non_joiner_stays_inside_its_word() {
        assert_eq!(
            pieces("نمی\u{200c}خوریم امروز"),
            ["نمی\u{200c}خوریم", "امروز"]
        );
    }

    #[test]
    fn a_latin_run_stays_together_in_reading_order() {
        assert_eq!(
            pieces("راهنمای Harry Potter برای خوانندگان"),
            ["راهنمای", "Harry Potter", "برای", "خوانندگان"]
        );
    }

    #[test]
    fn numbers_after_a_latin_word_join_its_run() {
        assert_eq!(
            pieces("کتاب Catch 22 Again خوب"),
            ["کتاب", "Catch 22 Again", "خوب"]
        );
        assert_eq!(pieces("کتاب Catch 22 خوب"), ["کتاب", "Catch 22", "خوب"]);
        // Punctuation after the run's last word is not part of it.
        assert_eq!(pieces("کتاب Dune : خوب"), ["کتاب", "Dune", ":", "خوب"]);
        // Persian digits, and digits after a Persian word, stand alone.
        assert_eq!(pieces("چاپ ۲۰۲۴ کتاب"), ["چاپ", "۲۰۲۴", "کتاب"]);
        assert_eq!(pieces("چاپ 2024 کتاب"), ["چاپ", "2024", "کتاب"]);
    }

    #[test]
    fn empty_text_has_no_pieces() {
        assert!(pieces("").is_empty());
        assert!(pieces("   ").is_empty());
    }
}
