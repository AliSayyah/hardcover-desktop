//! The pure part of the automatic scale: what an output is, the factor it
//! gets, which output the window is on, and the window sizes. Tested here;
//! the probes and GPUI are in the parent module.

/// What the compositor (or the X server) says about one monitor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Output {
    /// The connector's name (`DP-1`, `eDP-1`): GPUI's Wayland display uuid
    /// is a v5 uuid of it.
    pub name: Option<String>,
    /// The physical size in mm (`wl_output.geometry`, from the EDID), in the
    /// panel's own orientation.
    pub mm: (i32, i32),
    /// The current mode in device pixels, in the panel's own orientation.
    pub mode: (i32, i32),
    /// The size in logical pixels (`zxdg_output_v1`), rotated with the
    /// output. Wayland only.
    pub logical: Option<(i32, i32)>,
    /// Where the monitor is, in device pixels of the root window (X11 only:
    /// a window is matched to its monitor by position there).
    pub origin: Option<(i32, i32)>,
}

/// The reference setup a logical pixel is measured against: a 24"
/// 1920×1080 monitor at scale 1 (531.3 × 298.9 mm). A 14" 1920×1200 laptop
/// at 1.25 gives almost exactly the same angle.
const REFERENCE_MM: (f64, f64) = (531.3, 298.9);
const REFERENCE_PX: (f64, f64) = (1920., 1080.);

/// The factor's steps: eighths.
const STEPS: f64 = 8.;
/// How far below a step the raw factor may be and still reach it, in steps.
const GRACE: f64 = 0.2;
/// Never smaller than the desktop chose, never more than half again.
const FLOOR: f64 = 1.0;
const CAP: f64 = 1.5;

/// The angle (radians) one logical pixel subtends on the reference setup.
fn reference_angle() -> f64 {
    pixel_angle(REFERENCE_MM, REFERENCE_PX, 1.0)
}

/// The usual viewing distance, in mm, of a screen whose shorter side is
/// `short_mm`: about 45 cm for a 14" laptop, 64 cm for a 24" monitor, 70 cm
/// for a 27", 80 cm for a 32". The shorter side, not the diagonal, so an
/// ultrawide counts as the monitor of its height. A heuristic: no source
/// measures it.
pub fn viewing_distance_mm(short_mm: f64) -> f64 {
    (145. + 1.65 * short_mm).clamp(400., 1000.)
}

/// The angle one logical pixel subtends, in radians.
fn pixel_angle(mm: (f64, f64), px: (f64, f64), desktop_scale: f64) -> f64 {
    let px_per_mm = ((px.0 * px.1) / (mm.0 * mm.1)).sqrt();
    let logical_px_mm = desktop_scale / px_per_mm;
    logical_px_mm / viewing_distance_mm(mm.0.min(mm.1))
}

/// Why an output gets no factor (it then gets 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoFactor {
    /// No size, or one too small to be a screen: projectors, virtual and
    /// headless outputs report 0.
    NoSize,
    /// The size is the aspect ratio (16 × 9 cm and the like), as some EDIDs
    /// encode it.
    AspectAsSize,
    /// Taller than 70 cm: a TV, viewed from a distance we can't guess.
    Television,
}

/// The factor for `output` at `desktop_scale`, before stepping.
fn raw_factor(output: &Output, desktop_scale: f64) -> Result<f64, NoFactor> {
    let (w, h) = (f64::from(output.mm.0), f64::from(output.mm.1));
    if w < 100. || h < 60. || output.mode.0 <= 0 || output.mode.1 <= 0 {
        return Err(NoFactor::NoSize);
    }
    if !desktop_scale.is_finite() || desktop_scale <= 0. {
        return Err(NoFactor::NoSize);
    }
    const ASPECTS: [(f64, f64); 3] = [(16., 9.), (16., 10.), (4., 3.)];
    let (long, short) = (w.max(h), w.min(h));
    if ASPECTS
        .iter()
        .any(|&(a, b)| (long, short) == (a * 10., b * 10.) || (long, short) == (a * 100., b * 100.))
    {
        return Err(NoFactor::AspectAsSize);
    }
    if short > 700. {
        return Err(NoFactor::Television);
    }
    let angle = pixel_angle(
        (w, h),
        (f64::from(output.mode.0), f64::from(output.mode.1)),
        desktop_scale,
    );
    Ok(reference_angle() / angle)
}

