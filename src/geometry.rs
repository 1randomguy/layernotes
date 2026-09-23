//! Read the compositor's output layout (positions and logical sizes) so a
//! cross-monitor drag can pick the target monitor without relying on pointer
//! `enter` events (which some compositors only send once the pointer moves).
//!
//! This uses a short-lived secondary Wayland connection, independent of the
//! iced_layershell one.

use std::collections::HashMap;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle};

/// An output's rectangle in the compositor's global logical coordinate space.
#[derive(Debug, Clone, Copy)]
pub struct OutputRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl OutputRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let x0 = self.x as f32;
        let y0 = self.y as f32;
        x >= x0 && x < x0 + self.width as f32 && y >= y0 && y < y0 + self.height as f32
    }
}

#[derive(Default)]
struct Partial {
    name: Option<String>,
    position: Option<(i32, i32)>,
    mode: Option<(i32, i32)>,
    scale: i32,
}

#[derive(Default)]
struct Collector {
    outputs: HashMap<u32, Partial>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Collector {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, u32> for Collector {
    fn event(
        state: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        data: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let entry = state.outputs.entry(*data).or_default();
        match event {
            wl_output::Event::Geometry { x, y, .. } => entry.position = Some((x, y)),
            wl_output::Event::Mode { width, height, .. } => entry.mode = Some((width, height)),
            wl_output::Event::Scale { factor } => entry.scale = factor,
            wl_output::Event::Name { name } => entry.name = Some(name),
            _ => {}
        }
    }
}

/// Enumerate connected outputs and their logical layout rectangles, keyed by
/// output name. Returns an empty map if the layout cannot be read.
pub fn enumerate() -> HashMap<String, OutputRect> {
    let mut result = HashMap::new();

    let Ok(connection) = Connection::connect_to_env() else {
        return result;
    };
    let Ok((globals, mut queue)) = registry_queue_init::<Collector>(&connection) else {
        return result;
    };
    let qh = queue.handle();
    let mut collector = Collector::default();

    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            collector.outputs.entry(global.name).or_default();
            let _ = globals.registry().bind::<wl_output::WlOutput, _, _>(
                global.name,
                global.version.min(4),
                &qh,
                global.name,
            );
        }
    }

    // One roundtrip to flush the binds, one to receive the output events.
    if queue.roundtrip(&mut collector).is_err() || queue.roundtrip(&mut collector).is_err() {
        return result;
    }

    for partial in collector.outputs.into_values() {
        let (Some(name), Some((x, y)), Some((width, height))) =
            (partial.name, partial.position, partial.mode)
        else {
            continue;
        };
        let scale = partial.scale.max(1);
        result.insert(
            name,
            OutputRect {
                x,
                y,
                width: width / scale,
                height: height / scale,
            },
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_uses_half_open_bounds() {
        let rect = OutputRect {
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440,
        };
        assert!(rect.contains(1920.0, 0.0));
        assert!(rect.contains(2000.0, 100.0));
        assert!(!rect.contains(1919.0, 100.0));
        assert!(!rect.contains(4480.0, 100.0)); // right edge is exclusive
        assert!(!rect.contains(2000.0, 1440.0)); // bottom edge is exclusive
    }
}
