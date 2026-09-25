use iced::{
    Anchor, KeyboardInteractivity, Layer, LayerShellSettings, OutputId, SurfaceId, Task,
    destroy_layer_surface, new_layer_surface, set_layer,
};

use crate::config;

pub const NAMESPACE: &str = "layernotes";
pub const TOP_NAMESPACE: &str = "layernotes-top";

#[derive(Debug, Clone)]
pub struct OutputEntry {
    pub name: String,
    pub output_id: Option<OutputId>,
    /// Surface rendering notes on the configured (bottom) layer.
    pub surface_id: SurfaceId,
    /// Surface rendering pinned/edited notes above normal windows.
    pub top_surface: Option<SurfaceId>,
    pub logical_size: Option<(i32, i32)>,
}

/// Which of an output's two surfaces a [`SurfaceId`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Bottom,
    Top,
}

/// Tracks two fullscreen layer surfaces per monitor: one on the configured
/// layer and one raised above normal windows.
pub struct Outputs {
    pub entries: Vec<OutputEntry>,
    pub layer: config::Layer,
}

impl Outputs {
    pub fn new(layer: config::Layer) -> Self {
        Self {
            entries: vec![OutputEntry {
                name: "Fallback".to_owned(),
                output_id: None,
                surface_id: SurfaceId::MAIN,
                top_surface: None,
                logical_size: None,
            }],
            layer,
        }
    }

    pub fn get(&self, id: SurfaceId) -> Option<&OutputEntry> {
        self.entry_for_surface(id).map(|(entry, _)| entry)
    }

    /// The output (and which of its surfaces) backing `id`.
    pub fn entry_for_surface(&self, id: SurfaceId) -> Option<(&OutputEntry, SurfaceKind)> {
        for entry in &self.entries {
            if entry.surface_id == id {
                return Some((entry, SurfaceKind::Bottom));
            }
            if entry.top_surface == Some(id) {
                return Some((entry, SurfaceKind::Top));
            }
        }
        None
    }

    /// The output backing `window`, whichever of its surfaces it is.
    pub fn entry_for_window(&self, window: iced::window::Id) -> Option<&OutputEntry> {
        self.entries.iter().find(|entry| {
            iced::window::Id::from(entry.surface_id) == window
                || entry
                    .top_surface
                    .is_some_and(|top| iced::window::Id::from(top) == window)
        })
    }

