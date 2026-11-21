//! Other readers' reviews in the demo, from `demo/reviews.json`.
//!
//! The reviewers and their texts are invented. Project Hail Mary has more
//! than a page of them (with a spoiler review, a long one, a Persian one, one
//! without a rating and one with no text, which the client drops), a few
//! other books have some, and the rest have none. Dates are days from today.
//! The demo reader's own reviews are in the pool too, so `$me` has something
//! to leave out. A few reviewers have one of Hardcover's default avatars
//! (`static/avatars/profileN.png`, generic pictures, not anyone's); the rest
//! have a colour or nothing.

use std::cmp::Reverse;
use std::sync::LazyLock;

use chrono::{Duration, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};

use super::CATALOGUE;
use super::state::State;

#[derive(Deserialize)]
struct Reviews {
    reviewers: Vec<DemoReviewer>,
    reviews: Vec<DemoCommunityReview>,
}

#[derive(Deserialize)]
struct DemoReviewer {
    id: i64,
    username: String,
    name: String,
    color: Option<String>,
    /// One of Hardcover's default avatars.
    avatar: Option<String>,
}

#[derive(Deserialize)]
struct DemoCommunityReview {
    id: i64,
    book: i64,
    user: i64,
    rating: Option<f64>,
    likes: i64,
    /// Days from today.
    reviewed: Option<i64>,
    #[serde(default)]
    spoilers: bool,
    text: String,
}

static REVIEWS: LazyLock<Reviews> = LazyLock::new(|| {
    serde_json::from_str(include_str!("reviews.json")).expect("demo/reviews.json parses")
});

/// The reviewers' avatars, which the demo may download.
pub(super) fn avatars() -> impl Iterator<Item = &'static str> {
    REVIEWS.reviewers.iter().filter_map(|r| r.avatar.as_deref())
}

/// One review row as `BookReviews` returns it, with its sort keys.
struct Row {
    likes: i64,
    reviewed_at: Option<NaiveDate>,
    id: i64,
    value: Value,
}

