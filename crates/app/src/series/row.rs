//! A series row (`hardcover-design`, Screens → Series): a sequence row (no
//! fill, a hairline between rows, 16 above and below), top-aligned.
//!
//! - The series name in `display-24`, a text button that opens its page; a
//!   right-to-left name is one truncated line, which GPUI lays out right.
//!   On the line's trailing edge, a small "more" icon button whose menu
//!   hides the series from the page ("Hide series"), or, under the Hidden
//!   filter, shows it again ("Show series again").
//! - A muted line: the author, "#6 not out yet" when that applies, and what
//!   last happened ("Last finished Sep 28").
//! - The series' meter (read of what is out, at most `size::METER_MAX`) with
//!   "2 of 6 read" beside it, 8 below.
//! - The key, a caption line 4 below: each mark the strip shows, with how
//!   many books carry it ("6 read", "11 not in your library").
//! - The strip of its books, 24 below, room for the next book's "Next"
//!   ([`crate::ui::SeriesStrip`]): one line, from the book before the next
//!   one, with "Show the other N books" when they don't all fit; opened,
//!   every book. Which series are opened is the tracker's, for the session.
//! - The next-step lane on the trailing edge, 240 wide on every row so the
//!   lanes line up: the next book's md cover, a caption ("Reading · #1",
//!   "Next · #3"), its title as a text button, a line (progress, where it
//!   paused, or its release date) and one small command. A finished series
//!   keeps the lane, empty.
//!
//! A row's height is known before it is drawn ([`row_height`]), so the list
//! can be virtual: the name and every line are single, truncated lines, and
//! the strip's lines are worked out from the width ([`strip_lines`]).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, IconName, h_flex};
use std::rc::Rc;

use gpui_kit::component::Icon;
use gpui_kit::{
    App, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Rems, RenderOnce, ScrollHandle, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{Date, ReadingStatus, SeriesId, UserBookId};

use super::progress::{
    Group, Mark, NextKind, SeriesBook, SeriesSummary, key_text, lane_caption, reading_line,
};
use crate::bidi::is_rtl;
use crate::book::progress_dialog::open_progress;
use crate::catalog::navigation::Page;
use crate::chip::{StatusChip, status_icon};
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::{count_text, needs_permission, progress_summary};
use crate::library::model::{LibraryModel, WriteKind};
use crate::library::writes;
use crate::session;
use crate::ui::focus::{keyed_focus, reveal_when};
use crate::ui::kit::KeyboardRing as _;
use crate::ui::kit::OnTextSpine as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::SmallIconButton as _;
use crate::ui::{MISSING_OPACITY, Meter, SeriesStrip, StripItem, TextButton, Toggle, strip_layout};
use crate::upcoming::row::add_to_want_to_read;
use crate::upcoming::schedule::{release_text, soon_text};

// The tracker's virtual list must know each row's height before drawing
// it, so the rows are laid out from these and the heights summed from the
// same values (`a_rows_height_is_what_it_draws` checks the two agree).

/// Above and below a row (16).
const ROW_PADDING: Rems = rems(1.);
/// Between the facts and the meter's line (8).
const METER_ABOVE: Rems = rems(0.5);
/// Between the meter's line and the key (4).
const KEY_ABOVE: Rems = rems(0.25);
/// Between the key and the strip, where the next book's "Next" sits (24).
const STRIP_ABOVE: Rems = rems(1.5);
/// Between the key's entries (16).
const KEY_GAP: Rems = rems(1.);
/// The key's mark for a book not in the library: a small faded cover
/// (8 × 12).
const KEY_COVER_WIDTH: Rems = rems(0.5);
const KEY_COVER_HEIGHT: Rems = rems(0.75);
/// Above a group's label (24).
const LABEL_ABOVE: Rems = rems(1.5);
/// Between a group's label and its first row (4).
const LABEL_BELOW: Rems = rems(0.25);

/// What sits over a row: its group's label (the first of a group), the
/// hairline between rows, or nothing (the first row of a list without
/// groups, right under the header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowTop {
    Label(Group, usize),
    Hairline,
    Bare,
}

