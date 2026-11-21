//! SPIKE: an automatic interface scale on top of the desktop's own scale.
//!
//! The desktop's scale (`window.scale_factor()`) makes one logical pixel a
//! certain size in millimetres. This module makes the interface bigger when
//! that pixel, seen from the distance a screen of that size is usually viewed
//! from, is smaller (in angle) than a reference setup's. It never makes it
//! smaller than the desktop chose.
//!
//! Facts come from a second Wayland connection of our own (`wl_output`:
//! name, size in mm, current mode; `zxdg_output_v1`: logical size), because
//! GPUI drops the millimetres. The window's output is found by matching
//! GPUI's display uuid (a v5 uuid of the `wl_output` name).
//!
//! `HC_SCALE_PROBE=1` logs everything to stderr. `HC_UI_SCALE=<k>` forces a
//! factor; `HC_UI_SCALE_REF=<angle>` overrides the reference angle.

use std::time::Instant;

use gpui_kit::{App, AppContext as _, Window};

/// What one output reports.
#[derive(Debug, Clone, Default)]
pub struct OutputFacts {
    pub name: Option<String>,
    pub make: String,
    pub model: String,
    /// Physical size from `wl_output.geometry` (EDID), untransformed.
    pub mm: (i32, i32),
    /// The current mode in device pixels, untransformed.
    pub mode: (i32, i32),
    /// The integer `wl_output.scale`.
    pub int_scale: i32,
    pub transform: String,
    /// The logical size from `zxdg_output_v1`, if the compositor has it.
    pub logical: Option<(i32, i32)>,
}

/// The angle (radians) one logical pixel subtends on the reference setup: a
/// 24" 1920×1080 monitor at scale 1, seen from its usual distance. A 14"
/// 1920×1200 laptop at 1.25 gives almost exactly the same angle.
pub fn reference_angle() -> f64 {
    if let Some(v) = std::env::var("HC_UI_SCALE_REF")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        return v;
    }
    // 24" 16:9 is 531.3 × 298.9 mm.
    pixel_angle((531.3, 298.9), (1920., 1080.), 1.0)
}

/// The usual viewing distance, in mm, for a screen whose shorter side is
/// `short_mm`: about 45 cm for a 14" laptop, 64 cm for a 24" monitor, 70 cm
/// for a 27", 80 cm for a 32". The shorter side, not the diagonal, so an
/// ultrawide counts as the monitor of the same height. A heuristic; no
/// source measures it.
pub fn viewing_distance_mm(short_mm: f64) -> f64 {
    (145. + 1.65 * short_mm).clamp(400., 1000.)
}

/// The angle one logical pixel subtends, in radians.
pub fn pixel_angle(mm: (f64, f64), px: (f64, f64), desktop_scale: f64) -> f64 {
    let px_per_mm = ((px.0 * px.1) / (mm.0 * mm.1)).sqrt();
    let logical_px_mm = desktop_scale / px_per_mm;
    logical_px_mm / viewing_distance_mm(mm.0.min(mm.1))
}

/// Why no factor could be worked out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoScale {
    /// No size, or one too small to be a real screen (projectors, VMs,
    /// headless outputs report 0).
    NoSize,
    /// The size is the aspect ratio (16×9, 16×10 cm), as some EDIDs encode.
    AspectAsSize,
    /// Taller than 70 cm: a TV, viewed from a distance we can't guess.
    Television,
}

/// The raw factor (before stepping), or why there is none.
pub fn raw_factor(facts: &OutputFacts, desktop_scale: f64) -> Result<f64, NoScale> {
    let (w, h) = (f64::from(facts.mm.0), f64::from(facts.mm.1));
    if w < 100. || h < 60. || facts.mode.0 <= 0 || facts.mode.1 <= 0 {
        return Err(NoScale::NoSize);
    }
    let aspect_sizes = [
        (16., 9.),
        (16., 10.),
        (160., 90.),
        (160., 100.),
        (4., 3.),
        (40., 30.),
    ];
    if aspect_sizes
        .iter()
        .any(|&(a, b)| (w, h) == (a, b) || (w, h) == (b, a) || (w, h) == (a * 10., b * 10.))
    {
        return Err(NoScale::AspectAsSize);
    }
    if w.min(h) > 700. {
        return Err(NoScale::Television);
    }
    let angle = pixel_angle(
        (w, h),
        (f64::from(facts.mode.0), f64::from(facts.mode.1)),
        desktop_scale,
    );
    Ok(reference_angle() / angle)
}

