//! The book's "Reviews" block (`hardcover-design`, Reviews): other readers'
//! reviews of the book, the most liked first, a page at a time, as sequence
//! rows 16 apart with hairlines between them.
//!
//! Asked for a moment after the book is shown, whether or not its details
//! load, and kept per book in the session's catalogue (`resource::Catalog`,
//! which also says when stale pages are fetched again and folded back into
//! a fresh first page). Hidden without `read:social`. The user's own review
//! is left out (it has its own section): the query excludes the signed-in
//! user, whose id the library knows, or `me()` tells when it doesn't yet.

use std::collections::HashSet;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_SOCIAL;
use hardcover_core::{BookId, CommunityReview, UserBookId};

use super::{BookDetailView, Fetch};
use crate::bidi::WrappingText;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{count_text, date_text, rating_text};
use crate::resource::Demand;
use crate::session;
use crate::ui::kit::OnTextSpine as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::{Avatar, Block, BylineLink, RowSpacing, Sequence, Skeleton, Stars};

/// A collapsed review shows about this many lines...
const COLLAPSED_LINES: usize = 6;
/// ...counting this many characters to a line of the pane's text.
const CHARS_PER_LINE: usize = 75;

/// What the section keeps of its own: which reviews the reader opened.
#[derive(Default)]
pub(super) struct CommunityReviews {
    /// Long reviews opened with "Read more".
    expanded: HashSet<UserBookId>,
    /// Spoiler reviews opened with "Show".
    spoilers_shown: HashSet<UserBookId>,
}

impl BookDetailView {
    /// Asks for the reviews when `read:social` is granted while a book is
    /// shown ("Sign in again…" keeps the pane as it is).
    pub(super) fn observe_review_scope(cx: &mut Context<Self>) -> Subscription {
        session::observe_grant(cx, |this, cx| {
            if let Some(book) = this.book(cx) {
                this.fetch(book.id, Fetch::Reviews, Demand::Shown, cx);
            }
        })
    }

    /// Fetches the next page, or the first again after it failed.
    fn more_reviews(&mut self, cx: &mut Context<Self>) {
        let (Some(book), Some(catalog)) = (self.book(cx), self.catalog.clone()) else {
            return;
        };
        let loaded = catalog
            .reviews
            .read(cx)
            .resource()
            .value(&book.id)
            .is_some();
        if loaded {
            let me = self.model.read(cx).verified_user_id();
            catalog.more_reviews(book.id, me, cx);
        } else {
            self.fetch(book.id, Fetch::Reviews, Demand::Now, cx);
        }
    }

    // ----- rendering -----

