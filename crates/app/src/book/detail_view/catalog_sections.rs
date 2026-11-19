//! The detail pane's catalogue parts: contributor and series links in the
//! header, and the ratings, tags, other titles and similar books sections.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, IntoElement, ObjectFit, ParentElement as _,
    SharedString, Styled as _, StyledImage as _, Window, div, img, prelude::FluentBuilder as _,
    relative, rems,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{BookDetails, BookId, BookSummary, Contributor, RatingBucket, SeriesRef};

use super::{BookDetailView, Details, section};
use crate::app_state::{Services, cover_thumbnail};
use crate::catalog::navigation::{self, Page};
use crate::format::{authors_text, series_position_text};

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
        self._similar_task = Some(cx.spawn(async move |this, cx| {
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

    fn retry_similar(&mut self, cx: &mut Context<Self>) {
        if let Some(book) = self.book(cx) {
            self.similar.remove(&book.id);
        }
        self.similar_loading = None;
        self.load_similar(cx);
        cx.notify();
    }

    // ----- header parts -----

    /// The authors as links once the details name them, otherwise as text.
    /// Other contributors follow on their own line with their roles.
    pub(super) fn render_contributors(
        &self,
        book: &BookSummary,
        details: Option<&BookDetails>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let contributors = details.map(|d| d.contributors.as_slice()).unwrap_or(&[]);
        let authors: Vec<&Contributor> = contributors.iter().filter(|c| c.is_author()).collect();
        let others: Vec<&Contributor> = contributors.iter().filter(|c| !c.is_author()).collect();
        let authors_line = if authors.is_empty() && !others.is_empty() {
            // Only non-author credits: the roles line below names everyone, and
            // `book.authors` would repeat the same people without their roles.
            div().into_any_element()
        } else if authors.is_empty() {
            div()
                .text_base()
                .child(authors_text(&book.authors))
                .into_any_element()
        } else {
            h_flex()
                .flex_wrap()
                .text_base()
                .children(authors.iter().enumerate().map(|(i, c)| {
                    h_flex()
                        .when(i > 0, |this| this.child(div().pr_1().child(",")))
                        .child(contributor_name(c, ("author", i)))
                }))
                .into_any_element()
        };
        v_flex()
            .gap_0p5()
            .child(authors_line)
            .when(!others.is_empty(), |this| {
                this.child(
                    h_flex()
                        .flex_wrap()
                        .gap_x_1()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .children(others.iter().enumerate().map(|(i, c)| {
                            let role = c.role.clone().unwrap_or_default();
                            h_flex()
                                .gap_1()
                                .child(contributor_name(c, ("contributor", i)))
                                .child(if i + 1 < others.len() {
                                    format!("({role}),")
                                } else {
                                    format!("({role})")
                                })
                        })),
                )
            })
            .into_any_element()
    }

    /// The series fact as a link to the series page, when Hardcover gave its id.
    pub(super) fn render_series_link(&self, series: &SeriesRef) -> Option<AnyElement> {
        let id = series.id?;
        let label = match series.position {
            Some(p) => format!("{} {}", series.name, series_position_text(p)),
            None => series.name.clone(),
        };
        Some(
            Button::new("open-series")
                .link()
                .label(label)
                .tooltip("Open the series")
                .on_click(move |_, _, cx| navigation::open(Page::Series(id), cx))
                .into_any_element(),
        )
    }

    // ----- sections -----

    /// The catalogue sections that have something to show, in order.
    pub(super) fn render_catalog_sections(
        &self,
        book: &BookSummary,
        details: Option<&BookDetails>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(details) = details else {
            return Vec::new();
        };
        let mut sections = Vec::new();
        if !details.ratings_distribution.is_empty() {
            sections.push(self.render_ratings(details, cx));
        }
        if let Some(tags) = self.render_tags(details, cx) {
            sections.push(tags);
        }
        if !details.alternative_titles.is_empty() {
            sections.push(render_other_titles(&details.alternative_titles, cx));
        }
        if let Some(similar) = self.render_similar(book, details, window, cx) {
            sections.push(similar);
        }
        sections
    }

    fn render_ratings(&self, details: &BookDetails, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let buckets = &details.ratings_distribution;
        let total: u32 = buckets.iter().map(|b| b.count).sum();
        let max = buckets.iter().map(|b| b.count).max().unwrap_or(1).max(1);
        let caption = match details.ratings_count.unwrap_or(total) {
            1 => "1 rating".to_owned(),
            n => format!("{n} ratings"),
        };
        section("Ratings")
            .child(div().text_sm().text_color(muted).child(caption))
            .child(v_flex().w_full().max_w(rems(24.)).gap_1().children(
                rating_rows(buckets).into_iter().map(|bucket| {
                    let (rating, count) = (bucket.rating, bucket.count);
                    let fraction = count as f32 / max as f32;
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .w_10()
                                .justify_end()
                                .gap_0p5()
                                .text_color(muted)
                                .child(rating_label(rating))
                                .child(Icon::new(IconName::StarFill).xsmall()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h_2()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().muted)
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(fraction))
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().chart_1),
                                ),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .w_12()
                                .text_right()
                                .text_color(muted)
                                .child(count.to_string()),
                        )
                }),
            ))
            .into_any_element()
    }

    fn render_tags(&self, details: &BookDetails, cx: &mut Context<Self>) -> Option<AnyElement> {
        let groups: Vec<(&'static str, &[String])> = [
            ("Genres", details.genres.as_slice()),
            ("Moods", details.moods.as_slice()),
            ("Content warnings", details.content_warnings.as_slice()),
            ("Other tags", details.tags.as_slice()),
        ]
        .into_iter()
        .filter(|(_, tags)| !tags.is_empty())
        .collect();
        if groups.is_empty() {
            return None;
        }
        let hidden: usize = groups
            .iter()
            .map(|(_, tags)| tags.len().saturating_sub(TAGS_SHOWN))
            .sum();
        let expanded = self.tags_expanded;
        let muted = cx.theme().muted_foreground;
        Some(
            section("Tags")
                .children(groups.into_iter().map(|(label, tags)| {
                    let shown = if expanded {
                        tags.len()
                    } else {
                        tags.len().min(TAGS_SHOWN)
                    };
                    // The label column lines up with the form labels above.
                    h_flex()
                        .items_start()
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(rems(6.))
                                .pr_2()
                                .text_sm()
                                .text_color(muted)
                                .child(label),
                        )
                        .child(
                            h_flex().flex_1().min_w_0().flex_wrap().gap_1().children(
                                tags[..shown]
                                    .iter()
                                    .map(|t| Tag::secondary().small().child(t.clone())),
                            ),
                        )
                }))
                .when(hidden > 0, |this| {
                    this.child(
                        // A row, so the button keeps its own width.
                        h_flex().child(
                            Button::new("toggle-tags")
                                .ghost()
                                .xsmall()
                                .label(if expanded {
                                    "Show fewer tags".to_owned()
                                } else {
                                    format!("Show {hidden} more")
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.tags_expanded = !this.tags_expanded;
                                    cx.notify();
                                })),
                        ),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_similar(
        &self,
        book: &BookSummary,
        details: &BookDetails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if details.similar_book_ids.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let body = match self.similar.get(&book.id) {
            Some(Similar::Ready(books)) if books.is_empty() => return None,
            Some(Similar::Ready(books)) => h_flex()
                .flex_wrap()
                .items_start()
                .gap_3()
                .children(
                    books
                        .iter()
                        .map(|b| similar_book(b, window, cx).into_any_element()),
                )
                .into_any_element(),
            Some(Similar::Failed(message)) => h_flex()
                .gap_2()
                .text_sm()
                .child(div().text_color(muted).child(message.clone()))
                .child(
                    Button::new("retry-similar")
                        .small()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_similar(cx))),
                )
                .into_any_element(),
            // Waiting out the delay or loading: the strip's shape, still.
            None => h_flex()
                .gap_3()
                .children((0..4).map(|_| {
                    div()
                        .w_16()
                        .h_24()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().skeleton)
                }))
                .into_any_element(),
        };
        Some(section("Similar books").child(body).into_any_element())
    }
}

