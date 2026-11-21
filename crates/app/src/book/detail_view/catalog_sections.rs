//! The book's catalogue parts: About this book, the tags, other titles, On
//! Hardcover (the average and the ratings breakdown; "Open on Hardcover" is
//! in the book's "more" menu, `header.rs`),
//! and similar books (`hardcover-design`, Shelf → Book pane, and Full book
//! page).

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookDetails, BookId, BookSummary, RatingBucket};

use super::header::HEADER_GENRES;
use super::your_book::paragraphs;
use super::{BookDetailView, Details};
use crate::app_state::{Services, release_cover_medium};
use crate::bidi::WrappingText;
use crate::catalog::navigation::Page;
use crate::chip::{TagChip, TagGroup, tag_groups};
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::{average_rating_text, count_text};
use crate::ui::kit::SmallControl as _;
use crate::ui::{
    Block, CoverShelf, Lane, Meter, RailSection, Skeleton, SkeletonText, Stars, TextButton,
};

/// Similar books fetched for one book.
pub(super) enum Similar {
    Ready(Arc<Vec<BookSummary>>),
    Failed(SharedString),
}

/// Similar books wait this long after the details, so arrowing through a
/// list spends one request per book passed over, not two.
const SIMILAR_DELAY: Duration = Duration::from_millis(600);
/// Similar books shown.
const SIMILAR_SHOWN: usize = 8;
/// Tags shown per group until "Show all tags".
const TAGS_SHOWN: usize = 6;
/// Other titles shown.
const TITLES_SHOWN: usize = 5;

impl BookDetailView {
    // ----- similar books -----

