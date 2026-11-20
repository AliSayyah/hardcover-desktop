//! Home's "Coming soon" (`hardcover-design`, Screens → Home): the section
//! that closes the main column. The `section` title with a small ghost "See
//! all 5", then up to five upcoming rows due in the coming 12 months. With
//! none due but some further off, one muted sentence says so and "See all"
//! stays. It hides when nothing is coming.

use gpui_kit::{
    AnyElement, App, Entity, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::Date;

use crate::catalog::navigation::{self, Page};
use crate::design::{TypeRoles as _, colors};
use crate::format::books_count_text;
use crate::library::model::LibraryModel;
use crate::ui::{SectionHeader, Sequence, see_all};
use crate::upcoming::model::UpcomingModel;
use crate::upcoming::row::UpcomingRow;
use crate::upcoming::schedule::{next_releases, shown};

/// The section, or `None` when nothing is coming in the reader's series.
pub fn coming_soon_section(
    upcoming: &Entity<UpcomingModel>,
    library: &Entity<LibraryModel>,
    today: Date,
    cx: &App,
) -> Option<AnyElement> {
    let model = library.read(cx);
    let books = shown(
        upcoming.read(cx).books(),
        |id| model.entry_for_book(id).map(|e| e.status),
        today,
    );
    if books.is_empty() {
        return None;
    }
    let next = next_releases(&books, today);
    let later = books.len() - next.len();
    let rows = next
        .iter()
        .map(|book| UpcomingRow::new("home-upcoming", book, today, library.clone()));
    let total = u32::try_from(books.len()).unwrap_or(u32::MAX);
    Some(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(SectionHeader::new("Coming soon").action(see_all(
                "home-upcoming-all",
                (total > 1).then_some(total),
                |_, _, cx| navigation::open(Page::Upcoming, cx),
            )))
            .when(next.is_empty(), |this| {
                this.child(
                    div()
                        .body()
                        .text_color(colors(cx).fg_muted)
                        .child(further_off_text(later)),
                )
            })
            .when(!next.is_empty(), |this| {
                this.child(Sequence::new().children(rows))
            })
            .into_any_element(),
    )
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
