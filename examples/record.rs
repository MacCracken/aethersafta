//! Mini recording session: encode 60 frames to a file.
//!
//! Accepts an optional image path as the first CLI argument. If provided,
//! the image is loaded as a source layer; otherwise a synthetic solid
//! color source is used as a fallback. Prints per-second frame timing.

use std::collections::HashMap;
use std::time::Instant;

use aethersafta::{
    Compositor, EncodePipeline, EncoderConfig, FrameClock, Layer, SceneGraph,
    output::{OutputSink, file::FileOutput},
    scene::LayerContent,
    source::Source,
    source::image::ImageSource,
    source::synthetic::{Pattern, SyntheticSource},
};

fn main() -> anyhow::Result<()> {
    let (width, height, fps) = (1280, 720, 30);
    let total_frames: u64 = 60;
    let out_path = "recording.h264";

    let args: Vec<String> = std::env::args().collect();

    // -- scene ---------------------------------------------------------------
    let mut scene = SceneGraph::new(width, height, fps);

    // Background
    let mut bg = Layer::new(
        "bg",
        LayerContent::ColorFill {
            color: [20, 20, 20, 255],
        },
    );
    bg.z_index = 0;
    scene.add_layer(bg);

    // Content layer: image from CLI arg, or synthetic fallback
    let source: Box<dyn Source> = if let Some(path) = args.get(1) {
        println!("loading image: {path}");
        Box::new(ImageSource::open(path)?)
    } else {
        println!("no image path given, using synthetic source");
        Box::new(SyntheticSource::new(
            "solid",
            width,
            height,
            fps,
            Pattern::Solid([255, 100, 60, 200]),
        ))
    };

    let mut content = Layer::new(
        source.name(),
        LayerContent::Source {
            source_id: source.id(),
        },
    );
    content.z_index = 1;
    content.size = Some((width, height));
    let content_id = scene.add_layer(content);

    // -- pipeline ------------------------------------------------------------
    let compositor = Compositor::new(width, height);
    let mut encoder = EncodePipeline::new(EncoderConfig {
        bitrate_kbps: 4000,
        keyframe_interval: 30,
        ..EncoderConfig::default()
    });
    encoder.init(width, height, fps)?;

    let mut output = FileOutput::create(out_path)?;
    let mut clock = FrameClock::new(fps);
    let start = Instant::now();

    // -- record loop ---------------------------------------------------------
    for _ in 0..total_frames {
        let pts = clock.current_pts_us();

        let mut frames = HashMap::new();
        if let Some(f) = source.capture_frame()? {
            frames.insert(content_id, f);
        }

        let composited = compositor.compose(&scene, &frames, pts);
        let packet = encoder.encode_frame(&composited)?;
        output.write_packet(&packet)?;

        clock.tick();

        // Print timing every second of video time
        if clock.frame_count().is_multiple_of(fps as u64) {
            let wall = start.elapsed().as_secs_f64();
            let video = clock.frame_count() as f64 / fps as f64;
            let ratio = video / wall;
            println!(
                "  t={:.1}s  frames={}  wall={:.2}s  speed={:.1}x",
                video,
                clock.frame_count(),
                wall,
                ratio,
            );
        }
    }

    output.close()?;
    println!(
        "recorded {} frames to {out_path} ({} bytes)",
        clock.frame_count(),
        output.bytes_written(),
    );
    Ok(())
}
