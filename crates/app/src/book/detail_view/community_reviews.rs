//! The book's "Reviews" block (`hardcover-design`, Reviews): other readers'
//! reviews of the book, the most liked first, a page at a time, as sequence
//! rows 16 apart with hairlines between them.
//!
//! Its own view, [`CommunityReviews`], so the book view around it doesn't
//! rebuild it on every frame it draws (a caret blink in Notes, a star
//! hovered): it re-renders when its reviews change, when the reader opens
//! one, or when the grant changes. Each review's paragraphs and where it is
//! cut when collapsed are worked out once, when its page arrives.
//!
//! Asked for a moment after the book is shown, whether or not its details
//! load, and kept per book in the session's catalogue (`resource::Catalog`,
//! which also says when stale pages are fetched again and folded back into
//! a fresh first page). Hidden without `read:social`. The user's own review
//! is left out (it has its own section): the query excludes the signed-in
//! user, whose id the library knows, or `me()` tells when it doesn't yet.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_SOCIAL;
use hardcover_core::{BookId, CommunityReview, UserBookId};

use crate::bidi::WrappingText;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{count_text, date_text, rating_text};
use crate::library::model::LibraryModel;
use crate::resource::catalog::ReviewPages;
use crate::resource::{Catalog, Demand, SharedChanged};
use crate::session;
use crate::ui::kit::OnTextSpine as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::{Avatar, Block, BylineLink, LoadFailed, RowSpacing, Sequence, Skeleton, Stars};

/// A collapsed review shows about this many lines...
const COLLAPSED_LINES: usize = 6;
/// ...counting this many characters to a line of the pane's text.
const CHARS_PER_LINE: usize = 75;

/// A review's text as it is drawn: its paragraphs, and those a collapsed
/// review shows when it is long.
#[derive(Debug, PartialEq)]
struct Prepared {
    paragraphs: Vec<SharedString>,
    collapsed: Option<Vec<SharedString>>,
}

impl Prepared {
    fn of(review: &CommunityReview) -> Self {
        let paragraphs: Vec<&str> = review.paragraphs().collect();
        Self {
            collapsed: collapse(&paragraphs, COLLAPSED_LINES, CHARS_PER_LINE)
                .map(|shown| shown.into_iter().map(SharedString::from).collect()),
            paragraphs: paragraphs
                .into_iter()
                .map(|p| SharedString::from(p.to_owned()))
                .collect(),
        }
    }
}

/// The "Reviews" block of the book shown. See the module doc.
pub(super) struct CommunityReviews {
    model: Entity<LibraryModel>,
    /// The session's catalogue; `None` once signed out.
    catalog: Option<Catalog>,
    book: Option<BookId>,
    /// The reviews shown, as last prepared, and what was made of each.
    shown: Option<Arc<ReviewPages>>,
    prepared: HashMap<UserBookId, Rc<Prepared>>,
    /// Long reviews opened with "Read more".
    expanded: HashSet<UserBookId>,
    /// Spoiler reviews opened with "Show".
    spoilers_shown: HashSet<UserBookId>,
    _subscriptions: Vec<Subscription>,
}

