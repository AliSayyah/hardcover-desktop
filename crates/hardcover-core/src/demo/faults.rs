//! Faults the demo can inject, from `HARDCOVER_DEMO_FAULTS`, so loading,
//! error, rate-limit and missing-scope states can be seen without breaking a
//! real connection.
//!
//! The value is a comma-separated list:
//! - `latency=800ms` (or `2s`): every request waits that long first.
//! - What fails: `offline` (no connection; covers fail too), `timeout`,
//!   `429` (the rate limit, with a `Retry-After` of a minute, so the client
//!   reports it at once instead of retrying), `401` (the token is refused)
//!   and `500` (a server error).
//! - `error-rate=0.2`: the share of requests that fail, one of the listed
//!   failures picked at random for each. Without a failure listed it means
//!   `500`; without a rate, every request fails.
//! - `writes`: only mutations fail; reads work.
//! - `scope=read:goals`: the demo's grant lacks that scope (the app then
//!   hides or disables what needs it), and Hardcover refuses with
//!   `insufficient_scope` every request that needs it. `refuse=read:goals`
//!   keeps the scope in the grant and only refuses the requests, as when
//!   Hardcover is the one to say no. Both can be repeated.
//! - `vibes=unbuilt`: the reader's Recommendations and Top Picks were never
//!   built (no build time, no books), as for a reader who never opened
//!   Discover on hardcover.app. `vibes=none`: the reader has no system
//!   vibes yet, so the app's `ensure_system_vibes` makes them, unbuilt.
//!
//! The scope a request needs is the scope map's (`required_scopes`), so a
//! scope that gates nothing there (`read:social`, which the app checks for
//! other readers' reviews) can only be taken out of the grant.

use std::fmt;
use std::time::Duration;

use super::vibes::SystemVibes;
use crate::auth::{DEFAULT_SCOPES, GrantedScopes};

/// The variable that holds the faults.
pub const ENV: &str = "HARDCOVER_DEMO_FAULTS";

/// What a failing request does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Offline,
    Timeout,
    RateLimited,
    Unauthorized,
    ServerError,
}

impl Failure {
    fn name(self) -> &'static str {
        match self {
            Failure::Offline => "offline",
            Failure::Timeout => "timeout",
            Failure::RateLimited => "429",
            Failure::Unauthorized => "401",
            Failure::ServerError => "500",
        }
    }
}

/// The faults to inject. The default injects none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Faults {
    latency: Duration,
    failures: Vec<Failure>,
    /// The share of requests that fail; `None` is every one.
    rate: Option<f64>,
    writes_only: bool,
    /// Taken out of the grant and refused.
    missing: Vec<&'static str>,
    /// Granted, but refused.
    refused: Vec<&'static str>,
    /// How the reader's own vibes start.
    system_vibes: SystemVibes,
}

impl Faults {
    /// The faults in `HARDCOVER_DEMO_FAULTS`. An unreadable value is reported
    /// on stderr and injects nothing.
    #[must_use]
    pub fn from_env() -> Self {
        let Ok(spec) = std::env::var(ENV) else {
            return Self::default();
        };
        Self::parse(&spec).unwrap_or_else(|e| {
            eprintln!("hardcover: ignoring {ENV}: {e}");
            Self::default()
        })
    }

    /// Reads a value of `HARDCOVER_DEMO_FAULTS` (see the module docs).
    ///
    /// # Errors
    ///
    /// Names the first item it doesn't understand.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut faults = Self::default();
        for item in spec.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            let (key, value) = match item.split_once('=') {
                Some((k, v)) => (k.trim(), Some(v.trim())),
                None => (item, None),
            };
            match (key, value) {
                ("offline", None) => faults.failures.push(Failure::Offline),
                ("timeout", None) => faults.failures.push(Failure::Timeout),
                ("429", None) => faults.failures.push(Failure::RateLimited),
                ("401", None) => faults.failures.push(Failure::Unauthorized),
                ("500", None) => faults.failures.push(Failure::ServerError),
                ("writes", None) => faults.writes_only = true,
                ("latency", Some(v)) => faults.latency = parse_duration(v)?,
                ("error-rate", Some(v)) => {
                    let rate: f64 = v
                        .parse()
                        .ok()
                        .filter(|r| (0.0..=1.0).contains(r))
                        .ok_or_else(|| format!("error-rate {v:?} is not between 0 and 1"))?;
                    faults.rate = Some(rate);
                }
                ("scope", Some(v)) => faults.missing.push(known_scope(v)?),
                ("refuse", Some(v)) => faults.refused.push(known_scope(v)?),
                ("vibes", Some("unbuilt")) => faults.system_vibes = SystemVibes::Unbuilt,
                ("vibes", Some("none")) => faults.system_vibes = SystemVibes::Missing,
                _ => return Err(format!("unknown fault {item:?}")),
            }
        }
        if faults.rate.is_some() && faults.failures.is_empty() {
            faults.failures.push(Failure::ServerError);
        }
        faults.failures.dedup();
        Ok(faults)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The scopes the demo session holds: every scope, less those taken out.
    #[must_use]
    pub fn granted_scopes(&self) -> GrantedScopes {
        if self.missing.is_empty() {
            return GrantedScopes::all();
        }
        let granted: Vec<&str> = DEFAULT_SCOPES
            .into_iter()
            .filter(|s| !self.missing.contains(s))
            .collect();
        GrantedScopes::parse(&granted.join(" "))
    }

    /// How the reader's own vibes start (`vibes=`).
    #[must_use]
    pub fn system_vibes(&self) -> SystemVibes {
        self.system_vibes
    }

    pub(super) fn latency(&self) -> Duration {
        self.latency
    }

    /// The scope Hardcover refuses `op` for, if any.
    pub(super) fn refused_scope(&self, op: &str) -> Option<&'static str> {
        required_scopes(op)
            .iter()
            .copied()
            .find(|s| self.missing.contains(s) || self.refused.contains(s))
    }

    /// How this request to the API fails, if it does. `roll` is a number in
    /// `0..1` drawn for the request.
    pub(super) fn failure(&self, is_write: bool, roll: impl FnMut() -> f64) -> Option<Failure> {
        if self.writes_only && !is_write {
            return None;
        }
        self.pick(&self.failures, roll)
    }

    /// How a cover download fails, if it does: only for no connection.
    pub(super) fn cover_failure(&self, roll: impl FnMut() -> f64) -> Option<Failure> {
        if self.writes_only {
            return None;
        }
        let network: Vec<Failure> = self
            .failures
            .iter()
            .copied()
            .filter(|f| matches!(f, Failure::Offline | Failure::Timeout))
            .collect();
        self.pick(&network, roll)
    }

    fn pick(&self, from: &[Failure], mut roll: impl FnMut() -> f64) -> Option<Failure> {
        if from.is_empty() || roll() >= self.rate.unwrap_or(1.0) {
            return None;
        }
        // Truncation is the point: an index from a number in 0..1.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ix = ((roll() * from.len() as f64) as usize).min(from.len() - 1);
        Some(from[ix])
    }
}

