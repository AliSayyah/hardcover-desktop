//! The contrast every meaningful token pair must keep, in both themes
//! (`hardcover-design`, Tokens → Colour → Contrast). A token edit that
//! breaks one fails here, with the pair and its ratio.
//!
//! - Text that carries information (`fg`, `fg-muted`, accent, danger) is
//!   at least 4.5:1 (WCAG 1.4.3) on every surface it sits on, alone,
//!   hovered and selected.
//! - Marks that carry meaning without words are at least 3:1 (WCAG
//!   1.4.11): a field's outline, the user's rating against an empty star,
//!   the catalogue's stars, a warning icon, `fg-faint`.
//!
//! Translucent tokens (the overlays, `star-empty`, `chip`) are composited
//! onto their host first, as GPUI paints them.

use gpui_kit::Hsla;

use super::color::{Palette, contrast, solid};

/// A surface alone, hovered and selected.
fn states(p: &Palette, host: Hsla) -> [Hsla; 3] {
    [host, solid(host, p.hover), solid(host, p.selected)]
}

/// Every surface text sits on: paper, the sidebar, the panel and raised
/// controls, each alone, hovered and selected.
fn fills(p: &Palette) -> Vec<Hsla> {
    [p.bg, p.sidebar, p.surface, p.raised]
        .into_iter()
        .flat_map(|host| states(p, host))
        .collect()
}

/// A chip on paper, on the sidebar and on a hovered or selected row.
fn chips(p: &Palette) -> Vec<Hsla> {
    [
        p.bg,
        p.sidebar,
        solid(p.bg, p.hover),
        solid(p.bg, p.selected),
    ]
    .into_iter()
    .map(|host| solid(host, p.chip))
    .collect()
}

fn themes() -> [(&'static str, &'static Palette); 2] {
    [("light", Palette::light()), ("dark", Palette::dark())]
}

/// Fails with every pair under `min`, not just the first.
fn check(min: f32, pairs: impl IntoIterator<Item = (String, Hsla, Hsla)>) {
    let failures: Vec<String> = pairs
        .into_iter()
        .filter_map(|(name, fg, bg)| {
            let ratio = contrast(fg, bg);
            (ratio < min).then(|| format!("{name}: {ratio:.2}:1"))
        })
        .collect();
    assert!(failures.is_empty(), "under {min}:1: {failures:#?}");
}

fn over(theme: &str, name: &str, fg: Hsla, hosts: Vec<Hsla>) -> Vec<(String, Hsla, Hsla)> {
    hosts
        .into_iter()
        .enumerate()
        .map(|(ix, host)| (format!("{theme} {name} on host {ix}"), fg, host))
        .collect()
}

#[test]
fn text_that_carries_information_reads_at_aa() {
    for (theme, p) in themes() {
        check(
            4.5,
            [
                over(theme, "fg", p.fg, fills(p)),
                over(theme, "fg-muted", p.fg_muted, fills(p)),
                over(theme, "fg-muted on a chip", p.fg_muted, chips(p)),
                over(theme, "accent", p.accent, fills(p)),
                over(theme, "accent (a genre icon) on a chip", p.accent, chips(p)),
                over(theme, "danger", p.danger, states(p, p.bg).to_vec()),
                over(
                    theme,
                    "danger in a dialog",
                    p.danger,
                    states(p, p.raised).to_vec(),
                ),
                vec![(
                    format!("{theme} accent-fg on accent"),
                    p.accent_fg,
                    p.accent,
                )],
            ]
            .concat(),
        );
    }
}

#[test]
fn marks_that_carry_meaning_read_at_three_to_one() {
    for (theme, p) in themes() {
        // Stars sit on paper: the book header, Home's rows and the library
        // rows, which can be hovered or selected.
        let star_hosts = states(p, p.bg);
        let empty_stars: Vec<Hsla> = star_hosts
            .iter()
            .map(|host| solid(*host, p.star_empty))
            .collect();
        check(
            3.,
            [
                over(
                    theme,
                    "gold against an empty star",
                    p.gold,
                    empty_stars.clone(),
                ),
                over(theme, "gold (a row's star)", p.gold, star_hosts.to_vec()),
                over(
                    theme,
                    "a catalogue star against an empty one",
                    p.fg_muted,
                    empty_stars,
                ),
                over(
                    theme,
                    "border-strong (a field's outline)",
                    p.border_strong,
                    vec![p.raised, p.bg, p.surface, p.sidebar],
                ),
                over(theme, "fg-faint", p.fg_faint, fills(p)),
                over(theme, "warning on a chip", p.warning, chips(p)),
                over(theme, "accent (the focus edge)", p.accent, fills(p)),
            ]
            .concat(),
        );
    }
}

#[test]
fn cloth_ink_reads_on_every_cloth() {
    let p = Palette::light();
    check(
        4.5,
        p.cloth
            .iter()
            .enumerate()
            .map(|(ix, cloth)| (format!("cloth-ink on cloth {ix}"), p.cloth_ink, *cloth)),
    );
}

/// The scrollbar's thumb steps visibly when hovered, now that its rest
/// colour (`border-strong`) is as dark as `fg-faint`.
#[test]
fn the_scrollbar_thumb_steps_on_hover() {
    for (theme, p) in themes() {
        let ratio = contrast(p.fg_muted, p.border_strong);
        assert!(ratio >= 1.5, "{theme}: thumb hover {ratio:.2}:1 from rest");
    }
}
