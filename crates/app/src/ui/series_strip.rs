//! A series as a shelf of books (`hardcover-design`, Screens → Series), and
//! the empty slot that stands in for a book not out yet.
//!
//! - [`CoverSlot`]: a dashed outline in `border-strong` the size and shape of
//!   a cover at a system size, empty, or with a 12 px icon (the clock for a
//!   book not out yet). It is not a cover: no fill, no edge. Like a cover, a
//!   click can open the book ([`CoverSlot::opens`]).
//! - [`SeriesStrip`]: the books of a series in order, 8 apart, each 72 wide:
//!   its md cover (faded to [`MISSING_OPACITY`] for a book that is out but
//!   not in the library; a clock slot for one not out yet) inside a 2 px
//!   inset that holds the `accent` ring of the series' next book, with
//!   "Next" over it; a 2 px line under it (a meter for the book being read,
//!   else empty, so every position sits on one line); its shelf's 12 px icon
//!   and its position in a caption, the next book's in ink at weight 600;
//!   then its title, two lines at most, centred. State never relies on
//!   colour: the shelf icon (a faded cover has none), the slot, the clock and
//!   the "Next" label carry it.
//! - A strip is one line until it is opened ([`strip_layout`]): as many books
//!   as fit, from the one before the next book, then the toggle ("Show the
//!   other 15 books"); opened, every book wraps onto as many lines as it
//!   needs, ending in "Show fewer". The toggle takes the room of
//!   [`MORE_SLOTS`] or [`FEWER_SLOTS`] books at the end of its line, so the
//!   lines are worked out before they are drawn ([`SeriesStrip::height`]).
//! - Each book is named for a screen reader with everything the eye reads
//!   from it ([`strip_label`]): "#2, Dune Messiah: Currently reading, 40%,
//!   next"; a slot says "not in your library" or "not out yet". Hovering a
//!   book shows its whole title in a tooltip.
//!
//! ```ignore
//! SeriesStrip::new()
//!     .item(StripItem::cover(("strip", id), cover).shelf(ReadingStatus::Read).position("#1"))
//!     .item(StripItem::missing(("strip", id2), cover2).position("#2").next(true))
//!     .item(StripItem::unreleased(("strip", id3), "Untitled Dune #3").opens(page).position("#3"))
//!     .toggle(MORE_SLOTS, show_more_button)
//! ```

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Rems, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::ReadingStatus;

use super::Meter;
use crate::bidi::WrappingText;
use crate::catalog::navigation::{self, Page};
use crate::chip::status_icon;
use crate::cover::{Cover, CoverSize, cover_label};
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::format::status_label;

/// How visible a book that is out but not in the library is: its cover,
/// faded, so it reads as a place on the shelf rather than a book on it.
pub const MISSING_OPACITY: f32 = 0.4;

/// Between a book's ringed cover, line, position and title (4).
const GAP: Rems = rems(0.25);
/// The cover's size in a strip.
const COVER: CoverSize = CoverSize::Md;
/// A title is at most two lines.
const TITLE_LINES: usize = 2;
/// "Next" sits over the ring, a caption line and a [`GAP`] above it.
const NEXT_LABEL_TOP: Rems = rems(-1.25);
/// The ring's corners: the cover's, plus the inset (6).
const RING_RADIUS: Rems = rems(0.375);
/// The room "Show the other N books" takes at the end of its line, in books.
pub const MORE_SLOTS: usize = 3;
/// The room "Show fewer" takes at the end of the last line, in books.
pub const FEWER_SLOTS: usize = 2;

/// An empty place for a book. See the module doc.
#[derive(IntoElement)]
pub struct CoverSlot {
    size: CoverSize,
    icon: Option<Icon>,
    click: Option<SlotClick>,
}

/// A slot's click: its element id, the book's title (the accessible name)
/// and the handler.
type SlotClick = (
    ElementId,
    SharedString,
    Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
);

impl CoverSlot {
    pub fn new(size: CoverSize) -> Self {
        Self {
            size,
            icon: None,
            click: None,
        }
    }

