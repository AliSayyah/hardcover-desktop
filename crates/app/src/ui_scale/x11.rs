//! Every RandR monitor's name, size in mm and place, over an X connection
//! of our own. Blocking, so it runs off the UI thread.

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;

use super::Facts;
use super::policy::Output;

/// The monitors, and the centre of `window` (GPUI's X window) in device
/// pixels of the root window. GPUI's own bounds can't be used for the
/// place: a reparenting window manager reports them inside its frame.
pub fn read(window: Option<u32>) -> Result<Facts, String> {
    let (connection, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or("no X screen")?
        .root;
    let monitors = connection
        .randr_get_monitors(root, true)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .monitors;
    let centre = window.and_then(|window| {
        let geometry = connection.get_geometry(window).ok()?.reply().ok()?;
        let at = connection
            .translate_coordinates(
                window,
                root,
                i16::try_from(geometry.width / 2).ok()?,
                i16::try_from(geometry.height / 2).ok()?,
            )
            .ok()?
            .reply()
            .ok()?;
        Some((f64::from(at.dst_x), f64::from(at.dst_y)))
    });
    let outputs = monitors
        .into_iter()
        .map(|m| Output {
            name: connection
                .get_atom_name(m.name)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|reply| String::from_utf8_lossy(&reply.name).into_owned()),
            mm: (
                i32::try_from(m.width_in_millimeters).unwrap_or(0),
                i32::try_from(m.height_in_millimeters).unwrap_or(0),
            ),
            mode: (i32::from(m.width), i32::from(m.height)),
            logical: None,
            origin: Some((i32::from(m.x), i32::from(m.y))),
        })
        .collect();
    Ok(Facts { outputs, centre })
}
