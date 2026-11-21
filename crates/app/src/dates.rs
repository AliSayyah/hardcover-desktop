//! Calendar days on this computer. Hardcover's timestamps are UTC; a reader
//! in Tehran who updates a book at 1 a.m. did it on that day, not the one
//! before, so every timestamp that becomes a day ("Paused since", "Last
//! updated 8 days ago", "Updated yesterday") goes through [`local_day`], and
//! every "today" is [`today`].
//!
//! Views that read today (Home, Series, Stats, Coming soon) key their caches
//! on it, and the workspace's clock tells them when it changes
//! (`Workspace::day_changed`), so a window left open across midnight moves
//! to the new day.

use chrono::{DateTime, Local, TimeZone, Utc};
use hardcover_core::{Date, Timestamp};

/// Today on this computer.
pub fn today() -> Date {
    Local::now().date_naive()
}

/// Now on this computer, for times of day and relative times.
pub fn now() -> DateTime<Local> {
    Local::now()
}

/// The day `at` falls on in this computer's time zone.
pub fn local_day(at: Timestamp) -> Date {
    day_in(at, &Local)
}

/// The day `at` falls on in `zone`.
pub fn day_in<Tz: TimeZone>(at: DateTime<Utc>, zone: &Tz) -> Date {
    at.with_timezone(zone).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn a_timestamp_falls_on_the_day_of_its_time_zone() {
        let tehran = FixedOffset::east_opt(3 * 3600 + 1800).unwrap();
        // 22:00 UTC on Jun 1 is 01:30 on Jun 2 in Tehran.
        let late: Timestamp = "2024-06-01T22:00:00Z".parse().unwrap();
        assert_eq!(day_in(late, &tehran), "2024-06-02".parse::<Date>().unwrap());
        assert_eq!(day_in(late, &Utc), "2024-06-01".parse::<Date>().unwrap());
        // And west of UTC, the day before.
        let early: Timestamp = "2024-06-02T02:00:00Z".parse().unwrap();
        let new_york = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(
            day_in(early, &new_york),
            "2024-06-01".parse::<Date>().unwrap()
        );
    }
}
