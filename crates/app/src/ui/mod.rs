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
//! | [`Block`], [`blocks`] | A pane's blocks ("About this book", "Details"): a heading, an optional small action, 24 apart |
//! | [`PageHeader`] | A destination's title, with its count or a muted line |
//! | [`Greeting`], [`greeting`], [`date_line`] | Home's header: the date over "Good morning" |
//! | [`SectionHeader`], [`see_all`] | A section's title on Home or a page, with "See all 5" |
//! | [`Sequence`], [`RowSpacing`] | Read-only rows between hairlines: Continue reading, Coming soon, reviews |
//! | [`ListRow`] | The list row treatment on a collection's `ListItem`: padding, fills, focus edge |
//! | [`Lane`] | A fixed, right-aligned lane for trailing figures or actions, kept when empty |
//! | [`PropertyList`] | Key and value rows: Edition, Owned, Privacy, Added |
//! | [`Meter`] | Progress: 6 px beside words, 2 px in a row, with an optional pace tick |
//! | [`EmptyState`], [`empty_state`] | A region with nothing to show: icon, title, one sentence, a button |
//! | [`Skeleton`], [`SkeletonText`], [`SkeletonRow`] | Content still loading: pulsing shapes, lines, a row |
//! | [`TextButton`] | In-app navigation inside content: a book title, an author, a series |
//! | [`BylineLink`] | A person's name that opens their Hardcover profile in the browser |
//! | [`NavRow`] | A sidebar destination: icon, label, count or shortcut, selected state |
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
mod empty_state;
pub mod focus;
#[cfg(feature = "demo")]
pub mod gallery;
mod headers;
pub mod kit;
mod meter;
mod nav_row;
mod object_bar;
mod rows;
mod skeleton;
mod surfaces;
mod text_button;

pub use avatar::{Avatar, AvatarSize, initials};
pub use byline_link::BylineLink;
pub use empty_state::{EmptyState, empty_state, empty_state_with_action};
pub use headers::{Greeting, PageHeader, SectionHeader, date_line, greeting, greeting_at, see_all};
pub use meter::Meter;
pub use nav_row::NavRow;
pub use object_bar::{BackTo, ObjectBar, ObjectTools, back_label, tools_of};
pub use rows::{Lane, ListRow, PropertyList, RowSpacing, Sequence};
pub use skeleton::{Skeleton, SkeletonRow, SkeletonText};
pub use surfaces::{Block, Rail, RailSection, blocks, panel};
pub use text_button::TextButton;
