//! The library destination: the model of the user's library, its shelves and
//! the master–detail view.

pub mod book_row;
mod cache_worker;
pub mod collection;
pub mod library_view;
pub mod model;
pub(crate) mod writes;

use gpui_kit::{Entity, SharedString};

use crate::ui::LoadFailed;
use model::LibraryModel;

/// What Home, Stats, Series and the shelves show in place of their empty
/// states while the library has never loaded (`LibraryModel::load_failure`):
/// the failure, filling the region, and "Try again", which syncs.
pub fn load_failed(
    id: &'static str,
    model: &Entity<LibraryModel>,
    message: SharedString,
) -> LoadFailed {
    let model = model.clone();
    LoadFailed::new(id, message)
        .region("Couldn't load your library")
        .on_retry(move |_, _, cx| model.update(cx, LibraryModel::sync))
}
