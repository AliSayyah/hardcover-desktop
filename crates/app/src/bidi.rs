//! Text that may wrap and may be right-to-left, such as Persian or Arabic
//! titles, descriptions and reviews.
//!
//! GPUI's Linux text layout gets right-to-left text wrong in two ways:
//! - a wrapped right-to-left paragraph lands mostly left of its own box,
//!   behind the cover and off the pane;
//! - every text run is laid out as if the paragraph read left to right. A run
//!   of Persian letters reads correctly, but the punctuation, brackets and
//!   quotes at its edges land on the wrong side: `امید.` is drawn with the
//!   full stop on the right, before the word, and `(کتاب)` keeps its
//!   brackets unmirrored.
//!
//! So [`WrappingText`] lays right-to-left text out itself. The Unicode
//! bidirectional algorithm (`unicode-bidi`, with a right-to-left paragraph)
//! resolves each character's level; the text is cut into pieces at the
//! spaces where the line may break ([`pieces`]), and each piece into
//! segments that each read one way only: a run of right-to-left letters, a
//! left-to-right run ("Harry Potter", "2024"), or one neutral character
//! (`.`, `،`, `«`), mirrored where the algorithm mirrors it. The pieces and
//! their segments flow right to left in reversed flex rows, packed to the
//! right as right-to-left text reads. They stay in reading order in the
//! element tree, so assistive technology reads them in order.
//!
//! Left-to-right text takes GPUI's own path, unless the width is known
//! ([`WrappingText::width`]): then both directions are broken into lines here
//! ([`break_lines`]), between words only, the last line ending in "…" when a
//! clamp cuts the rest. GPUI's own wrapping in a narrow box starts the next
//! line with the space it broke at, cuts a word that doesn't fit between
//! letters, and lets a clamped last line run past its box.
//!
//! [`WrappingText`] also names the Persian and Arabic fallback, Vazirmatn,
//! for the family it inherits. A type role does that too, but dialogs and
//! popovers don't inherit from the shell, and GPUI's own fallback would pick
//! a system font.

use std::collections::VecDeque;

use gpui_kit::{
    AnyElement, App, FontFallbacks, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use unicode_bidi::{BidiClass, BidiInfo, Direction, Level, bidi_class, get_base_direction};

use crate::design::type_roles::Family;

/// What ends a line cut short by a clamp.
const ELLIPSIS: &str = "…";

/// A paragraph of text that wraps, in either direction. Font, size and color
/// come from the parent, as for a plain string child.
#[derive(IntoElement)]
pub struct WrappingText {
    text: SharedString,
    line_clamp: Option<usize>,
    centered: bool,
    width: Option<Pixels>,
    min_scale: Option<f32>,
}

impl WrappingText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            line_clamp: None,
            centered: false,
            width: None,
            min_scale: None,
        }
    }

    /// Centres each line, as on a cloth cover. Left-to-right text without a
    /// [`width`](Self::width) also needs the parent's `text_center`.
    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    /// Shows at most `lines` lines. Without a [`width`](Self::width),
    /// left-to-right text ends the last one with an ellipsis (from the
    /// parent's `text_ellipsis`) and right-to-left text is cut at the last
    /// line; with one, the last line ends in "…" at a word.
    pub fn line_clamp(mut self, lines: usize) -> Self {
        self.line_clamp = Some(lines);
        self
    }

    /// Breaks the lines here, for a box this wide (a cover's title): between
    /// words only, so a line never starts with a space or with the
    /// punctuation that follows a word. A word wider than the line on its own
    /// is the one thing cut between letters; see [`Self::fit_words`].
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    /// With a [`width`](Self::width): sets the text smaller, down to
    /// `min_scale` of its size, until its longest word fits on a line, so
    /// the word isn't cut. The line height scales with it.
    pub fn fit_words(mut self, min_scale: f32) -> Self {
        self.min_scale = Some(min_scale);
        self
    }
}

