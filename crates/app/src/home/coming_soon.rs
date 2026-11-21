//! Home's "Coming soon" (`hardcover-design`, Screens → Home): the section
//! that closes the main column. The `section` title with a small ghost "See
//! all 5", then up to five upcoming rows due in the coming 12 months. With
//! none due but some further off, one muted sentence says so and "See all"
//! stays. It hides when nothing is coming.

use gpui_kit::{
    AnyElement, App, Entity, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::{Date, UpcomingBook};

use crate::catalog::navigation::{self, Page};
use crate::design::{TypeRoles as _, colors};
use crate::format::books_count_text;
use crate::library::model::LibraryModel;
use crate::ui::{SectionHeader, Sequence, see_all};
use crate::upcoming::model::UpcomingModel;
use crate::upcoming::row::UpcomingRow;
use crate::upcoming::schedule::{next_releases, shown};

/// What the section shows, worked out with Home's digest.
#[derive(Debug, Clone)]
pub struct ComingSoon {
    /// The next releases within the coming 12 months.
    next: Vec<UpcomingBook>,
    /// How many are coming in all.
    total: usize,
}

/// What is coming in the reader's series, or `None` when nothing is.
pub fn coming_soon(
    upcoming: &UpcomingModel,
    library: &LibraryModel,
    today: Date,
) -> Option<ComingSoon> {
    let books = shown(
        upcoming.books(),
        |id| library.entry_for_book(id).map(|e| e.status),
        today,
    );
    if books.is_empty() {
        return None;
    }
    Some(ComingSoon {
        next: next_releases(&books, today).into_iter().cloned().collect(),
        total: books.len(),
    })
}

/// The section for `soon`.
pub fn coming_soon_section(
    soon: &ComingSoon,
    library: &Entity<LibraryModel>,
    today: Date,
    cx: &App,
) -> AnyElement {
    let later = soon.total - soon.next.len();
    let rows = soon
        .next
        .iter()
        .map(|book| UpcomingRow::new("home-upcoming", book, today, library.clone()));
    let total = u32::try_from(soon.total).unwrap_or(u32::MAX);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(SectionHeader::new("Coming soon").action(see_all(
            "home-upcoming-all",
            (total > 1).then_some(total),
            |_, _, cx| navigation::open(Page::Upcoming, cx),
        )))
        .when(soon.next.is_empty(), |this| {
            this.child(
                div()
                    .body()
                    .text_color(colors(cx).fg_muted)
                    .child(further_off_text(later)),
            )
        })
        .when(!soon.next.is_empty(), |this| {
            this.child(Sequence::new().children(rows))
        })
        .into_any_element()
}

/// What Home says when nothing is due within 12 months but `later` books are
/// further off.
pub fn further_off_text(later: usize) -> String {
    let books = books_count_text(u32::try_from(later).unwrap_or(u32::MAX));
    let verb = if later == 1 { "is" } else { "are" };
    format!("Nothing is due in the next 12 months. {books} {verb} further off.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_off_books_are_counted() {
        assert_eq!(
            further_off_text(1),
            "Nothing is due in the next 12 months. 1 book is further off."
        );
        assert_eq!(
            further_off_text(2),
            "Nothing is due in the next 12 months. 2 books are further off."
        );
    }
}