    /// A click on the slot runs `handler`. `title` is the book's, which
    /// names the slot for a screen reader; `id` must be unique among the
    /// slot's siblings.
    pub fn on_click(
        mut self,
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some((id.into(), title.into(), Rc::new(handler)));
        self
    }

    /// A click on the slot opens `page`, usually the book's, as a cover's
    /// does ([`Cover::opens`]).
    pub fn opens(
        self,
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        page: Page,
    ) -> Self {
        self.on_click(id, title, move |_, _, cx| {
            navigation::open(page.clone(), cx);
        })
    }

    /// A 12 px icon in the middle, in `fg-muted`.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

impl RenderOnce for CoverSlot {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let slot = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(self.size.width())
            .h(self.size.height())
            .rounded(self.size.radius())
            .border(size::HAIRLINE)
            .border_dashed()
            .border_color(colors.border_strong)
            .text_color(colors.fg_muted)
            .children(self.icon.map(gpui_kit::Styled::size_3));
        // Like a cover: a link for a screen reader's pointer, but no tab
        // stop and the arrow cursor (`cover.rs`).
        match self.click {
            Some((id, title, handler)) => slot
                .id(id)
                .role(Role::Link)
                .aria_label(cover_label(&title))
                .on_click(move |event, window, cx| handler(event, window, cx))
                .into_any_element(),
            None => slot.into_any_element(),
        }
    }
}

/// What stands for a book in the strip.
enum Body {
    Cover(Cover),
    Missing(Cover),
    Unreleased,
}

/// What a book's place in the strip says, for [`strip_label`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StripState {
    /// On a shelf.
    Shelf(ReadingStatus),
    /// Out, but not in the library.
    Missing,
    /// Not out yet.
    Unreleased,
}

/// A strip book's accessible name: its position, its title and what its
/// place shows (the shelf, the meter's percentage, a slot), then whether
/// it is the series' next book.
pub fn strip_label(
    position: &str,
    title: &str,
    state: StripState,
    progress: Option<f32>,
    next: bool,
) -> String {
    let mut parts = vec![match state {
        StripState::Shelf(status) => status_label(status).to_owned(),
        StripState::Missing => "not in your library".to_owned(),
        StripState::Unreleased => "not out yet".to_owned(),
    }];
    if let Some(percent) = progress {
        parts.push(format!("{:.0}%", super::meter::fraction(percent) * 100.));
    }
    if next {
        parts.push("next".to_owned());
    }
    let place = if position.is_empty() {
        String::new()
    } else {
        format!("#{position}, ")
    };
    format!("{place}{title}: {}", parts.join(", "))
}

/// One book of a [`SeriesStrip`].
pub struct StripItem {
    id: ElementId,
    title: SharedString,
    body: Body,
    opens: Option<Page>,
    shelf: Option<ReadingStatus>,
    position: SharedString,
    progress: Option<f32>,
    next: bool,
}

impl StripItem {
    fn new(id: impl Into<ElementId>, title: SharedString, body: Body) -> Self {
        Self {
            id: id.into(),
            title,
            body,
            opens: None,
            shelf: None,
            position: SharedString::default(),
            progress: None,
            next: false,
        }
    }

    /// A book in the library: its md cover. Give its shelf with
    /// [`StripItem::shelf`], and its click with [`Cover::opens`].
    pub fn cover(id: impl Into<ElementId>, cover: Cover) -> Self {
        let title = cover.title().clone();
        Self::new(id, title, Body::Cover(cover))
    }

    /// A book that is out but not in the library: its md cover, faded to
    /// [`MISSING_OPACITY`]. Give it its click with [`Cover::opens`].
    pub fn missing(id: impl Into<ElementId>, cover: Cover) -> Self {
        let title = cover.title().clone();
        Self::new(id, title, Body::Missing(cover))
    }

    /// A book not out yet: a slot with the clock.
    pub fn unreleased(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self::new(id, title.into(), Body::Unreleased)
    }