    /// Fetches the shown book's similar books once its details are in.
    pub(super) fn load_similar(&mut self, cx: &mut Context<Self>) {
        let Some(book) = self.book(cx) else { return };
        let id = book.id;
        let Some(Details::Loaded(details)) = self.details.get(&id) else {
            return;
        };
        if details.similar_book_ids.is_empty()
            || matches!(self.similar.get(&id), Some(Similar::Ready(_)))
            || self.similar_loading == Some(id)
        {
            return;
        }
        let ids: Vec<BookId> = details
            .similar_book_ids
            .iter()
            .copied()
            .filter(|b| *b != id)
            .take(SIMILAR_SHOWN)
            .collect();
        self.similar.remove(&id);
        self.similar_loading = Some(id);
        let client = Services::global(cx).client().clone();
        // Replacing the task cancels the fetch for a book no longer shown.
        self.similar_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SIMILAR_DELAY).await;
            let result = cx
                .background_spawn(async move { client.books_by_ids(&ids) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.similar_loading == Some(id) {
                    this.similar_loading = None;
                }
                let similar = match result {
                    Ok(books) => Similar::Ready(Arc::new(books)),
                    Err(e) => {
                        Similar::Failed(Services::error_message(&e, Some(READ_CATALOG), cx).into())
                    }
                };
                this.similar.insert(id, similar);
                cx.notify();
            });
        }));
    }

    /// Releases the md/lg covers of similar books no longer shown, now that
    /// `book` (if any) is: those of the book shown before, once it changes.
    pub(super) fn keep_similar_covers(
        &mut self,
        book: Option<BookId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown: Vec<String> = match book.and_then(|id| self.similar.get(&id)) {
            Some(Similar::Ready(books)) => books
                .iter()
                .filter_map(|b| b.cover.as_ref().map(|c| c.url.clone()))
                .collect(),
            _ => Vec::new(),
        };
        if shown == self.similar_covers {
            return;
        }
        for url in std::mem::replace(&mut self.similar_covers, shown) {
            if !self.similar_covers.contains(&url) {
                release_cover_medium(&url, Some(window), cx);
            }
        }
    }

    fn retry_similar(&mut self, cx: &mut Context<Self>) {
        if let Some(book) = self.book(cx) {
            self.similar.remove(&book.id);
        }
        self.similar_loading = None;
        self.load_similar(cx);
        cx.notify();
    }

    // ----- blocks -----

    /// About this book: the description in `prose`, or why there is none.
    pub(super) fn render_about(&self, book: &BookSummary, cx: &mut Context<Self>) -> AnyElement {
        let muted = colors(cx).fg_muted;
        let body: AnyElement = match self.details.get(&book.id) {
            Some(Details::Loaded(details)) => match details.description.as_deref().map(str::trim) {
                Some(text) if !text.is_empty() => v_flex()
                    .gap_2()
                    .max_w(size::PROSE)
                    .prose()
                    .children(paragraphs(text).map(WrappingText::new))
                    .into_any_element(),
                _ => div()
                    .body()
                    .text_color(muted)
                    .child("No description on Hardcover")
                    .into_any_element(),
            },
            Some(Details::Failed(message)) => h_flex()
                .flex_wrap()
                .gap_2()
                .child(div().body().text_color(muted).child(message.clone()))
                .child(
                    Button::new("retry-details")
                        .small_control()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_details(cx))),
                )
                .into_any_element(),
            None if self.details_fetching == Some(book.id) => {
                SkeletonText::new(3).into_any_element()
            }
            // Waiting out the delay: the same placeholder, still.
            None => SkeletonText::new(3).still().into_any_element(),
        };
        Block::new("About this book").child(body).into_any_element()
    }

    /// Tags: the moods, content warnings and other tags, a few per group
    /// until "Show N more". Genres are in the header, and here only when
    /// there are more than it shows.
    pub(super) fn render_tags(
        &self,
        details: Option<&BookDetails>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let details = details?;
        /// Which chips a group's tags get.
        #[derive(Clone, Copy, PartialEq)]
        enum Group {
            Genres,
            Warnings,
            Plain,
        }
        // The label, the tags, and which chips they get.
        let groups: Vec<(&'static str, &[String], Group)> = [
            ("Genres", details.genres.as_slice(), Group::Genres),
            ("Moods", details.moods.as_slice(), Group::Plain),
            (
                "Content warnings",
                details.content_warnings.as_slice(),
                Group::Warnings,
            ),
            ("Other tags", details.tags.as_slice(), Group::Plain),
        ]
        .into_iter()
        .filter(|(_, tags, group)| {
            !tags.is_empty() && (*group != Group::Genres || tags.len() > HEADER_GENRES)
        })
        .collect();
        if groups.is_empty() {
            return None;
        }
        let hidden: usize = groups
            .iter()
            .map(|(_, tags, _)| tags.len().saturating_sub(TAGS_SHOWN))
            .sum();
        let expanded = self.tags_expanded;
        Some(
            Block::new("Tags")
                .when(hidden > 0, |this| {
                    this.action(
                        Button::new("toggle-tags")
                            .ghost()
                            .small_control()
                            .label(if expanded {
                                "Show fewer".to_owned()
                            } else {
                                format!("Show {} more", count_text(hidden))
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tags_expanded = !this.tags_expanded;
                                cx.notify();
                            })),
                    )
                })
                .child(tag_groups(groups.into_iter().map(|(label, tags, kind)| {
                    let shown = if expanded {
                        tags.len()
                    } else {
                        tags.len().min(TAGS_SHOWN)
                    };
                    TagGroup::new(
                        label,
                        tags[..shown].iter().map(|t| match kind {
                            Group::Genres => TagChip::genre(t.clone()),
                            Group::Warnings => TagChip::new(t.clone()).warning(true),
                            Group::Plain => TagChip::new(t.clone()),
                        }),
                    )
                })))
                .into_any_element(),
        )
    }

    /// Also known as: the book's other titles, muted.
    pub(super) fn render_other_titles(
        details: Option<&BookDetails>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let titles = &details?.alternative_titles;
        if titles.is_empty() {
            return None;
        }
        let mut text = titles
            .iter()
            .take(TITLES_SHOWN)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        if titles.len() > TITLES_SHOWN {
            write!(text, " · and {} more", titles.len() - TITLES_SHOWN)
                .expect("writing to a String");
        }
        Some(
            Block::new("Also known as")
                .child(
                    div()
                        .body()
                        .text_color(colors(cx).fg_muted)
                        .child(WrappingText::new(text)),
                )
                .into_any_element(),
        )
    }

    /// On Hardcover: the counts and the ratings breakdown. The
    /// page's rail leads with the average as a figure; the pane has the
    /// average in its header.
    pub(super) fn render_on_hardcover(
        book: &BookSummary,
        details: Option<&BookDetails>,
        in_rail: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let muted = colors(cx).fg_muted;
        let average = book
            .average_rating
            .or_else(|| details.and_then(|d| d.summary.average_rating))
            .filter(|a| *a > 0.0);
        let ratings = details.and_then(|d| d.ratings_count);
        let readers = book
            .users_count
            .or_else(|| details.and_then(|d| d.summary.users_count))
            .filter(|n| *n > 0);
        let counts: Vec<String> = [
            ratings
                .filter(|n| *n > 0)
                .map(|n| plural(n, "rating", "ratings")),
            readers.map(|n| plural(n, "reader", "readers")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let breakdown = details
            .filter(|d| !d.ratings_distribution.is_empty())
            .map(|d| render_breakdown(d, cx));
        if in_rail {
            let figure = average.map(|average| {
                let text = average_rating_text(average);
                // The stars sit on the figure's baseline, as a run of body
                // text beside it would.
                h_flex()
                    .gap_2()
                    .child(div().display_30().child(text.clone()))
                    .child(
                        div()
                            .body()
                            .baseline_on(TypeRole::Body, TypeRole::Display30)
                            .child(
                                Stars::new(average)
                                    .accessibility_label("rail-average", format!("{text} average")),
                            ),
                    )
            });
            if figure.is_none() && breakdown.is_none() && counts.is_empty() {
                return None;
            }
            return Some(
                RailSection::new("On Hardcover")
                    .children(figure)
                    .when(!counts.is_empty(), |this| {
                        this.child(
                            div()
                                .body()
                                .tabular()
                                .text_color(muted)
                                .child(counts.join(" · ")),
                        )
                    })
                    .children(breakdown.map(|b| div().pt_2().child(b)))
                    .into_any_element(),
            );
        }
        // Without a spread there is nothing the header's average doesn't
        // say. "Open on Hardcover" is in the book's "more" menu.
        breakdown.as_ref()?;
        Some(
            Block::new("On Hardcover")
                .when(!counts.is_empty(), |this| {
                    this.action(div().caption(cx).tabular().child(counts.join(" · ")))
                })
                .children(breakdown)
                .into_any_element(),
        )
    }

    /// Similar books: lg covers with their shadow, 16 apart, each with its
    /// title under it, a text button in two lines at most.
    pub(super) fn render_similar(
        &self,
        book: &BookSummary,
        details: Option<&BookDetails>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let details = details?;
        if details.similar_book_ids.is_empty() {
            return None;
        }
        let muted = colors(cx).fg_muted;
        let body = match self.similar.get(&book.id) {
            Some(Similar::Ready(books)) if books.is_empty() => return None,
            Some(Similar::Ready(books)) => CoverShelf::new(CoverSize::Lg)
                .books(books.iter().map(|book| {
                    let id = book.id.get().cast_unsigned();
                    (
                        Cover::new(CoverSize::Lg, book.title.clone())
                            .author(book.authors.first().cloned())
                            .thumbnail(book.cover.as_ref().map(|c| c.url.clone()))
                            .shadow(true)
                            .opens(("similar-cover", id), Page::Book(book.clone())),
                        TextButton::book(("similar-title", id), book),
                    )
                }))
                .into_any_element(),
            Some(Similar::Failed(message)) => h_flex()
                .flex_wrap()
                .gap_2()
                .child(div().body().text_color(muted).child(message.clone()))
                .child(
                    Button::new("retry-similar")
                        .small_control()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_similar(cx))),
                )
                .into_any_element(),
            // Waiting out the delay or loading: the strip's shape, still.
            None => h_flex()
                .gap_4()
                .children((0..4).map(|_| Skeleton::cover(CoverSize::Lg).still()))
                .into_any_element(),
        };
        Some(Block::new("Similar books").child(body).into_any_element())
    }
}