    /// The "Reviews" block, or `None` without `read:social`: the count on the
    /// heading's trailing edge, then the reviews between hairlines.
    pub(super) fn render_community_reviews(
        &self,
        book: BookId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !session::has_scope(cx, READ_SOCIAL) {
            return None;
        }
        let muted = colors(cx).fg_muted;
        let (state, first_error, more_error, loading) = match &self.catalog {
            Some(catalog) => {
                let store = catalog.reviews.read(cx).resource();
                (
                    store.value(&book).cloned(),
                    store.error(&book).cloned(),
                    store.more_error(&book).cloned(),
                    store.is_loading(&book),
                )
            }
            None => (None, None, None, false),
        };
        let retry = |id: &'static str, cx: &mut Context<Self>| {
            Button::new(id)
                .small_control()
                .label("Try again")
                .on_click(cx.listener(|this, _, _, cx| this.more_reviews(cx)))
        };
        let body: AnyElement = match &state {
            None => match first_error {
                Some(message) if !loading => h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(div().body().text_color(muted).child(message))
                    .child(retry("retry-reviews", cx))
                    .into_any_element(),
                // Waiting out the delay or loading: the shape of two reviews.
                _ => placeholder(loading).into_any_element(),
            },
            Some(state) if state.reviews.is_empty() && !state.has_more => div()
                .body()
                .text_color(muted)
                .child("No reviews yet")
                .into_any_element(),
            Some(state) => v_flex()
                .items_start()
                .gap_2()
                .child(
                    Sequence::new()
                        .spacing(RowSpacing::Reviews)
                        .flush_top()
                        .children(
                            state
                                .reviews
                                .iter()
                                .map(|review| self.render_community_review(review, cx)),
                        ),
                )
                .when(state.has_more || more_error.is_some(), |this| {
                    this.child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .items_center()
                            .when_some(more_error.clone().filter(|_| !loading), |this, e| {
                                this.child(div().body().text_color(muted).child(e))
                                    .child(retry("retry-more-reviews", cx))
                            })
                            .when(more_error.is_none() || loading, |this| {
                                this.child(
                                    Button::new("more-reviews")
                                        .small_control()
                                        .label("Show more reviews")
                                        .loading(loading)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.more_reviews(cx)),
                                        ),
                                )
                            }),
                    )
                })
                .into_any_element(),
        };
        let caption = state
            .as_ref()
            .and_then(|s| s.total)
            .filter(|n| *n > 0)
            .map(|n| match n {
                1 => "1 review".to_owned(),
                n => format!("{} reviews", count_text(n)),
            });
        Some(
            Block::new("Reviews")
                .when_some(caption, |this, caption| {
                    this.action(div().caption(cx).tabular().child(caption))
                })
                .child(body)
                .into_any_element(),
        )
    }

    fn review_body(&self, review: &CommunityReview, cx: &mut Context<Self>) -> AnyElement {
        let id = review.id;
        let key = id.get().cast_unsigned();
        let muted = colors(cx).fg_muted;
        let hidden = review.has_spoilers && !self.community_reviews.spoilers_shown.contains(&id);
        if hidden {
            h_flex()
                .gap_2()
                .items_center()
                .body()
                .text_color(muted)
                .child(Icon::new(IconName::EyeOff).size_3())
                .child("Contains spoilers")
                .child(
                    Button::new(("review-spoilers", key))
                        .small_control()
                        .label("Show")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.community_reviews.spoilers_shown.insert(id);
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else {
            let paragraphs: Vec<&str> = review.paragraphs().collect();
            let expanded = self.community_reviews.expanded.contains(&id);
            let collapsed = collapse(&paragraphs, COLLAPSED_LINES, CHARS_PER_LINE);
            let shown: Vec<String> = match (&collapsed, expanded) {
                (Some(short), false) => short.clone(),
                _ => paragraphs.iter().map(|p| (*p).to_owned()).collect(),
            };
            v_flex()
                .items_start()
                .gap_2()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(size::PROSE)
                        .gap_2()
                        .reader()
                        .children(shown.into_iter().map(WrappingText::new)),
                )
                .when(collapsed.is_some(), |this| {
                    this.child(
                        // Its label on the text's spine: back by the
                        // ghost button's padding.
                        Button::new(("review-more", key))
                            .ghost()
                            .small_control()
                            .on_text_spine(true)
                            .label(if expanded { "Show less" } else { "Read more" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let expanded = &mut this.community_reviews.expanded;
                                if !expanded.remove(&id) {
                                    expanded.insert(id);
                                }
                                cx.notify();
                            })),
                    )
                })
                .into_any_element()
        }
    }

    /// One review: the avatar beside the byline and rating, the date and
    /// likes, and the text in `reader`, collapsed when long; a spoiler review
    /// shows only that it has spoilers, and "Show".
    fn render_community_review(
        &self,
        review: &CommunityReview,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = review.id;
        let key = id.get().cast_unsigned();
        let name = review.reviewer.display_name();
        let mut facts: Vec<String> = Vec::new();
        if let Some(date) = review.reviewed_at {
            facts.push(date_text(date));
        }
        match review.likes {
            0 => {}
            1 => facts.push("1 like".to_owned()),
            n => facts.push(format!("{} likes", count_text(n))),
        }
        let body = self.review_body(review, cx);
        let name_element: AnyElement = match review.reviewer.profile_url() {
            Some(url) => BylineLink::new(("reviewer", key), name.clone(), url).into_any_element(),
            None => div()
                .body_strong()
                .min_w_0()
                .truncate()
                .child(name.clone())
                .into_any_element(),
        };
        h_flex()
            .items_start()
            .gap_3()
            .child(
                Avatar::new(name.clone())
                    // `Avatar` draws SVG pictures from the cached file and
                    // falls back to initials when a picture fails.
                    .image(review.reviewer.avatar.as_ref().map(|a| a.url.clone()))
                    .data_color(review.reviewer.color),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_x_2()
                                    .child(name_element)
                                    .when_some(review.rating, |this, rating| {
                                        this.child(
                                            Stars::new(rating.value()).small().accessibility_label(
                                                ("review-rating", key),
                                                format!("Rated {} of 5", rating_text(rating)),
                                            ),
                                        )
                                    }),
                            )
                            .when(!facts.is_empty(), |this| {
                                this.child(div().caption(cx).tabular().child(facts.join(" · ")))
                            }),
                    )
                    .child(body),
            )
            .into_any_element()
    }
}

/// The shape of two reviews while the first page loads: an avatar circle and
/// three bars, between the same hairline. It pulses only while the request
/// is in flight; waiting out the delay, it holds still.
fn placeholder(loading: bool) -> impl IntoElement {
    let still = !loading;
    let shape = move |skeleton: Skeleton| if still { skeleton.still() } else { skeleton };
    Sequence::new()
        .spacing(RowSpacing::Reviews)
        .flush_top()
        .children((0..2).map(move |_| {
            h_flex()
                .items_start()
                .gap_3()
                .child(shape(Skeleton::circle()))
                .child(
                    v_flex()
                        .flex_1()
                        .gap_2()
                        .child(shape(Skeleton::line().w_1_3()))
                        .child(shape(Skeleton::line()))
                        .child(shape(Skeleton::line().w_2_3())),
                )
        }))
}