    /// A click on a book not out yet opens `page`, usually the book's
    /// ([`CoverSlot::opens`]). A cover gets its own with [`Cover::opens`].
    pub fn opens(mut self, page: Page) -> Self {
        self.opens = Some(page);
        self
    }

    /// The book's shelf: its icon before the position.
    pub fn shelf(mut self, status: ReadingStatus) -> Self {
        self.shelf = Some(status);
        self
    }

    /// "#3", "#3.5".
    pub fn position(mut self, position: impl Into<SharedString>) -> Self {
        self.position = position.into();
        self
    }

    /// The book being read: a 2 px meter under the cover.
    pub fn progress(mut self, percent: Option<f32>) -> Self {
        self.progress = percent;
        self
    }

    /// The series' next book: ringed, "Next" over it, its position in ink
    /// at weight 600.
    pub fn next(mut self, next: bool) -> Self {
        self.next = next;
        self
    }

    fn state(&self) -> StripState {
        match (&self.body, self.shelf) {
            (Body::Missing(_), _) => StripState::Missing,
            (Body::Unreleased, _) => StripState::Unreleased,
            (Body::Cover(_), Some(status)) => StripState::Shelf(status),
            // A cover with no shelf given: in the library, as a cover means.
            (Body::Cover(_), None) => StripState::Shelf(ReadingStatus::WantToRead),
        }
    }

    fn render(self, window: &Window, cx: &App) -> AnyElement {
        let colors = colors(cx);
        let rem = window.rem_size();
        let width = SeriesStrip::item_width(rem);
        let label = strip_label(
            self.position.trim_start_matches('#'),
            &self.title,
            self.state(),
            self.progress,
            self.next,
        );
        let missing = matches!(self.body, Body::Missing(_));
        let body = match self.body {
            Body::Cover(cover) => cover.into_any_element(),
            Body::Missing(cover) => div()
                .flex_none()
                .opacity(MISSING_OPACITY)
                .child(cover)
                .into_any_element(),
            Body::Unreleased => {
                let slot = CoverSlot::new(COVER).icon(crate::icons::clock());
                match self.opens {
                    Some(page) => slot.opens(
                        SharedString::from(format!("{}-slot", self.id)),
                        self.title.clone(),
                        page,
                    ),
                    None => slot,
                }
                .into_any_element()
            }
        };
        // Every cover keeps the ring's room, so the books line up.
        let ringed = div()
            .relative()
            .flex_none()
            .p(size::STRIP_RING_INSET)
            .border(size::STRIP_RING)
            .rounded(RING_RADIUS)
            .border_color(if self.next {
                colors.accent
            } else {
                gpui_kit::transparent_black()
            })
            .child(body)
            .when(self.next, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(NEXT_LABEL_TOP)
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .caption(cx)
                        .text_color(colors.accent)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Next"),
                )
            });
        let line = match self.progress {
            Some(percent) => Meter::new(self.id.clone(), percent, "Reading progress")
                .thin()
                .w(COVER.width())
                .into_any_element(),
            None => div().flex_none().h(size::METER_THIN).into_any_element(),
        };
        let title = self.title;
        let tooltip = title.clone();
        div()
            .id(self.id)
            .role(Role::ListItem)
            .aria_label(label)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap(GAP)
            .w(width)
            .child(ringed)
            .child(line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .caption(cx)
                    .tabular()
                    .whitespace_nowrap()
                    .when(self.next, |this| {
                        this.text_color(colors.fg).font_weight(FontWeight::SEMIBOLD)
                    })
                    .children(
                        self.shelf
                            .map(|status| Icon::new(status_icon(status)).size_3().flex_none()),
                    )
                    .child(self.position),
            )
            .child(
                div()
                    .flex()
                    .justify_center()
                    .w(width)
                    .h(TypeRole::Caption.line_height() * TITLE_LINES as f32)
                    .overflow_hidden()
                    .caption(cx)
                    .text_center()
                    .when(!missing, |this| this.text_color(colors.fg))
                    .child(
                        WrappingText::new(title)
                            .width(width)
                            .line_clamp(TITLE_LINES)
                            .centered(),
                    ),
            )
            .into_any_element()
    }
}

