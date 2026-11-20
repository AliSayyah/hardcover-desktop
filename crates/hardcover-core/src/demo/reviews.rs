//! Other readers' reviews in the demo, from `demo/reviews.json`.
//!
//! The reviewers and their texts are invented. Project Hail Mary has more
//! than a page of them (with a spoiler review, a long one, a Persian one and
//! one without a rating), a few other books have some, and the rest have none.
//! The demo reader's own reviews are in the pool too, so `$me` has something
//! to leave out. Reviewers have a colour but no avatar image: the demo
//! downloads only the catalogue's covers.

use std::cmp::Reverse;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::{Value, json};

use super::Catalogue;

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
}

#[derive(Deserialize)]
struct DemoCommunityReview {
    id: i64,
    book: i64,
    user: i64,
    rating: Option<f64>,
    likes: i64,
    reviewed_at: Option<String>,
    #[serde(default)]
    spoilers: bool,
    text: String,
}

static REVIEWS: LazyLock<Reviews> = LazyLock::new(|| {
    serde_json::from_str(include_str!("reviews.json")).expect("demo/reviews.json parses")
});

/// One review row as `BookReviews` returns it, with its sort keys.
struct Row {
    likes: i64,
    reviewed_at: Option<String>,
    id: i64,
    value: Value,
}

/// The reviews of `book` not written by `me`, in the query's order: most
/// liked, then newest, then the highest id.
fn rows(catalogue: &Catalogue, book: i64, me: i64) -> Vec<Row> {
    let reviews = &*REVIEWS;
    let mut rows: Vec<Row> = reviews
        .reviews
        .iter()
        .filter(|r| r.book == book && r.user != me)
        .filter_map(|r| {
            let user = reviews.reviewers.iter().find(|u| u.id == r.user)?;
            let image = match &user.color {
                Some(color) => json!({ "color": color }),
                None => json!({}),
            };
            Some(Row {
                likes: r.likes,
                reviewed_at: r.reviewed_at.clone(),
                id: r.id,
                value: json!({
                    "id": r.id,
                    "rating": r.rating,
                    "likes_count": r.likes,
                    "reviewed_at": r.reviewed_at.as_ref().map(|d| format!("{d}T00:00:00")),
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
    let reader = &catalogue.user;
    rows.extend(
        catalogue
            .library
            .iter()
            .filter(|e| e.book == book && reader.id != me)
            .filter_map(|e| {
                let review = e.review.as_ref()?;
                Some(Row {
                    likes: 0,
                    reviewed_at: Some(review.reviewed_at.clone()),
                    id: e.id,
                    value: json!({
                        "id": e.id,
                        "rating": e.rating,
                        "likes_count": 0,
                        "reviewed_at": format!("{}T00:00:00", review.reviewed_at),
                        "review_has_spoilers": review.spoilers,
                        "review_raw": review.markdown,
                        "user": {
                            "id": reader.id,
                            "username": reader.username,
                            "name": reader.name,
                            "cached_image": {},
                        },
                    }),
                })
            }),
    );
    // Dates are ISO, so they sort as strings; no date sorts last.
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
pub(super) fn page(catalogue: &Catalogue, vars: &Value) -> Value {
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    let offset = usize::try_from(int("offset")).unwrap_or_default();
    let limit = usize::try_from(int("limit")).unwrap_or_default();
    let rows: Vec<Value> = rows(catalogue, int("bookId"), int("me"))
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|r| r.value)
        .collect();
    json!({ "user_books": rows })
}

/// `BookReviewsCount`.
pub(super) fn count(catalogue: &Catalogue, vars: &Value) -> Value {
    let int = |name: &str| vars[name].as_i64().unwrap_or_default();
    let count = rows(catalogue, int("bookId"), int("me")).len();
    json!({ "user_books_aggregate": { "aggregate": { "count": count } } })
}

#[cfg(test)]
mod tests {
    use crate::api::REVIEWS_PER_PAGE;
    use crate::demo::client;
    use crate::{BookId, UserId};

    const PROJECT_HAIL_MARY: BookId = BookId::new(427578);
    const DUNE: BookId = BookId::new(312460);
    const READER: UserId = UserId::new(1);

    #[test]
    fn one_book_has_more_than_a_page_of_reviews() {
        let client = client();
        let first = client.book_reviews(PROJECT_HAIL_MARY, READER, 0).unwrap();
        assert_eq!(first.reviews.len(), REVIEWS_PER_PAGE as usize);
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
            total
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
        let page = client.book_reviews(BookId::new(386446), READER, 0).unwrap();
        assert!(page.reviews.is_empty());
        assert!(!page.has_more);
    }
}