/// The paragraphs a collapsed review shows, or `None` if the whole review
/// fits in about `lines` lines of `per_line` characters. Each paragraph
/// starts a line. A paragraph that doesn't fit is cut at a word, with "…".
fn collapse(paragraphs: &[&str], lines: usize, per_line: usize) -> Option<Vec<String>> {
    let lines_of = |p: &str| p.chars().count().div_ceil(per_line).max(1);
    if paragraphs.iter().map(|p| lines_of(p)).sum::<usize>() <= lines {
        return None;
    }
    let mut shown = Vec::new();
    let mut left = lines;
    for paragraph in paragraphs {
        let needed = lines_of(paragraph);
        if needed <= left {
            shown.push((*paragraph).to_owned());
            left -= needed;
            if left == 0 {
                break;
            }
            continue;
        }
        shown.push(cut(paragraph, left * per_line));
        break;
    }
    // A cut between paragraphs needs no "…": "Read more" follows.
    Some(shown)
}

/// `text` cut to at most `max` characters, at the last space before the cut
/// when there is one, with "…".
fn cut(text: &str, max: usize) -> String {
    let head: String = text.chars().take(max).collect();
    let head = match head.rfind(char::is_whitespace) {
        Some(space) if space > 0 => &head[..space],
        _ => head.as_str(),
    };
    format!("{}…", head.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    use gpui_kit::{Entity, TestAppContext, VisualTestContext};

    use super::super::tests::{BOOK, demo_pane, settle};
    use crate::resource::ReviewPages;

    fn reviews(view: &Entity<BookDetailView>, cx: &mut VisualTestContext) -> ReviewPages {
        view.read_with(cx, |view, cx| {
            let catalog = view.catalog.as_ref().unwrap();
            (**catalog.reviews.read(cx).resource().value(&BOOK).unwrap()).clone()
        })
    }

    fn review_pages(view: &Entity<BookDetailView>, cx: &mut VisualTestContext) -> u32 {
        reviews(view, cx).pages
    }

    #[gpui_kit::test]
    fn the_same_book_with_newer_facts_keeps_the_pane(cx: &mut TestAppContext) {
        let (view, cx) = demo_pane(cx);
        view.update(cx, |view, cx| view.more_reviews(cx));
        settle(cx);
        assert_eq!(review_pages(&view, cx), 2);

        // Its list's page was fetched again (the reviews being stale too): a
        // new summary, same book.
        cx.update(|window, cx| {
            crate::resource::refresh_all(cx);
            view.update(cx, |view, cx| {
                let mut book = hardcover_core::BookSummary::new(BOOK, "Project Hail Mary");
                book.users_count = Some(12_345);
                view.set_subject(Some(super::super::Subject::Book(book)), window, cx);
            });
        });
        settle(cx);
        assert_eq!(review_pages(&view, cx), 2, "the reviews paged through stay");
        let count = view.read_with(cx, |view, cx| view.book(cx).and_then(|b| b.users_count));
        assert_eq!(count, Some(12_345), "the newer facts are shown");
    }

    #[gpui_kit::test]
    fn paged_reviews_are_replaced_only_when_shown_again(cx: &mut TestAppContext) {
        let (view, cx) = demo_pane(cx);
        assert_eq!(review_pages(&view, cx), 1);
        view.update(cx, |view, cx| view.more_reviews(cx));
        settle(cx);
        assert_eq!(review_pages(&view, cx), 2);

        // The window became active: the pages the reader opened stay.
        cx.update(|_, cx| crate::resource::refresh_all(cx));
        view.update(cx, |view, cx| view.revalidate(false, cx));
        settle(cx);
        assert_eq!(review_pages(&view, cx), 2);

        // Refresh, or the book shown again: a fresh first page replaces them.
        view.update(cx, |view, cx| view.revalidate(true, cx));
        settle(cx);
        assert_eq!(review_pages(&view, cx), 1);
        let reviews = reviews(&view, cx);
        assert!(reviews.has_more);
        assert!(!reviews.reviews.is_empty());
    }

    #[test]
    fn short_reviews_are_not_collapsed() {
        assert_eq!(collapse(&["Short."], 6, 10), None);
        // Six one-line paragraphs fit exactly.
        assert_eq!(collapse(&["a"; 6], 6, 10), None);
    }

    #[test]
    fn long_reviews_are_cut_at_a_word() {
        let long = "one two three four five six seven eight nine ten";
        let shown = collapse(&[long], 2, 10).unwrap();
        assert_eq!(shown, ["one two three four…"]);
        // Paragraphs count a line each; the one that doesn't fit is cut.
        let shown = collapse(&["first", "second", long], 3, 10).unwrap();
        assert_eq!(shown, ["first", "second", "one two…"]);
        // Whole paragraphs fill the lines: none is cut.
        let shown = collapse(&["a", "b", "c"], 2, 10).unwrap();
        assert_eq!(shown, ["a", "b"]);
    }

    #[test]
    fn persian_text_is_cut_on_characters() {
        let text = "کتابی پرکشش و پر از امید ".repeat(20);
        let shown = collapse(&[text.trim()], 1, 12).unwrap();
        assert!(shown[0].ends_with('…'));
        assert!(shown[0].chars().count() <= 13, "{}", shown[0]);
    }
}
