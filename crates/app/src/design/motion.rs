//! The motion tokens (`hardcover-design`, Motion): the few places the app
//! moves, and how. Motion explains a change (a row leaving, a value moving);
//! it is never decoration, and nothing waits for it. Under reduced motion
//! every one of these shows its end state at once: GPUI's animations and
//! springs honour the desktop's setting, and `ui::leaving` checks it.

use std::time::Duration;

use gpui_kit::SpringConfig;

/// A skeleton's pulse, there and back (`ui::Skeleton`).
pub const PULSE: Duration = Duration::from_secs(2);

/// A row leaving a collection fades out over this, ease-out, then the gap
/// closes at once (`ui::leaving`).
pub const ROW_LEAVE: Duration = Duration::from_millis(150);

/// A meter's fill moving to a new value (`ui::Meter`): a critically damped
/// spring (natural frequency 26 rad/s), so it never overshoots, is 90% of
/// the way in about 150 ms and settles in about 350 ms, like a 300 ms
/// ease-out. Being a spring, a value that changes again mid-way carries on
/// from where the fill is.
pub const METER: SpringConfig = SpringConfig::new(676., 52., 1.);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_spring_is_critically_damped() {
        let (_, damping_ratio) = METER.canonical();
        assert!((damping_ratio - 1.).abs() < 1e-3, "{damping_ratio}");
    }
}
