//! The detail pane's "Reviews" section: other readers' reviews of the book,
//! the most liked first, a page at a time.
//!
//! Fetched once the details are in, after [`REVIEWS_DELAY`], and kept per book
//! for the run. Hidden without `read:social`. The user's own review is left
//! out (it has its own section): the query excludes the signed-in user, whose
//! id the library knows, or `me()` tells when it doesn't yet.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::link::Link;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, IntoElement, ParentElement as _,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
    relative,
};
use hardcover_core::auth::scopes::READ_SOCIAL;
use hardcover_core::{BookId, CommunityReview, Rating, Reviewer, UserBookId};

use super::{BookDetailView, Details, section};
use crate::app_state::{Services, cover_thumbnail};
use crate::bidi::{WrappingText, is_rtl};
use crate::design::color::{colors, data_color, text_on_data};
use crate::format::{count_text, date_text};

/// Reviews wait this long after the details, a little longer than similar
/// books, so arrowing through a list fetches neither for books passed over.
const REVIEWS_DELAY: Duration = Duration::from_millis(700);
/// A collapsed review shows about this many lines...
const COLLAPSED_LINES: usize = 6;
/// ...counting this many characters to a line of the pane's text.
const CHARS_PER_LINE: usize = 75;

/// The reviews fetched for one book.
#[derive(Default)]
struct BookReviews {
    reviews: Vec<CommunityReview>,
    /// How many there are, when Hardcover said.
    total: Option<u32>,
    next_offset: u32,
    has_more: bool,
    /// The first page has arrived.
    loaded: bool,
    /// The last fetch failed: the first page, or the next one once loaded.
    error: Option<SharedString>,
}

/// The section's state, kept per book for the run.
#[derive(Default)]
pub(super) struct CommunityReviews {
    books: HashMap<BookId, BookReviews>,
    /// The book whose page is being fetched. Cleared when the pane moves on,
    /// which also drops the fetch.
    loading: Option<BookId>,
    task: Option<Task<()>>,
    /// Long reviews opened with "Read more".
    expanded: HashSet<UserBookId>,
    /// Spoiler reviews opened with "Show".
    spoilers_shown: HashSet<UserBookId>,
}

impl BookDetailView {
    /// Starts the fetch when `read:social` is granted while a book is shown
    /// ("Sign in again…" keeps the pane as it is).
    pub(super) fn observe_review_scope(cx: &mut Context<Self>) -> Subscription {
        cx.observe_global::<Services>(|this, cx| {
            if cx.has_global::<Services>() {
                this.load_community_reviews(cx);
            }
        })
    }

    /// The pane moved to another book: drops a fetch for the old one and
    /// starts the new one's if its details are in. A first page that failed
    /// is tried again.
    pub(super) fn community_reviews_subject_changed(&mut self, cx: &mut Context<Self>) {
        self.community_reviews.task = None;
        self.community_reviews.loading = None;
        if let Some(book) = self.book(cx) {
            let reviews = &mut self.community_reviews.books;
            if reviews.get(&book.id).is_some_and(|r| !r.loaded) {
                reviews.remove(&book.id);
            }
        }
        self.load_community_reviews(cx);
    }

    /// Fetches the shown book's first page of reviews once its details are
    /// in. Not again after a failure, until the book is shown again.
    pub(super) fn load_community_reviews(&mut self, cx: &mut Context<Self>) {
        if !Services::global(cx).has_scope(READ_SOCIAL) {
            return;
        }
        let Some(book) = self.book(cx) else { return };
        if !matches!(self.details.get(&book.id), Some(Details::Loaded(_)))
            || self.community_reviews.books.contains_key(&book.id)
            || self.community_reviews.loading == Some(book.id)
        {
            return;
        }
        self.fetch_reviews(book.id, Some(REVIEWS_DELAY), cx);
    }

    /// Fetches the next page, or the first one again after a failure.
    fn more_reviews(&mut self, cx: &mut Context<Self>) {
        let Some(book) = self.book(cx) else { return };
        if self.community_reviews.loading == Some(book.id) {
            return;
        }
        self.fetch_reviews(book.id, None, cx);
        cx.notify();
    }

