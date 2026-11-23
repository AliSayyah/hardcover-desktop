//! The demo reader's followed series: seeded from the catalogue's `follows`
//! (a series by name, followed a number of days ago), changed by
//! `FollowSeries` and `UnfollowSeries` as Hardcover changes them (verified
//! live, 2026-11-23): following a series already followed returns the
//! existing follow, an unknown series is an `error`, and unfollowing one not
//! followed succeeds.

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use super::Catalogue;
use super::state::State;

/// A followed series in `catalogue.json`.
#[derive(Deserialize)]
pub(super) struct SeedFollow {
    series: String,
    days_ago: i64,
}

/// A `follows` row of a series.
pub(super) struct Follow {
    pub(super) id: i64,
    pub(super) series: i64,
    pub(super) at: DateTime<Utc>,
}

/// The ids the demo gives follows, below the rows it creates.
const FIRST_FOLLOW_ID: i64 = 54_000;

pub(super) fn seed(catalogue: &Catalogue, now: DateTime<Utc>) -> Vec<Follow> {
    let mut id = FIRST_FOLLOW_ID;
    let mut follows: Vec<(i64, DateTime<Utc>)> = catalogue
        .follows
        .iter()
        .filter_map(|f| {
            let series = catalogue.series_id(&f.series);
            (series != 0).then(|| (series, now - Duration::days(f.days_ago)))
        })
        .collect();
    // Rows are numbered in the order they were followed, as Hardcover's are.
    follows.sort_by_key(|(_, at)| *at);
    follows
        .into_iter()
        .map(|(series, at)| {
            id += 1;
            Follow { id, series, at }
        })
        .collect()
}

/// `FollowedSeries`: a keyset page of the reader's follows.
pub(super) fn page(state: &State, mine: bool, after: i64, limit: usize) -> Value {
    let rows: Vec<Value> = if mine {
        state
            .follows
            .iter()
            .filter(|f| f.id > after)
            .take(limit)
            .map(row)
            .collect()
    } else {
        Vec::new()
    };
    json!({ "follows": rows })
}

fn row(f: &Follow) -> Value {
    json!({
        "id": f.id,
        "followable_id": f.series,
        // Hardcover's `timestamp`: UTC without an offset.
        "created_at": f.at.naive_utc().format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
        "series": { "id": f.series, "canonical_id": null },
    })
}

/// `FollowSeries`.
pub(super) fn follow(state: &mut State, series: i64) -> Value {
    if !super::CATALOGUE
        .series()
        .iter()
        .any(|(id, _)| *id == series)
    {
        return json!({ "insert_follow": {
            "id": null,
            "error": format!("Series with id {series} does not exist"),
            "follow": null,
        }});
    }
    if !state.follows.iter().any(|f| f.series == series) {
        let at = state.clock.now().with_timezone(&Utc);
        let id = state.new_id();
        state.follows.push(Follow { id, series, at });
    }
    let follow = state.follows.iter().find(|f| f.series == series).map(row);
    json!({ "insert_follow": {
        "id": follow.as_ref().map(|f| f["id"].clone()),
        "error": null,
        "follow": follow,
    }})
}

/// `UnfollowSeries`.
pub(super) fn unfollow(state: &mut State, series: i64) -> Value {
    state.follows.retain(|f| f.series != series);
    json!({ "delete_follow": { "success": true, "error": null } })
}
