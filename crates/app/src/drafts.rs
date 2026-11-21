//! Words the reader has typed and not saved: a review or notes in a book
//! view's editor, or set aside when the view moved to another book. Such a
//! draft lives only in memory, so what would end the run (restarting into an
//! update) asks first while one exists.
//!
//! A view that holds drafts registers a check when it is made; the check
//! answers `None` once the view is gone, and is dropped then.
//!
//! ```ignore
//! let view = cx.entity().downgrade();
//! drafts::register(move |cx| view.upgrade().map(|v| v.read(cx).has_unsaved_drafts(cx)), cx);
//! if drafts::any(cx) { /* ask first */ }
//! ```

use gpui_kit::{App, Global};

type Check = Box<dyn Fn(&App) -> Option<bool>>;

#[derive(Default)]
struct Drafts(Vec<Check>);

impl Global for Drafts {}

/// Registers a holder of drafts: `check` says whether it holds any, or
/// `None` once it is gone.
pub fn register(check: impl Fn(&App) -> Option<bool> + 'static, cx: &mut App) {
    cx.default_global::<Drafts>().0.push(Box::new(check));
}

/// Whether any holder still holds an unsaved draft.
pub fn any(cx: &mut App) -> bool {
    let Some(drafts) = cx.try_global::<Drafts>() else {
        return false;
    };
    let answers: Vec<Option<bool>> = drafts.0.iter().map(|check| check(cx)).collect();
    let held = answers.contains(&Some(true));
    let mut answers = answers.into_iter();
    cx.global_mut::<Drafts>()
        .0
        .retain(|_| answers.next().flatten().is_some());
    held
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[gpui_kit::test]
    fn a_held_draft_is_found_and_gone_holders_are_dropped(cx: &mut gpui_kit::TestAppContext) {
        let held = Rc::new(Cell::new(Some(false)));
        cx.update(|cx| {
            assert!(!any(cx), "nothing registered");
            let state = held.clone();
            register(move |_| state.get(), cx);
            register(|_| None, cx);
            assert!(!any(cx));
            assert_eq!(
                cx.global::<Drafts>().0.len(),
                1,
                "the gone holder is dropped"
            );
        });
        held.set(Some(true));
        cx.update(|cx| assert!(any(cx)));
        held.set(None);
        cx.update(|cx| {
            assert!(!any(cx));
            assert!(cx.global::<Drafts>().0.is_empty());
        });
    }
}