    fn fetch_reviews(&mut self, id: BookId, delay: Option<Duration>, cx: &mut Context<Self>) {
        let state = self.community_reviews.books.get(&id);
        let first = state.is_none_or(|s| !s.loaded);
        let offset = state.filter(|_| !first).map_or(0, |s| s.next_offset);
        let user = self.model.read(cx).user().map(|u| u.id);
        let client = Services::global(cx).client().clone();
        self.community_reviews.loading = Some(id);
        // Replacing the task cancels a fetch for a book no longer shown.
        self.community_reviews.task = Some(cx.spawn(async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let result = cx
                .background_spawn(async move {
                    // Before the first sync the library doesn't know whose it is.
                    let me = match user {
                        Some(me) => me,
                        None => client.me()?.id,
                    };
                    let page = client.book_reviews(id, me, offset)?;
                    // A first page that holds them all is its own count; a
                    // count that fails costs only the caption.
                    let total = match (first, page.has_more) {
                        (true, false) => u32::try_from(page.reviews.len()).ok(),
                        (true, true) => client.book_reviews_count(id, me).ok(),
                        (false, _) => None,
                    };
                    hardcover_core::Result::Ok((page, total))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.community_reviews.loading == Some(id) {
                    this.community_reviews.loading = None;
                }
                let error = result
                    .as_ref()
                    .err()
                    .map(|e| SharedString::from(Services::error_message(e, Some(READ_SOCIAL), cx)));
                let state = this.community_reviews.books.entry(id).or_default();
                match result {
                    Ok((page, total)) => {
                        // Likes change between requests, so a review can move
                        // into the next page: show it once.
                        let seen: HashSet<UserBookId> =
                            state.reviews.iter().map(|r| r.id).collect();
                        state
                            .reviews
                            .extend(page.reviews.into_iter().filter(|r| !seen.contains(&r.id)));
                        state.next_offset = page.next_offset;
                        state.has_more = page.has_more;
                        if total.is_some() {
                            state.total = total;
                        }
                        state.loaded = true;
                        state.error = None;
                    }
                    Err(_) => state.error = error,
                }
                cx.notify();
            });
        }));
    }

    // ----- rendering -----

    /// The "Reviews" section, or `None` without `read:social`.
    pub(super) fn render_community_reviews(
        &self,
        book: BookId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !Services::global(cx).has_scope(READ_SOCIAL) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let loading = self.community_reviews.loading == Some(book);
        let retry = |id: &'static str, cx: &mut Context<Self>| {
            Button::new(id)
                .small()
                .label("Try again")
                .on_click(cx.listener(|this, _, _, cx| this.more_reviews(cx)))
        };
        let state = self.community_reviews.books.get(&book);
        let body: AnyElement = match state {
            // Waiting out the delay or loading: the shape of a review, still.
            None => placeholder(cx).into_any_element(),
            Some(state) if !state.loaded => match &state.error {
                Some(message) if !loading => h_flex()
                    .gap_2()
                    .text_sm()
                    .child(div().text_color(muted).child(message.clone()))
                    .child(retry("retry-reviews", cx))
                    .into_any_element(),
                _ => placeholder(cx).into_any_element(),
            },
            Some(state) if state.reviews.is_empty() && !state.has_more => div()
                .text_sm()
                .text_color(muted)
                .child("No reviews yet")
                .into_any_element(),
            Some(state) => v_flex()
                .gap_5()
                .children(
                    state
                        .reviews
                        .iter()
                        .map(|review| self.render_community_review(review, window, cx)),
                )
                .when(state.has_more || state.error.is_some(), |this| {
                    this.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .when_some(state.error.clone().filter(|_| !loading), |this, e| {
                                this.child(div().text_color(muted).child(e))
                                    .child(retry("retry-more-reviews", cx))
                            })
                            .when(state.error.is_none() || loading, |this| {
                                this.child(
                                    Button::new("more-reviews")
                                        .outline()
                                        .small()
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
            .and_then(|s| s.total)
            .filter(|n| *n > 0)
            .map(|n| match n {
                1 => "1 review".to_owned(),
                n => format!("{} reviews", count_text(n)),
            });
        Some(
            section("Reviews")
                .when_some(caption, |this, caption| {
                    this.child(div().text_sm().text_color(muted).child(caption))
                })
                .child(body)
                .into_any_element(),
        )
    }

    fn render_community_review(
        &self,
        review: &CommunityReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let id = review.id;
        let key = id.get() as u64;
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
        let hidden = review.has_spoilers && !self.community_reviews.spoilers_shown.contains(&id);
        let body: AnyElement = if hidden {
            h_flex()
                .gap_2()
                .items_center()
                .text_sm()
                .text_color(muted)
                .child(Icon::new(IconName::EyeOff).xsmall())
                .child("Contains spoilers")
                .child("·")
                .child(
                    Button::new(("review-spoilers", key))
                        .ghost()
                        .xsmall()
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
                .gap_2()
                .text_sm()
                .children(shown.into_iter().map(WrappingText::new))
                .when(collapsed.is_some(), |this| {
                    this.child(
                        h_flex().child(
                            Button::new(("review-more", key))
                                .ghost()
                                .xsmall()
                                .label(if expanded { "Show less" } else { "Read more" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let expanded = &mut this.community_reviews.expanded;
                                    if !expanded.remove(&id) {
                                        expanded.insert(id);
                                    }
                                    cx.notify();
                                })),
                        ),
                    )
                })
                .into_any_element()
        };
        let name_element: AnyElement = match review.reviewer.profile_url() {
            // GPUI's Linux layout puts the underline beside a right-to-left
            // name rather than under it, so such a name goes without.
            Some(url) => Link::new(("reviewer", key))
                .href(url)
                .when(is_rtl(&name), |this| this.text_decoration_none())
                .child(name.clone())
                .into_any_element(),
            None => div().child(name.clone()).into_any_element(),
        };
        h_flex()
            .items_start()
            .gap_3()
            .child(reviewer_avatar(&review.reviewer, &name, window, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_x_2()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(name_element)
                                    .when_some(review.rating, |this, rating| {
                                        this.child(stars(rating, cx))
                                    }),
                            )
                            .when(!facts.is_empty(), |this| {
                                this.child(
                                    div().text_xs().text_color(muted).child(facts.join(" · ")),
                                )
                            }),
                    )
                    .child(body),
            )
            .into_any_element()
    }
}

/// The shape of two reviews while the first page loads.
fn placeholder(cx: &Context<BookDetailView>) -> impl IntoElement {
    let bar = || div().h_4().bg(cx.theme().skeleton);
    v_flex().gap_5().children((0..2).map(move |_| {
        h_flex()
            .items_start()
            .gap_3()
            .child(div().size_6().rounded_full().bg(cx.theme().skeleton))
            .child(
                v_flex()
                    .flex_1()
                    .gap_2()
                    .child(bar().w_1_3())
                    .child(bar().w_full())
                    .child(bar().w_2_3()),
            )
    }))
}

/// The reviewer's avatar: their picture through the cover pipeline once it
/// has loaded, else their initials on their avatar's colour, else the kit's
/// initials.
fn reviewer_avatar(
    reviewer: &Reviewer,
    name: &str,
    window: &mut Window,
    cx: &mut Context<BookDetailView>,
) -> AnyElement {
    let image = reviewer
        .raster_avatar()
        .and_then(|a| cover_thumbnail(&a.url, window, cx));
    match (image, reviewer.color) {
        (Some(image), _) => Avatar::new()
            .small()
            .name(name.to_owned())
            .src(image)
            .into_any_element(),
        (None, Some([r, g, b])) => {
            let background = data_color([r, g, b]);
            let foreground = text_on_data([r, g, b]);
            div()
                .flex_shrink_0()
                .size_6()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(background)
                .text_color(foreground)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child(initials(name))
                .into_any_element()
        }
        (None, None) => Avatar::new()
            .small()
            .name(name.to_owned())
            .into_any_element(),
    }
}

/// True if dark text reads better than white on this colour.
/// One or two letters for an avatar: the first letter of the first and last
/// words. "@user" counts from the letter after the "@".
fn initials(name: &str) -> String {
    let first_letter = |word: &str| word.chars().find(|c| c.is_alphanumeric());
    let words: Vec<&str> = name.split_whitespace().collect();
    let mut out = String::new();
    if let Some(c) = words.first().and_then(|w| first_letter(w)) {
        out.extend(c.to_uppercase());
    }
    if words.len() > 1
        && let Some(c) = words.last().and_then(|w| first_letter(w))
    {
        out.extend(c.to_uppercase());
    }
    out
}

/// Five small stars in the text's muted colour: another reader's rating,
/// never gold.
fn stars(rating: Rating, cx: &Context<BookDetailView>) -> impl IntoElement {
    let filled = cx.theme().muted_foreground;
    let empty = colors(cx).star_empty;
    let value = rating.value();
    h_flex().gap_0p5().children((1..=5).map(move |position| {
        let position = position as f32;
        let star = || crate::star::icon().size_3p5().flex_shrink_0();
        div().relative().flex_shrink_0().size_3p5().map(|this| {
            if value >= position {
                this.child(star().text_color(filled))
            } else if value + 0.5 >= position {
                // Half: the empty star with its leading half filled.
                this.child(star().text_color(empty)).child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(relative(0.5))
                        .overflow_hidden()
                        .child(star().text_color(filled)),
                )
            } else {
                this.child(star().text_color(empty))
            }
        })
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

    #[test]
    fn initials_take_the_first_and_last_words() {
        assert_eq!(initials("Jane Quill"), "JQ");
        assert_eq!(initials("Jane van der Quill"), "JQ");
        assert_eq!(initials("@quill_reads"), "Q");
        assert_eq!(initials("شیرین"), "ش");
        assert_eq!(initials(""), "");
    }
}
