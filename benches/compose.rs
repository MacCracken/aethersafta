use criterion::{criterion_group, criterion_main, Criterion};

use aethersafta::scene::{Layer, LayerContent, SceneGraph};

fn bench_scene_add_layers(c: &mut Criterion) {
    c.bench_function("scene_add_100_layers", |b| {
        b.iter(|| {
            let mut scene = SceneGraph::new(1920, 1080, 30);
            for i in 0..100 {
                let mut layer = Layer::new(
                    format!("layer-{i}"),
                    LayerContent::ColorFill {
                        color: [255, 0, 0, 255],
                    },
                );
                layer.z_index = i;
                scene.add_layer(layer);
            }
            scene
        })
    });
}

fn bench_visible_layers(c: &mut Criterion) {
    let mut scene = SceneGraph::new(1920, 1080, 30);
    for i in 0..50 {
        let mut layer = Layer::new(
            format!("layer-{i}"),
            LayerContent::ColorFill {
                color: [0, 0, 0, 255],
            },
        );
        layer.visible = i % 3 != 0; // hide every 3rd
        layer.z_index = i;
        scene.add_layer(layer);
    }

    c.bench_function("visible_layers_50", |b| {
        b.iter(|| scene.visible_layers())
    });
}

criterion_group!(benches, bench_scene_add_layers, bench_visible_layers);
criterion_main!(benches);
