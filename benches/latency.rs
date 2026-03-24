//! Latency percentile benchmark: measures per-frame pipeline times over 1000 frames.
//!
//! Reports p50/p95/p99 for compose and full pipeline (compose + color convert).
//! Not a criterion bench — prints results directly for CSV ingestion.

use std::collections::HashMap;
use std::time::Instant;

use aethersafta::scene::compositor::Compositor;
use aethersafta::scene::{Layer, LayerContent, SceneGraph};
use aethersafta::source::Source;
use aethersafta::source::synthetic::{Pattern, SyntheticSource};

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let idx = ((sorted.len() as f64) * p / 100.0).ceil() as usize;
    sorted[idx.saturating_sub(1).min(sorted.len() - 1)]
}

fn main() {
    let width = 1920u32;
    let height = 1080u32;
    let fps = 30u32;
    let num_frames = 1000usize;

    let src = SyntheticSource::new("bench", width, height, fps, Pattern::Gradient);
    let mut scene = SceneGraph::new(width, height, fps);

    // Background color fill
    let mut bg = Layer::new(
        "bg",
        LayerContent::ColorFill {
            color: [32, 32, 32, 255],
        },
    );
    bg.z_index = 0;
    scene.add_layer(bg);

    // Source layer
    let mut layer = Layer::new(
        "src",
        LayerContent::Source {
            source_id: src.id(),
        },
    );
    layer.z_index = 1;
    let lid = layer.id;
    scene.add_layer(layer);

    let mut compositor = Compositor::new(width, height);

    let mut compose_times = Vec::with_capacity(num_frames);
    let mut pipeline_times = Vec::with_capacity(num_frames);

    // Warmup
    for _ in 0..10 {
        let frame = src.capture_frame().unwrap().unwrap();
        let mut frames = HashMap::new();
        frames.insert(lid, frame);
        let result = compositor.compose(&scene, &frames, 0);
        compositor.reclaim_buffer(result.data);
    }

    // Measure
    for i in 0..num_frames {
        let pts = i as u64 * 33333;

        let pipeline_start = Instant::now();

        let frame = src.capture_frame().unwrap().unwrap();
        let mut frames = HashMap::new();
        frames.insert(lid, frame);

        let compose_start = Instant::now();
        let result = compositor.compose(&scene, &frames, pts);
        let compose_us = compose_start.elapsed().as_micros() as f64;

        // Simulate encode prep: color convert
        let _yuv = aethersafta::encode::argb_to_yuv420p(&result.data, width, height);

        let pipeline_us = pipeline_start.elapsed().as_micros() as f64;

        compositor.reclaim_buffer(result.data);

        compose_times.push(compose_us);
        pipeline_times.push(pipeline_us);
    }

    compose_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    pipeline_times.sort_by(|a, b| a.partial_cmp(b).unwrap());

    println!("=== Latency Percentiles ({num_frames} frames @ {width}x{height}) ===");
    println!();
    println!("Compose (µs):");
    println!(
        "  p50={:.0}  p95={:.0}  p99={:.0}  min={:.0}  max={:.0}",
        percentile(&compose_times, 50.0),
        percentile(&compose_times, 95.0),
        percentile(&compose_times, 99.0),
        compose_times[0],
        compose_times[num_frames - 1],
    );
    println!();
    println!("Full pipeline (capture + compose + yuv convert) (µs):");
    println!(
        "  p50={:.0}  p95={:.0}  p99={:.0}  min={:.0}  max={:.0}",
        percentile(&pipeline_times, 50.0),
        percentile(&pipeline_times, 95.0),
        percentile(&pipeline_times, 99.0),
        pipeline_times[0],
        pipeline_times[num_frames - 1],
    );

    let budget_us = 1_000_000.0 / fps as f64;
    let p99_pipeline = percentile(&pipeline_times, 99.0);
    println!();
    println!(
        "Frame budget: {:.0}µs  p99 pipeline: {:.0}µs  headroom: {:.0}µs",
        budget_us,
        p99_pipeline,
        budget_us - p99_pipeline
    );
}
