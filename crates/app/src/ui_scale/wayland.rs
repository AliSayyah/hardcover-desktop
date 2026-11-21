//! Every output's name, size in mm, current mode and logical size, over a
//! Wayland connection of our own. GPUI keeps its outputs to itself and drops
//! the millimetres. Two roundtrips, about 1 ms; blocking, so it runs off the
//! UI thread.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

use super::policy::Output;

#[derive(Default)]
struct State {
    outputs: Vec<Output>,
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
        let output = &mut state.outputs[*index];
        match event {
            wl_output::Event::Geometry {
                physical_width,
                physical_height,
                ..
            } => output.mm = (physical_width, physical_height),
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => output.mode = (width, height),
            wl_output::Event::Name { name } => output.name = Some(name),
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
        let output = &mut state.outputs[*index];
        match event {
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.logical = Some((width, height));
            }
            // `wl_output.name` is version 4; older compositors name the
            // output here.
            zxdg_output_v1::Event::Name { name } if output.name.is_none() => {
                output.name = Some(name);
            }
            _ => {}
        }
    }
}

/// The compositor's socket, from `WAYLAND_DISPLAY`. Never `WAYLAND_SOCKET`:
/// that is one inherited connection, and GPUI's.
fn socket_path() -> Result<PathBuf, String> {
    let display = std::env::var_os("WAYLAND_DISPLAY").ok_or("no WAYLAND_DISPLAY")?;
    let display = PathBuf::from(display);
    if display.is_absolute() {
        return Ok(display);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("no XDG_RUNTIME_DIR")?;
    Ok(PathBuf::from(runtime).join(display))
}

pub fn read_outputs() -> Result<Vec<Output>, String> {
    // A compositor that stops answering holds this thread: `roundtrip` waits
    // in `poll` with no timeout. `main` waits for the first read only briefly,
    // and the UI never waits for one.
    let stream = UnixStream::connect(socket_path()?).map_err(|e| e.to_string())?;
    let connection = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&connection).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let mut state = State::default();
    let mut outputs = Vec::new();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            let index = state.outputs.len();
            state.outputs.push(Output::default());
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
    // The first roundtrip delivers the outputs' events, the second the
    // xdg outputs'.
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    Ok(state.outputs)
}