/// How tall a row is, with what sits over it ([`RowTop`]): the name, the
/// facts, the meter, the key and a strip of `lines` lines ([`strip_lines`]),
/// inside the row's padding.
pub fn row_height(top: RowTop, lines: usize, rem: Pixels) -> Pixels {
    let body = (ROW_PADDING * 2.
        + TypeRole::Display24.line_height()
        + TypeRole::Body.line_height()
        + METER_ABOVE
        + TypeRole::Body.line_height()
        + KEY_ABOVE
        + TypeRole::Caption.line_height()
        + STRIP_ABOVE)
        .to_pixels(rem)
        + SeriesStrip::height(lines, rem);
    match top {
        RowTop::Label(..) => body + group_label_height(rem),
        RowTop::Hairline => body + size::HAIRLINE,
        RowTop::Bare => body,
    }
}

/// How tall a group's label is, with the space around it.
fn group_label_height(rem: Pixels) -> Pixels {
    (LABEL_ABOVE + LABEL_BELOW + TypeRole::Label.line_height()).to_pixels(rem)
}

/// A group's label and count, over its first row. Label and caption share
/// one size, so centring keeps one baseline.
fn group_label(group: Group, count: usize, cx: &App) -> gpui_kit::Div {
    h_flex()
        .gap_2()
        .pt(LABEL_ABOVE)
        .pb(LABEL_BELOW)
        .child(div().label(cx).child(group.label()))
        .child(div().caption(cx).tabular().child(count_text(count)))
}

/// How wide the strip gets in a column `column` wide: the row less the lane
/// and the gap before it.
pub fn strip_width(column: Pixels, rem: Pixels) -> Pixels {
    column - (size::NEXT_LANE + size::NEXT_LANE_GAP).to_pixels(rem)
}

/// How many books fit on a line of a row's strip in a column `column` wide:
/// as many as fit beside the lane.
pub fn strip_capacity(column: Pixels, rem: Pixels) -> usize {
    SeriesStrip::capacity(strip_width(column, rem), rem)
}

/// How many lines a series' strip takes, with room for `capacity` books on
/// a line, shut or `open`.
pub fn strip_lines(series: &SeriesSummary, capacity: usize, open: bool) -> usize {
    strip_layout(series.books.len(), series.next, capacity, open).lines
}

/// What a strip's toggle does: opens or shuts the series' strip.
pub type OnToggle = Rc<dyn Fn(SeriesId, &mut Window, &mut App)>;

/// What the row's menu does: hides the series (`true`) or shows it again.
pub type OnHide = Rc<dyn Fn(SeriesId, bool, &mut Window, &mut App)>;

/// Whether a row's series is hidden, and what its menu does about it.
#[derive(Clone)]
pub struct RowHiding {
    pub hidden: bool,
    pub on_hide: OnHide,
}

/// How a row draws its strip: the room on a line, whether it is opened to
/// every book, and what its toggle does.
#[derive(Clone)]
pub struct RowStrip {
    pub capacity: usize,
    pub open: bool,
    pub on_toggle: OnToggle,
}

#[derive(IntoElement)]
pub struct SeriesRow {
    series: SeriesSummary,
    /// The group's label and count over a group's first row, else the
    /// hairline or nothing.
    top: RowTop,
    library: Entity<LibraryModel>,
    today: Date,
    strip: RowStrip,
    hiding: RowHiding,
    /// The list's scroller, to keep a row with keyboard focus in view.
    scroll: ScrollHandle,
}

