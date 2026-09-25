use iced::{
    Anchor, KeyboardInteractivity, Layer, LayerShellSettings, OutputId, SurfaceId, Task,
    destroy_layer_surface, new_layer_surface, set_layer,
};

use crate::config;

pub const NAMESPACE: &str = "layernotes";

#[derive(Debug, Clone)]
pub struct OutputEntry {
    pub name: String,
    pub output_id: Option<OutputId>,
    pub surface_id: SurfaceId,
    pub logical_size: Option<(i32, i32)>,
}

/// Tracks one fullscreen layer surface per monitor.
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
                logical_size: None,
            }],
            layer,
        }
    }

    pub fn get(&self, id: SurfaceId) -> Option<&OutputEntry> {
        self.entries.iter().find(|entry| entry.surface_id == id)
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
        let fallback = self
            .entries
            .iter()
            .position(|entry| entry.output_id.is_none());

        self.entries.push(OutputEntry {
            name,
            output_id: Some(output_id),
            surface_id,
            logical_size,
        });

        match fallback {
            Some(index) => {
                let old = self.entries.remove(index);
                Task::batch(vec![destroy_layer_surface(old.surface_id), create])
            }
            None => create,
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
        let destroy = destroy_layer_surface(old.surface_id);

        if self.entries.is_empty() {
            let (surface_id, create) = new_layer_surface(Self::settings(self.layer, None));
            self.entries.push(OutputEntry {
                name: "Fallback".to_owned(),
                output_id: None,
                surface_id,
                logical_size: None,
            });
            return Task::batch(vec![destroy, create]);
        }

        destroy
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

    /// Move every existing surface to `layer` without recreating it.
    ///
    /// Recreating a surface makes some compositors (e.g. niri) hand keyboard
    /// focus to the freshly mapped `OnDemand` layer surface. Changing the layer
    /// of a mapped surface in place keeps the current focus untouched.
    pub fn apply_layer<M: 'static>(&mut self, layer: config::Layer) -> Task<M> {
        self.layer = layer;
        let iced_layer = map_layer(layer);
        Task::batch(
            self.entries
                .iter()
                .map(|entry| set_layer(entry.surface_id, iced_layer)),
        )
    }

    /// Recreate every surface, e.g. after the configured layer changed.
    pub fn recreate_all<M: 'static>(&mut self, layer: config::Layer) -> Task<M> {
        self.layer = layer;
        let old = std::mem::take(&mut self.entries);
        let mut tasks = Vec::new();

        for entry in old {
            tasks.push(destroy_layer_surface(entry.surface_id));
            let (surface_id, create) = new_layer_surface(Self::settings(layer, entry.output_id));
            tasks.push(create);
            self.entries.push(OutputEntry {
                surface_id,
                ..entry
            });
        }

        if self.entries.is_empty() {
            let (surface_id, create) = new_layer_surface(Self::settings(layer, None));
            self.entries.push(OutputEntry {
                name: "Fallback".to_owned(),
                output_id: None,
                surface_id,
                logical_size: None,
            });
            tasks.push(create);
        }

        Task::batch(tasks)
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