/// The factor to apply: in eighths, rounded down (0.2 of a step's grace),
/// never below 1 (the desktop's choice is the floor) nor above 1.5.
pub fn step(raw: f64) -> f64 {
    ((raw * 8. + 0.2).floor() / 8.).clamp(1.0, 1.5)
}

pub fn probe_enabled() -> bool {
    std::env::var_os("HC_SCALE_PROBE").is_some()
}

macro_rules! probe {
    ($($arg:tt)*) => {
        if probe_enabled() {
            eprintln!("[scale-probe] {}", format!($($arg)*));
        }
    };
}

/// Reads every output over a connection of our own.
#[cfg(target_os = "linux")]
pub fn read_outputs() -> Result<Vec<OutputFacts>, String> {
    wl::read_outputs()
}

#[cfg(not(target_os = "linux"))]
pub fn read_outputs() -> Result<Vec<OutputFacts>, String> {
    Err("not Linux".into())
}

/// The session's outputs, read at startup and again on a display change.
struct Outputs {
    facts: Vec<OutputFacts>,
    applied: Option<f64>,
    /// The (display, desktop scale) the factor was last worked out for.
    evaluated: Option<(Option<gpui_kit::DisplayId>, f32)>,
    /// Keeps the bounds and display subscriptions.
    _watcher: Option<gpui_kit::Entity<Watcher>>,
}

/// Owns the subscriptions: the desktop scale arrives after the display (the
/// fractional-scale event comes later than `wl_surface.enter`), and only a
/// bounds observer sees it change.
struct Watcher {
    _subscriptions: Vec<gpui_kit::Subscription>,
}

impl gpui_kit::Global for Outputs {}

fn uuid_of(name: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, name.as_bytes())
}

/// Reads the outputs, works out the factor for the window's output and
/// applies it, now and on every display change.
pub fn follow(window: &mut Window, cx: &mut App) {
    let opened = Instant::now();
    let t = Instant::now();
    let facts = read_outputs().unwrap_or_else(|e| {
        probe!("reading outputs failed: {e}");
        Vec::new()
    });
    probe!("read {} outputs in {:?}", facts.len(), t.elapsed());
    for f in &facts {
        probe!("  {f:?}");
        if let Some((lw, lh)) = f.logical {
            probe!(
                "    mode/logical = {:.4} x {:.4}",
                f64::from(f.mode.0) / f64::from(lw),
                f64::from(f.mode.1) / f64::from(lh)
            );
        }
    }
    cx.set_global(Outputs {
        facts,
        applied: None,
        evaluated: None,
        _watcher: None,
    });
    evaluate(window, cx, opened, "open");
    let watcher = cx.new(|cx: &mut gpui_kit::Context<Watcher>| Watcher {
        _subscriptions: vec![
            cx.observe_window_bounds(window, move |_, window, cx| {
                evaluate(window, cx, opened, "bounds changed");
            }),
            window.observe_window_display(move |window, cx| {
                // Modes and hotplug: read again (a few ms) before deciding.
                if let Ok(facts) = read_outputs() {
                    cx.global_mut::<Outputs>().facts = facts;
                }
                evaluate(window, cx, opened, "display changed");
            }),
        ],
    });
    cx.global_mut::<Outputs>()._watcher = Some(watcher);
}