/// A contributor's name: a link to their author page when the id is known.
fn contributor_name(c: &Contributor, id: (&'static str, usize)) -> AnyElement {
    match c.author_id {
        Some(author) => Button::new(id)
            .link()
            .label(c.name.clone())
            .on_click(move |_, _, cx| navigation::open(Page::Author(author), cx))
            .into_any_element(),
        None => div().child(c.name.clone()).into_any_element(),
    }
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
        let found = buckets.iter().find(|b| b.rating == rating).copied();
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

fn render_other_titles(titles: &[String], cx: &mut Context<BookDetailView>) -> AnyElement {
    let mut text = titles
        .iter()
        .take(TITLES_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");
    if titles.len() > TITLES_SHOWN {
        text.push_str(&format!(" · and {} more", titles.len() - TITLES_SHOWN));
    }
    section("Also known as")
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
        .into_any_element()
}

/// A cover with its title under it; opens the book.
fn similar_book(
    book: &BookSummary,
    window: &mut Window,
    cx: &mut Context<BookDetailView>,
) -> impl IntoElement {
    let thumbnail = book
        .cover
        .as_ref()
        .and_then(|c| cover_thumbnail(&c.url, window, cx));
    let muted = cx.theme().muted_foreground;
    let page = Page::Book(book.clone());
    Button::new(("similar-book", book.id.get() as u64))
        .text()
        .tooltip(book.title.clone())
        .on_click(move |_, _, cx| navigation::open(page.clone(), cx))
        .child(
            v_flex()
                .w_16()
                .gap_1()
                .child(
                    div()
                        .w_16()
                        .h_24()
                        .rounded(cx.theme().radius)
                        .overflow_hidden()
                        .bg(cx.theme().muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .map(|this| match thumbnail {
                            Some(image) => {
                                this.child(img(image).size_full().object_fit(ObjectFit::Cover))
                            }
                            None => {
                                this.child(Icon::new(IconName::BookOpen).small().text_color(muted))
                            }
                        }),
                )
                .child(
                    // Buttons keep labels on one line; titles may take two.
                    div()
                        .w_16()
                        .whitespace_normal()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .line_clamp(2)
                        .child(book.title.clone()),
                ),
        )
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
        assert!(rows(&[]).is_empty());
    }
}
