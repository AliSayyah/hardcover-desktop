//! A headless window for the components' UI tests: one element, built
//! afresh each frame, with the kit initialized. Keys go through the real
//! dispatch path, as the kit's own button tests send them.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, ClickEvent, Context, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div,
};

type Build = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

struct Host(Build);

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app: &mut App = cx;
        div().size_full().child((self.0)(window, app))
    }
}

/// Opens a window showing what `build` returns, and draws its first frame.
pub fn host(
    cx: &mut TestAppContext,
    build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> &mut VisualTestContext {
    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|_, _| Host(Box::new(build)));
    redraw(cx);
    cx
}

pub fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Moves focus as Tab does, and draws the frame that shows it.
pub fn tab(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.focus_next(cx));
    redraw(cx);
}

pub fn has_focus(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.focused(cx).is_some())
}

/// Presses and releases `key` ("enter", "space") on whatever has focus.
pub fn press(cx: &mut VisualTestContext, key: &str) {
    let keystroke = cx.update(|_, cx| {
        Keystroke::parse_with_os(key, cx.platform_info().os).expect("a key the platform knows")
    });
    cx.simulate_event(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke });
}

/// A click handler and the number of times it ran.
pub fn counter() -> (
    Rc<Cell<usize>>,
    impl Fn(&ClickEvent, &mut Window, &mut App) + Clone + 'static,
) {
    let count = Rc::new(Cell::new(0));
    let handler = {
        let count = count.clone();
        move |_: &ClickEvent, _: &mut Window, _: &mut App| count.set(count.get() + 1)
    };
    (count, handler)
}