impl SeriesRow {
    pub fn new(
        series: SeriesSummary,
        top: RowTop,
        library: Entity<LibraryModel>,
        today: Date,
        strip: RowStrip,
        hiding: RowHiding,
        scroll: ScrollHandle,
    ) -> Self {
        Self {
            series,
            top,
            library,
            today,
            strip,
            hiding,
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
        let label = match self.top {
            RowTop::Label(group, count) => Some(group_label(group, count, cx)),
            RowTop::Hairline | RowTop::Bare => None,
        };

        let row = h_flex()
            .w_full()
            .items_start()
            .gap(size::NEXT_LANE_GAP)
            .py(ROW_PADDING)
            .when(self.top == RowTop::Hairline, |this| {
                this.border_t(size::HAIRLINE).border_color(colors.border)
            })
            .child(series_body(
                &self.series,
                self.today,
                &self.strip,
                Some(more_button(self.series.id, &self.hiding, window, cx)),
                cx,
            ))
            .child(next_lane(&self.series, &self.library, self.today, cx));

        // The label rides along, so revealing a group's first row shows its
        // label too.
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

/// The row's main column: the name with the "more" button, the facts, the
/// meter, the key and the strip.
pub fn series_body(
    series: &SeriesSummary,
    today: Date,
    strip_of: &RowStrip,
    more: Option<gpui_kit::AnyElement>,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            h_flex()
                .h(TypeRole::Display24.line_height())
                .gap_2()
                .child(
                    div().flex_1().min_w_0().child(
                        TextButton::new(
                            SharedString::from(format!("series-name-{}", series.id)),
                            series.name.clone(),
                        )
                        .display_24()
                        .opens(Page::Series(series.id)),
                    ),
                )
                .children(more.map(|more| div().flex_none().child(more))),
        )
        .child(facts_line(series, today, cx))
        .child(meter_line(series, cx))
        .child(key_line(series, cx))
        .child(div().pt(STRIP_ABOVE).child(strip(series, strip_of)))
}

/// The small "more" button on the name's line: "Hide series", or "Show
/// series again" for a hidden one.
pub fn more_button(
    id: SeriesId,
    hiding: &RowHiding,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::AnyElement {
    let trigger = keyed_focus(SharedString::from(format!("series-more-{id}")), window, cx);
    let hide = !hiding.hidden;
    let on_hide = hiding.on_hide.clone();
    Button::new(SharedString::from(format!("series-more-button-{id}")))
        .ghost()
        .small_icon_button()
        .icon(IconName::Ellipsis)
        .tooltip("Series actions")
        .keyboard_ring(&trigger, window)
        .dropdown_menu(move |menu, _, _| {
            let on_hide = on_hide.clone();
            let (label, icon) = if hide {
                ("Hide series", IconName::EyeOff)
            } else {
                ("Show series again", IconName::Eye)
            };
            menu.item(
                PopupMenuItem::new(label)
                    .icon(icon)
                    .on_click(move |_, window, cx| on_hide(id, hide, window, cx)),
            )
        })
        .into_any_element()
}

/// The series' meter and "2 of 6 read" beside it.
fn meter_line(series: &SeriesSummary, cx: &App) -> impl IntoElement {
    h_flex()
        .pt(METER_ABOVE)
        .h(METER_ABOVE + TypeRole::Body.line_height())
        .gap_3()
        .child(div().flex_1().max_w(size::METER_MAX).child(Meter::new(
            SharedString::from(format!("series-meter-{}", series.id)),
            series.read_percent(),
            "Series read",
        )))
        .child(
            div()
                .flex_none()
                .body_strong()
                .tabular()
                .text_color(colors(cx).fg)
                .child(series.read_text()),
        )
}

/// What the strip's marks stand for: "✓ 6 read · ▯ 11 not in your
/// library", each with the mark the strip draws.
fn key_line(series: &SeriesSummary, cx: &App) -> impl IntoElement {
    let colors = colors(cx);
    h_flex()
        .pt(KEY_ABOVE)
        .h(KEY_ABOVE + TypeRole::Caption.line_height())
        .min_w_0()
        .overflow_hidden()
        .gap(KEY_GAP)
        .caption(cx)
        .tabular()
        .whitespace_nowrap()
        .children(series.key().into_iter().map(|(mark, count)| {
            let symbol = match mark {
                Mark::Shelved(status) => Icon::new(status_icon(status))
                    .size_3()
                    .flex_none()
                    .into_any_element(),
                Mark::Missing => div()
                    .flex_none()
                    .w(KEY_COVER_WIDTH)
                    .h(KEY_COVER_HEIGHT)
                    .rounded(rems(0.125))
                    .bg(colors.fg_muted)
                    .opacity(MISSING_OPACITY)
                    .into_any_element(),
                Mark::Unreleased => Icon::from(crate::icons::clock())
                    .size_3()
                    .flex_none()
                    .into_any_element(),
            };
            h_flex()
                .flex_none()
                .gap_1()
                .child(symbol)
                .child(key_text(mark, count))
        }))
}

/// "Brandon Sanderson · #6 not out yet · Reading now". A right-to-left
/// author is its own run, so the line keeps its order.
fn facts_line(series: &SeriesSummary, today: Date, cx: &App) -> impl IntoElement {
    let muted = colors(cx).fg_muted;
    let facts = series.facts(today);
    let line = h_flex()
        .min_w_0()
        .h(TypeRole::Body.line_height())
        .body()
        .tabular()
        .text_color(muted);
    let joined = |author: &str| {
        if facts.is_empty() {
            author.to_owned()
        } else {
            format!("{author} · {facts}")
        }
    };
    match &series.author {
        Some(author) if is_rtl(author) && !facts.is_empty() => line
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
        Some(author) => line.child(div().min_w_0().truncate().child(joined(author))),
        None => line.child(div().min_w_0().truncate().child(facts)),
    }
}

/// One line of books from the one before the next, and the toggle; opened,
/// every book.
fn strip(series: &SeriesSummary, row: &RowStrip) -> SeriesStrip {
    let layout = strip_layout(series.books.len(), series.next, row.capacity, row.open);
    let start = layout.range.start;
    let items = series.books[layout.range]
        .iter()
        .enumerate()
        .map(|(offset, book)| strip_item(series, book, start + offset));
    let strip = SeriesStrip::new().items(items);
    match layout.toggle {
        Some(toggle) => strip.toggle(
            toggle.slots(),
            toggle_button(series.id, toggle, &row.on_toggle),
        ),
        None => strip,
    }
}

/// "Show the other 15 books" or "Show fewer": a small ghost button.
fn toggle_button(id: SeriesId, toggle: Toggle, on_toggle: &OnToggle) -> Button {
    let label = match toggle {
        Toggle::More(1) => "Show the other book".to_owned(),
        Toggle::More(n) => format!("Show the other {} books", count_text(n)),
        Toggle::Fewer => "Show fewer".to_owned(),
    };
    let on_toggle = on_toggle.clone();
    Button::new(SharedString::from(format!("series-strip-toggle-{id}")))
        .ghost()
        .small_control()
        .label(label)
        .on_click(move |_, window, cx| on_toggle(id, window, cx))
}

fn strip_item(series: &SeriesSummary, book: &SeriesBook, ix: usize) -> StripItem {
    let id = SharedString::from(format!("series-{}-book-{}", series.id, book.book.id));
    let position = crate::format::series_position_text(book.position);
    let item = match book.mark() {
        Mark::Shelved(status) => StripItem::cover(
            id.clone(),
            Cover::new(CoverSize::Md, book.book.title.clone())
                .thumbnail(book.book.cover.as_ref().map(|c| c.url.clone()))
                .opens(
                    SharedString::from(format!("{id}-cover")),
                    Page::Book(book.book.clone()),
                ),
        )
        .shelf(status),
        Mark::Missing => StripItem::missing(
            id.clone(),
            Cover::new(CoverSize::Md, book.book.title.clone())
                .thumbnail(book.book.cover.as_ref().map(|c| c.url.clone()))
                .opens(
                    SharedString::from(format!("{id}-cover")),
                    Page::Book(book.book.clone()),
                ),
        ),
        Mark::Unreleased => StripItem::unreleased(id.clone(), book.book.title.clone())
            .opens(Page::Book(book.book.clone())),
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
    let can_write = session::has_scope(cx, WRITE_LIBRARY);
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
        Cover::new(CoverSize::Md, book.book.title.clone())
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
        .on_text_spine(true)
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
    writes::set_status(&library, id, ReadingStatus::CurrentlyReading, window, cx);
}

/// A row while the series load: a skeleton name line and a strip of
/// skeleton md covers.
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
                .child(Skeleton::line().w_1_2())
                .child(
                    h_flex()
                        .pt_4()
                        .gap(size::STRIP_GAP)
                        .children((0..5).map(|_| Skeleton::cover(CoverSize::Md))),
                ),
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
        // The minimum window: 496 − 264 = 232, three books (3 × 80 − 8).
        assert_eq!(strip_capacity(column(800.), REM), 3);
        // The default window: 896 − 264 = 632, eight.
        assert_eq!(strip_capacity(column(1200.), REM), 8);
        // Wider, the strip takes the width: 1032 at 1600, 1992 at 2560.
        assert_eq!(strip_capacity(column(1600.), REM), 13);
        assert_eq!(strip_capacity(column(2560.), REM), 25);
        // At the edge: n books need n × 80 − 8.
        assert_eq!(strip_capacity(px(12. * 80. - 8. + 264.), REM), 12);
        assert_eq!(strip_capacity(px(12. * 80. - 9. + 264.), REM), 11);
    }

    /// A started series of three books: the first read, the second being
    /// read (so its strip book has a meter), the third not in the library.
    fn started_series(name: &str) -> SeriesSummary {
        use super::super::progress::{LibraryIndex, summarize};
        use hardcover_core::{
            AuthorId, AuthorRef, BookId, BookSummary, LibraryEntry, ReadId, ReadSession, SeriesId,
            SeriesRef, TrackedBook, TrackedSeries, UserId,
        };
        let day = |s: &str| -> Date { s.parse().unwrap() };
        let book = |id: i64| {
            let mut b = BookSummary::new(BookId::new(id), format!("Book {id}"));
            b.release_year = Some(2020);
            b
        };
        let mut series = TrackedSeries::new(SeriesId::new(1), name);
        series.author = Some(AuthorRef::new(AuthorId::new(1), "Ada Quill"));
        for id in 1..=3 {
            let mut b = TrackedBook::new(book(id), id as f32);
            b.release_date = Some(day("2020-01-01"));
            series.books.push(b);
        }
        let entry = |id: i64, status| {
            let mut e =
                LibraryEntry::new(UserBookId::new(id * 10), UserId::new(1), book(id), status);
            let mut r = SeriesRef::new(name);
            r.id = Some(SeriesId::new(1));
            r.position = Some(id as f32);
            e.series.push(r);
            e
        };
        let mut read = entry(1, ReadingStatus::Read);
        let mut finished = ReadSession::new(ReadId::new(10));
        finished.finished_at = Some(day("2026-01-01"));
        read.reads.push(finished);
        let mut reading = entry(2, ReadingStatus::CurrentlyReading);
        let mut open = ReadSession::new(ReadId::new(20));
        open.started_at = Some(day("2026-11-01"));
        open.progress_pages = Some(100);
        reading.reads.push(open);
        reading.book.pages = Some(400);
        let entries = [read, reading];
        summarize(
            &series,
            &LibraryIndex::new(&entries),
            &std::collections::HashMap::new(),
            day("2026-11-20"),
        )
    }

    /// The tracker's virtual list sizes each row with [`row_height`] before
    /// drawing it; a row drawn taller or shorter would overlap or gap.
    #[gpui_kit::test]
    fn a_rows_height_is_what_it_draws(cx: &mut gpui_kit::TestAppContext) {
        use crate::ui::test_support::host;
        use gpui_kit::InteractiveElement as _;

        // A Latin and a Persian name (another font's line box), at the
        // default rem and a larger one.
        for (name, rem) in [
            ("The Long Road", REM),
            ("کلیدر (جلد اول)", REM),
            ("The Long Road", px(20.)),
        ] {
            let series = started_series(name);
            assert!(
                series.books.iter().any(|b| b.progress.is_some()),
                "the strip draws a meter"
            );
            // Room for two books on a line: shut, one book and the toggle on
            // a line of its own; opened (the Persian name), the three on two
            // lines and "Show fewer" on a third.
            let open = name.starts_with('ک');
            let lines = strip_lines(&series, 2, open);
            assert_eq!(lines, if open { 3 } else { 2 }, "{name}");
            let strip_of = RowStrip {
                capacity: 2,
                open,
                on_toggle: Rc::new(|_, _, _| {}),
            };
            let hiding = RowHiding {
                hidden: false,
                on_hide: Rc::new(|_, _, _, _| {}),
            };
            let cx = host(cx, move |window, cx| {
                window.set_rem_size(rem);
                let today: Date = "2026-11-20".parse().unwrap();
                div()
                    .w(px(1000.))
                    .child(group_label(Group::InProgress, 3, cx).debug_selector(|| "label".into()))
                    .child(
                        // As wide as two books, so the strip wraps where the
                        // tracker's width would make it.
                        h_flex()
                            .w(SeriesStrip::item_width(rem) * 2. + size::STRIP_GAP.to_pixels(rem))
                            .py(ROW_PADDING)
                            .items_start()
                            .debug_selector(|| "row".into())
                            .child(series_body(
                                &series,
                                today,
                                &strip_of,
                                Some(more_button(series.id, &hiding, window, cx)),
                                cx,
                            )),
                    )
                    .into_any_element()
            });
            let label = cx.debug_bounds("label").expect("the label is drawn");
            let row = cx.debug_bounds("row").expect("the row is drawn");
            assert_eq!(
                label.size.height,
                group_label_height(rem),
                "{name} at {rem:?}"
            );
            assert_eq!(
                row.size.height,
                row_height(RowTop::Bare, lines, rem),
                "{name} at {rem:?}"
            );
            assert_eq!(
                row.size.height + size::HAIRLINE,
                row_height(RowTop::Hairline, lines, rem),
                "{name} at {rem:?}"
            );
            assert_eq!(
                label.size.height + row.size.height,
                row_height(RowTop::Label(Group::InProgress, 3), lines, rem),
                "{name} at {rem:?}"
            );
        }
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
