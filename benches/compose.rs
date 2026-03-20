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

fn bench_argb_to_yuv(c: &mut Criterion) {
    use aethersafta::encode::argb_to_yuv420p;

    let mut group = c.benchmark_group("argb_to_yuv420p");
    for &(w, h, label) in &[(1920, 1080, "1080p"), (3840, 2160, "4K")] {
        let argb = vec![128u8; (w * h * 4) as usize];
        group.bench_with_input(BenchmarkId::new("convert", label), &(), |b, _| {
            b.iter(|| argb_to_yuv420p(&argb, w, h))
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_scene_add_layers,
    bench_visible_layers,
    bench_composite_color_fill,
    bench_composite_source_layers,
    bench_composite_scaled,
    bench_argb_to_yuv,
);
criterion_main!(benches);
