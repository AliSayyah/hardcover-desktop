//! Home's "Coming soon" card: the next releases in the reader's series within
//! the coming 12 months, and the way to the full list.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, Entity, FontWeight, ParentElement as _, Styled as _, div, prelude::FluentBuilder as _,
};
use hardcover_core::Date;

use super::home_view::card;
use crate::catalog::navigation::{self, Page};
use crate::library::model::LibraryModel;
use crate::upcoming::model::UpcomingModel;
use crate::upcoming::row::UpcomingRow;
use crate::upcoming::schedule::{next_releases, shown};

/// The card, or `None` when nothing is coming in the reader's series.
pub fn coming_soon_card(
    upcoming: &Entity<UpcomingModel>,
    library: &Entity<LibraryModel>,
    today: Date,
    cx: &App,
) -> Option<gpui_kit::Div> {
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
    let rows = next.iter().enumerate().map(|(ix, book)| {
        UpcomingRow::new("home-upcoming", book, today, library.clone()).divided(ix > 0)
    });
    let muted = cx.theme().muted_foreground;
    Some(
        card(cx)
            .child(
                h_flex()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Coming soon"))
                    .child(
                        Button::new("home-upcoming-all")
                            .ghost()
                            .xsmall()
                            .label(match books.len() {
                                1 => "See all".to_owned(),
                                n => format!("See all {n}"),
                            })
                            .on_click(|_, _, cx| navigation::open(Page::Upcoming, cx)),
                    ),
            )
            // With nothing in the coming 12 months, says what is further off.
            .when(next.is_empty(), |this| {
                this.child(div().text_sm().text_color(muted).child(match later {
                    1 => "Nothing is due in the next 12 months. 1 book is further off.".to_owned(),
                    n => {
                        format!("Nothing is due in the next 12 months. {n} books are further off.")
                    }
                }))
            })
            .child(v_flex().children(rows)),
    )
}
