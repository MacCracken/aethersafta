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
        data: vec![0u8; 100 * 100 * 4].into(),
        format: PixelFormat::Argb8888,
        width: 100,
        height: 100,
        pts_us: 0,
    };
    assert!(valid.is_valid());

    let invalid = RawFrame {
        data: vec![0u8; 10].into(),
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

    let mut compositor = Compositor::new(64, 64);

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

    let mut compositor = Compositor::new(128, 128);
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

    let mut compositor = Compositor::new(4, 4);
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

    let mut compositor = Compositor::new(16, 16);
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
            data: composited.data.to_vec(),
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

    let mut compositor = Compositor::new(320, 240);
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

// ---------------------------------------------------------------------------
// BT.709 YUV conversion
// ---------------------------------------------------------------------------

#[test]
fn bt709_yuv_white() {
    // Pure white ARGB: A=255, R=255, G=255, B=255
    let w = 8u32;
    let h = 8u32;
    let argb: Vec<u8> = vec![255; (w * h * 4) as usize];
    let yuv = crate::encode::argb_to_yuv420p(&argb, w, h);

    let y_plane = &yuv[..(w * h) as usize];
    let u_plane = &yuv[(w * h) as usize..(w * h + w * h / 4) as usize];
    let v_plane = &yuv[(w * h + w * h / 4) as usize..];

    for &y in y_plane {
        assert!(y > 250, "Y={y}, expected near 255 (full-range white)");
    }
    for &u in u_plane {
        assert!((u as i32 - 128).abs() <= 5, "U={u}, expected near 128");
    }
    for &v in v_plane {
        assert!((v as i32 - 128).abs() <= 5, "V={v}, expected near 128");
    }
}

#[test]
fn bt709_yuv_black() {
    let w = 8u32;
    let h = 8u32;
    // Black ARGB: A=255, R=0, G=0, B=0
    let mut argb = vec![0u8; (w * h * 4) as usize];
    for chunk in argb.chunks_exact_mut(4) {
        chunk[0] = 255; // A
    }
    let yuv = crate::encode::argb_to_yuv420p(&argb, w, h);

    let y_plane = &yuv[..(w * h) as usize];
    let u_plane = &yuv[(w * h) as usize..(w * h + w * h / 4) as usize];
    let v_plane = &yuv[(w * h + w * h / 4) as usize..];

    for &y in y_plane {
        assert!(y < 5, "Y={y}, expected near 0 (full-range black)");
    }
    for &u in u_plane {
        assert!((u as i32 - 128).abs() <= 5, "U={u}, expected near 128");
    }
    for &v in v_plane {
        assert!((v as i32 - 128).abs() <= 5, "V={v}, expected near 128");
    }
}

#[test]
fn bt709_yuv_red() {
    let w = 8u32;
    let h = 8u32;
    // Red ARGB: A=255, R=255, G=0, B=0
    let mut argb = vec![0u8; (w * h * 4) as usize];
    for chunk in argb.chunks_exact_mut(4) {
        chunk[0] = 255; // A
        chunk[1] = 255; // R
    }
    let yuv = crate::encode::argb_to_yuv420p(&argb, w, h);

    let y_plane = &yuv[..(w * h) as usize];
    let u_plane = &yuv[(w * h) as usize..(w * h + w * h / 4) as usize];
    let v_plane = &yuv[(w * h + w * h / 4) as usize..];

    for &y in y_plane {
        assert!((y as i32 - 63).abs() <= 15, "Y={y}, expected around 63");
    }
    for &u in u_plane {
        assert!((u as i32) < 100, "U={u}, expected below 100");
    }
    for &v in v_plane {
        assert!((v as i32) > 200, "V={v}, expected above 200");
    }
}

// ---------------------------------------------------------------------------
// Timing (additional)
// ---------------------------------------------------------------------------

#[test]
fn latency_budget_over_budget() {
    let mut budget = LatencyBudget::new(std::time::Duration::from_millis(33));
    budget.capture_us = 12000;
    budget.composite_us = 10000;
    budget.encode_us = 12000;
    budget.output_us = 6000; // total = 40000us > 33000us
    assert!(!budget.within_budget());
    assert!(
        budget.headroom_us() < 0,
        "headroom={}",
        budget.headroom_us()
    );
}

#[test]
fn frame_clock_high_fps() {
    let mut clock = FrameClock::new(120);
    for _ in 0..240 {
        clock.tick();
    }
    assert_eq!(clock.frame_count(), 240);
    let pts = clock.current_pts_us();
    // 240 frames at 120fps = 2 seconds = 2_000_000us
    assert!(
        pts > 1_900_000 && pts < 2_100_000,
        "pts={pts}, expected near 2_000_000"
    );
}

// ---------------------------------------------------------------------------
// Scene graph (additional)
// ---------------------------------------------------------------------------

#[test]
fn scene_graph_empty_compose() {
    use crate::scene::compositor::Compositor;

    let scene = SceneGraph::new(16, 16, 30);
    let frames = HashMap::new();
    let mut compositor = Compositor::new(16, 16);
    let result = compositor.compose(&scene, &frames, 0);

    // All pixels should be zero (no layers = blank output)
    assert!(
        result.data.iter().all(|&b| b == 0),
        "expected all zeros for empty scene"
    );
}

// ---------------------------------------------------------------------------
// Audio mixer integration
// ---------------------------------------------------------------------------

#[test]
fn audio_mixer_integration() {
    let config = AudioMixerConfig {
        sample_rate: 48000,
        channels: 2,
        master_gain_db: 0.0,
        master_limiter: true,
    };
    let mut mixer = AudioMixer::new(config);

    let src1_cfg = AudioSourceConfig::new("Source 1");
    let src2_cfg = AudioSourceConfig::new("Source 2");
    let id1 = mixer.add_source(src1_cfg);
    let id2 = mixer.add_source(src2_cfg);

    let frames = 1024usize;
    let channels = 2u32;
    let sample_rate = 48000u32;
    let buf1 = dhvani::buffer::AudioBuffer::from_interleaved(
        vec![0.3f32; frames * channels as usize],
        channels,
        sample_rate,
    )
    .unwrap();
    let buf2 = dhvani::buffer::AudioBuffer::from_interleaved(
        vec![0.2f32; frames * channels as usize],
        channels,
        sample_rate,
    )
    .unwrap();

    let mut source_buffers = HashMap::new();
    source_buffers.insert(id1, buf1);
    source_buffers.insert(id2, buf2);

    let output = mixer.mix(&mut source_buffers);
    assert!(
        output.is_some(),
        "mix should produce output with active sources"
    );

    let master_peak = mixer.master_peak_db(0);
    assert!(
        master_peak > -60.0,
        "peak_db={master_peak}, expected reasonable level"
    );
}

// ---------------------------------------------------------------------------
// Color fill gradient layers
// ---------------------------------------------------------------------------

#[test]
fn color_fill_gradient_layers() {
    use crate::scene::{LayerContent, compositor::Compositor};

    let mut scene = SceneGraph::new(4, 4, 30);

    // Red background (RGBA: R=255, G=0, B=0, A=255)
    let mut bg = Layer::new(
        "red-bg",
        LayerContent::ColorFill {
            color: [255, 0, 0, 255],
        },
    );
    bg.z_index = 0;
    scene.add_layer(bg);

    // Green overlay at 50% opacity (RGBA: R=0, G=255, B=0, A=255)
    let mut fg = Layer::new(
        "green-overlay",
        LayerContent::ColorFill {
            color: [0, 255, 0, 255],
        },
    );
    fg.z_index = 10;
    fg.opacity = 0.5;
    scene.add_layer(fg);

    let mut compositor = Compositor::new(4, 4);
    let frames = HashMap::new();
    let result = compositor.compose(&scene, &frames, 0);

    // Blended: red + 50% green => approximately (127, 127, 0) for RGB
    for chunk in result.data.chunks_exact(4) {
        // chunk is ARGB: [A, R, G, B]
        let r = chunk[1];
        let g = chunk[2];
        let b = chunk[3];
        assert!(r > 110 && r < 145, "R={r}, expected near 127");
        assert!(g > 110 && g < 145, "G={g}, expected near 127");
        assert!(b < 15, "B={b}, expected near 0");
    }
}

// ---------------------------------------------------------------------------
// Serialisation roundtrips (additional)
// ---------------------------------------------------------------------------

#[test]
fn source_config_serialization_roundtrip() {
    let cfg = AudioSourceConfig {
        name: "Desktop Audio".into(),
        device_id: Some(7),
        gain_db: -6.0,
        muted: true,
        pan: -0.5,
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let back: AudioSourceConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "Desktop Audio");
    assert_eq!(back.device_id, Some(7));
    assert!((back.gain_db - (-6.0)).abs() < f32::EPSILON);
    assert!(back.muted);
    assert!((back.pan - (-0.5)).abs() < f32::EPSILON);
}

#[test]
fn memory_stability_buffer_reclaim() {
    use crate::scene::{LayerContent, compositor::Compositor};
    use crate::source::synthetic::{Pattern, SyntheticSource};

    let src = SyntheticSource::new("mem", 1920, 1080, 30, Pattern::Solid([255, 128, 64, 255]));
    let mut scene = SceneGraph::new(1920, 1080, 30);
    let layer = Layer::new(
        "src",
        LayerContent::Source {
            source_id: src.id(),
        },
    );
    let lid = layer.id;
    scene.add_layer(layer);

    let mut compositor = Compositor::new(1920, 1080);

    // Run 300 frames (10s at 30fps) with buffer reclaim
    for i in 0..300 {
        let frame = src.capture_frame().unwrap().unwrap();
        let mut frames = HashMap::new();
        frames.insert(lid, frame);
        let composited = compositor.compose(&scene, &frames, i * 33333);
        assert!(composited.is_valid());
        compositor.reclaim_buffer(composited.data);
    }

    // The compositor should have a reusable scratch buffer (not growing)
    // Verify it still works after many cycles
    let frame = src.capture_frame().unwrap().unwrap();
    let mut frames = HashMap::new();
    frames.insert(lid, frame);
    let final_frame = compositor.compose(&scene, &frames, 300 * 33333);
    assert!(final_frame.is_valid());
    assert_eq!(final_frame.width, 1920);
    assert_eq!(final_frame.height, 1080);
}

#[test]
fn mixer_config_custom() {
    let cfg = AudioMixerConfig {
        sample_rate: 44100,
        channels: 1,
        master_gain_db: -3.0,
        master_limiter: false,
    };
    assert_eq!(cfg.sample_rate, 44100);
    assert_eq!(cfg.channels, 1);
    assert!((cfg.master_gain_db - (-3.0)).abs() < f32::EPSILON);
    assert!(!cfg.master_limiter);
}
