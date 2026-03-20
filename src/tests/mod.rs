//! Integration tests for aethersafta.

use crate::*;

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

#[test]
fn frame_clock_basic() {
    let mut clock = FrameClock::new(60);
    for _ in 0..60 {
        clock.tick();
    }
    assert_eq!(clock.frame_count(), 60);
    // 60 frames at 60fps = ~1 second of PTS
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

#[test]
fn raw_frame_validation() {
    use crate::source::RawFrame;

    let valid = RawFrame {
        data: vec![0u8; 100 * 100 * 4],
        width: 100,
        height: 100,
        pts_us: 0,
    };
    assert!(valid.is_valid());

    let invalid = RawFrame {
        data: vec![0u8; 10],
        width: 100,
        height: 100,
        pts_us: 0,
    };
    assert!(!invalid.is_valid());
}
