//! What "Update progress…" accepts, and what it means: a page, an audiobook
//! time or a percentage, turned into the position to write. Plain functions
//! without GPUI, so they can be unit tested directly.
//!
//! A percentage is converted here, with the book's length and the same rule
//! as Hardcover's other clients ([`Length::position_at`]), and written as a
//! page or a time: what the dialog shows beside the field is what is saved.
//! Without a known length a percentage can't be converted, so it isn't
//! offered.

use hardcover_core::{Length, Measure, ProgressUnit, ProgressUpdate};

use crate::format::{count_text, duration_text, parse_position, position_text};

/// What the dialog's field counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputUnit {
    Page,
    /// An audiobook position: "2:05", "2h 5m".
    Time,
    Percent,
}

impl InputUnit {
    /// The field's own unit for a book counted in `measure`: time for an
    /// audiobook, else pages (also when nothing is known).
    pub fn native(measure: Option<Measure>) -> Self {
        match measure.map(|m| m.unit) {
            Some(ProgressUnit::Time) => Self::Time,
            Some(ProgressUnit::Pages) | None => Self::Page,
        }
    }

    /// The segmented control's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Page => "Page",
            Self::Time => "Time",
            Self::Percent => "%",
        }
    }

    /// The field's placeholder: what to type.
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Page => "Page",
            Self::Time => "2:05 or 2h 5m",
            Self::Percent => "0–100",
        }
    }

    /// Whether `text` could become valid input as it is typed.
    pub fn accepts_typing(self, text: &str) -> bool {
        let (allowed, max): (fn(char) -> bool, usize) = match self {
            Self::Page => (|c| c.is_ascii_digit(), 7),
            Self::Percent => (
                |c| c.is_ascii_digit() || matches!(c, '.' | ',' | '%' | ' '),
                8,
            ),
            Self::Time => (
                |c| {
                    c.is_ascii_digit()
                        || matches!(c, ':' | '.' | ',' | ' ' | 'h' | 'm' | 's' | 'H' | 'M' | 'S')
                },
                16,
            ),
        };
        text.chars().count() <= max && text.chars().all(allowed)
    }
}

/// Why a percentage isn't offered: the book's length is unknown.
pub const PERCENT_NEEDS_LENGTH: &str =
    "Percent needs the edition's length, which Hardcover doesn't have.";

/// What `text`, typed in `unit`, puts the reader at in a book counted in
/// `measure`. The error says what to type instead.
pub fn parse(
    unit: InputUnit,
    measure: Option<Measure>,
    text: &str,
) -> Result<ProgressUpdate, String> {
    let length = measure.and_then(Measure::length);
    match unit {
        InputUnit::Page => match text.trim().parse::<u32>() {
            Err(_) => Err("Enter the page you're on.".to_owned()),
            Ok(page) => match length {
                Some(Length::Pages(total)) if page > total => {
                    Err(format!("This edition has {} pages.", count_text(total)))
                }
                _ => Ok(ProgressUpdate::Pages(page)),
            },
        },
        InputUnit::Time => match parse_time(text) {
            None => Err("Enter the time you're at, like 2:05 or 2h 5m.".to_owned()),
            Some(s) => match length {
                Some(Length::Seconds(total)) if s > total => {
                    Err(format!("This edition is {} long.", duration_text(total)))
                }
                _ => Ok(ProgressUpdate::Seconds(s)),
            },
        },
        InputUnit::Percent => {
            let Some(length) = length else {
                return Err(PERCENT_NEEDS_LENGTH.to_owned());
            };
            match parse_percent(text) {
                None => Err("Enter a percentage from 0 to 100.".to_owned()),
                Some(percent) => Ok(length.position_at(percent)),
            }
        }
    }
}

/// What the field's input means, for the muted text beside it: the total
/// ("of 476") for a page, the time read back ("2h 5m of 10h 0m") for an
/// audiobook, and the place a percentage stands for ("Page 200 of 476").
/// `None` when there is nothing to add. `exact` is the place `text` stands
/// for when the dialog filled it in, which a rounded percentage only
/// approximates.
pub fn preview(
    unit: InputUnit,
    measure: Option<Measure>,
    text: &str,
    exact: Option<ProgressUpdate>,
) -> Option<String> {
    let length = measure.and_then(Measure::length);
    match unit {
        InputUnit::Page => match length {
            Some(Length::Pages(total)) => Some(format!("of {}", count_text(total))),
            _ => None,
        },
        InputUnit::Time => {
            let total = match length {
                Some(Length::Seconds(total)) => Some(duration_text(total)),
                _ => None,
            };
            match (parse_time(text), total) {
                (Some(s), Some(total)) => Some(format!("{} of {total}", duration_text(s))),
                (Some(s), None) => Some(duration_text(s)),
                (None, Some(total)) => Some(format!("of {total}")),
                (None, None) => None,
            }
        }
        InputUnit::Percent => {
            let length = length?;
            let position = match exact {
                Some(position) => position,
                None => length.position_at(parse_percent(text)?),
            };
            Some(position_words(position, length))
        }
    }
}

