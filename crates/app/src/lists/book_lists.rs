//! The book page's list controls: the "Lists" menu, with the user's lists
//! checked where the book is on them, and the Owned switch for a library
//! entry. They live here so the book page only places them.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, WindowExt as _, h_flex};
use gpui_kit::{
    AnyElement, App, Entity, IntoElement, ParentElement as _, SharedString, Styled as _, Task,
    Window, div,
};
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LIBRARY, WRITE_LISTS};
use hardcover_core::{BookId, BookSummary, EditionSummary, LibraryEntry};

use super::list_dialog::open_new_list;
use super::model::ListsModel;
use crate::app_state::Services;
use crate::format::needs_permission;

/// The model, when the session can read lists.
fn readable(cx: &App) -> Option<Entity<ListsModel>> {
    let services = cx.try_global::<Services>()?;
    services.has_scope(READ_LISTS).then_some(())?;
    ListsModel::global(cx)
}

/// The book page now shows `book`: makes sure its lists are known.
pub fn book_shown(book: BookId, cx: &mut App) {
    if let Some(lists) = readable(cx) {
        lists.update(cx, |m, cx| m.ensure_book(book, cx));
    }
}

/// Shows a failed list change as a notification.
fn notify_failure(
    task: Task<Result<(), SharedString>>,
    title: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    notify_result(task, title, None, window, cx);
}

/// Shows a list change's outcome: always a failure, and success when
/// `success` says what to show.
fn notify_result(
    task: Task<Result<(), SharedString>>,
    title: &'static str,
    success: Option<&'static str>,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            let note = match task.await {
                Err(message) => Notification::error(message).title(title),
                Ok(()) => match success {
                    Some(text) => Notification::success(text),
                    None => return,
                },
            };
            let _ = cx.update(|window, cx| window.push_notification(note, cx));
        })
        .detach();
}

/// The "Lists" button and its menu, or `None` without `read:lists`.
pub fn lists_button(book: &BookSummary, cx: &App) -> Option<AnyElement> {
    let lists = readable(cx)?;
    let book = book.clone();
    Some(
        Button::new("book-lists")
            .ghost()
            .icon(IconName::GalleryVerticalEnd)
            .label("Lists")
            .dropdown_menu(move |menu, _, cx| {
                let model = lists.read(cx);
                let can_write = Services::global(cx).has_scope(WRITE_LISTS);
                let busy = model.is_busy(book.id);
                let known = model.rows_for(book.id).is_some();
                // Capped at half the window; many lists scroll.
                let mut menu = menu.scrollable(true);
                if !known {
                    menu = menu.item(PopupMenuItem::label("Loading your lists…"));
                }
                let mut any = false;
                for list in model.user_lists() {
                    any = true;
                    let on = model.is_on(list.id, book.id).unwrap_or(false);
                    let (lists, id, book_id) = (lists.clone(), list.id, book.id);
                    menu = menu.item(
                        PopupMenuItem::new(list.name.clone())
                            .checked(on)
                            .disabled(!known || busy || !can_write)
                            .on_click(move |_, window, cx| {
                                let task = lists.update(cx, |m, cx| {
                                    if on {
                                        m.remove(id, book_id, cx)
                                    } else {
                                        m.add(id, book_id, cx)
                                    }
                                });
                                notify_failure(task, "Couldn't change the list", window, cx);
                            }),
                    );
                }
                if !any && known {
                    menu = menu.item(PopupMenuItem::label("No lists yet"));
                }
                if !can_write {
                    menu = menu.item(PopupMenuItem::label(needs_permission(WRITE_LISTS)));
                }
                let (new_lists, new_book) = (lists.clone(), book.clone());
                menu.separator().item(
                    PopupMenuItem::new("New list…")
                        .icon(IconName::Plus)
                        .disabled(!can_write)
                        .on_click(move |_, window, cx| {
                            open_new_list(new_lists.clone(), Some(new_book.clone()), window, cx)
                        }),
                )
            })
            .into_any_element(),
    )
}

/// "The paperback edition", from the edition's format.
fn edition_phrase(edition: &EditionSummary) -> String {
    edition
        .format
        .as_ref()
        .or(edition.reading_format.as_ref())
        .map_or_else(
            || "This edition".to_owned(),
            |format| format!("The {} edition", format.to_lowercase()),
        )
}

/// The confirmation's text. `one_way`: the app can't read the list, so it
/// can't show the book as owned or take it off again.
fn owned_description(edition: &EditionSummary, one_way: bool) -> String {
    let mut text = format!(
        "{} goes on your Owned list on Hardcover and appears in your activity, \
         which others may see even if this book is private.",
        edition_phrase(edition)
    );
    if one_way {
        text.push_str(" To take it off the list, use the Hardcover website.");
    }
    text
}

