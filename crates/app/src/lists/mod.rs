//! The user's Hardcover lists: the sidebar section, a list's page, the list
//! dialogs, and the book page's "Lists" menu and Owned switch. Shown only when
//! the session holds `read:lists`; changes need `write:lists`, except adding
//! an edition to the Owned list, which is `write:library`.

pub mod book_lists;
pub mod list_dialog;
pub mod list_view;
pub mod model;
pub mod sidebar;