impl fmt::Display for Faults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = self.failures.iter().map(|x| x.name().to_owned()).collect();
        if let Some(rate) = self.rate {
            parts.push(format!("error-rate={rate}"));
        }
        if self.writes_only {
            parts.push("writes".to_owned());
        }
        if !self.latency.is_zero() {
            parts.push(format!("latency={}ms", self.latency.as_millis()));
        }
        parts.extend(self.missing.iter().map(|s| format!("scope={s}")));
        parts.extend(self.refused.iter().map(|s| format!("refuse={s}")));
        match self.system_vibes {
            SystemVibes::Built => {}
            SystemVibes::Unbuilt => parts.push("vibes=unbuilt".to_owned()),
            SystemVibes::Missing => parts.push("vibes=none".to_owned()),
        }
        f.write_str(&parts.join(","))
    }
}

fn parse_duration(v: &str) -> Result<Duration, String> {
    let bad = || format!("latency {v:?} is not like 800ms or 2s");
    if let Some(ms) = v.strip_suffix("ms") {
        return ms
            .trim()
            .parse()
            .map(Duration::from_millis)
            .map_err(|_| bad());
    }
    let s = v.strip_suffix('s').ok_or_else(bad)?;
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64)
        .ok_or_else(bad)
}

fn known_scope(v: &str) -> Result<&'static str, String> {
    DEFAULT_SCOPES
        .into_iter()
        .find(|s| *s == v)
        .ok_or_else(|| format!("{v:?} is not a scope the app requests"))
}

/// The scopes the scope map says an operation needs: a scope its top-level
/// field, a selected column or an argument written in the document needs,
/// which no other requested scope grants. A test checks it against
/// `schema/hardcover-capabilities.json`.
pub(super) fn required_scopes(op: &str) -> &'static [&'static str] {
    use crate::auth::scopes::{
        READ_CATALOG, READ_GOALS, READ_LIBRARY, READ_LISTS, READ_ME, READ_VIBES, WRITE_GOALS,
        WRITE_LIBRARY, WRITE_LISTS, WRITE_REVIEWS, WRITE_VIBES,
    };
    match op {
        "Me" => &[READ_ME],
        "LibraryPage" | "UserBookById" | "UserBookForUser" | "UserBookForBook"
        | "ProgressContext" | "UserBookReview" | "BookReviews" | "BookReviewsCount" => {
            &[READ_LIBRARY]
        }
        "SearchBooks" | "SearchAuthors" | "SearchSeries" | "BooksTrending" | "BookDetails"
        | "SeriesPage" | "AuthorPage" | "BooksByIds" | "BookEditions" | "UpcomingInSeries"
        | "SeriesOverview" | "BooksByTag" | "BooksByTagWith" | "BookTags" => &[READ_CATALOG],
        "InsertUserBook" | "UpdateUserBook" | "DeleteUserBook" | "InsertUserBookRead"
        | "UpdateUserBookRead" | "DeleteUserBookRead" | "EditionOwned" => &[WRITE_LIBRARY],
        "UpdateUserBookReview" | "DeleteUserBookReview" => &[WRITE_LIBRARY, WRITE_REVIEWS],
        "Goals" | "GoalById" => &[READ_GOALS],
        "InsertGoal" | "UpdateGoal" | "DeleteGoal" => &[WRITE_GOALS],
        "Lists" | "ListById" | "ListMemberships" | "BookListMemberships" | "ListBooks" => {
            &[READ_LISTS]
        }
        "InsertList" | "UpdateList" | "DeleteList" | "InsertListBook" | "DeleteListBook" => {
            &[WRITE_LISTS]
        }
        "SystemVibes" | "FeaturedVibes" | "VibeById" => &[READ_VIBES],
        "EnsureSystemVibes" | "ExcludeFromVibe" => &[WRITE_VIBES],
        _ => &[],
    }
}