/// What ends a strip that can't show every book on one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// Shut: "Show the other N books".
    More(usize),
    /// Opened: "Show fewer".
    Fewer,
}

impl Toggle {
    /// The books' room the toggle takes at the end of its line.
    pub fn slots(self) -> usize {
        match self {
            Self::More(_) => MORE_SLOTS,
            Self::Fewer => FEWER_SLOTS,
        }
    }
}

/// Which books a strip shows, what ends it, and on how many lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StripLayout {
    pub range: Range<usize>,
    pub toggle: Option<Toggle>,
    pub lines: usize,
}

/// The layout of a strip of `len` books with room for `capacity` on a line
/// ([`SeriesStrip::capacity`]). A strip that fits shows every book on one
/// line. One that doesn't is shut to one line: as many books as fit beside
/// "Show the other N books", from the one before the `next` book (so the
/// line shows what came last and what comes next), the next book always
/// among them; opened, every book, then "Show fewer".
pub fn strip_layout(len: usize, next: Option<usize>, capacity: usize, open: bool) -> StripLayout {
    let capacity = capacity.max(1);
    if len <= capacity {
        return StripLayout {
            range: 0..len,
            toggle: None,
            lines: 1,
        };
    }
    if open {
        return StripLayout {
            range: 0..len,
            toggle: Some(Toggle::Fewer),
            lines: lines_for(len, FEWER_SLOTS.min(capacity), capacity),
        };
    }
    let toggle = MORE_SLOTS.min(capacity);
    let shown = capacity.saturating_sub(toggle).max(1);
    let next = next.unwrap_or(0).min(len - 1);
    let start = if shown >= 2 {
        next.saturating_sub(1)
    } else {
        next
    }
    .min(len - shown);
    StripLayout {
        range: start..start + shown,
        toggle: Some(Toggle::More(len - shown)),
        lines: lines_for(shown, toggle, capacity),
    }
}

/// How many lines `items` books and a toggle `toggle` books wide take, with
/// room for `capacity` books on a line. The toggle goes on the last line when
/// it has room, else on a line of its own.
fn lines_for(items: usize, toggle: usize, capacity: usize) -> usize {
    let full = items / capacity;
    let left = items % capacity;
    let lines = full + usize::from(left > 0);
    let toggle_fits = left > 0 && left + toggle <= capacity;
    (lines + usize::from(toggle > 0 && !toggle_fits)).max(1)
}

/// A series' books on one line or, opened, several. See the module doc.
#[derive(IntoElement, Default)]
pub struct SeriesStrip {
    items: Vec<StripItem>,
    toggle: Option<(usize, AnyElement)>,
}

impl SeriesStrip {
    pub fn new() -> Self {
        Self::default()
    }

    // One at a time is how the gallery builds its strips; screens pass
    // their items at once.
    #[cfg_attr(not(feature = "demo"), allow(dead_code))]
    pub fn item(mut self, item: StripItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = StripItem>) -> Self {
        self.items.extend(items);
        self
    }

    /// What ends the strip (the "Show the other N books" button), in a
    /// place `slots` books wide at the end of the last line.
    pub fn toggle(mut self, slots: usize, toggle: impl IntoElement) -> Self {
        self.toggle = Some((slots, toggle.into_any_element()));
        self
    }

    /// How wide a book is: the md cover inside the ring's inset and ring.
    pub fn item_width(rem: Pixels) -> Pixels {
        COVER.width().to_pixels(rem) + Self::ring_room(rem) * 2.
    }

    /// The inset and the ring, on one side of a cover.
    fn ring_room(rem: Pixels) -> Pixels {
        size::STRIP_RING_INSET.to_pixels(rem) + size::STRIP_RING
    }