/// The raw factor in eighths, rounded down with 0.2 of a step's grace,
/// between 1 and 1.5.
fn step(raw: f64) -> f64 {
    ((raw * STEPS + GRACE).floor() / STEPS).clamp(FLOOR, CAP)
}

/// The factor for `output` at `desktop_scale`: how much larger than the
/// desktop's own scale the interface is drawn.
pub fn factor(output: &Output, desktop_scale: f64) -> Result<f64, NoFactor> {
    raw_factor(output, desktop_scale).map(step)
}

/// The output's own scale: its mode over its logical size, long side to
/// long side, so a rotated output (whose logical size is rotated and whose
/// mode isn't) compares the same sides. `None` without a logical size.
pub fn output_scale(output: &Output) -> Option<f64> {
    let (lw, lh) = output.logical?;
    let logical = lw.max(lh);
    let mode = output.mode.0.max(output.mode.1);
    (logical > 0 && mode > 0).then(|| f64::from(mode) / f64::from(logical))
}

/// Whether `output`'s own scale is `window_scale`.
fn at_scale(output: &Output, window_scale: f64) -> bool {
    output_scale(output).is_some_and(|s| (s - window_scale).abs() < 0.01)
}

/// Which output a Wayland window is on, from GPUI's display (a v5 uuid of an
/// output's name) and the window's scale. Only an output whose own scale is
/// the window's qualifies, as GPUI's display can lag (the fractional scale
/// arrives after `wl_surface.enter`) or go stale after a hotplug:
/// - the output GPUI names, when it qualifies;
/// - else the one output that qualifies;
/// - else none, and the factor stays as it is.
pub fn choose_by_scale(
    outputs: &[Output],
    named: impl Fn(&str) -> bool,
    window_scale: f64,
) -> Option<usize> {
    let gpui = outputs
        .iter()
        .position(|o| o.name.as_deref().is_some_and(&named));
    if let Some(ix) = gpui.filter(|&ix| at_scale(&outputs[ix], window_scale)) {
        return Some(ix);
    }
    let mut qualifying = outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| at_scale(o, window_scale))
        .map(|(ix, _)| ix);
    match (qualifying.next(), qualifying.next()) {
        (Some(ix), None) => Some(ix),
        _ => None,
    }
}

/// Which X11 monitor holds `point` (device pixels of the root window): the
/// window's centre. X11 has one scale for every monitor, so position is all
/// there is to go by.
pub fn choose_by_position(outputs: &[Output], point: (f64, f64)) -> Option<usize> {
    outputs.iter().position(|o| {
        o.origin.is_some_and(|(x, y)| {
            let (x, y) = (f64::from(x), f64::from(y));
            let (w, h) = (f64::from(o.mode.0), f64::from(o.mode.1));
            point.0 >= x && point.0 < x + w && point.1 >= y && point.1 < y + h
        })
    })
}

/// `HARDCOVER_UI_SCALE`: a factor between 0.5 and 3, else nothing.
pub fn parse_forced(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|k| k.is_finite() && (0.5..=3.).contains(k))
}

/// A width and a height in logical pixels.
pub type Dimensions = (f32, f32);

/// The window's default size at factor 1: where the README's screenshots
/// are taken.
pub const DEFAULT_SIZE: Dimensions = (1200., 800.);
/// The smallest window the layouts are designed for, at factor 1.
pub const MIN_SIZE: Dimensions = (800., 560.);

