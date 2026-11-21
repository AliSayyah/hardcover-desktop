//! The content of one row in a collection (`hardcover-design`, Rows → List
//! row and Catalogue row), inside the `ListItem` that `ui::ListRow` styles.
//! Shared by the shelves, search, Discover, list pages and the series and
//! author pages.
//!
//! - an sm cover (an author's avatar, a series' icon, in the same 32 wide
//!   slot, so the text keeps its spine);
//! - the title in `body-strong` and the author or facts in `body` muted,
//!   both one line and truncated;
//! - the trailing lane: on a shelf, 48 wide and kept when empty, with
//!   tabular caption figures (progress over a 2 px meter, the reader's own
//!   rating with its gold star, a release year); in a catalogue, the
//!   `StatusChip` (its icon alone in a narrow list, [`BookRow::compact`]).
//!   The figures turn from `fg-muted` to `fg` on the selected row.

use gpui_kit::component::{Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    Window, div,
};
use hardcover_core::{BookSummary, LibraryEntry, ReadingStatus};
use std::fmt::Write as _;

use crate::chip::{StatusChip, StatusIconChip};
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors};
use crate::format::{authors_text, progress_summary, rating_text};
use crate::ui::{Avatar, Lane, Meter, SkeletonRow};

/// What the trailing lane shows.
#[derive(Clone)]
pub enum Trailing {
    /// Nothing in a shelf row's lane, which it keeps.
    Empty,
    /// Reading progress: "66%" over a 2 px meter, or "p. 12" alone when the
    /// book's length is unknown. `id` keys the meter.
    Progress {
        id: u64,
        text: SharedString,
        percent: Option<f32>,
    },
    /// The reader's own rating, with the gold star.
    Rating(SharedString),
    /// A year: when a Want to read book came out.
    Year(i32),
    /// A catalogue book's library status.
    Status(ReadingStatus),
    /// No lane: a catalogue book not in the library, an author, a series.
    None,
}

/// What leads the row.
#[derive(Clone, Copy)]
enum Lead {
    Book,
    /// A person: their photo or initials, in a circle.
    Person,
    /// A series: its icon in the cover's slot.
    Series,
}

#[derive(IntoElement)]
pub struct BookRow {
    title: SharedString,
    secondary: SharedString,
    image_url: Option<String>,
    lead: Lead,
    trailing: Trailing,
    selected: bool,
    compact: bool,
}

impl BookRow {
    /// A row on a library shelf.
    pub fn for_entry(entry: &LibraryEntry) -> Self {
        let trailing = match entry.status {
            ReadingStatus::CurrentlyReading | ReadingStatus::Paused => {
                let progress = progress_summary(entry);
                progress
                    .short_text()
                    .map_or(Trailing::Empty, |text| Trailing::Progress {
                        id: entry.user_book_id.get().cast_unsigned(),
                        text: text.into(),
                        percent: progress.percent,
                    })
            }
            ReadingStatus::Read => entry
                .rating
                .map_or(Trailing::Empty, |r| Trailing::Rating(rating_text(r).into())),
            ReadingStatus::WantToRead => entry
                .book
                .release_year
                .map_or(Trailing::Empty, Trailing::Year),
            ReadingStatus::DidNotFinish | ReadingStatus::Ignored => Trailing::Empty,
        };
        Self {
            title: entry.book.title.clone().into(),
            secondary: authors_text(&entry.book.authors).into(),
            image_url: entry.cover().map(|c| c.url.clone()),
            lead: Lead::Book,
            trailing,
            selected: false,
            compact: false,
        }
    }

    /// A search result, with its library status when the book is shelved.
    pub fn for_search_result(book: &BookSummary, status: Option<ReadingStatus>) -> Self {
        let mut secondary = authors_text(&book.authors);
        if let Some(year) = book.release_year {
            write!(secondary, " · {year}").expect("writing to a String");
        }
        Self::for_catalog(book, secondary, status)
    }

