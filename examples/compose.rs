//! Scene graph compositing with latency tracking.
//!
//! Creates a 1280x720 scene with a solid blue background and a synthetic
//! gradient overlay, composes 30 frames, and reports per-frame timing
//! via LatencyBudget.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aethersafta::{
    Compositor, FrameClock, LatencyBudget, Layer, SceneGraph,
    scene::LayerContent,
    source::Source,
    source::synthetic::{Pattern, SyntheticSource},
};

fn main() -> anyhow::Result<()> {
    let (width, height, fps) = (1280, 720, 30);

    // -- scene ---------------------------------------------------------------
    let mut scene = SceneGraph::new(width, height, fps);

    // Background: solid dark blue
    let bg = Layer::new(
        "background",
        LayerContent::ColorFill {
            color: [30, 40, 80, 255],
        },
    );
    scene.add_layer(bg);

    // Overlay: synthetic gradient bound to a Source layer
    let gradient = SyntheticSource::new("gradient", width, height, fps, Pattern::Gradient);
    let src_id = gradient.id();
    let mut overlay = Layer::new("gradient", LayerContent::Source { source_id: src_id });
    overlay.opacity = 0.6;
    overlay.z_index = 1;
    let overlay_id = scene.add_layer(overlay);

    println!("{scene}");

    // -- compose loop --------------------------------------------------------
    let compositor = Compositor::new(width, height);
    let mut clock = FrameClock::new(fps);
    let mut budget = LatencyBudget::new(Duration::from_secs_f64(1.0 / fps as f64));

    for _ in 0..30 {
        let pts = clock.current_pts_us();

        // Capture
        let t0 = Instant::now();
        let mut source_frames = HashMap::new();
        if let Some(frame) = gradient.capture_frame()? {
            source_frames.insert(overlay_id, frame);
        }
        budget.capture_us = t0.elapsed().as_micros() as u64;

        // Composite
        let t1 = Instant::now();
        let _composited = compositor.compose(&scene, &source_frames, pts);
        budget.composite_us = t1.elapsed().as_micros() as u64;

        clock.tick();

        if clock.frame_count().is_multiple_of(10) {
            let ok = if budget.within_budget() { "OK" } else { "OVER" };
            println!(
                "frame {:>3}  pts={:>8}us  capture={:>5}us  composite={:>5}us  headroom={:>6}us  [{}]",
                clock.frame_count(),
                pts,
                budget.capture_us,
                budget.composite_us,
                budget.headroom_us(),
                ok,
            );
        }
    }

    println!(
        "done: {} frames, {} valid",
        clock.frame_count(),
        if !clock.is_behind() {
            "on time"
        } else {
            "behind schedule"
        },
    );
    Ok(())
}
