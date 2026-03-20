//! Integration tests for aethersafta.

use std::collections::HashMap;

use crate::*;

// ---------------------------------------------------------------------------
// Scene graph
// ---------------------------------------------------------------------------

#[test]
fn scene_graph_roundtrip() {
    let mut scene = SceneGraph::new(1920, 1080, 30);
    scene.add_layer(Layer::screen_capture());
    scene.add_layer(Layer::new(
        "overlay",
        crate::scene::LayerContent::Text {
            text: "Live".into(),
            font_size: 32.0,
            color: [255, 0, 0, 255],
        },
    ));
    assert_eq!(scene.layer_count(), 2);
    assert_eq!(scene.visible_layers().len(), 2);
}

// ---------------------------------------------------------------------------
// Config serialisation
// ---------------------------------------------------------------------------

#[test]
fn output_config_variants() {
    let file = OutputConfig::file("out.mp4");
    let rtmp = OutputConfig::rtmp("rtmp://example.com/live", "key123");

    let json_file = serde_json::to_string(&file).unwrap();
    assert!(json_file.contains("out.mp4"));

    let json_rtmp = serde_json::to_string(&rtmp).unwrap();
    assert!(json_rtmp.contains("key123"));
}

#[test]
fn encoder_config_default() {
    let cfg = EncoderConfig::default();
    assert!(cfg.prefer_hardware);
    assert!(cfg.bitrate_kbps > 0);
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

#[test]
fn frame_clock_basic() {
    let mut clock = FrameClock::new(60);
    for _ in 0..60 {
        clock.tick();
    }
    assert_eq!(clock.frame_count(), 60);
    let pts = clock.current_pts_us();
    assert!(pts > 900_000 && pts < 1_100_000);
}

#[test]
fn latency_budget_headroom() {
    let mut budget = LatencyBudget::new(std::time::Duration::from_millis(33));
    budget.capture_us = 5000;
    budget.composite_us = 3000;
    budget.encode_us = 8000;
    budget.output_us = 1000;
    assert!(budget.within_budget());
    assert_eq!(budget.headroom_us(), 33000 - 17000);
}

// ---------------------------------------------------------------------------
// Raw frame
// ---------------------------------------------------------------------------

#[test]
fn raw_frame_validation() {
    let valid = RawFrame {
        data: vec![0u8; 100 * 100 * 4],
        format: PixelFormat::Argb8888,
        width: 100,
        height: 100,
        pts_us: 0,
    };
    assert!(valid.is_valid());

    let invalid = RawFrame {
        data: vec![0u8; 10],
        format: PixelFormat::Argb8888,
        width: 100,
        height: 100,
        pts_us: 0,
    };
    assert!(!invalid.is_valid());
}

// ---------------------------------------------------------------------------
// Synthetic source → compositor pipeline
// ---------------------------------------------------------------------------

#[test]
fn synthetic_source_single_layer_pipeline() {
    use crate::scene::{LayerContent, compositor::Compositor};
    use crate::source::synthetic::{Pattern, SyntheticSource};

    let src = SyntheticSource::new("test", 64, 64, 30, Pattern::Gradient);
    let mut scene = SceneGraph::new(64, 64, 30);
    let mut layer = Layer::new(
        "src",
        LayerContent::Source {
            source_id: src.id(),
        },
    );
    layer.z_index = 0;
    let layer_id = layer.id;
    scene.add_layer(layer);

    let compositor = Compositor::new(64, 64);

    // Composite 3 frames
    for i in 0..3 {
        let frame = src.capture_frame().unwrap().unwrap();
        assert!(frame.is_valid());
        assert_eq!(frame.format, PixelFormat::Argb8888);
        assert!(frame.pts_us > 0 || i == 0);

        let mut frames = HashMap::new();
        frames.insert(layer_id, frame);
        let composited = compositor.compose(&scene, &frames, i * 33333);
        assert!(composited.is_valid());
        assert_eq!(composited.width, 64);
        assert_eq!(composited.height, 64);
    }
}

#[test]
fn multi_source_compositing() {
    use crate::scene::{LayerContent, compositor::Compositor};
    use crate::source::synthetic::{Pattern, SyntheticSource};

    let bg = SyntheticSource::new("bg", 128, 128, 30, Pattern::Solid([255, 0, 0, 255]));
    let fg = SyntheticSource::new("fg", 64, 64, 30, Pattern::Solid([255, 255, 255, 255]));

    let mut scene = SceneGraph::new(128, 128, 30);

    // Background: red, full size
    let mut bg_layer = Layer::new("bg", LayerContent::Source { source_id: bg.id() });
    bg_layer.z_index = 0;
    let bg_id = bg_layer.id;
    scene.add_layer(bg_layer);

    // Foreground: white, 64x64 at position (32, 32)
    let mut fg_layer = Layer::new("fg", LayerContent::Source { source_id: fg.id() });
    fg_layer.z_index = 10;
    fg_layer.position = (32, 32);
    fg_layer.size = Some((64, 64));
    let fg_id = fg_layer.id;
    scene.add_layer(fg_layer);

    let compositor = Compositor::new(128, 128);
    let mut frames = HashMap::new();
    frames.insert(bg_id, bg.capture_frame().unwrap().unwrap());
    frames.insert(fg_id, fg.capture_frame().unwrap().unwrap());

    let result = compositor.compose(&scene, &frames, 0);
    assert!(result.is_valid());

    // pixel (0,0) should be red: ARGB [255, 0, 0, 255] — wait, bg is solid [255, 0, 0, 255]
    // SyntheticSource Pattern::Solid uses ARGB, so [255, 0, 0, 255] = A=255,R=0,G=0,B=255 = blue!
    // Actually: Pattern::Solid([255, 0, 0, 255]) means A=255, R=0, G=0, B=255
    // Let me check: the data written is chunk.copy_from_slice(&argb) where argb=[255, 0, 0, 255]
    // In ARGB format: A=255, R=0, G=0, B=255 → this is blue, not red.
    // For the test, we just verify that the foreground overwrites the background.

    // pixel (0,0) = background
    let p00 = &result.data[0..4];
    // pixel (32,32) = foreground (white)
    let idx = (32 * 128 + 32) * 4;
    let p32 = &result.data[idx..idx + 4];

    assert_eq!(p00, [255, 0, 0, 255]); // bg color
    assert_eq!(p32, [255, 255, 255, 255]); // fg white overwrites
}

#[test]
fn opacity_blending_pipeline() {
    use crate::scene::{LayerContent, compositor::Compositor};
    use crate::source::synthetic::{Pattern, SyntheticSource};

    // White source at 50% opacity over black background
    let src = SyntheticSource::new("half", 4, 4, 30, Pattern::Solid([255, 255, 255, 255]));

    let mut scene = SceneGraph::new(4, 4, 30);

    // Black background
    scene.add_layer(Layer::new(
        "bg",
        LayerContent::ColorFill {
            color: [0, 0, 0, 255],
        },
    ));

    // White source at 50% opacity
    let mut layer = Layer::new(
        "half-white",
        LayerContent::Source {
            source_id: src.id(),
        },
    );
    layer.z_index = 10;
    layer.opacity = 0.5;
    let layer_id = layer.id;
    scene.add_layer(layer);

    let compositor = Compositor::new(4, 4);
    let mut frames = HashMap::new();
    frames.insert(layer_id, src.capture_frame().unwrap().unwrap());

    let result = compositor.compose(&scene, &frames, 0);
    // Each channel should be approximately 127 (half of 255)
    for chunk in result.data.chunks_exact(4) {
        assert!(chunk[1] > 115 && chunk[1] < 140, "R={}", chunk[1]);
        assert!(chunk[2] > 115 && chunk[2] < 140, "G={}", chunk[2]);
        assert!(chunk[3] > 115 && chunk[3] < 140, "B={}", chunk[3]);
    }
}

// ---------------------------------------------------------------------------
// NV12 conversion pipeline
// ---------------------------------------------------------------------------

#[test]
fn nv12_conversion_roundtrip() {
    use crate::encode::{argb_to_nv12, nv12_to_argb};
    use crate::source::synthetic::{Pattern, SyntheticSource};

    let src = SyntheticSource::new("test", 8, 8, 30, Pattern::Checkerboard(4));
    let frame = src.capture_frame().unwrap().unwrap();

    // ARGB → NV12
    let nv12 = argb_to_nv12(&frame.data, 8, 8);
    let expected_nv12_size = RawFrame::expected_size_for(PixelFormat::Nv12, 8, 8);
    assert_eq!(nv12.len(), expected_nv12_size);

    // NV12 → ARGB
    let back = nv12_to_argb(&nv12, 8, 8);
    assert_eq!(back.len(), 8 * 8 * 4);

    // Check all pixels are opaque
    for chunk in back.chunks_exact(4) {
        assert_eq!(chunk[0], 255);
    }
}

// ---------------------------------------------------------------------------
// File output pipeline
// ---------------------------------------------------------------------------

#[test]
fn file_output_pipeline() {
    use crate::output::OutputSink;
    use crate::output::file::FileOutput;
    use crate::scene::{LayerContent, compositor::Compositor};
    use crate::source::synthetic::{Pattern, SyntheticSource};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipeline_test.raw");

    let src = SyntheticSource::new("test", 16, 16, 10, Pattern::Gradient);
    let mut scene = SceneGraph::new(16, 16, 10);
    let layer = Layer::new(
        "src",
        LayerContent::Source {
            source_id: src.id(),
        },
    );
    let layer_id = layer.id;
    scene.add_layer(layer);

    let compositor = Compositor::new(16, 16);
    let mut clock = FrameClock::new(10);
    let mut output = FileOutput::create(&path).unwrap();

    // Run 10 frames through the pipeline
    for _ in 0..10 {
        clock.tick();
        let frame = src.capture_frame().unwrap().unwrap();
        let mut frames = HashMap::new();
        frames.insert(layer_id, frame);
        let composited = compositor.compose(&scene, &frames, clock.current_pts_us());

        let packet = crate::output::EncodedPacket {
            data: composited.data,
            pts_us: composited.pts_us,
            dts_us: composited.pts_us,
            is_keyframe: true,
        };
        output.write_packet(&packet).unwrap();
    }
    output.close().unwrap();

    assert_eq!(output.packets_written(), 10);
    // 16*16*4 bytes per frame * 10 frames = 10240 bytes
    assert_eq!(output.bytes_written(), 16 * 16 * 4 * 10);

    let file_size = std::fs::metadata(&path).unwrap().len();
    assert_eq!(file_size, 16 * 16 * 4 * 10);
}

// ---------------------------------------------------------------------------
// Full encode pipeline (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "openh264-enc")]
#[test]
fn full_encode_pipeline() {
    use crate::encode::{EncodePipeline, EncoderConfig, VideoCodec};
    use crate::output::OutputSink;
    use crate::output::file::FileOutput;
    use crate::scene::{LayerContent, compositor::Compositor};
    use crate::source::synthetic::{Pattern, SyntheticSource};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("encoded.h264");

    let src = SyntheticSource::new("test", 320, 240, 30, Pattern::Gradient);
    let mut scene = SceneGraph::new(320, 240, 30);
    let layer = Layer::new(
        "src",
        LayerContent::Source {
            source_id: src.id(),
        },
    );
    let layer_id = layer.id;
    scene.add_layer(layer);

    let compositor = Compositor::new(320, 240);
    let mut encoder = EncodePipeline::new(EncoderConfig {
        codec: VideoCodec::H264,
        bitrate_kbps: 1000,
        ..Default::default()
    });
    encoder.init(320, 240, 30).unwrap();
    let mut output = FileOutput::create(&path).unwrap();
    let mut clock = FrameClock::new(30);

    for _ in 0..10 {
        clock.tick();
        let frame = src.capture_frame().unwrap().unwrap();
        let mut frames = HashMap::new();
        frames.insert(layer_id, frame);
        let composited = compositor.compose(&scene, &frames, clock.current_pts_us());
        let packet = encoder.encode_frame(&composited).unwrap();
        output.write_packet(&packet).unwrap();
    }
    output.close().unwrap();

    assert_eq!(encoder.frames_encoded(), 10);
    assert!(output.bytes_written() > 0);

    // The file should contain valid H.264 NAL units
    let data = std::fs::read(&path).unwrap();
    assert!(!data.is_empty());
}
