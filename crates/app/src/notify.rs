//! Notifications for changes that finish in the background: a list or goal
//! write started from a dialog or a control that may be gone by the time it
//! lands. A failure always shows, titled with what couldn't be done; success
//! shows only when the change itself isn't visible (`hardcover-design`,
//! Feedback: skip success messages when the result is visible).
//!
//! ```ignore
//! let task = lists.update(cx, |m, cx| m.delete(id, cx));
//! notify_failure(task, "Couldn't delete the list", window, cx);
//! ```

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, SharedString, Task, Window};

/// Shows a notification titled `title` if `task` fails. A task that
/// resolves to `None` was dropped because the session ended, and shows
/// nothing.
pub fn notify_failure<T: 'static>(
    task: Task<Option<Result<T, SharedString>>>,
    title: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    notify_outcome(task, title, |_, _, _| None, window, cx);
}

/// Shows `task`'s outcome: a notification titled `title` if it fails, and
/// what `success` makes of its result (if anything) when it succeeds;
/// nothing once the session has ended (`None`).
pub fn notify_outcome<T: 'static>(
    task: Task<Option<Result<T, SharedString>>>,
    title: &'static str,
    success: impl FnOnce(T, &mut Window, &mut App) -> Option<Notification> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            let Some(result) = task.await else {
                return;
            };
            let _ = cx.update(|window, cx| {
                let note = match result {
                    Err(message) => Some(Notification::error(message).title(title)),
                    Ok(value) => success(value, window, cx),
                };
                if let Some(note) = note {
                    window.push_notification(note, cx);
                }
            });
        })
        .detach();
}
