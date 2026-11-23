//! Theme tiles (`hardcover-design`, Components → Theme tiles): Settings'
//! choice of theme. Each tile draws its theme as a miniature of the app (the
//! sidebar with its selected row, a title and byline, a meter, a rating and
//! the inset panel), the light palette and the dark one split on
//! a diagonal, from the palettes themselves, so it shows true whatever the
//! app shows now. The name sits under it.
//!
//! - The chosen tile has the `selected` fill, a check before its name and
//!   its name in medium weight; another tile takes the `hover` fill under
//!   the pointer; focus is the ring, after keyboard input.
//! - The row is one Tab stop, as a radio group is: Tab lands on the chosen
//!   tile, Left and Right (Home and End) choose the tile beside it (at the
//!   ends) and move focus with the choice. Choosing applies at once.
//!
//! ```ignore
//! ThemeTiles::new("theme", family, |family, _, cx| { /* save it */ })
//! ```

use std::rc::Rc;

use gpui_kit::component::{Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, BorderStyle, Bounds, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Path, Pixels, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, actions, canvas, div, point,
    prelude::FluentBuilder as _, px, quad, rems, transparent_black,
};
use hardcover_core::prefs::ThemeFamily;

use crate::design::color::Palette;
use crate::design::{TypeRoles as _, colors, size};
use crate::ui::focus::{FocusTreatment as _, box_radius, focus_visible, keyed_focus};

actions!(
    theme_tiles,
    [PreviousTheme, NextTheme, FirstTheme, LastTheme]
);

/// The key context of a row of theme tiles.
const CONTEXT: &str = "ThemeTiles";

/// The row's keys: deeper than the dialog's, so they win while focus is in
/// the row.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", PreviousTheme, Some(CONTEXT)),
        KeyBinding::new("right", NextTheme, Some(CONTEXT)),
        KeyBinding::new("home", FirstTheme, Some(CONTEXT)),
        KeyBinding::new("end", LastTheme, Some(CONTEXT)),
    ]);
}

type OnSelect = Rc<dyn Fn(ThemeFamily, &mut Window, &mut App)>;

/// The three theme tiles, one chosen.
#[derive(IntoElement)]
pub struct ThemeTiles {
    id: SharedString,
    selected: ThemeFamily,
    on_select: OnSelect,
}

impl ThemeTiles {
    pub fn new(
        id: impl Into<SharedString>,
        selected: ThemeFamily,
        on_select: impl Fn(ThemeFamily, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            selected,
            on_select: Rc::new(on_select),
        }
    }
}

/// A theme's name.
pub fn family_name(family: ThemeFamily) -> &'static str {
    match family {
        ThemeFamily::Paper => "Paper",
        ThemeFamily::Graphite => "Graphite",
        ThemeFamily::Hardcover => "Hardcover",
    }
}

impl RenderOnce for ThemeTiles {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handles: Rc<Vec<FocusHandle>> = Rc::new(
            ThemeFamily::ALL
                .iter()
                .map(|family| keyed_focus(self.tile_id(*family), window, cx))
                .collect(),
        );
        let current = ThemeFamily::ALL
            .iter()
            .position(|family| *family == self.selected)
            .unwrap_or(0);
        let last = ThemeFamily::ALL.len() - 1;
        let choose = {
            let handles = handles.clone();
            let on_select = self.on_select.clone();
            move |ix: usize, window: &mut Window, cx: &mut App| {
                on_select(ThemeFamily::ALL[ix], window, cx);
                window.focus(&handles[ix], cx);
            }
        };
        let (c1, c2, c3) = (choose.clone(), choose.clone(), choose.clone());
        let tiles: Vec<_> = ThemeFamily::ALL
            .iter()
            .enumerate()
            .map(|(ix, family)| self.tile(*family, &handles[ix], window, cx))
            .collect();
        h_flex()
            .id(ElementId::Name(self.id.clone()))
            .key_context(CONTEXT)
            .role(Role::RadioGroup)
            .aria_label("Theme")
            .gap_2()
            .items_start()
            .on_action(move |_: &PreviousTheme, window, cx| {
                c1(current.saturating_sub(1), window, cx);
            })
            .on_action(move |_: &NextTheme, window, cx| {
                c2((current + 1).min(last), window, cx);
            })
            .on_action(move |_: &FirstTheme, window, cx| c3(0, window, cx))
            .on_action(move |_: &LastTheme, window, cx| choose(last, window, cx))
            .children(tiles)
    }
}