    /// Create the raised surface for every output that lacks one.
    pub fn ensure_top_surfaces<M: 'static>(&mut self) -> Task<M> {
        let layer = self.layer;
        let mut tasks = Vec::new();
        for entry in &mut self.entries {
            if entry.top_surface.is_none() {
                let (surface_id, create) =
                    new_layer_surface(Self::top_settings(layer, entry.output_id));
                entry.top_surface = Some(surface_id);
                tasks.push(create);
            }
        }
        Task::batch(tasks)
    }

    pub fn has_name(&self, name: &str) -> bool {
        self.entries.iter().any(|entry| entry.name == name)
    }

    pub fn primary_name(&self) -> Option<&str> {
        self.entries.first().map(|entry| entry.name.as_str())
    }

    pub fn settings(layer: config::Layer, output: Option<OutputId>) -> LayerShellSettings {
        LayerShellSettings {
            anchor: Anchor::all(),
            layer: map_layer(layer),
            exclusive_zone: 0,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            size: Some((0, 0)),
            margin: (0, 0, 0, 0),
            namespace: NAMESPACE.to_owned(),
            output,
        }
    }

    /// Settings for the raised surface: always strictly above normal windows,
    /// and above the configured bottom layer so a pinned note never sinks below
    /// the other notes.
    fn top_settings(bottom_layer: config::Layer, output: Option<OutputId>) -> LayerShellSettings {
        LayerShellSettings {
            anchor: Anchor::all(),
            layer: raised_layer(bottom_layer),
            exclusive_zone: 0,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            size: Some((0, 0)),
            margin: (0, 0, 0, 0),
            namespace: TOP_NAMESPACE.to_owned(),
            output,
        }
    }

    /// Add (or refresh) the surface for an output. The initial fallback surface
    /// is destroyed as soon as the first real output appears.
    pub fn add<M: 'static>(
        &mut self,
        name: String,
        output_id: OutputId,
        logical_size: Option<(i32, i32)>,
    ) -> Task<M> {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.output_id == Some(output_id))
        {
            entry.name = name;
            entry.logical_size = logical_size;
            return Task::none();
        }

        let (surface_id, create) = new_layer_surface(Self::settings(self.layer, Some(output_id)));
        let (top_surface, top_create) =
            new_layer_surface(Self::top_settings(self.layer, Some(output_id)));
        let fallback = self
            .entries
            .iter()
            .position(|entry| entry.output_id.is_none());

        self.entries.push(OutputEntry {
            name,
            output_id: Some(output_id),
            surface_id,
            top_surface: Some(top_surface),
            logical_size,
        });

        match fallback {
            Some(index) => {
                let old = self.entries.remove(index);
                let mut tasks = vec![destroy_layer_surface(old.surface_id), create, top_create];
                if let Some(top) = old.top_surface {
                    tasks.push(destroy_layer_surface(top));
                }
                Task::batch(tasks)
            }
            None => Task::batch(vec![create, top_create]),
        }
    }

    pub fn remove<M: 'static>(&mut self, output_id: OutputId) -> Task<M> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.output_id == Some(output_id))
        else {
            return Task::none();
        };

        let old = self.entries.remove(index);
        let mut tasks = vec![destroy_layer_surface(old.surface_id)];
        if let Some(top) = old.top_surface {
            tasks.push(destroy_layer_surface(top));
        }

        if self.entries.is_empty() {
            let (surface_id, create) = new_layer_surface(Self::settings(self.layer, None));
            let (top_surface, top_create) = new_layer_surface(Self::top_settings(self.layer, None));
            self.entries.push(OutputEntry {
                name: "Fallback".to_owned(),
                output_id: None,
                surface_id,
                top_surface: Some(top_surface),
                logical_size: None,
            });
            tasks.push(create);
            tasks.push(top_create);
        }

        Task::batch(tasks)
    }

    pub fn set_logical_size(&mut self, output_id: OutputId, size: Option<(i32, i32)>) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.output_id == Some(output_id))
        {
            entry.logical_size = size;
        }
    }

    /// Move every bottom surface to `layer`, and the raised surfaces one step
    /// above it, without recreating anything.
    ///
    /// Recreating a surface makes some compositors (e.g. niri) hand keyboard
    /// focus to the freshly mapped `OnDemand` layer surface. Changing the layer
    /// of a mapped surface in place keeps the current focus untouched.
    pub fn apply_layer<M: 'static>(&mut self, layer: config::Layer) -> Task<M> {
        self.layer = layer;
        let bottom = map_layer(layer);
        let top = raised_layer(layer);
        let mut tasks = Vec::new();
        for entry in &self.entries {
            tasks.push(set_layer(entry.surface_id, bottom));
            if let Some(top_surface) = entry.top_surface {
                tasks.push(set_layer(top_surface, top));
            }
        }
        Task::batch(tasks)
    }

    /// Recreate every surface, e.g. after the configured layer changed.
    pub fn recreate_all<M: 'static>(&mut self, layer: config::Layer) -> Task<M> {
        self.layer = layer;
        let old = std::mem::take(&mut self.entries);
        let mut tasks = Vec::new();

        for entry in old {
            tasks.push(destroy_layer_surface(entry.surface_id));
            if let Some(top) = entry.top_surface {
                tasks.push(destroy_layer_surface(top));
            }
            let (surface_id, create) = new_layer_surface(Self::settings(layer, entry.output_id));
            let (top_surface, top_create) =
                new_layer_surface(Self::top_settings(layer, entry.output_id));
            tasks.push(create);
            tasks.push(top_create);
            self.entries.push(OutputEntry {
                surface_id,
                top_surface: Some(top_surface),
                ..entry
            });
        }

        if self.entries.is_empty() {
            let (surface_id, create) = new_layer_surface(Self::settings(layer, None));
            let (top_surface, top_create) = new_layer_surface(Self::top_settings(layer, None));
            self.entries.push(OutputEntry {
                name: "Fallback".to_owned(),
                output_id: None,
                surface_id,
                top_surface: Some(top_surface),
                logical_size: None,
            });
            tasks.push(create);
            tasks.push(top_create);
        }

        Task::batch(tasks)
    }
}

fn raised_layer(layer: config::Layer) -> Layer {
    match layer {
        config::Layer::Background => Layer::Bottom,
        config::Layer::Bottom => Layer::Top,
        config::Layer::Top | config::Layer::Overlay => Layer::Overlay,
    }
}

fn map_layer(layer: config::Layer) -> Layer {
    match layer {
        config::Layer::Background => Layer::Background,
        config::Layer::Bottom => Layer::Bottom,
        config::Layer::Top => Layer::Top,
        config::Layer::Overlay => Layer::Overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raised_layer_is_never_below_the_configured_layer() {
        assert_eq!(raised_layer(config::Layer::Background), Layer::Bottom);
        assert_eq!(raised_layer(config::Layer::Bottom), Layer::Top);
        assert_eq!(raised_layer(config::Layer::Top), Layer::Overlay);
        assert_eq!(raised_layer(config::Layer::Overlay), Layer::Overlay);
    }
}
