//! The sidebar's "Lists" section: the user's lists with their book counts,
//! and a way to start one.

use std::rc::Rc;

use gpui_kit::component::sidebar::{SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{ActiveTheme as _, IconName};
use gpui_kit::{App, Entity, ParentElement as _, Styled as _, Window, div};
use hardcover_core::ListId;
use hardcover_core::auth::scopes::{READ_LISTS, WRITE_LISTS};

use super::list_dialog::open_new_list;
use super::model::{ListsModel, ListsStatus};
use crate::app_state::Services;

/// Opens a list as the workspace's destination.
pub type SelectList = Rc<dyn Fn(ListId, &mut Window, &mut App)>;

/// The section, once the session can read lists and they have loaded (or
/// failed to) at least once. `active` is the list shown, if any.
pub fn lists_group(
    lists: &Entity<ListsModel>,
    active: Option<ListId>,
    select: SelectList,
    cx: &App,
) -> Option<SidebarGroup<SidebarMenu>> {
    let model = lists.read(cx);
    let services = Services::global(cx);
    if !services.has_scope(READ_LISTS)
        || matches!(model.status(), ListsStatus::Idle)
        || !model.has_loaded()
    {
        return None;
    }
    let can_write = services.has_scope(WRITE_LISTS);
    let muted = cx.theme().sidebar_foreground.opacity(0.7);
    let mut items: Vec<SidebarMenuItem> = model
        .lists()
        .iter()
        .map(|list| {
            let (id, count) = (list.id, list.books_count);
            let select = select.clone();
            SidebarMenuItem::new(list.name.clone())
                .icon(if list.is_owned() {
                    IconName::Folder
                } else {
                    IconName::GalleryVerticalEnd
                })
                .active(active == Some(id))
                .suffix(move |_, _| div().text_xs().text_color(muted).child(count.to_string()))
                .on_click(move |_, window, cx| select(id, window, cx))
        })
        .collect();
    if let ListsStatus::Failed(_) = model.status()
        && model.lists().is_empty()
    {
        let lists = lists.clone();
        items.push(
            SidebarMenuItem::new("Couldn't load lists. Retry")
                .icon(IconName::CircleAlert)
                .on_click(move |_, _, cx| lists.update(cx, |m, cx| m.load(cx))),
        );
    }
    let new_list = lists.clone();
    items.push(
        SidebarMenuItem::new("New list…")
            .icon(IconName::Plus)
            .disable(!can_write)
            .on_click(move |_, window, cx| open_new_list(new_list.clone(), None, window, cx)),
    );
    Some(SidebarGroup::new("Lists").child(SidebarMenu::new().children(items)))
}
