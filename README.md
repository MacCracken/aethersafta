# aethersafta

**Real-time media compositing engine for Rust.**

Multi-source capture, scene graph compositing, hardware-accelerated encoding, and streaming output — in a single crate. Built on [tarang](https://crates.io/crates/tarang) for encoding/muxing and [ai-hwaccel](https://crates.io/crates/ai-hwaccel) for hardware encoder selection.

> **Name**: Greek *aether* (upper sky) + Arabic *safta* (clarity/purity).
> Extracted from the [AGNOS](https://github.com/MacCracken/agnosticos) desktop compositor as a standalone, reusable engine.

[![Crates.io](https://img.shields.io/crates/v/aethersafta.svg)](https://crates.io/crates/aethersafta)
[![CI](https://github.com/MacCracken/aethersafta/actions/workflows/ci.yml/badge.svg)](https://github.com/MacCracken/aethersafta/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

---

## What it does

aethersafta is the **compositing backend** — it captures, mixes, encodes, and outputs. It is not a GUI application. Applications like OBS-style streaming tools, screen recorders, and video conferencing clients build their UI on top of this engine.

| Capability | Details |
|------------|---------|
| **Multi-source capture** | Screen (Wayland), cameras (V4L2), media files (via tarang), static images |
| **Scene graph** | Z-ordered layers with position, scale, crop, rotation, opacity |
| **Compositing** | Per-frame alpha blending, ARGB8888 and NV12 pixel formats |
| **Hardware encoding** | NVENC, VA-API, QSV, AMF — auto-selected via ai-hwaccel |
| **Software encoding** | H.264, H.265, VP9, AV1 via tarang (pure Rust + thin FFI) |
| **File recording** | MP4, MKV, WebM containers via tarang muxers |
| **RTMP streaming** | Twitch, YouTube, custom RTMP servers |
| **SRT streaming** | Low-latency encrypted streaming |
| **Audio capture** | PipeWire per-source capture with mixing, gain, DSP |
| **Latency tracking** | Per-stage timing with configurable budget and alerting |

---

## Architecture

```
Sources (screen, camera, media, image)
    │
    ▼
Scene Graph (layers with z-order, transforms, opacity)
    │
    ▼
Compositor (alpha blend, crop, scale → composited frame)
    │
    ▼
Encode Pipeline (ai-hwaccel selects encoder → tarang encodes)
    │
    ▼
Output Sinks (file, RTMP, SRT)
```

Audio runs as a parallel pipeline (PipeWire capture → mixer → DSP → tarang audio encode) and is muxed with video at the output stage via PTS synchronisation.

See [docs/architecture/overview.md](docs/architecture/overview.md) for the full architecture document with module structure and key types.

---

## Quick start

### Library usage

```toml
[dependencies]
aethersafta = "0.20.4"
```

```rust
use aethersafta::{SceneGraph, Layer, OutputConfig, EncoderConfig};

// Build a scene
let mut scene = SceneGraph::new(1920, 1080, 30);
scene.add_layer(Layer::screen_capture());

// Configure encoding
let encoder = EncoderConfig {
    bitrate_kbps: 6000,
    prefer_hardware: true,
    ..Default::default()
};

// Choose output
let output = OutputConfig::file("recording.mp4");
// let output = OutputConfig::rtmp("rtmp://live.twitch.tv/app", "your_stream_key");
```

### CLI usage

```bash
# System info: available sources, encoders, hardware
aethersafta info

# Record screen to file
aethersafta record --source screen --output recording.mp4 --fps 30

# Preview composited output (no recording)
aethersafta preview --source screen
```

---

## Features

Each capability can be individually enabled or disabled via Cargo features:

| Feature | Backend | Default |
|---------|---------|---------|
| `pipewire` | PipeWire audio capture | yes |
| `hwaccel` | Hardware encoder selection via ai-hwaccel | yes |
| `rtmp` | RTMP streaming output | no |
| `srt` | SRT streaming output | no |
| `full` | All of the above | no |

To include only specific features:

```toml
[dependencies]
aethersafta = { version = "0.20", default-features = false, features = ["hwaccel"] }
```

---

## Key types

### `SceneGraph`

Central data structure. Owns an ordered list of `Layer`s. The compositor reads the scene graph on every frame tick and produces a composited frame buffer.

```rust
use aethersafta::{SceneGraph, Layer};
use aethersafta::scene::LayerContent;

let mut scene = SceneGraph::new(1920, 1080, 60);

// Add a screen capture as the base layer
let mut base = Layer::screen_capture();
base.z_index = 0;
scene.add_layer(base);

// Add a text overlay on top
let mut overlay = Layer::new("Live Indicator", LayerContent::Text {
    text: "LIVE".into(),
    font_size: 24.0,
    color: [255, 0, 0, 255],
});
overlay.z_index = 10;
overlay.position = (50, 50);
scene.add_layer(overlay);

println!("{}", scene); // Scene(1920x1080 @60fps, 2 layers)
```

### `Source` (trait)

Anything that can produce frames:

```rust
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;
    fn name(&self) -> &str;
    fn capture_frame(&self) -> Result<Option<RawFrame>>;
    fn resolution(&self) -> (u32, u32);
    fn is_live(&self) -> bool;
}
```

Built-in sources: `ScreenSource` (Wayland), `CameraSource` (V4L2), `MediaSource` (tarang decode), `ImageSource` (static).

### `OutputSink` (trait)

Anything that can consume encoded packets:

```rust
pub trait OutputSink: Send + Sync {
    fn write_packet(&mut self, packet: &EncodedPacket) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}
```

Built-in sinks: `FileOutput` (MP4/MKV/WebM), `RtmpOutput`, `SrtOutput`.

### `FrameClock` and `LatencyBudget`

Frame-accurate timing and per-stage latency tracking:

```rust
use aethersafta::timing::{FrameClock, LatencyBudget};
use std::time::Duration;

let mut clock = FrameClock::new(30); // 30 fps
clock.tick();
println!("PTS: {} µs", clock.current_pts_us());

let mut budget = LatencyBudget::new(Duration::from_millis(33)); // 30fps budget
budget.capture_us = 5000;
budget.composite_us = 3000;
budget.encode_us = 10000;
budget.output_us = 2000;
assert!(budget.within_budget()); // 20ms < 33ms target
println!("Headroom: {} µs", budget.headroom_us());
```

---

## Dependencies

| Crate | Role |
|-------|------|
| [tarang](https://crates.io/crates/tarang) | Video/audio encoding, container muxing (MP4/MKV/WebM). 18-33x faster than GStreamer for video operations |
| [ai-hwaccel](https://crates.io/crates/ai-hwaccel) | Hardware encoder detection across 13 accelerator families (NVENC, VA-API, QSV, AMF, and more) |
| [pipewire](https://crates.io/crates/pipewire) | Audio capture from system audio graph |
| [tokio](https://crates.io/crates/tokio) | Async runtime for concurrent capture + encode + output |

---

## Who uses this

| Project | Usage |
|---------|-------|
| **[AGNOS](https://github.com/MacCracken/agnosticos)** (aethersafha compositor) | Built-in screen recording and capture |
| **Streaming app** (planned) | OBS-like live broadcast production |
| **[Tazama](https://github.com/MacCracken/tazama)** | Real-time preview during video editing |
| **[SecureYeoman](https://github.com/MacCracken/SecureYeoman)** | Sandbox session recording with security overlay annotations |
| **[Selah](https://github.com/MacCracken/selah)** | Screenshot with overlay annotations |

---

## Roadmap

| Version | Milestone | Key features |
|---------|-----------|--------------|
| ~~0.20.3~~ | ~~Core compositing~~ | ~~Scene graph, compositor, H.264 encode, SSE2 SIMD, NV12, CLI~~ |
| **0.20.4** | Hardening & optimization | 3–5 rounds code audit + 3–5 rounds perf/memory optimization |
| **0.6.0** | Multi-source & HW encode | Concurrent capture, V4L2 cameras, NVENC/VA-API/QSV via ai-hwaccel |
| **0.7.0** | Overlays & scene switching | Text/image overlays, transitions (cut/fade/slide), scene presets, IPC |
| **0.8.0** | Streaming output | RTMP (Twitch/YouTube), SRT low-latency, multi-output |
| **0.9.0** | Audio DSP & performance | Noise gate, compressor, EQ, zero-copy frames, GPU compositing |
| **1.0.0** | Stable API | Frozen traits, 90%+ coverage, production-tested, semver-checks in CI |

Full details: [docs/development/roadmap.md](docs/development/roadmap.md)

---

## Building from source

```bash
# Clone
git clone https://github.com/MacCracken/aethersafta.git
cd aethersafta

# Build (without PipeWire — no system deps needed)
cargo build --no-default-features

# Build with all features (requires libpipewire-dev)
sudo apt install libpipewire-0.3-dev  # Debian/Ubuntu
cargo build --features full

# Run tests
cargo test

# Run all CI checks locally
make check
```

### System dependencies

| Feature | System package | Platform |
|---------|---------------|----------|
| `pipewire` | `libpipewire-0.3-dev` | Linux |
| `hwaccel` | None (pure Rust detection) | All |
| `rtmp` / `srt` | None (pure Rust networking) | All |

---

## Versioning

Pre-1.0 releases use `0.D.M` (day.month) SemVer — e.g. `0.20.3` = March 20th.
Post-1.0 follows standard SemVer (`MAJOR.MINOR.PATCH`).

The `VERSION` file is the single source of truth. Use `./scripts/version-bump.sh <version>` to update all references.

---

## License

AGPL-3.0-only. See [LICENSE](LICENSE) for details.

---

## Contributing

1. Fork and create a feature branch
2. Run `make check` (fmt + clippy + test + audit)
3. Open a PR against `main`

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide.
