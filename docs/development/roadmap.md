# Aethersafta Roadmap

> **Principle**: Compositing pipeline correctness first, then performance, then protocol breadth.

Completed items are in [CHANGELOG.md](../../CHANGELOG.md).

### Crate delegation

Aethersafta delegates low-level media work to sibling crates:

| Crate | Version | Role | Key modules used |
|-------|---------|------|-----------------|
| [ranga](https://crates.io/crates/ranga) | 0.21.4 | Image processing, color conversion, blending, filters | `blend`, `convert`, `filter`, `transform`, `composite`, `histogram` |
| [tarang](https://crates.io/crates/tarang) | 0.21.3 | Media encoding/decoding, container muxing/demuxing | `audio`, `video`, `demux`, `core` |
| [dhvani](https://crates.io/crates/dhvani) | 0.22.4 | Audio DSP, capture, mixing, metering, MIDI | `dsp`, `capture`, `buffer`, `clock`, `meter`, `graph` |
| [ai-hwaccel](https://crates.io/crates/ai-hwaccel) | 0.23.3 | Hardware accelerator detection, disk-cached registry | encoder selection, fallback logic |

Items handled by these crates are noted inline. Aethersafta's scope is **orchestration**: scene graph, source management, pipeline plumbing, transport protocols, and CLI/IPC.

---

## v0.23.3 — Hardening (current)

### Code audit

- [ ] Round 1: `unsafe` review — verify all SIMD/FFI blocks, add `// SAFETY:` comments, fuzz edge cases (zero-size frames, odd dimensions, empty scenes)
- [ ] Round 2: Error handling — replace remaining `expect()` in non-test code with proper `Result` propagation, add error context with `anyhow::Context`
- [ ] Round 5: Dependency hygiene — remove unused deps, minimize feature flags, audit transitive `unsafe` with `cargo-geiger`, finalize `cargo-vet` supply chain
- [ ] Architecture Decision Records (`docs/decisions/`) for key choices (ARGB vs NV12 internal format, SSE2 vs portable SIMD, tarang vs direct FFI)

### Audit fixes

- [ ] `clear_source_effect` string API → `SourceEffect` enum with `#[non_exhaustive]`
- [ ] Port 5 DSP effects (noise gate, de-esser, graphic EQ, reverb, delay) to graph pipeline `DspChainNode`
- [ ] Activate `BufferPool` in mixer `mix()` — promote `to_mix`/`refs` to struct fields, eliminate per-cycle Vec allocations
- [ ] Fix graph `source_meters` — call `.process()` on `PeakMeter` during graph execution, or remove dead metering
- [ ] Compositor per-frame buffer reuse — accept `&mut Vec<u8>` scratch or store reusable buffer on `Compositor`

### Performance & memory optimization

- [ ] Round 1: Allocations — profile with DHAT, pool `RawFrame` buffers, eliminate per-frame allocations
- [ ] Round 3: Cache & prefetch — optimize memory access patterns for L2 cache locality, benchmark with `perf stat` for cache miss rates

> **Delegated to ranga**: SIMD color conversion (`ranga::convert`), SIMD scaled blending (`ranga::blend` + `ranga::transform`), pixel format interchange.

### Benchmarking infrastructure

- [ ] Benchmark regression CI gate (fail on >10% regression from baseline)
- [ ] Add memory benchmark: peak RSS during 10s recording at 1080p30
- [ ] Latency percentile tracking: p50/p95/p99 per-frame times over 1000-frame runs
- [ ] Compare across feature configs: `--no-default-features` vs `--features openh264-enc` vs `--features full`

### Testing hardening

- [ ] Fuzz targets: scene graph composition, frame validation (`fuzz/` crate with libfuzzer-sys)
- [ ] Property-based tests for compositor (proptest: random layers, positions, opacities, dimensions)
- [ ] Roundtrip tests: encode → decode → pixel comparison (via tarang)

> **Delegated to ranga**: NV12/YUV conversion fuzzing, ARGB frame validation. **Delegated to tarang**: encode/decode roundtrip codec coverage.

---

## v0.24.0 — Multi-Source & Capture

### Multi-source capture
- [ ] Concurrent capture from multiple sources (screen + camera + media)
- [ ] Per-source frame clock with independent capture rates
- [ ] Source hot-plug: add/remove sources while compositing is live

### Camera capture
- [ ] V4L2 camera source (webcam, capture cards)
- [ ] Device enumeration and capability querying
- [ ] Auto-detect resolution, framerate, pixel format

### Screen & media capture
- [ ] Screen capture via Wayland `wlr-screencopy-unstable-v1` protocol
- [ ] Media file source (video playback via tarang decode)

> **Delegated to tarang**: Hardware-accelerated encoding (NVENC, VA-API, QSV) — aethersafta selects encoder via `ai-hwaccel` and passes frames to tarang. **Delegated to dhvani**: PipeWire capture, audio mixing, metering.

---

## v0.25.0 — Overlays, Transitions & Scene Switching

### Overlays
- [ ] Text overlay with font rendering (position, size, color, background)
- [ ] Image watermark with alpha channel
- [ ] Animated overlays (fade in/out, scroll)
- [ ] Clock / timer overlay

### Transitions
- [ ] Cut (instant scene switch)
- [ ] Crossfade via `ranga::composite`
- [ ] Slide (push/reveal direction)
- [ ] Configurable transition duration

### Per-layer color correction

> **Delegated to ranga**: All color correction is handled by `ranga::filter` (brightness, contrast, saturation, hue shift, color temperature, 3D LUT, vignette) and `ranga::histogram` (auto white balance). Aethersafta wires these into the per-layer pipeline.

- [ ] Integrate `ranga::filter` into compositor layer pipeline
- [ ] Per-layer filter parameter API (runtime-adjustable)
- [ ] Apply filters during blend pass to avoid extra full-frame pass

### Scene switching API
- [ ] Scene presets: named collections of layers + layout
- [ ] Switch between scenes with transition
- [ ] IPC command interface for external controllers (stream deck, agnoshi)
- [ ] `aethersafta switch --scene camera-only --transition fade --duration 500ms`

---

## v0.26.0 — Streaming Output

### RTMP output
- [ ] RTMP client (connect to Twitch, YouTube, custom)
- [ ] FLV muxing via tarang
- [ ] Reconnect on network failure with backoff
- [ ] Bitrate adaptation on congestion detection

### SRT output
- [ ] SRT low-latency streaming
- [ ] Caller/listener modes
- [ ] Encryption (AES-128/256)

### Multi-output
- [ ] Simultaneous recording + streaming
- [ ] Per-output encoding settings (different bitrate/resolution)

---

## v0.27.0 — Latency & Performance

### Latency tracking
- [ ] A/V sync via `dhvani::clock` PTS alignment
- [ ] Alert when pipeline exceeds budget
- [ ] Nazar integration for monitoring dashboard

### Performance
- [ ] Noise suppression (RNNoise or similar — not yet in dhvani, may need new crate or dhvani feature)
- [ ] Zero-copy frame path (compositor → encoder without memcpy)
- [ ] GPU-accelerated compositing via wgpu compute (leverage `ranga` gpu feature)
- [ ] Memory pool for frame buffers (eliminate per-frame allocation)

> **Delegated to ranga**: AVX2/NEON alpha blending (`ranga::blend` with `simd` feature).

---

## v1.0.0 Criteria

All of the following must be true before cutting 1.0:

- [ ] Public API reviewed and marked stable
- [ ] `Source`, `OutputSink`, `SceneGraph` traits finalized
- [ ] Core types (`RawFrame`, `EncodedPacket`, `Layer`, `Scene`) frozen
- [ ] 90%+ line coverage
- [ ] Multi-source compositing at 1080p60 sustained without frame drops
- [ ] At least two downstream consumers running on stable aethersafta
- [ ] RTMP + file output both production-tested
- [ ] Hardware encoding working on NVIDIA + Intel + AMD
- [ ] docs.rs documentation complete with examples for every public module
- [ ] No `unsafe` blocks without `// SAFETY:` comments
- [ ] `cargo-semver-checks` in CI
- [ ] `cargo-vet` fully audited
- [ ] All project docs present: README, LICENSE, CHANGELOG, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY

---

## Post-v1

### Protocol expansion
- [ ] WHIP output (WebRTC ingest for ultra-low-latency)
- [ ] HLS/DASH segmented output for adaptive streaming
- [ ] NDI source/output (network video)

### Advanced compositing
- [ ] Chroma key (green screen removal)
- [ ] Picture-in-picture layout presets
- [ ] Virtual background (ML-based segmentation via hoosh)
- [ ] Face tracking auto-zoom
- [ ] Color matching between layers (match camera A to camera B for multi-cam)

> **Delegated to ranga**: Blur/sharpen (`ranga::filter`), vignette (`ranga::filter`).

### Cross-platform capture & camera

- [ ] **macOS: ScreenCaptureKit for screen capture** — replace
  `wlr-screencopy-unstable-v1` with Apple's ScreenCaptureKit
  (`SCStream`, `SCContentFilter`) for display and window capture on
  macOS. Requires macOS 12.3+. Add `screencapturekit` Cargo feature.
- [ ] **macOS: CoreMedia for camera** — `AVCaptureSession` via
  CoreMedia/AVFoundation for webcam input, replacing V4L2. Device
  enumeration via `AVCaptureDevice.DiscoverySession`.
- [ ] **Windows: Desktop Duplication API for screen capture** — DXGI
  Output Duplication via `windows-rs` for GPU-accelerated screen
  capture. Supports multi-monitor and cursor overlay.
- [ ] **Windows: Media Foundation for camera** — `IMFSourceReader` via
  Media Foundation for webcam capture, replacing V4L2.
- [ ] **Cross-platform: abstract capture sources behind platform trait** —
  `CaptureSource` trait with `start()`, `next_frame()`, `stop()`,
  `enumerate()` methods. Linux impl uses wlr-screencopy + V4L2, macOS
  uses ScreenCaptureKit + CoreMedia, Windows uses DXGI + Media
  Foundation. Feature-gated: `wayland` (default), `screencapturekit`,
  `dxgi`.
- [ ] Headless mode (no display server, for server-side compositing)
- [ ] Windows release builds in CI

### Ecosystem integration
- [ ] MCP tools for agnoshi (`aethersafta_record`, `aethersafta_stream`, `aethersafta_scene`)
- [ ] Daimon API handlers for remote scene control
- [ ] Plugin system for custom sources and effects

---

## Non-goals

- **Full OBS replacement** — aethersafta is the compositing *engine*. The production UI (scene management, chat, alerts) is a separate application that consumes this crate.
- **Browser source** — embedding a browser engine for web overlays is out of scope. Use screenshot/image sources or external rendering.
- **Audio-only DAW features** — audio mixing here is for stream/recording. Full DAW is Shruti's domain.
- **Media playback** — that's Jalwa. Aethersafta can use a media file as a source, but is not a player.
- **Reimplementing crate functionality** — color conversion, blending, DSP, encoding, and decoding belong in ranga/tarang/dhvani. Aethersafta orchestrates, not reimplements.
