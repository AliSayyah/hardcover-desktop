//! Vibes: Hardcover's lists of recommended books (`hardcover-design`,
//! Discover and A vibe's page). The user's Recommendations ("For you") and
//! Top Picks lead Discover ([`shelves`]); each, and any vibe Hardcover
//! features, has its page ([`vibe_view`]), where "Never recommend…" leaves a
//! book out of the user's own ones.
//!
//! Hardcover builds a user's vibes only when they open them (or Discover) on
//! hardcover.app, at most once a day; no API call does. So the page says how
//! old the list is, and past a week links to hardcover.app, where a visit
//! builds it again ([`facts_line`], `Vibe::web_url`).

pub mod model;
pub mod shelves;
pub mod vibe_view;

use chrono::TimeZone;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::{App, SharedString, Window};
use hardcover_core::{BookId, Date, Timestamp, Vibe, VibeId, VibeKind};

use self::model::VibesModel;
use crate::notify::notify_failure;
use crate::ui::kit::open_alert_dialog;

/// A list older than this many days links to hardcover.app, where a visit
/// builds it again.
pub const STALE_AFTER_DAYS: i64 = 7;

/// The vibe's name as the app shows it: "For you" for Hardcover's
/// Recommendations, "Top picks" for its Top Picks, else its own.
pub fn vibe_name(vibe: &Vibe) -> String {
    match vibe.kind {
        VibeKind::Recommendations => "For you".to_owned(),
        VibeKind::TopPicks => "Top picks".to_owned(),
        _ => vibe.title.clone(),
    }
}

/// The muted intro under the vibe's name: the app's own words for the
/// user's system vibes, a featured vibe's own description.
pub fn vibe_intro(vibe: &Vibe) -> Option<String> {
    match vibe.kind {
        VibeKind::Recommendations => Some("Hardcover's recommendations from your library.".into()),
        VibeKind::TopPicks => Some("Popular books Hardcover picked for your taste.".into()),
        _ => vibe.description.clone(),
    }
}

/// Whole days between the local day `built` fell on in `zone` and `today`.
fn days_since_in<Tz: TimeZone>(built: Timestamp, today: Date, zone: &Tz) -> i64 {
    (today - crate::dates::day_in(built, zone))
        .num_days()
        .max(0)
}

/// Whole local days since `built`.
pub fn days_since(built: Timestamp, today: Date) -> i64 {
    days_since_in(built, today, &chrono::Local)
}

/// "Updated today", "Updated yesterday", "Updated 3 days ago".
pub fn age_text(days: i64) -> String {
    match days {
        ..=0 => "Updated today".to_owned(),
        1 => "Updated yesterday".to_owned(),
        n => format!(
            "Updated {} days ago",
            crate::format::count_text(u64::try_from(n).unwrap_or(0))
        ),
    }
}

/// Whether a list built `days` ago is old enough to send the reader to
/// hardcover.app, which builds it again.
pub fn is_stale(days: i64) -> bool {
    days > STALE_AFTER_DAYS
}

/// The page's facts line: "250 books · Updated today", and past a week why
/// it is old.
pub fn facts_line(books: usize, days: i64) -> String {
    let count = crate::format::books_count_text(u32::try_from(books).unwrap_or(u32::MAX));
    let mut line = format!("{count} · {}", age_text(days));
    if is_stale(days) {
        line.push_str(". Hardcover updates this list when you open it on hardcover.app.");
    }
    line
}

/// "Never recommend “<title>”?": asks first, since it can't be undone from
/// the app, then leaves `book` out of `vibe`. Once Hardcover accepts it the
/// book is hidden from every vibe (the row leaves); a failure is a
/// notification.
pub fn confirm_never_recommend(
    vibe: VibeId,
    book: BookId,
    title: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(model) = VibesModel::global(cx) else {
        return;
    };
    let heading: SharedString = format!("Never recommend “{title}”?").into();
    open_alert_dialog(window, cx, move |alert, _, _| {
        let model = model.clone();
        alert
            .title(heading.clone())
            .description(
                "Hardcover will leave it out of your recommendations. You can't undo this \
                 from this app.",
            )
            .confirm()
            .ok_text("Never recommend")
            // Nothing is destroyed: the result is primary, Cancel has the focus.
            .ok_variant(ButtonVariant::Primary)
            .on_ok(move |_, window, cx| {
                let task = model.update(cx, |m, cx| m.exclude(vibe, book, cx));
                notify_failure(task, "Couldn't update your recommendations", window, cx);
                true
            })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn the_age_counts_local_days() {
        // Tehran, UTC+03:30.
        let tehran = FixedOffset::east_opt(3 * 3600 + 1800).unwrap();
        let today: Date = "2026-11-23".parse().unwrap();
        // 00:10 local today is 20:40 UTC yesterday: still today here.
        assert_eq!(days_since_in(at("2026-11-22T20:40:00Z"), today, &tehran), 0);
        // 23:50 local yesterday.
        assert_eq!(days_since_in(at("2026-11-22T20:20:00Z"), today, &tehran), 1);
        assert_eq!(days_since_in(at("2026-11-14T12:00:00Z"), today, &tehran), 9);
        // A build "after" today (a clock set back) reads as today.
        assert_eq!(days_since_in(at("2026-11-25T12:00:00Z"), today, &tehran), 0);
    }

    #[test]
    fn the_age_line_names_the_days() {
        assert_eq!(age_text(0), "Updated today");
        assert_eq!(age_text(1), "Updated yesterday");
        assert_eq!(age_text(3), "Updated 3 days ago");
        assert_eq!(facts_line(250, 0), "250 books · Updated today");
        assert_eq!(facts_line(1, 1), "1 book · Updated yesterday");
        // Up to a week, only the age; from the eighth day, why.
        assert!(!is_stale(7));
        assert_eq!(facts_line(35, 7), "35 books · Updated 7 days ago");
        assert!(is_stale(8));
        assert_eq!(
            facts_line(250, 9),
            "250 books · Updated 9 days ago. Hardcover updates this list when you open it \
             on hardcover.app."
        );
    }

    #[test]
    fn system_vibes_take_the_apps_names() {
        let mut vibe = Vibe::new(VibeId::new(1), VibeKind::Recommendations, "Recommendations");
        vibe.description = Some("Books picked for you based on your reading history.".into());
        assert_eq!(vibe_name(&vibe), "For you");
        assert_eq!(
            vibe_intro(&vibe).as_deref(),
            Some("Hardcover's recommendations from your library.")
        );
        let mut featured = Vibe::new(VibeId::new(2), VibeKind::Custom, "Worlds to get lost in");
        featured.description = Some("Long fantasy.".into());
        assert_eq!(vibe_name(&featured), "Worlds to get lost in");
        assert_eq!(vibe_intro(&featured).as_deref(), Some("Long fantasy."));
    }
}
