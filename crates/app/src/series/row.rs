//! A series row (`hardcover-design`, Screens → Series): a sequence row (no
//! fill, a hairline between rows, 16 above and below), top-aligned.
//!
//! - The series name in `title-18`, a text button that opens its page; a
//!   right-to-left name is one truncated line, which GPUI lays out right.
//! - A muted line: the author, "2 of 6 read", "#6 not out yet" when that
//!   applies, and what last happened ("Last finished Sep 28").
//! - The strip of its books, 12 below ([`crate::ui::SeriesStrip`]), as many
//!   as fit, the next one kept in view.
//! - The next-step lane on the trailing edge, 240 wide on every row so the
//!   lanes line up: the next book's sm cover, a caption ("Reading · #1",
//!   "Next · #3"), its title as a text button, a line (progress, where it
//!   paused, or its release date) and one small command. A finished series
//!   keeps the lane, empty.
//!
//! Every row of a group is the same height ([`row_height`]), so the list can
//! be virtual: the name and every line are single, truncated lines.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, IconName, h_flex};
use gpui_kit::{
    App, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, ScrollHandle, SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
    rems,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{Date, ReadingStatus, UserBookId};

use super::progress::{
    Group, Mark, NextKind, SeriesBook, SeriesSummary, lane_caption, reading_line,
};
use crate::app_state::Services;
use crate::bidi::is_rtl;
use crate::book::progress_dialog::open_progress;
use crate::catalog::navigation::Page;
use crate::chip::{StatusChip, status_icon};
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{count_text, needs_permission, progress_summary};
use crate::library::model::{LibraryModel, WriteKind};
use crate::library::writes;
use crate::ui::focus::{keyed_focus, reveal_when};
use crate::ui::kit::SmallControl as _;
use crate::ui::{SeriesStrip, StripItem, TextButton};
use crate::upcoming::row::add_to_want_to_read;
use crate::upcoming::schedule::{release_text, soon_text};

/// Above and below a row (16).
const ROW_PADDING: f32 = 1.;
/// The name (24), the facts (20), 12, then the strip: an sm cover (48), the
/// 2 px line and the caption (16), 4 apart.
const ROW_CONTENT: f32 = 1.5 + 1.25 + 0.75 + 3. + 0.25 + 0.125 + 0.25 + 1.;
/// A group's label over its first row: 24 above, the label (16), 4 below.
const GROUP_LABEL: f32 = 1.5 + 1. + 0.25;

/// How tall a row is, its group's label included when it's the first.
pub fn row_height(first_in_group: bool, rem: Pixels) -> Pixels {
    let body = rems(ROW_CONTENT + 2. * ROW_PADDING).to_pixels(rem);
    if first_in_group {
        body + rems(GROUP_LABEL).to_pixels(rem)
    } else {
        // The hairline above.
        body + size::HAIRLINE
    }
}

/// How wide the strip gets in a column `column` wide: the row less the lane
/// and the gap before it.
pub fn strip_width(column: Pixels, rem: Pixels) -> Pixels {
    column - size::NEXT_LANE.to_pixels(rem) - rems(1.5).to_pixels(rem)
}

/// How many books a row's strip shows in a column `column` wide: as many as
/// fit beside the lane, before [`super::progress::strip_window`] picks
/// which.
pub fn strip_capacity(column: Pixels, rem: Pixels) -> usize {
    SeriesStrip::capacity(strip_width(column, rem), rem)
}

#[derive(IntoElement)]
pub struct SeriesRow {
    series: SeriesSummary,
    /// The group's label and count, over its first row.
    label: Option<(Group, usize)>,
    library: Entity<LibraryModel>,
    today: Date,
    /// Books the strip has room for.
    capacity: usize,
    /// The list's scroller, to keep a row with keyboard focus in view.
    scroll: ScrollHandle,
}

impl SeriesRow {
    pub fn new(
        series: SeriesSummary,
        label: Option<(Group, usize)>,
        library: Entity<LibraryModel>,
        today: Date,
        capacity: usize,
        scroll: ScrollHandle,
    ) -> Self {
        Self {
            series,
            label,
            library,
            today,
            capacity,
            scroll,
        }
    }
}

impl RenderOnce for SeriesRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let id = self.series.id;
        let key = SharedString::from(format!("series-row-{id}"));
        let focus: FocusHandle = keyed_focus(key.clone(), window, cx);
        // A margin of a few pixels keeps the neighbouring rows drawn, so Tab
        // and Shift+Tab can move on to them.
        let reveal = reveal_when(
            key.clone(),
            focus.contains_focused(window, cx),
            &self.scroll,
            rems(0.5).to_pixels(window.rem_size()),
            window,
            cx,
        );
        let first = self.label.is_some();

        let label = self.label.map(|(group, count)| {
            // Label and caption share one size, so centring keeps one baseline.
            h_flex()
                .gap_2()
                .pt_6()
                .pb_1()
                .child(div().label(cx).child(group.label()))
                .child(div().caption(cx).tabular().child(count_text(count)))
        });

        let row = h_flex()
            .w_full()
            .items_start()
            .gap_6()
            .py_4()
            .when(!first, |this| {
                this.border_t(size::HAIRLINE).border_color(colors.border)
            })
            .child(series_body(&self.series, self.today, self.capacity, cx))
            .child(next_lane(&self.series, &self.library, self.today, cx));

        // The label rides along: revealing the first row of the list shows
        // a sliver of the header above it, so Shift+Tab reaches the filters.
        div()
            .id(key)
            .track_focus(&focus)
            .relative()
            .w_full()
            .child(reveal)
            .children(label)
            .child(row)
    }
}

