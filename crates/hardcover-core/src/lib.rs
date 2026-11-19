//! GPUI-free core of hardcover-desktop.
//!
//! Everything here is synchronous and blocking: call it from a background
//! thread pool, never from the UI thread. Nothing depends on an async runtime.
//!
//! - [`api`]: GraphQL client for <https://api.hardcover.app/v1/graphql> plus the data models.
//! - [`auth`]: OAuth (authorization code + PKCE over a loopback redirect), token storage and refresh.
//! - [`store`]: SQLite cache of the signed-in user's library.
//! - [`covers`]: on-disk cache of cover images.
//! - [`prefs`]: the user's preferences, a small JSON file.
//! - [`update`]: self-update from signed GitHub releases.
//! - `demo` (with the `demo` feature): a sample library of popular books, served without an account.

pub mod api;
pub mod auth;
pub mod covers;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
pub mod error;
pub mod prefs;
pub mod store;
pub mod update;

mod http;
mod ratelimit;

pub use api::{
    AuthorBook, AuthorDetails, AuthorHit, AuthorId, AuthorRef, BookDetails, BookId, BookSummary,
    Client, Contributor, Cover, Date, EDITIONS_PER_PAGE, EditionChange, EditionId, EditionSummary,
    Goal, GoalConditions, GoalId, GoalMetric, Length, LibraryEntry, NewGoal, Privacy,
    ProgressUpdate, Rating, RatingBucket, ReadId, ReadSession, ReadingStatus, SearchHits,
    SearchPage, SeriesBook, SeriesDetails, SeriesHit, SeriesId, SeriesRef, Timestamp, TokenSource,
    TrendingPeriod, User, UserBookId, UserId,
};
pub use error::{Error, Result};

/// `User-Agent` sent with every request (API, OAuth and cover downloads).
pub const USER_AGENT: &str = concat!(
    "hardcover-desktop/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/AliSayyah/hardcover-desktop)"
);

/// Directory name used under the XDG config, data and cache directories.
pub const APP_DIR_NAME: &str = "hardcover-desktop";

pub(crate) fn project_dirs() -> Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", APP_DIR_NAME)
        .ok_or_else(|| Error::Storage("could not determine the home directory".into()))
}