    /// How tall one book is: the ringed cover, the line, the position and
    /// two lines of title, [`GAP`] apart.
    pub fn item_height(rem: Pixels) -> Pixels {
        COVER.height().to_pixels(rem)
            + Self::ring_room(rem) * 2.
            + (GAP * 3.
                + size::METER_THIN
                + TypeRole::Caption.line_height() * (1 + TITLE_LINES) as f32)
                .to_pixels(rem)
    }

    /// How tall a strip of `lines` lines is. The tracker's virtual list sizes
    /// its rows from it.
    pub fn height(lines: usize, rem: Pixels) -> Pixels {
        let lines = lines.max(1);
        Self::item_height(rem) * lines as f32
            + size::STRIP_LINE_GAP.to_pixels(rem) * (lines - 1) as f32
    }

    /// How many books fit on a line `width` wide, counting the gaps between
    /// them.
    pub fn capacity(width: Pixels, rem: Pixels) -> usize {
        let item = Self::item_width(rem);
        let gap = size::STRIP_GAP.to_pixels(rem);
        let fits = ((width + gap) / (item + gap)).floor();
        if fits.is_finite() && fits > 0. {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "fits is a finite positive floor; the saturating cast bounds the slot count"
            )]
            let slots = fits as usize;
            slots
        } else {
            1
        }
    }

    /// The room of `slots` books, the gaps between them included, so a toggle
    /// wraps exactly as that many books would.
    fn slots_width(slots: usize, rem: Pixels) -> Pixels {
        let slots = slots.max(1);
        Self::item_width(rem) * slots as f32 + size::STRIP_GAP.to_pixels(rem) * (slots - 1) as f32
    }
}