impl ThemeTiles {
    fn tile_id(&self, family: ThemeFamily) -> ElementId {
        ElementId::Name(format!("{}-{}", self.id, family_name(family)).into())
    }

    fn tile(
        &self,
        family: ThemeFamily,
        focus: &FocusHandle,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let colors = colors(cx);
        let selected = family == self.selected;
        let radius = box_radius(cx);
        let on_select = self.on_select.clone();
        let name = family_name(family);
        gpui_kit::base::Button::new(self.tile_id(family))
            .track_focus(focus)
            .tab_stop(selected)
            .role(Role::RadioButton)
            .accessibility_label(name)
            .aria_toggled(selected.into())
            .relative()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(rems(6. / 16.))
            .p(rems(6. / 16.))
            .rounded(radius)
            .cursor_pointer()
            .when(selected, |this| this.bg(colors.selected))
            .when(!selected, |this| {
                let hover = colors.hover;
                this.hover(move |style| style.bg(hover))
            })
            .when(focus_visible(focus, window), |this| {
                this.focus_ring(radius, cx)
            })
            .on_click(move |_, window, cx| on_select(family, window, cx))
            .child(art(Palette::light(family), Palette::dark(family)))
            .child(
                h_flex()
                    .gap_1()
                    .px(rems(2. / 16.))
                    .items_center()
                    .min_w_0()
                    .when(selected, |this| {
                        this.child(
                            Icon::new(IconName::Check)
                                .small()
                                .flex_none()
                                .text_color(colors.fg),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .map(|this| {
                                if selected {
                                    this.body_strong()
                                } else {
                                    this.body()
                                }
                            })
                            .text_color(colors.fg)
                            .child(name),
                    ),
            )
    }
}

/// The miniature: `light`'s app, with `dark`'s over the part right of the
/// diagonal.
fn art(light: &'static Palette, dark: &'static Palette) -> impl IntoElement {
    v_flex().w_full().h(size::THEME_ART).child(
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| paint_art(bounds, light, dark, window),
        )
        .size_full(),
    )
}

/// The miniature's height in design px; its width is the tile's.
const ART_HEIGHT: f32 = 84.;
/// Where the diagonal meets the top and the bottom, as shares of the width.
const SPLIT_TOP: f32 = 0.64;
const SPLIT_BOTTOM: f32 = 0.36;
/// The miniature's corner radius, design px.
const ART_RADIUS: f32 = 6.;

/// A point in design px from the miniature's top left.
type Pt = (f32, f32);

fn paint_art(bounds: Bounds<Pixels>, light: &Palette, dark: &Palette, window: &mut Window) {
    // Design px to window px, so the miniature follows the interface scale.
    let unit = window.rem_size() / 16.;
    let width = bounds.size.width / unit;
    let split = ((SPLIT_TOP * width, 0.), (SPLIT_BOTTOM * width, ART_HEIGHT));
    let to_window = |(x, y): Pt| point(bounds.origin.x + unit * x, bounds.origin.y + unit * y);
    // The light miniature whole, then the dark one over its half: the dark
    // edge blends into the light one under it, with no seam.
    for (palette, half) in [(light, None), (dark, Some(split))] {
        for (shape, color) in shapes(palette, width) {
            let shape = match half {
                Some(line) => clip_right_of(&shape, line),
                None => shape,
            };
            let Some((first, rest)) = shape.split_first() else {
                continue;
            };
            let mut path = Path::new(to_window(*first));
            for p in rest {
                path.line_to(to_window(*p));
            }
            window.paint_path(path, color);
        }
    }
    // An edge that holds on any fill, in either mode.
    window.paint_quad(quad(
        bounds,
        unit * ART_RADIUS,
        transparent_black(),
        px(1.),
        gpui_kit::hsla(0., 0., 0.5, 0.25),
        BorderStyle::Solid,
    ));
}

/// The miniature's shapes in `p`, `width` design px wide, back to front:
/// each a convex outline and its colour.
fn shapes(p: &Palette, width: f32) -> Vec<(Vec<Pt>, gpui_kit::Hsla)> {
    let h = ART_HEIGHT;
    let r = ART_RADIUS;
    let side = (width * 0.3).round();
    let mut out = vec![
        (rounded(0., 0., width, h, [r, r, r, r]), p.bg),
        (rounded(0., 0., side, h, [r, 0., 0., r]), p.sidebar),
        (rounded(side - 1., 0., 1., h, [0.; 4]), p.border),
    ];
    // The sidebar's rows, the second one selected.
    let inner = side - 10.;
    let bar = p.fg_muted.opacity(0.45);
    for (y, share) in [(8., 0.7), (30., 0.6), (39., 0.75), (48., 0.5), (57., 0.65)] {
        out.push((rounded(5., y, inner * share, 4., [2.; 4]), bar));
    }
    out.push((rounded(3., 17., side - 6., 8., [3.; 4]), p.selected));
    out.push((rounded(6., 19., 4., 4., [1.; 4]), p.accent));
    // The page: a title and byline, a meter, a rating.
    let x = side + 8.;
    let tx = x;
    let tw = (width - 8. - tx).max(0.);
    out.push((rounded(tx, 10., tw * 0.8, 5., [2.; 4]), p.fg));
    out.push((rounded(tx, 19., tw * 0.55, 4., [2.; 4]), p.fg_muted));
    out.push((rounded(tx, 28., tw, 3., [1.5; 4]), p.track));
    out.push((rounded(tx, 28., tw * 0.55, 3., [1.5; 4]), p.accent));
    for star in 0..5u8 {
        let cx = tx + 2.5 + f32::from(star) * 7.;
        let color = if star < 4 { p.gold } else { p.star_empty };
        out.push((diamond(cx, 37.5, 2.5), color));
    }
    // The inset panel.
    out.push((
        rounded(x, h - 8. - 26., width - x - 8., 26., [4.; 4]),
        p.surface,
    ));
    out
}

/// A rectangle's outline with corner radii (top left, top right, bottom
/// right, bottom left), clockwise, each corner in four segments.
fn rounded(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> Vec<Pt> {
    let [tl, tr, br, bl] = radii.map(|r| r.min(w / 2.).min(h / 2.).max(0.));
    let corners = [
        (x + tl, y + tl, tl, 180.),
        (x + w - tr, y + tr, tr, 270.),
        (x + w - br, y + h - br, br, 0.),
        (x + bl, y + h - bl, bl, 90.),
    ];
    let mut out = Vec::with_capacity(20);
    for (cx, cy, r, start) in corners {
        if r == 0. {
            out.push((cx, cy));
            continue;
        }
        for step in 0..=4u8 {
            let angle = (start + f32::from(step) * 22.5_f32).to_radians();
            out.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
    }
    out
}

/// A rating star at this size: a square on its point.
fn diamond(cx: f32, cy: f32, r: f32) -> Vec<Pt> {
    vec![(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)]
}

/// The part of the convex `shape` right of the line from `a` to `b` (going
/// down the miniature), by Sutherland–Hodgman against one edge.
fn clip_right_of(shape: &[Pt], (a, b): (Pt, Pt)) -> Vec<Pt> {
    let side = |p: Pt| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let inside = |p: Pt| side(p) <= 0.;
    let cross = |p: Pt, q: Pt| {
        let (sp, sq) = (side(p), side(q));
        let t = sp / (sp - sq);
        (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1))
    };
    let mut out = Vec::with_capacity(shape.len() + 2);
    for (ix, &p) in shape.iter().enumerate() {
        let q = shape[(ix + 1) % shape.len()];
        match (inside(p), inside(q)) {
            (true, true) => out.push(q),
            (true, false) => out.push(cross(p, q)),
            (false, true) => {
                out.push(cross(p, q));
                out.push(q);
            }
            (false, false) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: (Pt, Pt) = ((64., 0.), (36., 84.));

    #[test]
    fn a_shape_left_of_the_diagonal_is_dropped_and_one_right_of_it_kept() {
        let left = rounded(0., 0., 20., 84., [0.; 4]);
        assert!(clip_right_of(&left, LINE).is_empty());
        let right = rounded(70., 10., 20., 20., [0.; 4]);
        let kept = clip_right_of(&right, LINE);
        assert_eq!(kept.len(), 4);
        for p in &right {
            assert!(kept.contains(p));
        }
    }

    #[test]
    fn a_shape_across_the_diagonal_is_cut_on_it() {
        let whole = rounded(0., 0., 100., 84., [0.; 4]);
        let cut = clip_right_of(&whole, LINE);
        // The dark half: the top right, the bottom right and the two points
        // where the diagonal meets the edges.
        for p in [(64., 0.), (100., 0.), (100., 84.), (36., 84.)] {
            assert!(
                cut.iter()
                    .any(|q| (q.0 - p.0).abs() < 1e-3 && (q.1 - p.1).abs() < 1e-3),
                "{p:?} in {cut:?}"
            );
        }
        assert!(cut.iter().all(|q| q.0 >= 36. - 1e-3));
    }

    /// The tiles are one Tab stop, landing on the chosen one; the arrows
    /// choose the tile beside it, stopping at the ends; Tab leaves the row.
    #[gpui_kit::test]
    fn the_tiles_are_one_tab_stop_and_the_arrows_choose(cx: &mut gpui_kit::TestAppContext) {
        use std::cell::{Cell, RefCell};

        use gpui_kit::component::button::Button;

        use crate::ui::test_support::{counter, host, press, redraw, tab};

        let chosen = Rc::new(Cell::new(ThemeFamily::Graphite));
        let picks: Rc<RefCell<Vec<ThemeFamily>>> = Rc::default();
        let (after, on_after) = counter();
        let cx = {
            let chosen = chosen.clone();
            let picks = picks.clone();
            host(cx, move |_, _| {
                let (chosen, picks) = (chosen.clone(), picks.clone());
                div()
                    .child(Button::new("before").label("Before"))
                    .child(ThemeTiles::new(
                        "theme",
                        chosen.get(),
                        move |family, _, _| {
                            chosen.set(family);
                            picks.borrow_mut().push(family);
                        },
                    ))
                    .child(
                        Button::new("after")
                            .label("After")
                            .on_click(on_after.clone()),
                    )
                    .into_any_element()
            })
        };
        cx.update(|window, _| window.activate_window());
        let key = |cx: &mut gpui_kit::VisualTestContext, key: &str| {
            press(cx, key);
            redraw(cx);
            cx.run_until_parked();
            redraw(cx);
        };

        tab(cx);
        tab(cx);
        // On the chosen tile: Space chooses it again.
        key(cx, "space");
        key(cx, "right");
        key(cx, "right");
        key(cx, "home");
        key(cx, "left");
        assert_eq!(
            *picks.borrow(),
            [
                ThemeFamily::Graphite,
                ThemeFamily::Hardcover,
                ThemeFamily::Hardcover,
                ThemeFamily::Paper,
                ThemeFamily::Paper,
            ]
        );
        tab(cx);
        key(cx, "enter");
        assert_eq!(after.get(), 1, "Tab left the row past the other tiles");
    }

    #[test]
    fn rounded_corners_stay_inside_the_rectangle() {
        let shape = rounded(10., 20., 30., 40., [6., 0., 6., 3.]);
        assert!(
            shape
                .iter()
                .all(|(x, y)| (10. - 1e-3..=40. + 1e-3).contains(x)
                    && (20. - 1e-3..=60. + 1e-3).contains(y))
        );
    }
}