/// The row's main column: the name, the facts and the strip.
pub fn series_body(
    series: &SeriesSummary,
    today: Date,
    capacity: usize,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            TextButton::new(
                SharedString::from(format!("series-name-{}", series.id)),
                series.name.clone(),
            )
            .title_18()
            .opens(Page::Series(series.id)),
        )
        .child(facts_line(series, today, cx))
        .child(div().pt_3().child(strip(series, capacity)))
}

/// "Brandon Sanderson · 2 of 6 read · #6 not out yet · Reading now". A
/// right-to-left author is its own run, so the line keeps its order.
fn facts_line(series: &SeriesSummary, today: Date, cx: &App) -> impl IntoElement {
    let muted = colors(cx).fg_muted;
    let facts = series.facts(today);
    let line = h_flex().min_w_0().body().tabular().text_color(muted);
    match &series.author {
        Some(author) if is_rtl(author) => line
            .child(
                div()
                    .min_w_0()
                    .flex_shrink(1.)
                    .truncate()
                    .child(author.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(format!("\u{a0}· {facts}")),
            ),
        Some(author) => line.child(
            div()
                .min_w_0()
                .truncate()
                .child(format!("{author} · {facts}")),
        ),
        None => line.child(div().min_w_0().truncate().child(facts)),
    }
}

/// The books that fit, the next one kept in view.
fn strip(series: &SeriesSummary, capacity: usize) -> SeriesStrip {
    let (range, more) = super::progress::strip_window(series.books.len(), series.next, capacity);
    let items = series.books[range.clone()]
        .iter()
        .enumerate()
        .map(|(offset, book)| strip_item(series, book, range.start + offset));
    SeriesStrip::new().items(items).more(more)
}

fn strip_item(series: &SeriesSummary, book: &SeriesBook, ix: usize) -> StripItem {
    let id = SharedString::from(format!("series-{}-book-{}", series.id, book.book.id));
    let position = crate::format::series_position_text(book.position)
        .trim_start_matches('#')
        .to_owned();
    let item = match book.mark() {
        Mark::Shelved(status) => StripItem::cover(
            id.clone(),
            Cover::new(CoverSize::Sm, book.book.title.clone())
                .thumbnail(book.book.cover.as_ref().map(|c| c.url.clone()))
                .opens(
                    SharedString::from(format!("{id}-cover")),
                    Page::Book(book.book.clone()),
                ),
        )
        .icon(status_icon(status)),
        Mark::Missing => StripItem::missing(id.clone()),
        Mark::Unreleased => StripItem::unreleased(id.clone(), crate::icons::clock()),
    };
    item.position(position)
        .progress(
            (book.status == Some(ReadingStatus::CurrentlyReading))
                .then(|| book.progress.unwrap_or(0.)),
        )
        .next(series.next == Some(ix))
}

/// The one next step, in the 240 lane. Empty for a finished series.
fn next_lane(
    series: &SeriesSummary,
    library: &Entity<LibraryModel>,
    today: Date,
    cx: &App,
) -> impl IntoElement {
    let lane = div()
        .flex()
        .flex_none()
        .w(size::NEXT_LANE)
        .items_start()
        .gap_3();
    let (Some(book), Some(kind)) = (series.next_book(), series.next_kind()) else {
        return lane;
    };
    let colors = colors(cx);
    let id = series.id;
    let model = library.read(cx);
    let entry = book.entry.and_then(|e| model.entry(e)).cloned();
    let pending = book.entry.and_then(|e| model.pending_write(e));
    let can_write = Services::global(cx).has_scope(WRITE_LIBRARY);
    let reason = (!can_write).then(|| needs_permission(WRITE_LIBRARY));

    let book_details = match kind {
        NextKind::Reading => entry
            .as_ref()
            .and_then(|e| reading_line(&progress_summary(e)))
            .map(|text| div().caption(cx).tabular().child(text).into_any_element()),
        NextKind::Paused => entry.as_ref().map(|e| {
            div()
                .caption(cx)
                .tabular()
                .child(crate::format::paused_text(
                    &progress_summary(e),
                    None,
                    today,
                ))
                .into_any_element()
        }),
        NextKind::Waiting => book.release_date.map(|date| {
            h_flex()
                .min_w_0()
                .tabular()
                .child(div().body_strong().child(release_text(date, today)))
                .children(soon_text(date, today).map(|soon| {
                    div()
                        .caption(cx)
                        .whitespace_nowrap()
                        .child(format!("\u{a0}· {soon}"))
                }))
                .into_any_element()
        }),
        NextKind::Start | NextKind::Add => None,
    };

    let action = match kind {
        NextKind::Reading | NextKind::Paused => entry.map(|entry| {
            let library = library.clone();
            Button::new(SharedString::from(format!("series-update-{id}")))
                .small_control()
                .label("Update progress…")
                .loading(pending == Some(WriteKind::Progress))
                .disabled(pending.is_some() || !can_write)
                .when_some(reason.clone(), Button::tooltip)
                .on_click(move |_, window, cx| open_progress(library.clone(), &entry, window, cx))
                .into_any_element()
        }),
        NextKind::Start => book.entry.map(|entry| {
            let library = library.clone();
            Button::new(SharedString::from(format!("series-start-{id}")))
                .small_control()
                .label("Start reading")
                .loading(pending == Some(WriteKind::Status))
                .disabled(pending.is_some() || !can_write)
                .when_some(reason.clone(), Button::tooltip)
                .on_click(move |_, window, cx| start_reading(library.clone(), entry, window, cx))
                .into_any_element()
        }),
        NextKind::Add => Some(want_to_read(series, book, library, can_write, cx)),
        NextKind::Waiting => Some(match book.status {
            Some(status) => StatusChip(status).into_any_element(),
            None => want_to_read(series, book, library, can_write, cx),
        }),
    };

    lane.child(
        Cover::new(CoverSize::Sm, book.book.title.clone())
            .thumbnail(book.book.cover.as_ref().map(|c| c.url.clone()))
            .opens(
                SharedString::from(format!("series-next-cover-{id}")),
                Page::Book(book.book.clone()),
            ),
    )
    .child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .caption(cx)
                    .tabular()
                    .child(lane_caption(kind, book.position)),
            )
            .child(
                TextButton::book(SharedString::from(format!("series-next-{id}")), &book.book)
                    .text_color(colors.fg),
            )
            .children(book_details)
            .children(action.map(|action| h_flex().pt_2().child(action))),
    )
}