impl RenderOnce for SeriesStrip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rem = window.rem_size();
        let cover_height = COVER.height().to_pixels(rem) + Self::ring_room(rem) * 2.;
        let window: &Window = window;
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_x(size::STRIP_GAP)
            .gap_y(size::STRIP_LINE_GAP)
            .children(self.items.into_iter().map(|item| item.render(window, cx)))
            // A book's whole height, so a line holding only the toggle is as
            // tall as [`Self::height`] counts it; the button sits level with
            // the covers.
            .children(self.toggle.map(|(slots, toggle)| {
                div()
                    .flex_none()
                    .w(Self::slots_width(slots, rem))
                    .h(Self::item_height(rem))
                    .child(div().flex().items_center().h(cover_height).child(toggle))
            }))
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, point, px,
    };

    use super::*;
    use crate::ui::test_support::{counter, host, redraw};

    #[test]
    fn a_strip_that_fits_shows_every_book_on_one_line() {
        let layout = strip_layout(6, Some(2), 6, false);
        assert_eq!(
            layout,
            StripLayout {
                range: 0..6,
                toggle: None,
                lines: 1
            }
        );
        // Opening changes nothing when everything fits.
        assert_eq!(strip_layout(6, Some(2), 6, true), layout);
    }

    #[test]
    fn a_shut_strip_starts_a_book_before_the_next_one() {
        // Room for 12: 9 books and the toggle's 3.
        assert_eq!(
            strip_layout(27, Some(0), 12, false),
            StripLayout {
                range: 0..9,
                toggle: Some(Toggle::More(18)),
                lines: 1
            }
        );
        assert_eq!(
            strip_layout(27, Some(19), 12, false),
            StripLayout {
                range: 18..27,
                toggle: Some(Toggle::More(18)),
                lines: 1
            }
        );
        assert_eq!(strip_layout(27, Some(10), 12, false).range, 9..18);
        // Near the end, the line is filled from the end.
        assert_eq!(strip_layout(27, Some(26), 12, false).range, 18..27);
        // A finished series (no next book) starts at the first.
        assert_eq!(strip_layout(27, None, 12, false).range, 0..9);
    }

    #[test]
    fn every_shut_strip_shows_the_next_book() {
        for len in 1..30 {
            for capacity in 1..14 {
                for next in 0..len {
                    let layout = strip_layout(len, Some(next), capacity, false);
                    assert!(layout.range.contains(&next), "{len} {capacity} {next}");
                    assert!(!layout.range.is_empty());
                    let hidden = len - layout.range.len();
                    match layout.toggle {
                        Some(Toggle::More(n)) => assert_eq!(n, hidden),
                        None => assert_eq!(hidden, 0),
                        Some(Toggle::Fewer) => panic!("a shut strip"),
                    }
                }
            }
        }
    }

    #[test]
    fn an_opened_strip_wraps_every_book_and_ends_in_show_fewer() {
        // 27 books, 12 a line: 12, 12, then 3 and "Show fewer" (2).
        assert_eq!(
            strip_layout(27, Some(0), 12, true),
            StripLayout {
                range: 0..27,
                toggle: Some(Toggle::Fewer),
                lines: 3
            }
        );
        // 35: 12, 12, 11, and "Show fewer" on a line of its own.
        assert_eq!(strip_layout(35, Some(0), 12, true).lines, 4);
        // 36 fills three lines; the toggle takes a fourth.
        assert_eq!(strip_layout(36, Some(0), 12, true).lines, 4);
    }

    #[test]
    fn a_narrow_strip_puts_its_toggle_on_a_line_of_its_own() {
        // Room for 3, the toggle's whole width: one book, then the toggle.
        assert_eq!(
            strip_layout(10, Some(4), 3, false),
            StripLayout {
                range: 4..5,
                toggle: Some(Toggle::More(9)),
                lines: 2
            }
        );
        // Room for 2: the toggle takes the line it can.
        assert_eq!(strip_layout(10, Some(4), 2, false).lines, 2);
    }

    #[test]
    fn a_strip_line_holds_as_many_books_as_fit() {
        use gpui_kit::px;
        let rem = px(16.);
        // 72 a book, 8 between: n books need 80n − 8.
        assert_eq!(SeriesStrip::item_width(rem), px(72.));
        assert_eq!(SeriesStrip::capacity(px(12. * 80. - 8.), rem), 12);
        assert_eq!(SeriesStrip::capacity(px(12. * 80. - 9.), rem), 11);
        assert_eq!(SeriesStrip::capacity(px(10.), rem), 1);
        // 104 for the ringed cover, 2 for the line, 16 for the position, 32
        // for the title, and 3 gaps of 4.
        assert_eq!(SeriesStrip::item_height(rem), px(166.));
        assert_eq!(SeriesStrip::height(2, rem), px(166. * 2. + 16.));
    }

    #[gpui_kit::test]
    fn a_slot_with_a_click_opens_its_book(cx: &mut TestAppContext) {
        let (count, handler) = counter();
        let cx = host(cx, move |_, _| {
            CoverSlot::new(CoverSize::Sm)
                .on_click("slot", "Dune Messiah", handler.clone())
                .into_any_element()
        });
        // The slot sits at the window's origin; its middle.
        let position = point(px(20.), px(30.));
        let modifiers = Modifiers::default();
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 1,
        });
        redraw(cx);
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn a_strip_book_says_what_its_place_shows() {
        assert_eq!(
            strip_label(
                "1",
                "Dune",
                StripState::Shelf(ReadingStatus::Read),
                None,
                false
            ),
            "#1, Dune: Read"
        );
        assert_eq!(
            strip_label(
                "2",
                "Dune Messiah",
                StripState::Shelf(ReadingStatus::CurrentlyReading),
                Some(40.),
                true
            ),
            "#2, Dune Messiah: Currently reading, 40%, next"
        );
        assert_eq!(
            strip_label("3", "Children of Dune", StripState::Missing, None, true),
            "#3, Children of Dune: not in your library, next"
        );
        assert_eq!(
            strip_label(
                "6.5",
                "Untitled Dune #7",
                StripState::Unreleased,
                None,
                false
            ),
            "#6.5, Untitled Dune #7: not out yet"
        );
    }
}