impl RenderOnce for WrappingText {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        // The parent's text style is in effect here.
        let mut style = window.text_style();
        let fallback = Family::of(&style.font_family).script_fallback();
        let fallbacks = FontFallbacks::from_fonts(vec![fallback.to_owned()]);
        style.font_fallbacks = Some(fallbacks.clone());
        let mut root = div();
        root.text_style().font_fallbacks = Some(fallbacks);
        let rtl = is_rtl(&self.text);

        if let Some(width) = self.width {
            let rem = window.rem_size();
            let mut font_size = style.font_size.to_pixels(rem);
            let mut line_height = style.line_height_in_pixels(rem);
            let units = if rtl {
                pieces(&self.text).iter().map(Piece::text).collect()
            } else {
                ltr_units(&self.text)
            };
            let measure = |text: &str, size: Pixels, window: &mut Window| -> f32 {
                let run = style.to_run(text.len());
                f32::from(
                    window
                        .text_system()
                        .shape_line(text.to_owned().into(), size, &[run], None)
                        .width,
                )
            };
            if let Some(min_scale) = self.min_scale {
                let widest = units
                    .iter()
                    .map(|u| measure(u, font_size, window))
                    .fold(0., f32::max);
                if widest > f32::from(width) {
                    let scale = (f32::from(width) / widest).max(min_scale);
                    font_size *= scale;
                    line_height *= scale;
                    root = root.text_size(font_size).line_height(line_height);
                }
            }
            let lines = break_lines(&units, f32::from(width), self.line_clamp, &mut |text| {
                measure(text, font_size, window)
            });
            let centered = self.centered;
            return root
                .flex()
                .flex_col()
                .children(lines.into_iter().map(move |line| -> AnyElement {
                    if rtl {
                        rtl_row(pieces(&line), centered, false).into_any_element()
                    } else {
                        div()
                            .flex()
                            .when(centered, |this| this.justify_center())
                            .whitespace_nowrap()
                            .child(line)
                            .into_any_element()
                    }
                }))
                .into_any_element();
        }

        if !rtl {
            return root
                .when_some(self.line_clamp, |this, lines| this.line_clamp(lines))
                .child(self.text)
                .into_any_element();
        }
        // The height of one line of this text.
        let line_height = style.line_height_in_pixels(window.rem_size());
        root.child(
            rtl_row(pieces(&self.text), self.centered, true)
                .when_some(self.line_clamp, |this, lines| {
                    this.max_h(line_height * lines as f32).overflow_hidden()
                }),
        )
        .into_any_element()
    }
}

/// Right-to-left pieces in a reversed row, packed to the right (or
/// centred), wrapping onto more lines when `wrap`.
fn rtl_row(pieces: Vec<Piece>, centered: bool, wrap: bool) -> gpui_kit::Div {
    let last = pieces.len().saturating_sub(1);
    div()
        .flex()
        .flex_row_reverse()
        .when(wrap, |this| this.flex_wrap())
        .when(centered, |this| this.justify_center())
        .children(pieces.into_iter().enumerate().map(move |(i, piece)| {
            // The space goes on the piece's left, where the next piece
            // follows.
            div()
                .flex()
                .flex_row_reverse()
                .children(piece.segments)
                .when(i < last, |this| this.child(" "))
        }))
}

/// Whether `text` reads right to left: its first strong character does.
pub fn is_rtl(text: &str) -> bool {
    get_base_direction(text) == Direction::Rtl
}

/// A piece of right-to-left text that a line never breaks inside: a word
/// with the punctuation around it, or a left-to-right phrase. Its segments
/// are in reading order, and each reads one way only, so GPUI draws it
/// correctly however it lays out a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub segments: Vec<String>,
    /// The piece's own text, before mirroring: what lines are built from,
    /// so that laying a line out runs the algorithm once.
    pub source: String,
}

impl Piece {
    /// The piece's text as written (for measuring and building lines).
    fn text(&self) -> String {
        self.source.clone()
    }