/// The reviews of `book` not written by `me`, in the query's order: most
/// liked, then newest, then the highest id.
fn rows(state: &State, book: i64, me: i64) -> Vec<Row> {
    let reviews = &*REVIEWS;
    let today = state.clock.today();
    let mut rows: Vec<Row> = reviews
        .reviews
        .iter()
        .filter(|r| r.book == book && r.user != me)
        .filter_map(|r| {
            let user = reviews.reviewers.iter().find(|u| u.id == r.user)?;
            let mut image = json!({});
            if let Some(color) = &user.color {
                image["color"] = json!(color);
            }
            if let Some(url) = &user.avatar {
                image["url"] = json!(url);
            }
            let reviewed_at = r.reviewed.map(|days| today + Duration::days(days));
            Some(Row {
                likes: r.likes,
                reviewed_at,
                id: r.id,
                value: json!({
                    "id": r.id,
                    "rating": r.rating,
                    "likes_count": r.likes,
                    "reviewed_at": reviewed_at.map(|d| format!("{d}T00:00:00")),
                    "review_has_spoilers": r.spoilers,
                    "review_raw": r.text,
                    "user": {
                        "id": user.id,
                        "username": user.username,
                        "name": user.name,
                        "cached_image": image,
                    },
                }),
            })
        })
        .collect();
    // The demo reader's own reviews, which `$me` leaves out.
    let reader = &CATALOGUE.user;
    rows.extend(
        state
            .library
            .iter()
            .filter(|e| e.book == book && reader.id != me && e.review.exists())
            .map(|e| Row {
                likes: 0,
                reviewed_at: e.review.reviewed_at,
                id: e.id,
                value: json!({
                    "id": e.id,
                    "rating": e.rating,
                    "likes_count": 0,
                    "reviewed_at": e.review.reviewed_at.map(|d| format!("{d}T00:00:00")),
                    "review_has_spoilers": e.review.spoilers,
                    "review_raw": e.review.raw,
                    "user": {
                        "id": reader.id,
                        "username": reader.username,
                        "name": reader.name,
                        "cached_image": {},
                    },
                }),
            }),
    );
    // The newest first; no date sorts last.
    rows.sort_by(|a, b| {
        Reverse(a.likes)
            .cmp(&Reverse(b.likes))
            .then_with(|| match (&a.reviewed_at, &b.reviewed_at) {
                (Some(x), Some(y)) => y.cmp(x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| b.id.cmp(&a.id))
    });
    rows
}

/// `BookReviews`.
pub(super) fn page(state: &State, vars: &Value) -> Value {
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    let offset = usize::try_from(int("offset")).unwrap_or_default();
    let limit = usize::try_from(int("limit")).unwrap_or_default();
    let rows: Vec<Value> = rows(state, int("bookId"), int("me"))
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|r| r.value)
        .collect();
    json!({ "user_books": rows })
}

/// `BookReviewsCount`.
pub(super) fn count(state: &State, vars: &Value) -> Value {
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    let count = rows(state, int("bookId"), int("me")).len();
    json!({ "user_books_aggregate": { "aggregate": { "count": count } } })
}

#[cfg(test)]
mod tests {
    use crate::api::REVIEWS_PER_PAGE;
    use crate::demo::client;
    use crate::{BookId, UserId};

    const PROJECT_HAIL_MARY: BookId = BookId::new(427_578);
    const DUNE: BookId = BookId::new(312_460);
    const READER: UserId = UserId::new(1);

    #[test]
    fn one_book_has_more_than_a_page_of_reviews() {
        let client = client();
        let first = client.book_reviews(PROJECT_HAIL_MARY, READER, 0).unwrap();
        // One row on the first page has no text: dropped, but counted.
        assert_eq!(first.reviews.len(), REVIEWS_PER_PAGE as usize - 1);
        assert_eq!(first.next_offset, REVIEWS_PER_PAGE);
        assert!(first.has_more);
        let likes: Vec<u32> = first.reviews.iter().map(|r| r.likes).collect();
        assert!(likes.windows(2).all(|w| w[0] >= w[1]), "{likes:?}");
        let second = client
            .book_reviews(PROJECT_HAIL_MARY, READER, first.next_offset)
            .unwrap();
        assert!(!second.has_more);
        let total = first.reviews.len() + second.reviews.len();
        assert_eq!(
            client
                .book_reviews_count(PROJECT_HAIL_MARY, READER)
                .unwrap() as usize,
            total + 1,
            "the review with no text counts too"
        );
        let all: Vec<_> = first.reviews.iter().chain(&second.reviews).collect();
        assert!(all.iter().any(|r| r.has_spoilers));
        assert!(all.iter().any(|r| r.rating.is_none()));
        assert!(all.iter().any(|r| r.reviewed_at.is_none()));
        assert!(all.iter().any(|r| r.paragraphs().count() >= 4));
        assert!(all.iter().any(|r| {
            r.text
                .chars()
                .any(|c| ('\u{0600}'..='\u{06FF}').contains(&c))
        }));
        assert!(all.iter().any(|r| r.reviewer.name.is_none()));
        assert!(all.iter().any(|r| r.reviewer.avatar.is_some()));
    }

    #[test]
    fn the_readers_own_review_is_left_out() {
        let client = client();
        let mine = client.book_reviews(DUNE, READER, 0).unwrap();
        assert!(mine.reviews.iter().all(|r| r.reviewer.id != READER));
        let someone_else = client.book_reviews(DUNE, UserId::new(2), 0).unwrap();
        assert_eq!(someone_else.reviews.len(), mine.reviews.len() + 1);
    }

    #[test]
    fn some_books_have_no_reviews() {
        let client = client();
        let page = client
            .book_reviews(BookId::new(386_446), READER, 0)
            .unwrap();
        assert_eq!(page.reviews, []);
        assert!(!page.has_more);
    }
}
