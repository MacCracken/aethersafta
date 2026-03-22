use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use aethersafta::scene::compositor::Compositor;
use aethersafta::scene::{Layer, LayerContent, SceneGraph};
use aethersafta::source::RawFrame;

// ---------------------------------------------------------------------------
// Scene graph benchmarks
// ---------------------------------------------------------------------------

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
        layer.visible = i % 3 != 0;
        layer.z_index = i;
        scene.add_layer(layer);
    }

    c.bench_function("visible_layers_50", |b| b.iter(|| scene.visible_layers()));
}

// ---------------------------------------------------------------------------
// Compositor benchmarks
// ---------------------------------------------------------------------------

fn make_argb_frame(width: u32, height: u32, val: u8) -> RawFrame {
    RawFrame {
        data: vec![val; (width * height * 4) as usize],
        format: aethersafta::source::PixelFormat::Argb8888,
        width,
        height,
        pts_us: 0,
    }
}

fn bench_composite_color_fill(c: &mut Criterion) {
    let mut group = c.benchmark_group("composite_color_fill");
    for &(w, h, label) in &[(1920, 1080, "1080p"), (3840, 2160, "4K")] {
        let comp = Compositor::new(w, h);
        let mut scene = SceneGraph::new(w, h, 30);
        scene.add_layer(Layer::new(
            "bg",
            LayerContent::ColorFill {
                color: [0, 0, 128, 255],
            },
        ));
        let frames = HashMap::new();

        group.bench_with_input(BenchmarkId::new("single_layer", label), &(), |b, _| {
            b.iter(|| comp.compose(&scene, &frames, 0))
        });
    }
    group.finish();
}

fn bench_composite_source_layers(c: &mut Criterion) {
    let mut group = c.benchmark_group("composite_source_layers");

    for &n_layers in &[1, 3, 5] {
        let w = 1920;
        let h = 1080;
        let comp = Compositor::new(w, h);
        let mut scene = SceneGraph::new(w, h, 30);
        let mut frames = HashMap::new();

        for i in 0..n_layers {
            let src_id = uuid::Uuid::new_v4();
            let mut layer = Layer::new(
                format!("src-{i}"),
                LayerContent::Source { source_id: src_id },
            );
            layer.z_index = i;
            layer.opacity = 0.8;
            scene.add_layer(layer);

            // Find the layer ID that was just added (last in z-order)
            let layer_id = scene
                .layers()
                .iter()
                .find(|l| l.name == format!("src-{i}"))
                .unwrap()
                .id;
            frames.insert(layer_id, make_argb_frame(w, h, 128));
        }

        group.bench_with_input(
            BenchmarkId::new("1080p", format!("{n_layers}_layers")),
            &(),
            |b, _| b.iter(|| comp.compose(&scene, &frames, 0)),
        );
    }
    group.finish();
}

fn bench_composite_scaled(c: &mut Criterion) {
    let w = 1920;
    let h = 1080;
    let comp = Compositor::new(w, h);
    let mut scene = SceneGraph::new(w, h, 30);

    // 640x480 source scaled up to fill 1920x1080
    let mut layer = Layer::new(
        "scaled",
        LayerContent::Source {
            source_id: uuid::Uuid::nil(),
        },
    );
    layer.size = Some((w, h));
    let layer_id = layer.id;
    scene.add_layer(layer);

    let mut frames = HashMap::new();
    frames.insert(layer_id, make_argb_frame(640, 480, 200));

    c.bench_function("composite_scaled_480p_to_1080p", |b| {
        b.iter(|| comp.compose(&scene, &frames, 0))
    });
}

// ---------------------------------------------------------------------------
// Color conversion benchmark
// ---------------------------------------------------------------------------

fn bench_composite_multi_scaled(c: &mut Criterion) {
    let mut group = c.benchmark_group("composite_multi_scaled");

    for &n_layers in &[2, 4] {
        let w = 1920;
        let h = 1080;
        let comp = Compositor::new(w, h);
        let mut scene = SceneGraph::new(w, h, 30);
        let mut frames = HashMap::new();

        for i in 0..n_layers {
            let mut layer = Layer::new(
                format!("scaled-{i}"),
                LayerContent::Source {
                    source_id: uuid::Uuid::new_v4(),
                },
            );
            layer.z_index = i;
            layer.opacity = 0.7;
            // Each layer is 640x480 scaled to 960x540
            layer.size = Some((960, 540));
            layer.position = (i * 100, i * 80);
            let lid = layer.id;
            scene.add_layer(layer);
            frames.insert(lid, make_argb_frame(640, 480, 128 + i as u8 * 30));
        }

        group.bench_with_input(
            BenchmarkId::new("1080p_bicubic", format!("{n_layers}_layers")),
            &(),
            |b, _| b.iter(|| comp.compose(&scene, &frames, 0)),
        );
    }
    group.finish();
}

fn bench_composite_4k(c: &mut Criterion) {
    let w = 3840;
    let h = 2160;
    let comp = Compositor::new(w, h);
    let mut scene = SceneGraph::new(w, h, 30);

    // Background fill
    let mut bg = Layer::new(
        "bg",
        LayerContent::ColorFill {
            color: [30, 30, 30, 255],
        },
    );
    bg.z_index = 0;
    scene.add_layer(bg);

    // Single 4K source layer
    let mut layer = Layer::new(
        "4k-src",
        LayerContent::Source {
            source_id: uuid::Uuid::nil(),
        },
    );
    layer.z_index = 1;
    layer.opacity = 0.9;
    let lid = layer.id;
    scene.add_layer(layer);

    let mut frames = HashMap::new();
    frames.insert(lid, make_argb_frame(w, h, 180));

    c.bench_function("composite_4k_bg_plus_source", |b| {
        b.iter(|| comp.compose(&scene, &frames, 0))
    });
}

criterion_group!(
    benches,
    bench_scene_add_layers,
    bench_visible_layers,
    bench_composite_color_fill,
    bench_composite_source_layers,
    bench_composite_scaled,
    bench_composite_multi_scaled,
    bench_composite_4k,
);
criterion_main!(benches);