impl CommunityReviews {
    pub(super) fn new(model: Entity<LibraryModel>, cx: &mut Context<Self>) -> Self {
        let catalog = Catalog::global(cx);
        let mut subscriptions = vec![
            // "Sign in again…" may grant `read:social` while a book is shown.
            session::observe_grant(cx, |this, cx| {
                this.fetch(Demand::Shown, cx);
                cx.notify();
            }),
        ];
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.subscribe(
                &catalog.reviews,
                |this, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if this.book == Some(*id) {
                        this.prepare(cx);
                        cx.notify();
                    }
                },
            ));
        }
        Self {
            model,
            catalog,
            book: None,
            shown: None,
            prepared: HashMap::new(),
            expanded: HashSet::new(),
            spoilers_shown: HashSet::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `book`'s reviews: what the catalogue has at once; the view asks
    /// for them ([`Self::fetch`]) once the book has settled.
    pub(super) fn set_book(&mut self, book: Option<BookId>, cx: &mut Context<Self>) {
        if self.book == book {
            return;
        }
        self.book = book;
        self.prepare(cx);
        cx.notify();
    }

    /// Whether the block shows at all: only with `read:social`.
    pub(super) fn visible(cx: &App) -> bool {
        session::has_scope(cx, READ_SOCIAL)
    }

    /// Asks the catalogue for the book's reviews, if `demand` calls for it.
    pub(super) fn fetch(&mut self, demand: Demand, cx: &mut Context<Self>) {
        let (Some(book), Some(catalog)) = (self.book, self.catalog.clone()) else {
            return;
        };
        if session::has_scope(cx, READ_SOCIAL) {
            let me = self.model.read(cx).verified_user_id();
            catalog.request_reviews(book, me, demand, cx);
        }
    }

    /// The pages the catalogue holds for the book shown.
    fn pages(&self, cx: &App) -> Option<Arc<ReviewPages>> {
        let book = self.book?;
        self.catalog
            .as_ref()?
            .reviews
            .read(cx)
            .resource()
            .value(&book)
            .cloned()
    }

    /// Works out the paragraphs and collapse points of reviews that arrived
    /// since the last time, and forgets those of reviews no longer shown.
    fn prepare(&mut self, cx: &App) {
        let pages = self.pages(cx);
        let same = match (&pages, &self.shown) {
            (Some(new), Some(old)) => Arc::ptr_eq(new, old),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        let reviews = pages.as_ref().map_or(&[][..], |p| p.reviews.as_slice());
        self.prepared
            .retain(|id, _| reviews.iter().any(|r| r.id == *id));
        for review in reviews {
            self.prepared
                .entry(review.id)
                .or_insert_with(|| Rc::new(Prepared::of(review)));
        }
        self.shown = pages;
    }

    /// Fetches the next page, or the first again after it failed.
    pub(super) fn more_reviews(&mut self, cx: &mut Context<Self>) {
        let (Some(book), Some(catalog)) = (self.book, self.catalog.clone()) else {
            return;
        };
        let loaded = catalog.reviews.read(cx).resource().value(&book).is_some();
        if loaded {
            let me = self.model.read(cx).verified_user_id();
            catalog.more_reviews(book, me, cx);
        } else {
            self.fetch(Demand::Now, cx);
        }
    }

    fn review_body(&self, review: &CommunityReview, cx: &Context<Self>) -> AnyElement {
        let id = review.id;
        let key = id.get().cast_unsigned();
        let muted = colors(cx).fg_muted;
        let hidden = review.has_spoilers && !self.spoilers_shown.contains(&id);
        if hidden {
            return h_flex()
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
                            this.spoilers_shown.insert(id);
                            cx.notify();
                        })),
                )
                .into_any_element();
        }
        let Some(prepared) = self.prepared.get(&id).cloned() else {
            return div().into_any_element();
        };
        let expanded = self.expanded.contains(&id);
        let shown = match (&prepared.collapsed, expanded) {
            (Some(short), false) => short,
            _ => &prepared.paragraphs,
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
                    .children(shown.iter().cloned().map(WrappingText::new)),
            )
            .when(prepared.collapsed.is_some(), |this| {
                this.child(
                    // Its label on the text's spine: back by the ghost
                    // button's padding.
                    Button::new(("review-more", key))
                        .ghost()
                        .small_control()
                        .on_text_spine(true)
                        .label(if expanded { "Show less" } else { "Read more" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&id) {
                                this.expanded.insert(id);
                            }
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }

    /// One review: the avatar beside the byline and rating, the date and
    /// likes, and the text in `reader`, collapsed when long; a spoiler review
    /// shows only that it has spoilers, and "Show".
    fn render_review(&self, review: &CommunityReview, cx: &Context<Self>) -> AnyElement {
        let key = review.id.get().cast_unsigned();
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
                    .child(self.review_body(review, cx)),
            )
            .into_any_element()
    }
}

impl Render for CommunityReviews {
    /// The "Reviews" block: the count on the heading's trailing edge, then
    /// the reviews between hairlines. Empty without `read:social`; the book
    /// view leaves the block out then ([`CommunityReviews::visible`]).
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(book) = self.book.filter(|_| Self::visible(cx)) else {
            return div().into_any_element();
        };
        let muted = colors(cx).fg_muted;
        let (first_error, more_error, loading) = match &self.catalog {
            Some(catalog) => {
                let store = catalog.reviews.read(cx).resource();
                (
                    store.error(&book).cloned(),
                    store.more_error(&book).cloned(),
                    store.is_loading(&book),
                )
            }
            None => (None, None, false),
        };
        let retry = |id: &'static str, message: SharedString, cx: &Context<Self>| {
            LoadFailed::new(id, message)
                .on_retry(cx.listener(|this, _, _, cx| this.more_reviews(cx)))
        };
        let state = self.shown.clone();
        let body: AnyElement = match &state {
            None => match first_error {
                Some(message) if !loading => retry("retry-reviews", message, cx).into_any_element(),
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
                                .map(|review| self.render_review(review, cx)),
                        ),
                )
                .map(|this| match more_error.clone().filter(|_| !loading) {
                    Some(error) => this.child(retry("retry-more-reviews", error, cx)),
                    None if state.has_more => this.child(
                        Button::new("more-reviews")
                            .small_control()
                            .label("Show more reviews")
                            .loading(loading)
                            .on_click(cx.listener(|this, _, _, cx| this.more_reviews(cx))),
                    ),
                    None => this,
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
        Block::new("Reviews")
            .when_some(caption, |this, caption| {
                this.action(div().caption(cx).tabular().child(caption))
            })
            .child(body)
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

    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::super::BookDetailView;
    use super::super::tests::{BOOK, demo_pane, settle};

    fn reviews(view: &Entity<BookDetailView>, cx: &mut VisualTestContext) -> ReviewPages {
        view.read_with(cx, |view, cx| {
            let catalog = view.catalog.as_ref().unwrap();
            (**catalog.reviews.read(cx).resource().value(&BOOK).unwrap()).clone()
        })
    }

    fn review_pages(view: &Entity<BookDetailView>, cx: &mut VisualTestContext) -> u32 {
        reviews(view, cx).pages
    }

    fn more_reviews(view: &Entity<BookDetailView>, cx: &mut VisualTestContext) {
        let child = view.read_with(cx, |view, _| view.community_reviews.clone());
        child.update(cx, CommunityReviews::more_reviews);
    }

    #[gpui_kit::test]
    fn the_same_book_with_newer_facts_keeps_the_pane(cx: &mut TestAppContext) {
        let (view, cx) = demo_pane(cx);
        more_reviews(&view, cx);
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
        more_reviews(&view, cx);
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

    /// C1: each review's paragraphs and cut are worked out when its page
    /// arrives, once, and kept for the reviews still shown.
    #[gpui_kit::test]
    fn reviews_are_prepared_when_they_arrive(cx: &mut TestAppContext) {
        let (view, cx) = demo_pane(cx);
        let child = view.read_with(cx, |view, _| view.community_reviews.clone());
        let prepared = |cx: &mut VisualTestContext| {
            child.read_with(cx, |c, _| {
                let reviews = &c.shown.as_ref().unwrap().reviews;
                assert_eq!(c.prepared.len(), reviews.len());
                c.prepared.get(&reviews[0].id).cloned().unwrap()
            })
        };
        let first = prepared(cx);
        more_reviews(&view, cx);
        settle(cx);
        assert!(
            Rc::ptr_eq(&first, &prepared(cx)),
            "a review already prepared isn't worked out again"
        );
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
