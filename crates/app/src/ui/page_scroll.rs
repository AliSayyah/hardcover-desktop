//! Scrolling a page from the keys (`hardcover-design`, desktop practice:
//! every page scrolls from the keyboard). A page with no list to move
//! through (Stats, Discover) scrolls itself:
//!
//! - [`scrolled_to`]: where Page Up and Page Down (and Space), Home and End
//!   and the arrows take a page's scroll.
//! - [`scroll_page`]: applies it to a `ScrollHandle` from a key press. With
//!   `all_keys` false only Page Up and Page Down count: for a page whose
//!   controls (a chip row, a title) have focus and keep the other keys.

use gpui_kit::{KeyDownEvent, Pixels, ScrollHandle, Window, point, px, rems};

/// Scrolls `scroll` for `event`, if it is a page's key (only Page Up and
/// Page Down unless `all_keys`). Returns whether it was one; the caller
/// then stops its propagation and redraws.
pub fn scroll_page(
    scroll: &ScrollHandle,
    event: &KeyDownEvent,
    all_keys: bool,
    window: &Window,
) -> bool {
    let keystroke = &event.keystroke;
    let mods = keystroke.modifiers;
    if mods.control || mods.alt || mods.platform || mods.function {
        return false;
    }
    if !all_keys && !matches!(keystroke.key.as_str(), "pageup" | "pagedown") {
        return false;
    }
    let offset = scroll.offset();
    let Some(y) = scrolled_to(
        &keystroke.key,
        mods.shift,
        offset.y,
        scroll.bounds().size.height,
        rems(SCROLL_LINE).to_pixels(window.rem_size()),
        scroll.max_offset().y,
    ) else {
        return false;
    };
    if y != offset.y {
        scroll.set_offset(point(offset.x, y));
    }
    true
}

/// An arrow key's step, in rem: a few lines of body text.
pub const SCROLL_LINE: f32 = 3.;

/// Where a page scrolled to `offset` (0 at the top, down to `-max` at the
/// bottom, as GPUI counts) goes for `key`: Page Down, Page Up and Space
/// (Shift+Space up) by 90% of the `viewport`, Home and End to the ends, Down
/// and Up by `line`. `None` for any other key.
pub fn scrolled_to(
    key: &str,
    shift: bool,
    offset: Pixels,
    viewport: Pixels,
    line: Pixels,
    max: Pixels,
) -> Option<Pixels> {
    let page = viewport * 0.9;
    let y = match key {
        "pagedown" => offset - page,
        "space" if !shift => offset - page,
        "pageup" => offset + page,
        "space" => offset + page,
        "down" if !shift => offset - line,
        "up" if !shift => offset + line,
        "home" if !shift => px(0.),
        "end" if !shift => -max,
        _ => return None,
    };
    Some(y.clamp(-max.max(px(0.)), px(0.)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_scroll_the_page_within_its_ends() {
        let (viewport, line, max) = (px(500.), px(48.), px(1000.));
        let at = |key: &str, shift: bool, offset: f32| {
            scrolled_to(key, shift, px(offset), viewport, line, max)
        };
        assert_eq!(at("pagedown", false, 0.), Some(px(-450.)));
        assert_eq!(at("space", false, 0.), Some(px(-450.)));
        assert_eq!(at("space", true, -450.), Some(px(0.)));
        assert_eq!(at("pageup", false, -900.), Some(px(-450.)));
        assert_eq!(at("down", false, 0.), Some(px(-48.)));
        assert_eq!(at("up", false, -48.), Some(px(0.)));
        assert_eq!(at("end", false, 0.), Some(px(-1000.)));
        assert_eq!(at("home", false, -700.), Some(px(0.)));
        // Clamped at both ends.
        assert_eq!(at("pagedown", false, -900.), Some(px(-1000.)));
        assert_eq!(at("up", false, 0.), Some(px(0.)));
        // Other keys are left alone.
        assert_eq!(at("left", false, 0.), None);
        assert_eq!(at("a", false, 0.), None);
        // Nothing to scroll: stays at the top.
        assert_eq!(
            scrolled_to("end", false, px(0.), viewport, line, px(0.)),
            Some(px(0.))
        );
    }
}