    /// Whether the piece has no letter or digit: punctuation standing alone,
    /// which stays with the word before it.
    fn is_punctuation(&self) -> bool {
        !self
            .segments
            .iter()
            .flat_map(|s| s.chars())
            .any(char::is_alphanumeric)
    }
}

/// The pieces right-to-left `text` is laid out in, in reading order. The
/// line may break only at a space the bidirectional algorithm puts in the
/// right-to-left paragraph; spaces inside a left-to-right run ("Harry
/// Potter", "Catch 22") keep it together. Punctuation standing alone after a
/// space joins the piece before it, so a line never starts with it.
///
/// Inside a piece, each run of right-to-left letters is one segment, each
/// left-to-right run is one, and every other character (punctuation, quotes,
/// brackets) is a segment of its own, mirrored where right-to-left text
/// mirrors it: laid out alone, GPUI would draw it as if the line read left
/// to right.
pub fn pieces(text: &str) -> Vec<Piece> {
    let info = BidiInfo::new(text, Some(Level::rtl()));
    let mut pieces: Vec<Piece> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut source = String::new();
    // The segment being built: its text and whether it reads left to right.
    let mut segment = String::new();
    let mut segment_ltr = false;

    let flush_segment = |segment: &mut String, current: &mut Vec<String>| {
        if !segment.is_empty() {
            current.push(std::mem::take(segment));
        }
    };
    let flush_piece = |current: &mut Vec<String>, source: &mut String, pieces: &mut Vec<Piece>| {
        let source = std::mem::take(source);
        if current.is_empty() {
            return;
        }
        let piece = Piece {
            segments: std::mem::take(current),
            source,
        };
        match pieces.last_mut() {
            Some(previous) if piece.is_punctuation() => {
                previous.segments.push(" ".to_owned());
                previous.segments.extend(piece.segments);
                previous.source.push(' ');
                previous.source.push_str(&piece.source);
            }
            _ => pieces.push(piece),
        }
    };

    for (ix, c) in text.char_indices() {
        let level = info.levels[ix];
        let class = bidi_class(c);
        if level.is_rtl() && (c.is_whitespace() || class == BidiClass::B) {
            flush_segment(&mut segment, &mut current);
            flush_piece(&mut current, &mut source, &mut pieces);
            continue;
        }
        source.push(c);
        if level.is_ltr() {
            if !segment_ltr {
                flush_segment(&mut segment, &mut current);
                segment_ltr = true;
            }
            segment.push(c);
            continue;
        }
        let letter = matches!(
            class,
            BidiClass::R | BidiClass::AL | BidiClass::NSM | BidiClass::BN
        );
        if letter && (segment_ltr || !segment_is_letters(&segment)) {
            flush_segment(&mut segment, &mut current);
        }
        segment_ltr = false;
        if letter {
            segment.push(c);
        } else {
            flush_segment(&mut segment, &mut current);
            current.push(
                unicode_bidi_mirroring::get_mirrored(c)
                    .unwrap_or(c)
                    .to_string(),
            );
        }
    }
    flush_segment(&mut segment, &mut current);
    flush_piece(&mut current, &mut source, &mut pieces);
    pieces
}

/// Whether the right-to-left segment being built is a run of letters (empty
/// counts: a new one starts there).
fn segment_is_letters(segment: &str) -> bool {
    segment.chars().all(|c| {
        matches!(
            bidi_class(c),
            BidiClass::R | BidiClass::AL | BidiClass::NSM | BidiClass::BN
        )
    })
}

/// Left-to-right text's words, the units its lines break between.
/// Punctuation standing alone ("Dune – Messiah") stays with the word before
/// it, so a line never starts with it.
fn ltr_units(text: &str) -> Vec<String> {
    let mut units: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match units.last_mut() {
            Some(previous) if !word.chars().any(char::is_alphanumeric) => {
                previous.push(' ');
                previous.push_str(word);
            }
            _ => units.push(word.to_owned()),
        }
    }
    units
}