/// "Page 200 of 476" or "3h 12m of 9h 40m".
fn position_words(position: ProgressUpdate, length: Length) -> String {
    match position {
        ProgressUpdate::Pages(p) => {
            format!("Page {} of {}", count_text(p), count_text(length.total()))
        }
        ProgressUpdate::Seconds(s) => {
            format!("{} of {}", duration_text(s), duration_text(length.total()))
        }
        ProgressUpdate::Percent(p) => format!("{}%", percent_text(f64::from(p))),
    }
}

/// `position` as the field shows it in `unit`, or `None` when it can't be
/// (a percentage of an unknown length, a page for an audiobook).
pub fn field_text(
    position: ProgressUpdate,
    unit: InputUnit,
    length: Option<Length>,
) -> Option<String> {
    match (position, unit) {
        (ProgressUpdate::Pages(p), InputUnit::Page) => Some(p.to_string()),
        (ProgressUpdate::Seconds(s), InputUnit::Time) => Some(position_text(s)),
        (ProgressUpdate::Percent(p), InputUnit::Percent) => Some(percent_text(f64::from(p))),
        (ProgressUpdate::Pages(done), InputUnit::Percent) => match length? {
            Length::Pages(total) => Some(percent_text(fraction(done, total))),
            Length::Seconds(_) => None,
        },
        (ProgressUpdate::Seconds(done), InputUnit::Percent) => match length? {
            Length::Seconds(total) => Some(percent_text(fraction(done, total))),
            Length::Pages(_) => None,
        },
        _ => None,
    }
}

/// Text the dialog put in the field, with the exact place it stands for.
#[derive(Debug, Clone, PartialEq)]
pub struct Filled {
    pub text: String,
    pub position: ProgressUpdate,
}

impl Filled {
    /// `position` as the field shows it in `unit`, or `None` when it can't.
    pub fn new(position: ProgressUpdate, unit: InputUnit, length: Option<Length>) -> Option<Self> {
        field_text(position, unit, length).map(|text| Self { text, position })
    }

    /// The exact place, while the field still holds `text` as filled in.
    pub fn exact(&self, text: &str) -> Option<ProgressUpdate> {
        (self.text == text).then_some(self.position)
    }
}

/// The place the field's `text` stands for: the one the dialog filled in
/// while it's untouched, so a converted percentage saved unchanged writes
/// the exact place it came from; else what was typed, parsed.
pub fn position_of(
    unit: InputUnit,
    measure: Option<Measure>,
    text: &str,
    filled: Option<&Filled>,
) -> Result<ProgressUpdate, String> {
    match filled.and_then(|f| f.exact(text)) {
        Some(position) => Ok(position),
        None => parse(unit, measure, text),
    }
}

fn fraction(done: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(done.min(total)) / f64::from(total) * 100.0
    }
}

/// A percentage for the field: "42", "65.5". Rounded down to a tenth, so a
/// place short of the end never reads as 100.
pub fn percent_text(percent: f64) -> String {
    let tenths = (percent.clamp(0.0, 100.0) * 10.0).floor() / 10.0;
    let text = format!("{tenths:.1}");
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
}

/// A number with at most one decimal separator, `.` or `,`: "42", "5.5",
/// "5,5". Only digits otherwise, so "nan", "1e2" and "+5" are refused, as
/// `f64::from_str` would accept them.
fn parse_decimal(text: &str) -> Option<f64> {
    let (whole, fraction) = match text.split_once(['.', ',']) {
        Some((w, f)) => (w, Some(f)),
        None => (text, None),
    };
    let digits = |s: &str| !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit());
    let ok = match fraction {
        // "5." and ".5" read as 5 and 0.5.
        Some(f) => {
            (whole.is_empty() || digits(whole))
                && (f.is_empty() || digits(f))
                && !(whole.is_empty() && f.is_empty())
        }
        None => digits(whole),
    };
    if !ok {
        return None;
    }
    let normal = format!(
        "{}.{}",
        if whole.is_empty() { "0" } else { whole },
        fraction.filter(|f| !f.is_empty()).unwrap_or("0")
    );
    normal.parse().ok()
}