/// "1 rating", "9,223 ratings".
fn plural(n: u32, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{} {many}", count_text(n)),
    }
}

/// The ratings breakdown: a row per star level, its share of the most
/// common level as a bar, and its count. Hardcover's figures, so muted ink
/// on `track`, not the accent of progress.
fn render_breakdown(details: &BookDetails, cx: &mut Context<BookDetailView>) -> AnyElement {
    let colors = colors(cx);
    let buckets = &details.ratings_distribution;
    let max = buckets.iter().map(|b| b.count).max().unwrap_or(1).max(1);
    v_flex()
        .w_full()
        .max_w(size::METER_MAX)
        .gap_1()
        .children(rating_rows(buckets).into_iter().map(|bucket| {
            let (rating, count) = (bucket.rating, bucket.count);
            let percent = crate::format::bar_percent(f64::from(count) / f64::from(max));
            h_flex()
                .gap_2()
                .caption(cx)
                .tabular()
                .child(
                    Lane::trailing().child(
                        h_flex()
                            .gap_0p5()
                            .child(rating_label(rating))
                            .child(crate::star::icon().size_3().text_color(colors.fg_muted)),
                    ),
                )
                .child(
                    Meter::new(SharedString::from(format!("rating-{rating}")), percent)
                        .muted()
                        .flex_1()
                        .w_auto()
                        .accessibility_label(format!("{} stars", rating_label(rating))),
                )
                .child(Lane::trailing().child(count_text(count)))
        }))
        .into_any_element()
}

