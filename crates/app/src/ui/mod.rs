//! The app's shared components: the design system's parts
//! (`hardcover-design`, Components and States), built once. **Screens
//! compose these; they never draw their own version of one.** If a screen
//! needs a part that isn't here, add it here, with its doc, rather than
//! beside the screen.
//!
//! Every component reads tokens only (`design::colors`, `design::size`,
//! `design::TypeRoles`) and has the system's one treatment for each state.
//! The other shared parts live beside their subject: [`crate::cover::Cover`],
//! the chips (`crate::chip`), the rating star (`crate::star`) and
//! `crate::bidi::WrappingText`.
//!
//! | Component | Use it for |
//! | --- | --- |
//! | [`ObjectBar`], [`ObjectTools`], [`tools_of`] | The 56 bar above a book pane or a page: Back on a page, the object's tools ("Lists", "more") |
//! | [`panel`] | The one inset panel of a pane: progress, Not started |
//! | [`Rail`], [`RailSection`] | The 272 column beside Home's or the book page's main column, sections between hairlines |
//! | [`Columns`], [`folds`], [`tier`], [`Tier`] | A page's main column and its rail, the rail folding under it when the region is narrow; a second content column when it is wide (Home) |
//! | [`CoverShelf`] | A few books in a row, each a cover over a two-line title: "Newest on Want to read", Similar books |
//! | [`Block`], [`blocks`] | A pane's blocks ("About this book", "Details"): a heading, an optional small action, 24 apart |
//! | [`PageHeader`] | A destination's title, with its count or a muted line |
//! | [`Greeting`], [`greeting`], [`date_line`] | Home's header: the date over "Good morning" |
//! | [`SectionHeader`], [`see_all`] | A section's title on Home or a page, with "See all 5" |
//! | [`Sequence`], [`RowSpacing`] | Read-only rows between hairlines: Continue reading, Coming soon, reviews |
//! | [`ListRow`] | The list row treatment on a collection's `ListItem`: padding, fills, focus ring and edge |
//! | [`RowFocus`], [`on_keyboard_entry`], [`select_first_row`], [`enter_list`] | A collection delegate's copy of its list's focus and selection, for `ListRow`'s focus treatment; the list as a tab stop that lands on a row |
//! | [`Lane`] | A fixed, right-aligned lane for trailing figures or actions, kept when empty |
//! | [`SeriesStrip`], [`StripItem`], [`strip_layout`] | A series as a shelf of md covers, each with its shelf's icon, position and title under it; the next one ringed; one line with "Show the other N books" until opened |
//! | [`CoverSlot`] | A dashed place the size of a cover, empty or with an icon (the series strip's book not out yet: a clock) |
//! | [`PropertyList`] | Key and value rows: Edition, Owned, Privacy, Added |
//! | [`Meter`] | Progress: 6 px beside words, 2 px in a row, with an optional pace tick |
//! | [`MonthBars`] | Books finished in each month of a year, the counts over the bars: Home's reading goal |
//! | [`EmptyState`], [`empty_state`] | A region with nothing to show: icon, title, one sentence, a button |
//! | [`LoadFailed`] | A load that failed: one muted sentence and "Try again", or the whole region's empty state when nothing else can show |
//! | [`leaving`] | A row leaving a collection while it is shown: it fades out in its place, then the gap closes |
//! | [`Skeleton`], [`SkeletonText`], [`SkeletonRow`] | Content still loading: pulsing shapes, lines, a row |
//! | [`TextButton`] | In-app navigation inside content: a book title, an author, a series |
//! | [`ExternalLink`] | Something that opens the browser: "Open on Hardcover" (accent, underlined, the external-link icon) |
//! | [`BylineLink`] | A person's name that opens their Hardcover profile in the browser |
//! | [`NavRow`] | A sidebar destination: icon, label, count or shortcut, selected state |
//! | [`Stars`] | A rating shown, not set: Hardcover's average, another reader's rating (muted, never gold) |
//! | [`brand_row`], [`sidebar_label`], [`GoalRow`], [`AccountRow`] | The sidebar's other parts: the brand, a section label, the reading goal, the account menu's trigger |
//! | [`Avatar`] | A person: their picture, else initials on their colour or `accent-soft` |
//! | [`focus`] | The focus ring, edge and row treatment, for a component that tracks its own focus; giving focus back after a page |
//! | [`reveal`] | Keeping the focused element in view: the opt-in on a scrolling region, and [`reveal::probe`] |
//! | [`kit`] | Fixes for the kit's own components: raised fields, small controls, dialogs and their keys, quiet buttons on the text spine |
//!
//! Each component's module doc says when to use it, with an example.

mod avatar;
mod byline_link;
mod columns;
mod cover_shelf;
mod empty_state;
mod external_link;
pub mod focus;
#[cfg(feature = "demo")]
pub mod gallery;
mod headers;
pub mod kit;
pub mod leaving;
mod load_failed;
mod meter;
mod month_bars;
mod nav_row;
mod object_bar;
pub mod reveal;
mod row_focus;
mod rows;
mod series_strip;
mod sidebar;
mod skeleton;
mod stars;
mod surfaces;
#[cfg(test)]
pub(crate) mod test_support;
mod text_button;

pub(crate) use self::stars::{StarInk, StarSize, render_star};
pub use self::{
    avatar::{Avatar, AvatarSize},
    byline_link::BylineLink,
    columns::{Columns, Tier, content_column_width, folds, main_column_width, region_width, tier},
    cover_shelf::{CoverShelf, books_that_fit},
    empty_state::{EmptyState, empty_state, empty_state_with_action},
    external_link::ExternalLink,
    headers::{Greeting, PageHeader, SectionHeader, date_line, greeting_at, see_all},
    load_failed::LoadFailed,
    meter::Meter,
    month_bars::MonthBars,
    nav_row::NavRow,
    object_bar::{BackTo, ObjectBar, ObjectTools, back_label, tools_of},
    row_focus::{RowFocus, enter_list, on_keyboard_entry, select_first_row},
    rows::{Lane, ListRow, PropertyList, RowSpacing, Sequence},
    series_strip::{MISSING_OPACITY, SeriesStrip, StripItem, Toggle, strip_layout},
    sidebar::{AccountRow, GoalRow, brand_row, sidebar_label},
    skeleton::{Skeleton, SkeletonRow, SkeletonText},
    stars::Stars,
    surfaces::{Block, Rail, RailSection, blocks, panel},
    text_button::TextButton,
};
/// Draws every view again on the next frame, cached ones too: what an
/// element learns as it is drawn (its width, whether its text was cut, a
/// scroll that reveals focus) shows only on a frame drawn after it.
/// `window.refresh()` does nothing while a frame is drawn, and a cached view
/// isn't drawn again unless something asks.
pub fn refresh_after_draw(window: &gpui_kit::Window) {
    window.on_next_frame(|window, _| window.refresh());
}

// Parts only the gallery names: `greeting` is Home's greeting for the current
// time (Home passes its own clock to `greeting_at`), and a lone `CoverSlot`
// is drawn by `SeriesStrip` on screens.
#[cfg(feature = "demo")]
pub use self::{headers::greeting, series_strip::CoverSlot};
