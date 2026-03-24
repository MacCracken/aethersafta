#![no_main]

use libfuzzer_sys::fuzz_target;

use aethersafta::scene::compositor::Compositor;
use aethersafta::scene::{Layer, LayerContent, SceneGraph};

/// Fuzz the compositor with random scene graph JSON.
///
/// Exercises: layer positions, sizes, opacities, z-indices, color fills,
/// clipping logic, and buffer management. The compositor must never panic.
fuzz_target!(|data: &[u8]| {
    // Try to parse as a scene graph
    if let Ok(scene) = serde_json::from_slice::<SceneGraph>(data) {
        // Clamp to reasonable dimensions to avoid OOM
        if scene.width == 0 || scene.height == 0 || scene.width > 512 || scene.height > 512 {
            return;
        }
        let mut comp = Compositor::new(scene.width, scene.height);
        let frames = std::collections::HashMap::new();
        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        return;
    }

    // Fallback: use raw bytes to construct random layers
    if data.len() < 8 {
        return;
    }
    let w = (data[0] as u32 % 64) + 1;
    let h = (data[1] as u32 % 64) + 1;
    let num_layers = (data[2] as usize % 8).min((data.len() - 3) / 5);

    let mut scene = SceneGraph::new(w, h, 30);
    for i in 0..num_layers {
        let base = 3 + i * 5;
        if base + 4 >= data.len() {
            break;
        }
        let color = [data[base], data[base + 1], data[base + 2], data[base + 3]];
        let mut layer = Layer::new("fuzz", LayerContent::ColorFill { color });
        layer.position = (
            (data[base + 4] as i32) - 128,
            if base + 5 < data.len() {
                (data[base + 5] as i32) - 128
            } else {
                0
            },
        );
        layer.opacity = (data[base] as f32) / 255.0;
        layer.z_index = i as i32;
        scene.add_layer(layer);
    }

    let mut comp = Compositor::new(w, h);
    let frames = std::collections::HashMap::new();
    let result = comp.compose(&scene, &frames, 0);
    assert!(result.is_valid());
});
