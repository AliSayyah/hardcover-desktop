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
//! | [`Columns`], [`folds`] | A page's main column and its rail, the rail folding under it when the region is narrow |
//! | [`CoverShelf`] | A few books in a row, each a cover over a two-line title: "Newest on Want to read", Similar books |
//! | [`Block`], [`blocks`] | A pane's blocks ("About this book", "Details"): a heading, an optional small action, 24 apart |
//! | [`PageHeader`] | A destination's title, with its count or a muted line |
//! | [`Greeting`], [`greeting`], [`date_line`] | Home's header: the date over "Good morning" |
//! | [`SectionHeader`], [`see_all`] | A section's title on Home or a page, with "See all 5" |
//! | [`Sequence`], [`RowSpacing`] | Read-only rows between hairlines: Continue reading, Coming soon, reviews |
//! | [`ListRow`] | The list row treatment on a collection's `ListItem`: padding, fills, focus edge |
//! | [`RowFocus`] | A collection delegate's copy of its list's focus and selection, for `ListRow`'s focus edge |
//! | [`Lane`] | A fixed, right-aligned lane for trailing figures or actions, kept when empty |
//! | [`PropertyList`] | Key and value rows: Edition, Owned, Privacy, Added |
//! | [`Meter`] | Progress: 6 px beside words, 2 px in a row, with an optional pace tick |
//! | [`EmptyState`], [`empty_state`] | A region with nothing to show: icon, title, one sentence, a button |
//! | [`Skeleton`], [`SkeletonText`], [`SkeletonRow`] | Content still loading: pulsing shapes, lines, a row |
//! | [`TextButton`] | In-app navigation inside content: a book title, an author, a series |
//! | [`ExternalLink`] | Something that opens the browser: "Open on Hardcover" (accent, underlined, the external-link icon) |
//! | [`BylineLink`] | A person's name that opens their Hardcover profile in the browser |
//! | [`NavRow`] | A sidebar destination: icon, label, count or shortcut, selected state |
//! | [`Stars`] | A rating shown, not set: Hardcover's average, another reader's rating (muted, never gold) |
//! | [`Avatar`] | A person: their picture, else initials on their colour or `accent-soft` |
//! | [`focus`] | The focus ring and edge, for a component that tracks its own focus |
//! | [`kit`] | Fixes for the kit's own components: raised fields and dialogs, the popover surface |
//!
//! Each component's module doc says when to use it, with an example.

// The foundation for the screens still migrating to the system: some parts
// have no user yet, and the screen waves will use them.
#![allow(dead_code)]

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
mod meter;
mod nav_row;
mod object_bar;
mod row_focus;
mod rows;
mod skeleton;
mod stars;
mod surfaces;
#[cfg(test)]
mod test_support;
mod text_button;

// Some components have no screen yet; the screen waves will use them.
#[allow(unused_imports)]
pub use self::{
    avatar::{Avatar, AvatarSize, initials},
    byline_link::BylineLink,
    columns::{Columns, MAIN_MIN, folds},
    cover_shelf::CoverShelf,
    empty_state::{EmptyState, empty_state, empty_state_with_action},
    external_link::ExternalLink,
    headers::{Greeting, PageHeader, SectionHeader, date_line, greeting, greeting_at, see_all},
    meter::Meter,
    nav_row::NavRow,
    object_bar::{BackTo, ObjectBar, ObjectTools, back_label, tools_of},
    row_focus::RowFocus,
    rows::{Lane, ListRow, PropertyList, RowSpacing, Sequence},
    skeleton::{Skeleton, SkeletonRow, SkeletonText},
    stars::Stars,
    surfaces::{Block, Rail, RailSection, blocks, panel},
    text_button::TextButton,
};
