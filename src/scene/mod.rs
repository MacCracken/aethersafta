//! Scene graph and layer management.
//!
//! A [`SceneGraph`] holds an ordered list of [`Layer`]s. On each frame tick,
//! the compositor iterates layers bottom-to-top, alpha-blending each onto
//! the output buffer.

pub mod compositor;

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique layer identifier.
pub type LayerId = Uuid;

/// What kind of content a layer displays.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerContent {
    /// Live source (screen, camera, media file).
    Source { source_id: crate::SourceId },
    /// Static image overlay.
    Image { path: String },
    /// Text overlay.
    Text {
        text: String,
        font_size: f32,
        color: [u8; 4], // RGBA
    },
    /// Solid color fill.
    ColorFill { color: [u8; 4] },
}

/// A single compositing layer with position, size, and opacity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub content: LayerContent,
    /// Position (x, y) in output pixels.
    pub position: (i32, i32),
    /// Size (width, height) in output pixels. `None` = source native size.
    pub size: Option<(u32, u32)>,
    /// Opacity 0.0 (transparent) to 1.0 (opaque).
    pub opacity: f32,
    /// Whether this layer is visible.
    pub visible: bool,
    /// Z-index (higher = on top). Layers are sorted by this.
    pub z_index: i32,
}

impl Layer {
    /// Create a new layer with default settings.
    pub fn new(name: impl Into<String>, content: LayerContent) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            content,
            position: (0, 0),
            size: None,
            opacity: 1.0,
            visible: true,
            z_index: 0,
        }
    }

    /// Convenience: create a screen capture layer.
    pub fn screen_capture() -> Self {
        Self::new(
            "Screen",
            LayerContent::Source {
                source_id: Uuid::nil(),
            },
        )
    }
}

/// The scene graph: an ordered collection of layers composited together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneGraph {
    /// Output resolution width.
    pub width: u32,
    /// Output resolution height.
    pub height: u32,
    /// Target framerate.
    pub fps: u32,
    /// Ordered layers (sorted by z_index before compositing).
    layers: Vec<Layer>,
}

impl SceneGraph {
    /// Create a new scene with the given output resolution and framerate.
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self {
            width,
            height,
            fps,
            layers: Vec::new(),
        }
    }

    /// Add a layer to the scene.
    pub fn add_layer(&mut self, layer: Layer) -> LayerId {
        let id = layer.id;
        self.layers.push(layer);
        self.layers.sort_by_key(|l| l.z_index);
        id
    }

    /// Remove a layer by ID. Returns `true` if found.
    pub fn remove_layer(&mut self, id: LayerId) -> bool {
        let before = self.layers.len();
        self.layers.retain(|l| l.id != id);
        self.layers.len() < before
    }

    /// Get a layer by ID.
    pub fn get_layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Get a mutable layer by ID.
    pub fn get_layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// All layers in compositing order (bottom to top).
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Visible layers only, in compositing order.
    pub fn visible_layers(&self) -> Vec<&Layer> {
        self.layers.iter().filter(|l| l.visible).collect()
    }

    /// Number of layers.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }
}

impl fmt::Display for SceneGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Scene({}x{} @{}fps, {} layers)",
            self.width,
            self.height,
            self.fps,
            self.layers.len()
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_new() {
        let scene = SceneGraph::new(1920, 1080, 30);
        assert_eq!(scene.width, 1920);
        assert_eq!(scene.height, 1080);
        assert_eq!(scene.fps, 30);
        assert_eq!(scene.layer_count(), 0);
    }

    #[test]
    fn add_and_remove_layer() {
        let mut scene = SceneGraph::new(1920, 1080, 30);
        let id = scene.add_layer(Layer::screen_capture());
        assert_eq!(scene.layer_count(), 1);
        assert!(scene.get_layer(id).is_some());
        assert!(scene.remove_layer(id));
        assert_eq!(scene.layer_count(), 0);
    }

    #[test]
    fn layers_sorted_by_z_index() {
        let mut scene = SceneGraph::new(1920, 1080, 30);
        let mut top = Layer::new(
            "top",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        top.z_index = 10;
        let mut bottom = Layer::new(
            "bottom",
            LayerContent::ColorFill {
                color: [0, 0, 255, 255],
            },
        );
        bottom.z_index = 1;

        scene.add_layer(top);
        scene.add_layer(bottom);

        let layers = scene.layers();
        assert_eq!(layers[0].name, "bottom");
        assert_eq!(layers[1].name, "top");
    }

    #[test]
    fn visible_layers_filters() {
        let mut scene = SceneGraph::new(1920, 1080, 30);
        let mut hidden = Layer::screen_capture();
        hidden.visible = false;
        hidden.name = "hidden".into();
        scene.add_layer(hidden);
        scene.add_layer(Layer::screen_capture());
        assert_eq!(scene.visible_layers().len(), 1);
    }

    #[test]
    fn display_impl() {
        let scene = SceneGraph::new(1920, 1080, 60);
        assert_eq!(scene.to_string(), "Scene(1920x1080 @60fps, 0 layers)");
    }

    #[test]
    fn layer_opacity_default() {
        let layer = Layer::screen_capture();
        assert!((layer.opacity - 1.0).abs() < f32::EPSILON);
        assert!(layer.visible);
    }

    #[test]
    fn modify_layer() {
        let mut scene = SceneGraph::new(1920, 1080, 30);
        let id = scene.add_layer(Layer::screen_capture());
        let layer = scene.get_layer_mut(id).unwrap();
        layer.opacity = 0.5;
        layer.position = (100, 200);
        assert!((scene.get_layer(id).unwrap().opacity - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn serde_roundtrip() {
        let mut scene = SceneGraph::new(1920, 1080, 30);
        scene.add_layer(Layer::new(
            "overlay",
            LayerContent::Text {
                text: "Hello".into(),
                font_size: 24.0,
                color: [255, 255, 255, 255],
            },
        ));
        let json = serde_json::to_string(&scene).unwrap();
        let back: SceneGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(back.layer_count(), 1);
    }
}