fn evaluate(window: &mut Window, cx: &mut App, opened: Instant, why: &str) {
    let display = window.display(cx);
    let key = (display.as_ref().map(|d| d.id()), window.scale_factor());
    if cx.global::<Outputs>().evaluated == Some(key) {
        return;
    }
    cx.global_mut::<Outputs>().evaluated = Some(key);
    let desktop_scale = f64::from(window.scale_factor());
    let forced = std::env::var("HC_UI_SCALE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok());
    let output_scale =
        |f: &OutputFacts| f.logical.map(|(lw, _)| f64::from(f.mode.0) / f64::from(lw));
    let agrees = |f: &OutputFacts| output_scale(f).is_none_or(|s| (s - desktop_scale).abs() < 0.01);
    let uuid = display.as_ref().and_then(|d| d.uuid().ok());
    probe!(
        "{why} +{:?}: display {:?} uuid {:?} gpui bounds {:?}; window scale_factor {desktop_scale}",
        opened.elapsed(),
        display.as_ref().map(|d| d.id()),
        uuid,
        display.as_ref().map(|d| d.bounds()),
    );
    let mut all = cx.global::<Outputs>().facts.clone();
    // An output added since the last read (hotplug) isn't known yet.
    if !all.iter().any(|f| agrees(f) && f.logical.is_some())
        && let Ok(facts) = read_outputs()
    {
        probe!(
            "  no known output at this scale; read again: {} outputs",
            facts.len()
        );
        cx.global_mut::<Outputs>().facts = facts.clone();
        all = facts;
    }
    let by_uuid = all
        .iter()
        .find(|f| uuid.is_some() && f.name.as_deref().map(uuid_of) == uuid);
    if let Some(f) = by_uuid {
        probe!(
            "  uuid matches {:?}: mode/logical {:?} vs window scale_factor {desktop_scale} ({})",
            f.name,
            output_scale(f),
            if agrees(f) { "agree" } else { "DISAGREE" }
        );
    }
    // GPUI's display can lag (the fractional scale arrives a few ms after
    // `wl_surface.enter`) or go stale (seen after an output hotplug). Then the
    // one output whose own scale is the window's decides; else wait.
    let chosen = match by_uuid {
        Some(f) if agrees(f) => Some(f.clone()),
        _ => {
            let same: Vec<_> = all
                .iter()
                .filter(|f| output_scale(f).is_some_and(|s| (s - desktop_scale).abs() < 0.01))
                .collect();
            if let [f] = same.as_slice() {
                probe!("  matched by scale: {:?}", f.name);
                Some((*f).clone())
            } else {
                None
            }
        }
    };
    let Some(chosen) = chosen else {
        probe!("  output unknown; factor unchanged");
        if let Some(k) = forced {
            apply(k, window, cx);
        }
        return;
    };
    let k = match Some(&chosen) {
        None => 1.0,
        Some(f) => match raw_factor(f, desktop_scale) {
            Ok(raw) => {
                let (w, h) = (f64::from(f.mm.0), f64::from(f.mm.1));
                let diag = (w * w + h * h).sqrt() / 25.4;
                let px_per_mm = (f64::from(f.mode.0) * f64::from(f.mode.1) / (w * h)).sqrt();
                probe!(
                    "  {:.1}\" {}x{} mm, {:.0} ppi physical, {:.1} ppi logical, distance {:.0} mm, pixel {:.3e} rad vs reference {:.3e}: raw {raw:.3}, stepped {}",
                    diag,
                    f.mm.0,
                    f.mm.1,
                    px_per_mm * 25.4,
                    px_per_mm * 25.4 / desktop_scale,
                    viewing_distance_mm(w.min(h)),
                    reference_angle() / raw,
                    reference_angle(),
                    step(raw)
                );
                step(raw)
            }
            Err(why) => {
                probe!("  no factor: {why:?}; factor 1");
                1.0
            }
        },
    };
    let k = forced.unwrap_or(k);
    probe!("  applying ui scale {k} (rem {} px)", 16. * k);
    apply(k, window, cx);
}

fn apply(k: f64, window: &mut Window, cx: &mut App) {
    if cx.global::<Outputs>().applied == Some(k) {
        return;
    }
    cx.global_mut::<Outputs>().applied = Some(k);
    crate::design::theme::set_ui_scale(k as f32, window, cx);
}

#[cfg(target_os = "linux")]
mod wl {
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    use wayland_client::{
        Connection, Dispatch, QueueHandle, WEnum,
        globals::{GlobalListContents, registry_queue_init},
        protocol::{wl_output, wl_registry},
    };
    use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

    use super::OutputFacts;

    #[derive(Default)]
    struct State {
        outputs: Vec<OutputFacts>,
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_output::WlOutput, usize> for State {
        fn event(
            state: &mut Self,
            _: &wl_output::WlOutput,
            event: wl_output::Event,
            index: &usize,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let o = &mut state.outputs[*index];
            match event {
                wl_output::Event::Geometry {
                    physical_width,
                    physical_height,
                    make,
                    model,
                    transform,
                    ..
                } => {
                    o.mm = (physical_width, physical_height);
                    o.make = make;
                    o.model = model;
                    o.transform = format!("{transform:?}");
                }
                wl_output::Event::Mode {
                    flags,
                    width,
                    height,
                    ..
                } => {
                    if let WEnum::Value(flags) = flags
                        && flags.contains(wl_output::Mode::Current)
                    {
                        o.mode = (width, height);
                    }
                }
                wl_output::Event::Scale { factor } => o.int_scale = factor,
                wl_output::Event::Name { name } => o.name = Some(name),
                _ => {}
            }
        }
    }

    impl Dispatch<zxdg_output_manager_v1::ZxdgOutputManagerV1, ()> for State {
        fn event(
            _: &mut Self,
            _: &zxdg_output_manager_v1::ZxdgOutputManagerV1,
            _: zxdg_output_manager_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for State {
        fn event(
            state: &mut Self,
            _: &zxdg_output_v1::ZxdgOutputV1,
            event: zxdg_output_v1::Event,
            index: &usize,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let o = &mut state.outputs[*index];
            match event {
                zxdg_output_v1::Event::LogicalSize { width, height } => {
                    o.logical = Some((width, height));
                }
                zxdg_output_v1::Event::Name { name } if o.name.is_none() => {
                    o.name = Some(name);
                }
                _ => {}
            }
        }
    }

    /// The compositor's socket from `WAYLAND_DISPLAY`, never `WAYLAND_SOCKET`:
    /// that one is a single inherited connection, and GPUI's.
    fn socket_path() -> Result<PathBuf, String> {
        let display = std::env::var_os("WAYLAND_DISPLAY").ok_or("no WAYLAND_DISPLAY")?;
        let display = PathBuf::from(display);
        if display.is_absolute() {
            return Ok(display);
        }
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("no XDG_RUNTIME_DIR")?;
        Ok(PathBuf::from(runtime).join(display))
    }

    pub fn read_outputs() -> Result<Vec<OutputFacts>, String> {
        let stream = UnixStream::connect(socket_path()?).map_err(|e| e.to_string())?;
        let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let mut state = State::default();
        let mut outputs = Vec::new();
        for global in globals.contents().clone_list() {
            if global.interface == "wl_output" {
                let index = state.outputs.len();
                state.outputs.push(OutputFacts::default());
                let output: wl_output::WlOutput =
                    globals
                        .registry()
                        .bind(global.name, global.version.min(4), &qh, index);
                outputs.push(output);
            }
        }
        let manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1> =
            globals.bind(&qh, 2..=3, ()).ok();
        if let Some(manager) = &manager {
            for (index, output) in outputs.iter().enumerate() {
                manager.get_xdg_output(output, &qh, index);
            }
        }
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        Ok(state.outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(mm: (i32, i32), mode: (i32, i32)) -> OutputFacts {
        OutputFacts {
            mm,
            mode,
            ..Default::default()
        }
    }

    /// The table in the report: (setup, mm, mode, desktop scale, factor).
    #[test]
    fn picks() {
        let cases = [
            ("24in 1080p @1", (531, 299), (1920, 1080), 1.0, 1.0),
            ("27in 1440p @1", (597, 336), (2560, 1440), 1.0, 1.25),
            (
                "27in 4K @1.5 (MD271UL)",
                (597, 336),
                (3840, 2160),
                1.5,
                1.25,
            ),
            ("27in 4K @2", (597, 336), (3840, 2160), 2.0, 1.0),
            ("32in 4K @1.5", (708, 398), (3840, 2160), 1.5, 1.25),
            ("34in UW 3440x1440 @1", (797, 334), (3440, 1440), 1.0, 1.25),
            (
                "16in 2560x1600 @1.6 (eDP-1)",
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
        for (name, mm, mode, s, want) in cases {
            let raw = raw_factor(&facts(mm, mode), s).unwrap();
            println!("{name}: raw {raw:.3} -> {}", step(raw));
            assert_eq!(step(raw), want, "{name}: raw {raw:.3}");
        }
    }

    #[test]
    fn bogus_sizes_give_none() {
        assert_eq!(
            raw_factor(&facts((0, 0), (1920, 1080)), 1.0),
            Err(NoScale::NoSize)
        );
        assert_eq!(
            raw_factor(&facts((160, 90), (1920, 1080)), 1.0),
            Err(NoScale::AspectAsSize)
        );
        assert_eq!(
            raw_factor(&facts((1660, 934), (3840, 2160)), 2.0),
            Err(NoScale::Television)
        );
    }
}
