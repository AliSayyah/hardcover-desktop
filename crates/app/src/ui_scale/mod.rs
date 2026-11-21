//! The automatic interface scale: a larger rem on large monitors.
//!
//! The desktop's scale (`window.scale_factor()`) makes one logical pixel a
//! certain size. Seen from the distance a screen of that size is usually
//! viewed from, a logical pixel on a 27" 4K monitor at 1.5 is smaller (in
//! angle) than on a 24" 1080p monitor at 1, so the interface reads small.
//! This module raises the rem (`16 * k` px, through the theme's `font_size`,
//! which gpui-component's `Root` sets as the window's rem size) until a
//! logical pixel subtends at least the 24" monitor's angle. It never makes the
//! interface smaller than the desktop chose. The policy is pure and tested,
//! in [`policy`].
//!
//! **Facts.** GPUI drops the outputs' millimetres, so they come from a
//! Wayland connection of our own: every `wl_output`'s name, size in mm and
//! current mode, and its logical size from `zxdg_output_v1`. Each read takes
//! about a millisecond and runs off the UI thread: once in a thread started
//! at the top of `main` (its result sizes the window), again in the
//! background when the window's display changes.
//!
//! **Which monitor.** An output qualifies only if its own scale (mode ÷
//! logical size) is the window's: GPUI's display (a v5 uuid of the output's
//! name) when it qualifies, else the one output that does, else the factor
//! stays as it is.
//!
//! **X11** gets a factor of 1. RandR has each monitor's mm, but GPUI's X11
//! window bounds are inside the window manager's frame, so finding the
//! window's monitor means asking the X server on every move; and with no
//! logical sizes, the window would open at the design size and keep a
//! minimum too small for a larger factor. Not worth it until someone runs
//! the app on X11 with a large monitor.
//!
//! **When.** On the window's bounds and display changes, which is where a new
//! desktop scale shows; the work is done only when the (output, scale) pair
//! changes. The factor stays put until the window lands on another monitor.
//!
//! **Window size.** The sizes are worked out before the window exists, from
//! the first read ([`policy::opening`]): the default size at the factor of
//! the largest output (Wayland doesn't say where a window will go; a docked
//! laptop usually works on its monitor), and a minimum at the largest factor
//! of any output, as GPUI sets the minimum only when the window opens. Both
//! stay within the smallest output. Resizing the window once it has landed
//! doesn't work: Hyprland keeps a floating window's size when GPUI resizes
//! its surface (GPUI then draws the new size inside the old frame), and a
//! tiling compositor decides the size anyway. So a window that lands on a
//! monitor with another factor keeps the size it opened at. Seen on a
//! visible window too: Hyprland kept its 1500 × 1000 frame.
//!
//! **Failure.** No socket, a missing protocol or sizes that can't be trusted
//! give a factor of 1, silently (a debug build logs one line).
//!
//! `HARDCOVER_UI_SCALE=<k>` forces a factor (`1` turns the feature off and
//! reads nothing; any factor opens the window at the design size times it,
//! not bounded by a monitor); `HARDCOVER_UI_SCALE_DEBUG=1` logs every
//! decision.

pub mod policy;
#[cfg(target_os = "linux")]
mod wayland;

use std::sync::mpsc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Size, Subscription, Window, px};

use policy::{Opening, Output};

/// Forces a factor.
const FORCE_ENV: &str = "HARDCOVER_UI_SCALE";
/// Logs every decision to stderr.
const DEBUG_ENV: &str = "HARDCOVER_UI_SCALE_DEBUG";
/// How long `main` waits for the first read before opening the window. The
/// read takes a millisecond and runs while the fonts load.
const FIRST_READ_WAIT: Duration = Duration::from_millis(250);

macro_rules! debug {
    ($($arg:tt)*) => {
        if std::env::var_os(DEBUG_ENV).is_some() {
            eprintln!("hardcover: ui scale: {}", format!($($arg)*));
        }
    };
}

/// One line, in a debug build, when the facts can't be read.
fn note_failure(why: &str) {
    if cfg!(debug_assertions) || std::env::var_os(DEBUG_ENV).is_some() {
        eprintln!("hardcover: ui scale: {why}; using 1");
    }
}

