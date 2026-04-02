# Aethersafta Roadmap

> **Principle**: Compositing pipeline correctness first, then performance, then protocol breadth.

Completed items are in [CHANGELOG.md](../../CHANGELOG.md).

### Crate delegation

| Crate | Version | Role |
|-------|---------|------|
| [ranga](https://crates.io/crates/ranga) | 1.0.0 | Image processing, color conversion, blending, filters, GPU compute |
| [tarang](https://crates.io/crates/tarang) | 1.0.0 | Media encoding/decoding, container muxing/demuxing |
| [dhvani](https://crates.io/crates/dhvani) | 1.1.0 | Audio DSP, capture, mixing, metering |
| [ai-hwaccel](https://crates.io/crates/ai-hwaccel) | 1.0.0 | Hardware accelerator detection |
| [soorat](https://crates.io/crates/soorat) | 1.0.0 | GPU rendering engine (sprite pipeline) |
| [mabda](https://crates.io/crates/mabda) | 1.0.0 | GPU foundation layer (device, buffers) |

---

## v0.60.0 — Overlays, Transitions & Scene Switching

### Overlays
- [ ] Text overlay with font rendering (position, size, color, background)
- [ ] Image watermark with alpha channel via `ranga::composite::composite_at_argb`
- [ ] Animated overlays (fade in/out, scroll)
- [ ] Clock / timer overlay

### Transitions
- [ ] Cut (instant scene switch)
- [ ] Crossfade via `ranga::composite::dissolve` / `ranga::gpu::gpu_dissolve`
- [ ] Fade via `ranga::composite::fade` / `ranga::gpu::gpu_fade`
- [ ] Wipe via `ranga::composite::wipe` / `ranga::gpu::gpu_wipe`
- [ ] Configurable transition duration
- [ ] GPU/CPU auto-selection via `ranga::should_use_gpu()`

### Per-layer color correction
- [ ] Integrate `ranga::filter` into compositor layer pipeline
- [ ] Per-layer filter parameter API (runtime-adjustable)
- [ ] GPU filter chain via `ranga::gpu::GpuChain` — batch multiple filters without CPU readback
- [ ] ICC profile per-source for multi-camera color matching via `ranga::icc`
- [ ] Auto white balance / auto-levels as one-click presets

### Per-layer transforms
- [ ] Perspective correction per layer (4-corner mapping via `ranga::transform::Perspective`)
- [ ] GPU-accelerated resize/crop/flip via `ranga::gpu`

### Scene switching API
- [ ] Scene presets: named collections of layers + layout
- [ ] Switch between scenes with transition
- [ ] IPC command interface for external controllers (stream deck)
- [ ] `aethersafta switch --scene camera-only --transition fade --duration 500ms`

---

## v0.70.0 — Streaming Output & Ecosystem Integration

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

### Daimon integration
- [ ] Agent API handlers for remote scene control via daimon HTTP endpoints
- [ ] WebSocket event stream for real-time compositor state (layer changes, source status, metering)
- [ ] Agent-driven scene switching — daimon agents can trigger transitions, swap sources, adjust parameters
- [ ] Approval workflow for sensitive operations (stream start/stop, output target changes)

### Agnoshi integration
- [ ] MCP tools via bote: `aethersafta_record`, `aethersafta_stream`, `aethersafta_scene`, `aethersafta_source`
- [ ] Natural language scene control: "add my webcam as picture-in-picture", "fade to camera 2", "start streaming to Twitch"
- [ ] Agnoshi translators for aethersafta CLI commands

---

## v0.80.0 — Latency, Performance & GPU Pipeline

### Latency tracking
- [ ] A/V sync via `dhvani::clock` PTS alignment
- [ ] Alert when pipeline exceeds budget
- [ ] Nazar integration for monitoring dashboard

### GPU compositing pipeline
- [ ] `GpuCompositor` mode using `ranga::gpu::GpuChain` for full-frame compositing
- [ ] GPU blend all layers → GPU filter → GPU output in single dispatch chain
- [ ] CPU/GPU compositor auto-selection based on `ranga::should_use_gpu()` and layer count
- [ ] GPU noise generation via `ranga::gpu::gpu_noise_gaussian`

### Performance
- [ ] Noise suppression (RNNoise or similar — not yet in dhvani)
- [ ] Zero-copy frame path (compositor → encoder without memcpy)
- [ ] Memory pool for frame buffers (eliminate per-frame allocation)

---

## v0.90.0 — Cross-Platform & Advanced Compositing

### Cross-platform capture
- [ ] macOS: ScreenCaptureKit for screen capture (`screencapturekit` feature)
- [ ] macOS: CoreMedia/AVFoundation for camera
- [ ] Windows: DXGI Desktop Duplication for screen capture (`dxgi` feature)
- [ ] Windows: Media Foundation for camera
- [ ] Cross-platform `CaptureSource` trait abstracting platform backends
- [ ] Headless mode (no display server, for server-side compositing)
- [ ] Windows release builds in CI

### Advanced compositing
- [ ] Chroma key (green screen removal)
- [ ] Picture-in-picture layout presets
- [ ] Virtual background (ML-based segmentation via hoosh)
- [ ] Face tracking auto-zoom
- [ ] Color matching between layers — ICC profile matching, Oklab perceptual color space
- [ ] Spectral color science via `ranga::spectral` (prakash integration)

### Plugin system
- [ ] Plugin API for custom sources and effects
- [ ] Plugin sandboxing via kavach (WASM/process sandbox)

---

## v1.0.0 Criteria

All of the following must be true before cutting 1.0:

- [ ] Public API reviewed and marked stable
- [ ] `Source`, `OutputSink`, `SceneGraph` traits finalized
- [ ] Core types (`RawFrame`, `EncodedPacket`, `Layer`, `Scene`) frozen
- [ ] 90%+ line coverage
- [ ] Multi-source compositing at 1080p60 sustained without frame drops
- [ ] At least two downstream consumers running on stable aethersafta (aethersafha, streaming app)
- [ ] RTMP + file output both production-tested
- [ ] Hardware encoding working on NVIDIA + Intel + AMD
- [ ] Daimon agent API fully integrated and tested
- [ ] Agnoshi MCP tools registered and functional
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

### Ecosystem
- [ ] gRPC transport for daimon integration (alongside HTTP)
- [ ] Distributed compositor (multi-node rendering via daimon federation)
- [ ] Agent-driven automated production (scene selection, camera switching, overlay triggers)

---

## Non-goals

- **Full OBS replacement** — aethersafta is the compositing *engine*. The production UI (scene management, chat, alerts) is a separate application that consumes this crate.
- **Browser source** — embedding a browser engine for web overlays is out of scope.
- **Audio-only DAW features** — audio mixing here is for stream/recording. Full DAW is Shruti's domain.
- **Media playback** — that's Jalwa. Aethersafta can use a media file as a source, but is not a player.
- **Reimplementing crate functionality** — color conversion, blending, DSP, encoding, and decoding belong in ranga/tarang/dhvani. Aethersafta orchestrates, not reimplements.