/// The lines `units` (words, joined by a space) take in `width`, as
/// `measure` sets a line. A line breaks between units only, except that a
/// unit wider than a line on its own is cut between characters. With
/// `max_lines`, the text beyond them is dropped and the last line ends in
/// "…", at a unit when one fits with it.
pub fn break_lines(
    units: &[String],
    width: f32,
    max_lines: Option<usize>,
    measure: &mut dyn FnMut(&str) -> f32,
) -> Vec<String> {
    // Half a pixel of slack for rounding in the shaper.
    let fits = |text: &str, measure: &mut dyn FnMut(&str) -> f32| measure(text) <= width + 0.5;
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut line: Vec<String> = Vec::new();
    let mut queue: VecDeque<String> = units.iter().cloned().collect();
    while let Some(unit) = queue.pop_front() {
        let mut candidate = line.clone();
        candidate.push(unit.clone());
        if fits(&candidate.join(" "), measure) {
            line = candidate;
            continue;
        }
        if line.is_empty() {
            let (head, tail) = cut_to_fit(&unit, width, measure);
            lines.push(vec![head]);
            if !tail.is_empty() {
                queue.push_front(tail);
            }
        } else {
            lines.push(std::mem::take(&mut line));
            queue.push_front(unit);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }

    let Some(max) = max_lines.filter(|max| lines.len() > *max && *max > 0) else {
        return lines.into_iter().map(|l| l.join(" ")).collect();
    };
    lines.truncate(max);
    let mut last = lines.pop().unwrap_or_default();
    let ended = loop {
        let joined = last.join(" ");
        let text = trim_for_ellipsis(&joined);
        let ended = format!("{text}{ELLIPSIS}");
        if fits(&ended, measure) {
            break ended;
        }
        if last.len() > 1 {
            last.pop();
            continue;
        }
        // One unit too wide even alone with the ellipsis: cut it.
        let mut chars: Vec<char> = text.chars().collect();
        loop {
            chars.pop();
            let ended = format!(
                "{}{ELLIPSIS}",
                trim_for_ellipsis(&chars.iter().collect::<String>())
            );
            if chars.is_empty() || fits(&ended, measure) {
                break;
            }
        }
        break format!(
            "{}{ELLIPSIS}",
            trim_for_ellipsis(&chars.iter().collect::<String>())
        );
    };
    let mut lines: Vec<String> = lines.into_iter().map(|l| l.join(" ")).collect();
    lines.push(ended);
    lines
}

/// The text before an ellipsis, without the spaces and punctuation that
/// would sit against it ("Tomorrow, and,…").
fn trim_for_ellipsis(text: &str) -> &str {
    text.trim_end_matches(|c: char| {
        c.is_whitespace() || c.is_ascii_punctuation() || matches!(c, '،' | '؛')
    })
}

/// A unit too wide for a line, cut into the longest head that fits and the
/// rest. The head has at least one character, and a combining mark stays
/// with the letter before it.
fn cut_to_fit(unit: &str, width: f32, measure: &mut dyn FnMut(&str) -> f32) -> (String, String) {
    let mut end = 0;
    for (ix, c) in unit.char_indices().skip(1) {
        if bidi_class(c) == BidiClass::NSM {
            continue;
        }
        if measure(&unit[..ix]) > width + 0.5 {
            break;
        }
        end = ix;
    }
    if end == 0 {
        // Even the first character doesn't fit: it goes alone.
        end = unit
            .char_indices()
            .skip(1)
            .find(|(_, c)| bidi_class(*c) != BidiClass::NSM)
            .map_or(unit.len(), |(ix, _)| ix);
    }
    // Everything fits up to the last character: take it all.
    if measure(unit) <= width + 0.5 {
        end = unit.len();
    }
    (unit[..end].to_owned(), unit[end..].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pieces' segments, for comparing.
    fn segs(text: &str) -> Vec<Vec<String>> {
        pieces(text).into_iter().map(|p| p.segments).collect()
    }

    fn v(pieces: &[&[&str]]) -> Vec<Vec<String>> {
        pieces
            .iter()
            .map(|p| p.iter().map(|s| (*s).to_owned()).collect())
            .collect()
    }

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
        assert_eq!(segs("کتاب   خوب\nبد"), v(&[&["کتاب"], &["خوب"], &["بد"]]));
    }

    #[test]
    fn a_full_stop_is_its_own_segment_after_the_word() {
        // In reading order: the word, then the stop. The reversed row puts
        // the stop on the word's left, where a right-to-left line ends.
        assert_eq!(
            segs("پر از امید. دوستی"),
            v(&[&["پر"], &["از"], &["امید", "."], &["دوستی"]])
        );
        assert_eq!(segs("زمین بگذارم."), v(&[&["زمین"], &["بگذارم", "."]]));
    }

    #[test]
    fn the_persian_comma_stands_apart() {
        assert_eq!(bidi_class('،'), BidiClass::CS);
        // The Arabic semicolon is a strong right-to-left character, like a
        // letter, so it stays in the run.
        assert_eq!(bidi_class('؛'), BidiClass::AL);
        assert_eq!(segs("خوب، و بد؛"), v(&[&["خوب", "،"], &["و"], &["بد؛"]]));
    }

    #[test]
    fn the_arabic_question_mark_is_a_letter_of_the_run() {
        // It is a strong right-to-left character, so GPUI draws it in place.
        assert_eq!(bidi_class('؟'), BidiClass::AL);
        assert_eq!(segs("چرا؟"), v(&[&["چرا؟"]]));
        // A Latin question mark is neutral.
        assert_eq!(segs("چرا?"), v(&[&["چرا", "?"]]));
    }

    #[test]
    fn brackets_and_quotes_are_mirrored() {
        // Logical "(" comes first, on the right, drawn as ")".
        assert_eq!(segs("(کتاب)"), v(&[&[")", "کتاب", "("]]));
        assert_eq!(segs("«سلام»"), v(&[&["»", "سلام", "«"]]));
        // Around a Latin title, too.
        assert_eq!(
            segs("کتاب (Dune) خوب"),
            v(&[&["کتاب"], &[")", "Dune", "("], &["خوب"]])
        );
    }

    #[test]
    fn digits_are_their_own_left_to_right_segment() {
        assert_eq!(segs("چاپ ۲۰۲۴ کتاب"), v(&[&["چاپ"], &["۲۰۲۴"], &["کتاب"]]));
        assert_eq!(segs("صفحه 12."), v(&[&["صفحه"], &["12", "."]]));
        assert_eq!(segs("سال ۱۴۰۳."), v(&[&["سال"], &["۱۴۰۳", "."]]));
        // A number with its separators stays one run.
        assert_eq!(segs("۱٬۲۰۰ صفحه"), v(&[&["۱٬۲۰۰"], &["صفحه"]]));
        assert_eq!(segs("1,200 صفحه"), v(&[&["1,200"], &["صفحه"]]));
    }

    #[test]
    fn a_zero_width_non_joiner_stays_inside_its_word() {
        assert_eq!(
            segs("نمی\u{200c}خوریم امروز"),
            v(&[&["نمی\u{200c}خوریم"], &["امروز"]])
        );
    }

    #[test]
    fn a_latin_run_stays_together_in_reading_order() {
        assert_eq!(
            segs("راهنمای Harry Potter برای خوانندگان"),
            v(&[&["راهنمای"], &["Harry Potter"], &["برای"], &["خوانندگان"]])
        );
        // A full stop after a Latin word at the end of a right-to-left
        // sentence belongs to the sentence: it goes on the left.
        assert_eq!(segs("کتاب Dune."), v(&[&["کتاب"], &["Dune", "."]]));
    }

    #[test]
    fn numbers_after_a_latin_word_join_its_run() {
        assert_eq!(
            segs("کتاب Catch 22 Again خوب"),
            v(&[&["کتاب"], &["Catch 22 Again"], &["خوب"]])
        );
        assert_eq!(
            segs("کتاب Catch 22 خوب"),
            v(&[&["کتاب"], &["Catch 22"], &["خوب"]])
        );
    }

    #[test]
    fn punctuation_standing_alone_stays_with_the_word_before() {
        assert_eq!(
            segs("کتاب Dune : خوب"),
            v(&[&["کتاب"], &["Dune", " ", ":"], &["خوب"]])
        );
        assert_eq!(segs("اول - دوم"), v(&[&["اول", " ", "-"], &["دوم"]]));
        assert_eq!(
            ltr_units("Dune – Messiah"),
            ["Dune –".to_owned(), "Messiah".to_owned()]
        );
    }

    #[test]
    fn empty_text_has_no_pieces() {
        assert!(pieces("").is_empty());
        assert!(pieces("   ").is_empty());
        assert!(ltr_units("  ").is_empty());
    }

    /// One unit of width per character.
    fn chars(text: &str) -> f32 {
        text.chars().count() as f32
    }

    fn lines(text: &str, width: f32, max: Option<usize>) -> Vec<String> {
        break_lines(&ltr_units(text), width, max, &mut chars)
    }

    #[test]
    fn lines_break_between_words_with_no_leading_space() {
        assert_eq!(
            lines("Tomorrow, and Tomorrow, and Tomorrow", 13., None),
            ["Tomorrow, and", "Tomorrow, and", "Tomorrow"]
        );
        for line in lines("Tomorrow, and Tomorrow, and Tomorrow", 9., None) {
            assert!(!line.starts_with([' ', ',']), "{line:?}");
        }
    }

    #[test]
    fn a_clamp_ends_the_last_line_at_a_word_with_an_ellipsis() {
        assert_eq!(
            lines("Tomorrow, and Tomorrow, and Tomorrow", 9., Some(2)),
            ["Tomorrow,", "and…"]
        );
        // The punctuation before the ellipsis goes.
        assert_eq!(
            lines("Tomorrow, and Tomorrow, and Tomorrow", 10., Some(1)),
            ["Tomorrow…"]
        );
        // Text that fits keeps its lines and gets no ellipsis.
        assert_eq!(lines("Fourth Wing", 6., Some(2)), ["Fourth", "Wing"]);
    }

    #[test]
    fn a_word_wider_than_the_line_is_the_only_cut() {
        assert_eq!(
            lines("Hitchhiker's Guide", 8., None),
            ["Hitchhik", "er's", "Guide"]
        );
        // With a clamp, the cut word still ends in an ellipsis.
        assert_eq!(lines("Supercalifragilistic", 6., Some(1)), ["Super…"]);
    }

    #[test]
    fn a_line_built_from_pieces_is_mirrored_once() {
        // The width path joins the pieces into lines, then lays each line
        // out again: the brackets must come out as from the first pass.
        let text = "کتاب (جلد اول) «سلام»";
        let units: Vec<String> = pieces(text).iter().map(Piece::text).collect();
        assert_eq!(segs(&units.join(" ")), segs(text));
        assert_eq!(
            segs(text),
            v(&[&["کتاب"], &[")", "جلد"], &["اول", "("], &["»", "سلام", "«"]])
        );
    }

    #[test]
    fn right_to_left_pieces_break_into_lines_too() {
        let units: Vec<String> = pieces("کتابی پرکشش و پر از امید.")
            .iter()
            .map(Piece::text)
            .collect();
        let lines = break_lines(&units, 11., Some(2), &mut chars);
        assert_eq!(lines, ["کتابی پرکشش", "و پر از…"]);
        // The ellipsis is a neutral segment of its own, on the line's left.
        assert_eq!(segs(&lines[1]), v(&[&["و"], &["پر"], &["از", "…"]]));
    }
}
