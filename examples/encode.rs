//! Encode pipeline: compose synthetic frames and write H.264 to a file.
//!
//! Generates 90 frames (3 seconds at 30 fps) from a checkerboard source,
//! composites them, encodes to H.264, and writes raw NAL units to disk.
//! Requires the `openh264-enc` or `vaapi` feature to be enabled.

use std::collections::HashMap;
use std::time::Instant;

use aethersafta::{
    Compositor, EncodePipeline, EncoderConfig, FrameClock, Layer, SceneGraph,
    output::{OutputSink, file::FileOutput},
    scene::LayerContent,
    source::Source,
    source::synthetic::{Pattern, SyntheticSource},
};

fn main() -> anyhow::Result<()> {
    let (width, height, fps) = (640, 480, 30);
    let total_frames: u64 = 90;
    let out_path = "output.h264";

    // -- scene ---------------------------------------------------------------
    let mut scene = SceneGraph::new(width, height, fps);
    let checker = SyntheticSource::new("checker", width, height, fps, Pattern::Checkerboard(32));
    let src_id = checker.id();
    let layer = Layer::new("checker", LayerContent::Source { source_id: src_id });
    let layer_id = scene.add_layer(layer);

    // -- pipeline ------------------------------------------------------------
    let compositor = Compositor::new(width, height);
    let config = EncoderConfig {
        bitrate_kbps: 2000,
        keyframe_interval: 30,
        ..EncoderConfig::default()
    };
    let mut encoder = EncodePipeline::new(config);
    encoder.init(width, height, fps)?;
    println!("encoder backend: {}", encoder.backend());

    let mut output = FileOutput::create(out_path)?;
    let mut clock = FrameClock::new(fps);
    let start = Instant::now();

    // -- frame loop ----------------------------------------------------------
    for _ in 0..total_frames {
        let pts = clock.current_pts_us();

        let mut frames = HashMap::new();
        if let Some(f) = checker.capture_frame()? {
            frames.insert(layer_id, f);
        }

        let composited = compositor.compose(&scene, &frames, pts);
        let packet = encoder.encode_frame(&composited)?;
        output.write_packet(&packet)?;

        clock.tick();
    }

    output.close()?;
    let elapsed = start.elapsed();

    println!(
        "wrote {} frames to {out_path} ({} packets, {} bytes)",
        clock.frame_count(),
        output.packets_written(),
        output.bytes_written(),
    );
    println!(
        "wall time: {:.2}s  ({:.1} fps)",
        elapsed.as_secs_f64(),
        clock.frame_count() as f64 / elapsed.as_secs_f64(),
    );
    Ok(())
}