    /// A book on a catalogue page (Discover, a series, an author, a list):
    /// `details` is its secondary line, with its library status when shelved.
    pub fn for_catalog(
        book: &BookSummary,
        details: impl Into<SharedString>,
        status: Option<ReadingStatus>,
    ) -> Self {
        Self {
            title: book.title.clone().into(),
            secondary: details.into(),
            image_url: book.cover.as_ref().map(|c| c.url.clone()),
            lead: Lead::Book,
            trailing: status.map_or(Trailing::None, Trailing::Status),
            selected: false,
            compact: false,
        }
    }

    /// An author in search results: their photo, else their initials.
    pub fn for_author(
        name: impl Into<SharedString>,
        details: impl Into<SharedString>,
        photo_url: Option<String>,
    ) -> Self {
        Self {
            title: name.into(),
            secondary: details.into(),
            image_url: photo_url,
            lead: Lead::Person,
            trailing: Trailing::None,
            selected: false,
            compact: false,
        }
    }

    /// A series in search results.
    pub fn for_series(name: impl Into<SharedString>, details: impl Into<SharedString>) -> Self {
        Self {
            title: name.into(),
            secondary: details.into(),
            image_url: None,
            lead: Lead::Series,
            trailing: Trailing::None,
            selected: false,
            compact: false,
        }
    }

    /// Whether the row is the selected one: its figures turn to ink.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// In a narrow list the `StatusChip` shrinks to its icon, so the title
    /// keeps its room ("As the window narrows": collapse secondary labels).
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
}

impl RenderOnce for BookRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = colors(cx);
        let figures = if self.selected {
            colors.fg
        } else {
            colors.fg_muted
        };
        let lead: AnyElement = match self.lead {
            Lead::Book => Cover::new(CoverSize::Sm, self.title.clone())
                .thumbnail(self.image_url)
                .into_any_element(),
            Lead::Person => Avatar::new(self.title.clone())
                .image(self.image_url)
                .into_any_element(),
            Lead::Series => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(CoverSize::Sm.width())
                .h(CoverSize::Sm.height())
                .child(Icon::new(IconName::GalleryVerticalEnd).text_color(colors.fg_muted))
                .into_any_element(),
        };
        let figure = || div().caption(cx).tabular().text_color(figures);
        let lane: Option<AnyElement> = match self.trailing {
            Trailing::None => None,
            Trailing::Empty => Some(Lane::trailing().into_any_element()),
            Trailing::Progress { id, text, percent } => Some(
                Lane::trailing()
                    .stacked()
                    .child(figure().child(text))
                    .children(percent.map(|percent| {
                        Meter::new(("row-progress", id), percent, "Reading progress")
                            .thin()
                            .w_full()
                    }))
                    .into_any_element(),
            ),
            Trailing::Rating(text) => Some(
                Lane::trailing()
                    .child(
                        h_flex()
                            .gap_1()
                            // Gold: the reader's own rating, as on the book page.
                            .child(crate::star::icon().xsmall().text_color(colors.gold))
                            .child(figure().child(text)),
                    )
                    .into_any_element(),
            ),
            Trailing::Year(year) => Some(
                Lane::trailing()
                    .child(figure().child(year.to_string()))
                    .into_any_element(),
            ),
            Trailing::Status(status) if self.compact => {
                Some(Lane::fit().child(StatusIconChip(status)).into_any_element())
            }
            Trailing::Status(status) => {
                Some(Lane::fit().child(StatusChip(status)).into_any_element())
            }
        };
        h_flex()
            .w_full()
            .gap_3()
            .child(lead)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().body_strong().truncate().child(self.title))
                    .child(
                        div()
                            .body()
                            .text_color(colors.fg_muted)
                            .truncate()
                            .child(self.secondary),
                    ),
            )
            .children(lane)
    }
}

/// A collection's rows while it loads: skeleton rows at the list's 8 inset,
/// 2 apart, in place of the kit's spinner.
pub fn loading_rows() -> impl IntoElement {
    v_flex()
        .w_full()
        .p_2()
        .gap_0p5()
        .children((0..6).map(|_| SkeletonRow::new()))
}