/// The small ghost "+ Want to read", its text on the title's spine.
fn want_to_read(
    series: &SeriesSummary,
    book: &SeriesBook,
    library: &Entity<LibraryModel>,
    can_write: bool,
    cx: &App,
) -> gpui_kit::AnyElement {
    let adding = library.read(cx).is_adding(book.book.id);
    let library = library.clone();
    let book_id = book.book.id;
    Button::new(SharedString::from(format!("series-add-{}", series.id)))
        .ghost()
        .small_control()
        .icon(IconName::Plus)
        .label("Want to read")
        .ml(rems(-0.5))
        .loading(adding)
        .disabled(adding || !can_write)
        .when(!can_write, |this| {
            this.tooltip(needs_permission(WRITE_LIBRARY))
        })
        .on_click(move |_, window, cx| {
            add_to_want_to_read(library.clone(), book_id, window, cx);
        })
        .into_any_element()
}

/// Moves a Want to read book to Currently reading, as Home's "Start
/// reading" does; the library applies the entry the write returns. A
/// failure is shown as a notification.
fn start_reading(library: Entity<LibraryModel>, id: UserBookId, window: &mut Window, cx: &mut App) {
    writes::set_status(library, id, ReadingStatus::CurrentlyReading, window, cx);
}

/// A row while the series load: a skeleton name line and a strip of
/// skeleton covers.
pub fn skeleton_row() -> impl IntoElement {
    use crate::ui::Skeleton;
    h_flex()
        .w_full()
        .items_start()
        .gap_6()
        .py_4()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(Skeleton::line().w_1_3().h_4())
                .child(Skeleton::line().w_1_2())
                .child(h_flex().pt_2().gap_1().children((0..5).map(|_| {
                    div()
                        .flex()
                        .justify_center()
                        .w(size::STRIP_ITEM)
                        .child(Skeleton::cover(CoverSize::Sm))
                }))),
        )
        .child(div().flex_none().w(size::NEXT_LANE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::px;

    const REM: Pixels = px(16.);

    /// The column in a window `window` wide: the region (less the sidebar)
    /// less the page's insets. It fills it.
    fn column(window: f32) -> Pixels {
        px(window - 240. - 64.)
    }

    #[test]
    fn the_strip_shows_as_many_books_as_fit_beside_the_lane() {
        // The minimum window: 496 − 264 = 232, five books (5 × 44 − 4).
        assert_eq!(strip_capacity(column(800.), REM), 5);
        // The default window: 896 − 264 = 632, fourteen, as before.
        assert_eq!(strip_capacity(column(1200.), REM), 14);
        // Wider, the strip takes the width: 1032 at 1600, 1992 at 2560.
        assert_eq!(strip_capacity(column(1600.), REM), 23);
        assert_eq!(strip_capacity(column(2560.), REM), 45);
        // At the edge: n books need n × 44 − 4.
        assert_eq!(strip_capacity(px(20. * 44. - 4. + 264.), REM), 20);
        assert_eq!(strip_capacity(px(20. * 44. - 5. + 264.), REM), 19);
    }

    #[test]
    fn the_lane_keeps_its_width_and_the_strip_takes_the_rest() {
        for window in [800., 1200., 1600., 2560.] {
            assert_eq!(
                column(window) - strip_width(column(window), REM),
                px(240. + 24.)
            );
        }
    }
}
