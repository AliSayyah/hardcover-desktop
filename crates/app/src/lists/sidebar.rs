//! The sidebar's "Lists" section: the user's lists with their book counts
//! (the Owned list first, then by name, as the model keeps them), and a way
//! to start one.

use std::rc::Rc;

use gpui_kit::component::IconName;
use gpui_kit::{App, Div, Entity, ParentElement as _, SharedString, Window, div};
use hardcover_core::ListId;
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LISTS};

use super::list_dialog::open_new_list;
use super::model::{ListsModel, ListsStatus};
use crate::app_state::Services;
use crate::format::{count_text, needs_permission};
use crate::ui::{NavRow, sidebar_label};

/// Opens a list as the workspace's destination.
pub type SelectList = Rc<dyn Fn(ListId, &mut Window, &mut App)>;

/// The section, once the session can read lists and they have loaded (or
/// failed to) at least once. `active` is the list shown, if any.
pub fn lists_section(
    lists: &Entity<ListsModel>,
    active: Option<ListId>,
    select: SelectList,
    cx: &App,
) -> Option<Div> {
    let model = lists.read(cx);
    let services = Services::global(cx);
    if !services.has_scope(READ_LISTS)
        || matches!(model.status(), ListsStatus::Idle)
        || !model.has_loaded()
    {
        return None;
    }
    let can_write = services.has_scope(WRITE_LISTS);
    let rows = model.lists().iter().map(|list| {
        let (id, count) = (list.id, list.books_count);
        let select = select.clone();
        NavRow::new(
            SharedString::from(format!("nav-list-{id}")),
            if list.is_owned() {
                IconName::Folder
            } else {
                IconName::GalleryVerticalEnd
            },
            list.name.clone(),
        )
        .selected(active == Some(id))
        .count(count_text(count))
        // A name that may be cut short shows whole in its tooltip (the row
        // fits about 20 characters beside its count).
        .tooltip((list.name.chars().count() > LONG_NAME).then(|| list.name.clone()))
        .on_click(move |_, window, cx| select(id, window, cx))
    });
    let failed = match model.status() {
        ListsStatus::Failed(message) if model.lists().is_empty() => Some(message.clone()),
        _ => None,
    };
    let retry = failed.map(|message| {
        let lists = lists.clone();
        NavRow::new(
            "nav-lists-retry",
            IconName::CircleAlert,
            "Try loading your lists again",
        )
        .tooltip(Some(format!("Couldn't load your lists. {message}")))
        .on_click(move |_, _, cx| lists.update(cx, |m, cx| m.load(cx)))
    });
    let new_list = lists.clone();
    let create = NavRow::new("nav-new-list", IconName::Plus, "New list…")
        .muted(true)
        .disabled(!can_write)
        .tooltip((!can_write).then(|| needs_permission(WRITE_LISTS)))
        .on_click(move |_, window, cx| open_new_list(new_list.clone(), None, window, cx));
    Some(
        div()
            .child(sidebar_label("Lists", cx))
            .children(rows)
            .children(retry)
            .child(create),
    )
}

/// The longest list name that surely fits its sidebar row, in characters.
const LONG_NAME: usize = 18;