/// `design` at factor `k`, in logical pixels, no larger than `fit` (a
/// monitor's logical size). A factor below 1 keeps the design size.
pub fn scaled(design: Dimensions, k: f64, fit: Option<Dimensions>) -> Dimensions {
    let k = k.max(1.) as f32;
    let (w, h) = (design.0 * k, design.1 * k);
    match fit {
        Some((fw, fh)) => (w.min(fw), h.min(fh)),
        None => (w, h),
    }
}

/// The sizes the window opens with, worked out before it exists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opening {
    /// The factor the window is expected to open at, applied before its first
    /// frame.
    pub k: f64,
    /// The size it opens at: the default at `k`, within the expected
    /// monitor.
    pub size: Dimensions,
    /// Its minimum size, which can't change once it has opened.
    pub min: Dimensions,
}

/// The opening sizes for these outputs, each at its own scale (there is no
/// window scale yet), or at `forced`.
///
/// Wayland doesn't say where a new window will go, so the guess is the
/// largest output: a laptop docked to a monitor usually works on the
/// monitor. Should the window land elsewhere, it is resized once (see the
/// parent module). GPUI sets the minimum only when the window opens, so it
/// is the design minimum at the largest factor of any output, within the
/// smallest output: the layouts then have their design minimum on every
/// monitor the window can move to.
pub fn opening(outputs: &[Output], forced: Option<f64>) -> Opening {
    let known: Vec<(f64, Dimensions)> = outputs
        .iter()
        .filter_map(|o| {
            let scale = output_scale(o)?;
            let (lw, lh) = o.logical?;
            let k = forced.unwrap_or_else(|| factor(o, scale).unwrap_or(1.));
            Some((k, (lw as f32, lh as f32)))
        })
        .collect();
    let forced_k = forced.unwrap_or(1.);
    let Some(&(k, fit)) = known
        .iter()
        .max_by(|a, b| (a.1.0 * a.1.1).total_cmp(&(b.1.0 * b.1.1)))
    else {
        return Opening {
            k: forced_k,
            size: scaled(DEFAULT_SIZE, forced_k, None),
            min: scaled(MIN_SIZE, forced_k, None),
        };
    };
    let largest_k = known.iter().map(|(k, _)| *k).fold(1., f64::max);
    let smallest = known
        .iter()
        .map(|(_, fit)| *fit)
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)));
    let size = scaled(DEFAULT_SIZE, k, Some(fit));
    let min = scaled(MIN_SIZE, largest_k, smallest);
    Opening {
        k,
        size,
        min: (min.0.min(size.0), min.1.min(size.1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(mm: (i32, i32), mode: (i32, i32)) -> Output {
        Output {
            mm,
            mode,
            ..Default::default()
        }
    }

    fn named(name: &str, mm: (i32, i32), mode: (i32, i32), scale: f64) -> Output {
        Output {
            name: Some(name.into()),
            mm,
            mode,
            logical: Some((
                (f64::from(mode.0) / scale).round() as i32,
                (f64::from(mode.1) / scale).round() as i32,
            )),
            origin: None,
        }
    }

    /// The owner's two monitors, as Hyprland reports them.
    fn owners_monitor() -> Output {
        named("DP-1", (597, 336), (3840, 2160), 1.5)
    }
    fn owners_laptop() -> Output {
        named("eDP-1", (345, 215), (2560, 1600), 1.6)
    }

    #[test]
    fn factors_for_common_setups() {
        let cases = [
            ("24in 1080p @1", (531, 299), (1920, 1080), 1.0, 1.0),
            ("27in 1440p @1", (597, 336), (2560, 1440), 1.0, 1.25),
            (
                "27in 4K @1.5: the owner's",
                (597, 336),
                (3840, 2160),
                1.5,
                1.25,
            ),
            ("27in 4K @2", (597, 336), (3840, 2160), 2.0, 1.0),
            ("27in 5K @2", (597, 336), (5120, 2880), 2.0, 1.25),
            ("32in 4K @1.5", (708, 398), (3840, 2160), 1.5, 1.25),
            ("34in UW 3440x1440 @1", (797, 334), (3440, 1440), 1.0, 1.25),
            (
                "16in 2560x1600 @1.6: the owner's",
                (345, 215),
                (2560, 1600),
                1.6,
                1.0,
            ),
            ("14in 1920x1200 @1.25", (302, 188), (1920, 1200), 1.25, 1.0),
            ("14in 1920x1200 @1", (302, 188), (1920, 1200), 1.0, 1.25),
            ("13.3in 2560x1600 @2", (287, 179), (2560, 1600), 2.0, 1.0),
            ("14in 2880x1800 @2", (302, 188), (2880, 1800), 2.0, 1.0),
        ];
        for (name, mm, mode, scale, want) in cases {
            assert_eq!(factor(&output(mm, mode), scale), Ok(want), "{name}");
        }
    }

    #[test]
    fn a_rotated_monitor_gets_the_same_factor() {
        let mut rotated = owners_monitor();
        rotated.logical = Some((1440, 2560));
        assert_eq!(output_scale(&rotated), Some(1.5));
        assert_eq!(factor(&rotated, 1.5), Ok(1.25));
    }

    #[test]
    fn steps_round_down_with_a_little_grace() {
        assert_eq!(step(1.2), 1.125);
        // 0.2 of a step below 1.25 still reaches it.
        assert_eq!(step(1.25 - 0.2 / 8.), 1.25);
        assert_eq!(step(1.25 - 0.21 / 8.), 1.125);
        assert_eq!(step(0.7), 1.0);
        assert_eq!(step(2.4), 1.5);
    }

    #[test]
    fn sizes_that_cant_be_trusted_get_none() {
        let mode = (1920, 1080);
        assert_eq!(factor(&output((0, 0), mode), 1.), Err(NoFactor::NoSize));
        assert_eq!(factor(&output((90, 50), mode), 1.), Err(NoFactor::NoSize));
        assert_eq!(
            factor(&output((600, 340), (0, 0)), 1.),
            Err(NoFactor::NoSize)
        );
        assert_eq!(factor(&output((600, 340), mode), 0.), Err(NoFactor::NoSize));
        for mm in [(160, 90), (160, 100), (1600, 900), (900, 1600), (400, 300)] {
            assert_eq!(
                factor(&output(mm, mode), 1.),
                Err(NoFactor::AspectAsSize),
                "{mm:?}"
            );
        }
        assert_eq!(
            factor(&output((1660, 934), (3840, 2160)), 2.),
            Err(NoFactor::Television)
        );
    }

    #[test]
    fn the_named_output_wins_when_it_is_at_the_windows_scale() {
        let outputs = [owners_laptop(), owners_monitor()];
        assert_eq!(choose_by_scale(&outputs, |n| n == "DP-1", 1.5), Some(1));
        assert_eq!(choose_by_scale(&outputs, |n| n == "eDP-1", 1.6), Some(0));
    }

    #[test]
    fn a_stale_name_gives_way_to_the_one_output_at_the_scale() {
        let outputs = [owners_laptop(), owners_monitor()];
        // GPUI still names the laptop, but the window is at the monitor's scale.
        assert_eq!(choose_by_scale(&outputs, |n| n == "eDP-1", 1.5), Some(1));
        // No name yet (before `wl_surface.enter`).
        assert_eq!(choose_by_scale(&outputs, |_| false, 1.6), Some(0));
    }

    #[test]
    fn two_outputs_at_the_scale_need_the_name() {
        let left = named("DP-1", (597, 336), (3840, 2160), 1.5);
        let right = named("DP-2", (531, 299), (2880, 1620), 1.5);
        let outputs = [left, right];
        assert_eq!(choose_by_scale(&outputs, |_| false, 1.5), None);
        assert_eq!(choose_by_scale(&outputs, |n| n == "DP-2", 1.5), Some(1));
        // Nothing at the window's scale (it hasn't arrived yet).
        assert_eq!(choose_by_scale(&outputs, |n| n == "DP-2", 1.0), None);
    }

    #[test]
    fn an_output_without_a_logical_size_never_qualifies() {
        let mut monitor = owners_monitor();
        monitor.logical = None;
        assert_eq!(choose_by_scale(&[monitor], |_| true, 1.5), None);
    }

    #[test]
    fn x11_monitors_are_found_by_position() {
        let at = |x, y, mode| Output {
            mode,
            origin: Some((x, y)),
            ..Default::default()
        };
        let outputs = [at(0, 0, (2560, 1600)), at(2560, 0, (3840, 2160))];
        assert_eq!(choose_by_position(&outputs, (100., 100.)), Some(0));
        assert_eq!(choose_by_position(&outputs, (2560., 2000.)), Some(1));
        assert_eq!(choose_by_position(&outputs, (100., 1700.)), None);
    }

    #[test]
    fn the_forced_factor_is_a_plain_number() {
        assert_eq!(parse_forced("1.25"), Some(1.25));
        assert_eq!(parse_forced(" 1 "), Some(1.));
        for bad in ["", "big", "0", "-1", "NaN", "inf", "10"] {
            assert_eq!(parse_forced(bad), None, "{bad}");
        }
    }

    #[test]
    fn window_sizes_keep_their_design_size() {
        assert_eq!(scaled(DEFAULT_SIZE, 1., None), (1200., 800.));
        assert_eq!(scaled(DEFAULT_SIZE, 1.25, None), (1500., 1000.));
        assert_eq!(scaled(MIN_SIZE, 1.25, None), (1000., 700.));
        // Never larger than the monitor.
        assert_eq!(
            scaled(DEFAULT_SIZE, 1.5, Some((1600., 1000.))),
            (1600., 1000.)
        );
        // A factor below 1 is forced; the window keeps its design size.
        assert_eq!(scaled(MIN_SIZE, 0.75, None), MIN_SIZE);
    }

    #[test]
    fn the_owner_opens_on_the_monitor_at_its_design_size() {
        let open = opening(&[owners_laptop(), owners_monitor()], None);
        assert_eq!(open.k, 1.25);
        assert_eq!(open.size, (1500., 1000.));
        // The minimum holds at 1.25 on the monitor, and fits the laptop.
        assert_eq!(open.min, (1000., 700.));
    }

    #[test]
    fn the_laptop_alone_opens_as_before() {
        let open = opening(&[owners_laptop()], None);
        assert_eq!(open.k, 1.);
        assert_eq!(open.size, DEFAULT_SIZE);
        assert_eq!(open.min, MIN_SIZE);
    }

    #[test]
    fn a_small_monitor_bounds_the_sizes() {
        // A 27" 1440p at 1 (k 1.25) beside a 1366×768 laptop at 1.
        let monitor = named("DP-1", (597, 336), (2560, 1440), 1.);
        let laptop = named("eDP-1", (344, 194), (1366, 768), 1.);
        let open = opening(&[laptop, monitor], None);
        assert_eq!(open.size, (1500., 1000.));
        assert_eq!(open.min, (1000., 700.));
        let tiny = named("eDP-1", (256, 144), (1024, 600), 1.);
        let open = opening(&[tiny, named("DP-1", (597, 336), (2560, 1440), 1.)], None);
        assert_eq!(open.min, (1000., 600.));
    }

    #[test]
    fn no_outputs_open_at_the_design_size() {
        let open = opening(&[], None);
        assert_eq!((open.k, open.size, open.min), (1., DEFAULT_SIZE, MIN_SIZE));
        let open = opening(&[], Some(1.25));
        assert_eq!(open.size, (1500., 1000.));
        let open = opening(&[owners_laptop(), owners_monitor()], Some(1.));
        assert_eq!((open.k, open.size, open.min), (1., DEFAULT_SIZE, MIN_SIZE));
    }
}