/// A percentage typed as "42", "42%", "42.5" or "42,5", from 0 to 100.
pub fn parse_percent(text: &str) -> Option<f64> {
    let text = text.trim();
    let number = text.strip_suffix('%').map_or(text, str::trim_end);
    parse_decimal(number).filter(|p| (0.0..=100.0).contains(p))
}

/// An audiobook position, in seconds: "2:05" or "2:05:30" (h:mm, h:mm:ss),
/// or hours, minutes and seconds with their units, in that order and each
/// once: "5h", "5h 30m", "5h30m", "90m", "45s", "1h 2m 3s". An amount may
/// have a decimal part, with a point or a comma: "5.5h", "5,5h". A bare
/// number is refused: it could be any of them.
pub fn parse_time(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.contains(':') {
        return parse_position(text);
    }
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.as_str();
    let mut seconds = 0.0;
    // The units still allowed: each after the ones before it.
    let mut units: &[(char, f64)] = &[('h', 3600.0), ('m', 60.0), ('s', 1.0)];
    let mut any = false;
    while !rest.is_empty() {
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
            .unwrap_or(rest.len());
        let amount = parse_decimal(&rest[..end])?;
        rest = rest[end..].trim_start();
        let unit = rest.chars().next()?;
        let at = units.iter().position(|(u, _)| *u == unit)?;
        seconds += amount * units[at].1;
        units = &units[at + 1..];
        rest = rest[1..].trim_start();
        any = true;
    }
    if !any || !seconds.is_finite() || seconds > f64::from(u32::MAX) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked above: a finite, non-negative number of seconds within u32"
    )]
    let seconds = seconds.round() as u32;
    Some(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGES: Option<Measure> = Some(Measure {
        unit: ProgressUnit::Pages,
        total: Some(476),
    });
    const AUDIO: Option<Measure> = Some(Measure {
        unit: ProgressUnit::Time,
        total: Some(34_800),
    });
    const AUDIO_UNKNOWN: Option<Measure> = Some(Measure {
        unit: ProgressUnit::Time,
        total: None,
    });

    #[test]
    fn the_native_unit_follows_the_measure() {
        assert_eq!(InputUnit::native(PAGES), InputUnit::Page);
        assert_eq!(InputUnit::native(None), InputUnit::Page);
        assert_eq!(InputUnit::native(AUDIO), InputUnit::Time);
        assert_eq!(InputUnit::native(AUDIO_UNKNOWN), InputUnit::Time);
    }

    #[test]
    fn a_page_is_within_the_length() {
        assert_eq!(
            parse(InputUnit::Page, PAGES, " 120 "),
            Ok(ProgressUpdate::Pages(120))
        );
        assert_eq!(
            parse(InputUnit::Page, PAGES, "476"),
            Ok(ProgressUpdate::Pages(476))
        );
        assert_eq!(
            parse(InputUnit::Page, PAGES, "477"),
            Err("This edition has 476 pages.".to_owned())
        );
        assert_eq!(
            parse(InputUnit::Page, PAGES, "1:05"),
            Err("Enter the page you're on.".to_owned())
        );
        // Without a known length any page goes.
        assert_eq!(
            parse(InputUnit::Page, None, "999"),
            Ok(ProgressUpdate::Pages(999))
        );
    }

    #[test]
    fn a_time_is_within_the_length() {
        assert_eq!(
            parse(InputUnit::Time, AUDIO, "2:05"),
            Ok(ProgressUpdate::Seconds(7_500))
        );
        assert_eq!(
            parse(InputUnit::Time, AUDIO, "5h 30m"),
            Ok(ProgressUpdate::Seconds(19_800))
        );
        assert_eq!(
            parse(InputUnit::Time, AUDIO, "9h 41m"),
            Err("This edition is 9h 40m long.".to_owned())
        );
        assert_eq!(
            parse(InputUnit::Time, AUDIO, "125"),
            Err("Enter the time you're at, like 2:05 or 2h 5m.".to_owned())
        );
        // An audiobook without a running time takes any time.
        assert_eq!(
            parse(InputUnit::Time, AUDIO_UNKNOWN, "40h"),
            Ok(ProgressUpdate::Seconds(144_000))
        );
    }

    #[test]
    fn friendly_times() {
        assert_eq!(parse_time("5h"), Some(18_000));
        assert_eq!(parse_time("5h 30m"), Some(19_800));
        assert_eq!(parse_time("5h30m"), Some(19_800));
        assert_eq!(parse_time(" 5 h 30 m "), Some(19_800));
        assert_eq!(parse_time("90m"), Some(5_400));
        assert_eq!(parse_time("45s"), Some(45));
        assert_eq!(parse_time("1h 2m 3s"), Some(3_723));
        assert_eq!(parse_time("5H 30M"), Some(19_800));
        // A decimal point or comma.
        assert_eq!(parse_time("5.5h"), Some(19_800));
        assert_eq!(parse_time("5,5h"), Some(19_800));
        assert_eq!(parse_time(".5h"), Some(1_800));
        assert_eq!(parse_time("1.25m"), Some(75));
        // h:mm and h:mm:ss as before.
        assert_eq!(parse_time("2:05"), Some(7_500));
        assert_eq!(parse_time("0:45:30"), Some(2_730));
        // Refused: a bare number, units out of order or twice, junk.
        for text in [
            "",
            "125",
            "5",
            "30m 5h",
            "5h 5h",
            "5m 30s 2h",
            "h",
            "5x",
            "5h 30",
            "5..5h",
            "5.5.5h",
            "nan h",
            "inf h",
            "1e2h",
            "+5h",
            "-5h",
            "5 hours",
            "2:75",
            // Overflows u32 seconds.
            "9999999h",
            "1193047:00",
        ] {
            assert_eq!(parse_time(text), None, "{text:?}");
        }
    }

    #[test]
    fn percentages() {
        assert_eq!(parse_percent("42"), Some(42.0));
        assert_eq!(parse_percent("42%"), Some(42.0));
        assert_eq!(parse_percent(" 42 % "), Some(42.0));
        assert_eq!(parse_percent("42.5"), Some(42.5));
        assert_eq!(parse_percent("42,5"), Some(42.5));
        assert_eq!(parse_percent("0"), Some(0.0));
        assert_eq!(parse_percent("100"), Some(100.0));
        for text in [
            "", "%", "101", "100.1", "-1", "nan", "inf", "1e2", "+5", "4 2", "42%%", "4.2.1",
        ] {
            assert_eq!(parse_percent(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_percentage_becomes_a_page_or_a_time() {
        assert_eq!(
            parse(InputUnit::Percent, PAGES, "42"),
            Ok(ProgressUpdate::Pages(200))
        );
        assert_eq!(
            parse(InputUnit::Percent, PAGES, "100%"),
            Ok(ProgressUpdate::Pages(476))
        );
        // Short of 100% never reaches the end, which would finish the read.
        assert_eq!(
            parse(InputUnit::Percent, PAGES, "99.9"),
            Ok(ProgressUpdate::Pages(475))
        );
        assert_eq!(
            parse(InputUnit::Percent, AUDIO, "33.1"),
            Ok(ProgressUpdate::Seconds(11_519))
        );
        assert_eq!(
            parse(InputUnit::Percent, PAGES, "120"),
            Err("Enter a percentage from 0 to 100.".to_owned())
        );
        // Never a guess without a length.
        assert_eq!(
            parse(InputUnit::Percent, None, "42"),
            Err(PERCENT_NEEDS_LENGTH.to_owned())
        );
        assert_eq!(
            parse(InputUnit::Percent, AUDIO_UNKNOWN, "42"),
            Err(PERCENT_NEEDS_LENGTH.to_owned())
        );
    }

    #[test]
    fn the_preview_says_what_the_input_means() {
        assert_eq!(
            preview(InputUnit::Page, PAGES, "12", None).as_deref(),
            Some("of 476")
        );
        assert_eq!(preview(InputUnit::Page, None, "12", None), None);
        assert_eq!(
            preview(InputUnit::Percent, PAGES, "42", None).as_deref(),
            Some("Page 200 of 476")
        );
        assert_eq!(
            preview(InputUnit::Percent, AUDIO, "33.1", None).as_deref(),
            Some("3h 11m of 9h 40m")
        );
        assert_eq!(preview(InputUnit::Percent, PAGES, "abc", None), None);
        // A filled-in 39.6% stands for 6h 25m exactly, not the 6h 24m that
        // 39.6% of the length rounds to.
        let hail_mary = Some(Measure {
            unit: ProgressUnit::Time,
            total: Some(58_200),
        });
        assert_eq!(
            field_text(
                ProgressUpdate::Seconds(23_100),
                InputUnit::Percent,
                Some(Length::Seconds(58_200))
            )
            .as_deref(),
            Some("39.6")
        );
        assert_eq!(
            preview(
                InputUnit::Percent,
                hail_mary,
                "39.6",
                Some(ProgressUpdate::Seconds(23_100))
            )
            .as_deref(),
            Some("6h 25m of 16h 10m")
        );
        assert_eq!(
            preview(InputUnit::Percent, hail_mary, "39.6", None).as_deref(),
            Some("6h 24m of 16h 10m")
        );
    }

    #[test]
    fn switching_units_shows_the_same_place() {
        let pages = Some(Length::Pages(476));
        let audio = Some(Length::Seconds(34_800));
        let shown = |p, unit, length| field_text(p, unit, length);
        assert_eq!(
            shown(ProgressUpdate::Pages(312), InputUnit::Percent, pages).as_deref(),
            Some("65.5")
        );
        assert_eq!(
            shown(ProgressUpdate::Pages(476), InputUnit::Percent, pages).as_deref(),
            Some("100")
        );
        assert_eq!(
            shown(ProgressUpdate::Pages(200), InputUnit::Page, pages).as_deref(),
            Some("200")
        );
        assert_eq!(
            shown(ProgressUpdate::Seconds(11_520), InputUnit::Percent, audio).as_deref(),
            Some("33.1")
        );
        assert_eq!(
            shown(ProgressUpdate::Seconds(7_530), InputUnit::Time, audio).as_deref(),
            Some("2:05:30")
        );
        // Nothing to convert with.
        assert_eq!(
            shown(ProgressUpdate::Pages(312), InputUnit::Percent, None),
            None
        );
        assert_eq!(
            shown(ProgressUpdate::Pages(312), InputUnit::Percent, audio),
            None
        );
        assert_eq!(
            shown(ProgressUpdate::Pages(312), InputUnit::Time, pages),
            None
        );
        // A place short of the end never reads as 100.
        assert_eq!(percent_text(99.98), "99.9");
        assert_eq!(percent_text(42.0), "42");
    }

    #[test]
    fn a_converted_value_saved_unchanged_writes_the_exact_place() {
        let pages = Some(Length::Pages(476));
        let audio = Some(Length::Seconds(58_200));
        let hail_mary = Some(Measure {
            unit: ProgressUnit::Time,
            total: Some(58_200),
        });
        // 6h 25m in, switched to %: "39.6", which on its own is 6h 24m.
        let filled =
            Filled::new(ProgressUpdate::Seconds(23_100), InputUnit::Percent, audio).unwrap();
        assert_eq!(filled.text, "39.6");
        assert_eq!(
            position_of(InputUnit::Percent, hail_mary, "39.6", Some(&filled)),
            Ok(ProgressUpdate::Seconds(23_100))
        );
        assert_eq!(
            parse(InputUnit::Percent, hail_mary, "39.6"),
            Ok(ProgressUpdate::Seconds(23_047))
        );
        // Edited, it is what was typed.
        assert_eq!(
            position_of(InputUnit::Percent, hail_mary, "50", Some(&filled)),
            Ok(ProgressUpdate::Seconds(29_100))
        );
        // Switched back, the field shows the place it started from.
        let back = Filled::new(filled.position, InputUnit::Time, audio).unwrap();
        assert_eq!(back.text, "6:25");
        // A page and a percentage that can't hold it.
        assert_eq!(
            Filled::new(ProgressUpdate::Pages(312), InputUnit::Percent, None),
            None
        );
        let page = Filled::new(ProgressUpdate::Pages(312), InputUnit::Percent, pages).unwrap();
        assert_eq!(page.exact("65.5"), Some(ProgressUpdate::Pages(312)));
        assert_eq!(page.exact("65"), None);
    }

    #[test]
    fn typing_is_limited_to_the_units_characters() {
        assert!(InputUnit::Page.accepts_typing("120"));
        assert!(!InputUnit::Page.accepts_typing("12%"));
        assert!(InputUnit::Percent.accepts_typing("42,5 %"));
        assert!(!InputUnit::Percent.accepts_typing("42h"));
        assert!(InputUnit::Time.accepts_typing("5H 30m"));
        assert!(InputUnit::Time.accepts_typing("2:05:30"));
        assert!(!InputUnit::Time.accepts_typing("5 hours"));
    }
}