/// Whether GPUI runs on Wayland, by its own rule: `WAYLAND_DISPLAY` set
/// (else X11, which gets a factor of 1).
fn on_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn forced() -> Option<f64> {
    let value = std::env::var(FORCE_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let k = policy::parse_forced(&value);
    if k.is_none() {
        note_failure(&format!(
            "{FORCE_ENV}={value:?} is not a factor from 0.5 to 3"
        ));
    }
    k
}

/// Reads the outputs. Blocking: never on the UI thread.
fn read() -> Result<Vec<Output>, String> {
    #[cfg(target_os = "linux")]
    {
        wayland::read_outputs()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("not Linux".into())
    }
}

/// The first read, started at the top of `main`.
pub struct Probe {
    forced: Option<f64>,
    first: Option<mpsc::Receiver<Result<Vec<Output>, String>>>,
}

impl Probe {
    /// Starts reading the outputs on a thread of its own, unless a factor is
    /// forced or GPUI won't run on Wayland.
    pub fn start() -> Self {
        let forced = forced();
        let first = (forced.is_none() && on_wayland()).then(|| {
            let (send, receive) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("ui-scale".into())
                .spawn(move || {
                    let _ = send.send(read());
                });
            if let Err(e) = spawned {
                note_failure(&e.to_string());
            }
            receive
        });
        Self { forced, first }
    }

    /// Waits a moment for the first read, and works out the window's opening
    /// sizes from it.
    pub fn finish(self) -> Start {
        let follow = self.first.is_some();
        let outputs = match self.first.map(|r| r.recv_timeout(FIRST_READ_WAIT)) {
            Some(Ok(Ok(outputs))) => outputs,
            Some(Ok(Err(why))) => {
                note_failure(&why);
                Vec::new()
            }
            Some(Err(_)) => {
                note_failure("reading the outputs took too long");
                Vec::new()
            }
            None => Vec::new(),
        };
        let opening = policy::opening(&outputs, self.forced);
        debug!("opening {opening:?} from {outputs:?}");
        Start {
            follow,
            outputs,
            opening,
        }
    }
}

/// The first read's result, for `main` to open the window with.
pub struct Start {
    /// Whether the factor follows the window's monitor: on Wayland, unless a
    /// factor is forced.
    follow: bool,
    outputs: Vec<Output>,
    opening: Opening,
}

impl Start {
    /// The size the window opens at, in logical pixels.
    pub fn window_size(&self) -> Size<gpui_kit::Pixels> {
        let (w, h) = self.opening.size;
        Size::new(px(w), px(h))
    }

    /// The window's minimum size, in logical pixels.
    pub fn min_window_size(&self) -> Size<gpui_kit::Pixels> {
        let (w, h) = self.opening.min;
        Size::new(px(w), px(h))
    }
}

/// Keeps the window's factor in step with its monitor.
struct UiScale {
    outputs: Vec<Output>,
    /// Bumped by every read, so a new read is evaluated even on the same
    /// output and scale.
    generation: u64,
    /// The (output, desktop scale, generation) last evaluated.
    evaluated: Option<(usize, u32, u64)>,
    /// The factor applied.
    applied: f64,
    reading: bool,
    read_again: bool,
    /// A window scale no output had, read again for already.
    reread_for: Option<u32>,
    _subscriptions: Vec<Subscription>,
}

struct UiScaleGlobal(#[allow(dead_code)] Entity<UiScale>);

impl Global for UiScaleGlobal {}

/// Applies the opening factor, then follows the window's monitor. Call it
/// when the window opens, before its first frame.
pub fn follow(start: Start, window: &mut Window, cx: &mut App) {
    let Start {
        follow,
        outputs,
        opening,
    } = start;
    apply(opening.k, window, cx);
    if !follow {
        return;
    }
    let entity = cx.new(|cx: &mut Context<UiScale>| UiScale {
        outputs,
        generation: 0,
        evaluated: None,
        applied: opening.k,
        reading: false,
        read_again: false,
        reread_for: None,
        // The desktop's scale arrives after the display (the fractional
        // scale comes later than `wl_surface.enter`), and only a bounds
        // observer sees it change.
        _subscriptions: vec![cx.observe_window_bounds(window, |this, window, cx| {
            this.evaluate(window, cx);
        })],
    });
    let weak = entity.downgrade();
    let display = window.observe_window_display(move |window, cx| {
        // Modes and hotplugs: read again before deciding.
        let _ = weak.update(cx, |this, cx| this.read(window, cx));
    });
    entity.update(cx, |this, cx| {
        this._subscriptions.push(display);
        if this.outputs.is_empty() {
            this.read(window, cx);
        } else {
            this.evaluate(window, cx);
        }
    });
    cx.set_global(UiScaleGlobal(entity));
}

/// v5 of the output's name: how GPUI names a Wayland display.
fn uuid_of(name: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, name.as_bytes())
}

impl UiScale {
    /// Reads the outputs in the background, then evaluates. One read at a
    /// time; a request meanwhile reads again once it lands.
    fn read(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            self.read_again = true;
            return;
        }
        self.reading = true;
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let outputs = cx.background_spawn(async move { read() }).await;
                let _ = cx.update(|window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.reading = false;
                        match outputs {
                            Ok(outputs) => {
                                this.outputs = outputs;
                                this.generation += 1;
                                this.evaluate(window, cx);
                            }
                            Err(why) => debug!("reading the outputs failed: {why}"),
                        }
                        if std::mem::take(&mut this.read_again) {
                            this.read(window, cx);
                        }
                    });
                });
            })
            .detach();
    }

    fn evaluate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scale = f64::from(window.scale_factor());
        let uuid = window.display(cx).and_then(|d| d.uuid().ok());
        let chosen =
            policy::choose_by_scale(&self.outputs, |name| uuid == Some(uuid_of(name)), scale);
        let Some(ix) = chosen else {
            // An output added since the last read isn't known yet: read again,
            // once for this scale. Until then the factor stays.
            let bits = window.scale_factor().to_bits();
            if self.reread_for != Some(bits) {
                self.reread_for = Some(bits);
                debug!("no output at scale {scale}; reading again");
                self.read(window, cx);
            }
            return;
        };
        self.reread_for = None;
        let key = (ix, window.scale_factor().to_bits(), self.generation);
        if self.evaluated == Some(key) {
            return;
        }
        self.evaluated = Some(key);
        let output = &self.outputs[ix];
        let k = policy::factor(output, scale).unwrap_or_else(|why| {
            debug!("no factor for {:?}: {why:?}", output.name);
            1.
        });
        debug!(
            "{:?} ({:?} mm, mode {:?}, logical {:?}) at scale {scale}: factor {k}",
            output.name, output.mm, output.mode, output.logical
        );
        if k != self.applied {
            self.applied = k;
            apply(k, window, cx);
        }
    }
}

/// Draws the interface at factor `k`.
fn apply(k: f64, window: &mut Window, cx: &mut App) {
    crate::design::theme::set_ui_scale(k as f32, window, cx);
}