/// One row per star level from 5 down to the lowest rated: half-star rows
/// only when the data has half stars. A missing level is an empty row.
fn rating_rows(buckets: &[RatingBucket]) -> Vec<RatingBucket> {
    if buckets.is_empty() {
        return Vec::new();
    }
    let halves = buckets.iter().any(|b| b.rating.fract() != 0.0);
    let step = if halves { 0.5 } else { 1.0 };
    let lowest = buckets
        .iter()
        .map(|b| b.rating)
        .fold(5.0_f32, f32::min)
        .max(step);
    let mut rows = Vec::new();
    let mut rating = 5.0_f32;
    while rating >= lowest {
        let found = buckets
            .iter()
            .find(|b| b.rating.total_cmp(&rating).is_eq())
            .copied();
        rows.push(found.unwrap_or(RatingBucket::new(rating, 0)));
        rating -= step;
    }
    rows
}

fn rating_label(rating: f32) -> String {
    if rating.fract() == 0.0 {
        format!("{rating:.0}")
    } else {
        format!("{rating:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(buckets: &[(f32, u32)]) -> Vec<(f32, u32)> {
        let buckets: Vec<RatingBucket> = buckets
            .iter()
            .map(|(r, c)| RatingBucket::new(*r, *c))
            .collect();
        rating_rows(&buckets)
            .into_iter()
            .map(|b| (b.rating, b.count))
            .collect()
    }

    #[test]
    fn rating_rows_fill_gaps_from_five_down() {
        assert_eq!(
            rows(&[(5.0, 4), (3.0, 1)]),
            vec![(5.0, 4), (4.0, 0), (3.0, 1)]
        );
        assert_eq!(
            rows(&[(4.5, 2), (1.0, 1)]),
            vec![
                (5.0, 0),
                (4.5, 2),
                (4.0, 0),
                (3.5, 0),
                (3.0, 0),
                (2.5, 0),
                (2.0, 0),
                (1.5, 0),
                (1.0, 1)
            ]
        );
        assert_eq!(rows(&[]), []);
    }
}