/// Asks before marking the entry's edition owned: Hardcover announces it.
pub fn confirm_owned(entry: &LibraryEntry, window: &mut Window, cx: &mut App) {
    let Some(lists) = ListsModel::global(cx) else {
        return;
    };
    let Some(edition) = entry.edition.clone() else {
        return;
    };
    let book = entry.book.id;
    // Without the switch, the notification is the only sign it worked.
    let one_way = offers_one_way_owned(cx);
    let title: SharedString = format!("Add “{}” to your Owned list?", entry.book.title).into();
    let description: SharedString = owned_description(&edition, one_way).into();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let lists = lists.clone();
        let edition = edition.id;
        alert
            .title(title.clone())
            .description(description.clone())
            .confirm()
            .ok_text("Add")
            .on_ok(move |_, window, cx| {
                let task = lists.update(cx, |m, cx| m.set_owned(book, Some(edition), true, cx));
                let success = one_way.then_some("Added to your Owned list on Hardcover.");
                notify_result(
                    task,
                    "Couldn't add it to your Owned list",
                    success,
                    window,
                    cx,
                );
                true
            })
    });
}

/// The Owned switch for a library entry, or `None` without `read:lists`
/// (the book page then offers only the one-way "Add to Owned list…").
///
/// Hardcover owns editions, and a book counts as owned when any edition of
/// it is on the Owned list. Turning it on adds the entry's edition (and asks
/// first, since Hardcover announces it); turning it off takes every edition
/// of the book off the list.
pub fn owned_control(entry: &LibraryEntry, cx: &App) -> Option<AnyElement> {
    let lists = readable(cx)?;
    let model = lists.read(cx);
    let services = Services::global(cx);
    let book = entry.book.id;
    let rows = model.owned_rows(book);
    let owned = rows.as_ref().is_some_and(|r| !r.is_empty());
    let busy = model.is_busy(book);
    let edition = entry.edition.as_ref();
    let muted = cx.theme().muted_foreground;
    // Adding goes through `edition_owned` (write:library); taking it off is
    // a list change (write:lists).
    let (allowed, reason): (bool, Option<SharedString>) = match (&rows, owned) {
        (None, _) => (false, Some("Checking your Owned list…".into())),
        (Some(_), true) if !services.has_scope(WRITE_LISTS) => {
            (false, Some(needs_permission(WRITE_LISTS).into()))
        }
        (Some(_), false) if !services.has_scope(WRITE_LIBRARY) => {
            (false, Some(needs_permission(WRITE_LIBRARY).into()))
        }
        (Some(_), false) if edition.is_none() => (false, Some("Choose an edition first.".into())),
        _ => (true, None),
    };
    let note: Option<SharedString> = match (&rows, reason) {
        (_, Some(reason)) => Some(reason),
        // Only worth saying when the entry names an edition that isn't the one owned.
        (Some(rows), None) if owned => edition
            .filter(|e| !rows.iter().any(|r| r.edition_id == Some(e.id)))
            .map(|_| "Another edition is on your Owned list.".into()),
        _ => None,
    };
    let entry = entry.clone();
    Some(
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .child(
                Switch::new("book-owned")
                    .checked(owned)
                    .accessibility_label("Owned")
                    .disabled(!allowed || busy)
                    .on_click(move |checked, window, cx| {
                        if *checked {
                            confirm_owned(&entry, window, cx);
                        } else if let Some(lists) = ListsModel::global(cx) {
                            let task = lists
                                .update(cx, |m, cx| m.set_owned(entry.book.id, None, false, cx));
                            notify_failure(
                                task,
                                "Couldn't take it off your Owned list",
                                window,
                                cx,
                            );
                        }
                    }),
            )
            .children(note.map(|note| div().min_w_0().text_sm().text_color(muted).child(note)))
            .into_any_element(),
    )
}

/// Whether the book page should offer the one-way "Add to Owned list…":
/// only when the session can't read the list, so the switch can't show.
pub fn offers_one_way_owned(cx: &App) -> bool {
    readable(cx).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::EditionId;

    #[test]
    fn edition_phrase_names_the_format() {
        let mut edition = EditionSummary::new(EditionId::new(1));
        assert_eq!(edition_phrase(&edition), "This edition");
        edition.format = Some("Paperback".into());
        assert_eq!(edition_phrase(&edition), "The paperback edition");
    }

    #[test]
    fn only_the_one_way_confirmation_says_how_to_undo() {
        let edition = EditionSummary::new(EditionId::new(1));
        let hint = "To take it off the list, use the Hardcover website.";
        assert!(owned_description(&edition, true).ends_with(hint));
        assert!(!owned_description(&edition, false).contains(hint));
        assert!(owned_description(&edition, false).starts_with("This edition goes on"));
    }
}
