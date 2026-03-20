# Aethersafta Roadmap

> **Principle**: Compositing pipeline correctness first, then performance, then protocol breadth.

Completed items are in [CHANGELOG.md](../../CHANGELOG.md).

---

## v0.20.3 — Core Compositing

Foundation: scene graph, single-source capture, file recording.

### Scene graph
- [ ] `SceneGraph` with ordered `Layer` list (z-index, position, size, opacity)
- [ ] `Layer` types: source, overlay (text/image), color fill
- [ ] Add/remove/reorder layers at runtime
- [ ] Per-layer transform: position, scale, crop, rotation

### Sources
- [ ] `Source` trait with `capture_frame()` → `RawFrame`
- [ ] Screen capture via Wayland `wlr-screencopy-unstable-v1` protocol
- [ ] Static image source (PNG/JPEG via `image` crate)
- [ ] Media file source (video playback via tarang decode)

### Compositor
- [ ] Per-frame compositing: iterate layers, alpha-blend onto output buffer
- [ ] ARGB8888 and NV12 pixel format support
- [ ] Configurable output resolution and framerate

### Recording output
- [ ] File output sink (MP4 via tarang mux)
- [ ] Software encoding fallback (tarang pure-Rust H.264/VP9)
- [ ] Frame-accurate A/V sync via PTS alignment

### CLI
- [ ] `aethersafta record --source screen --output recording.mp4`
- [ ] `aethersafta preview --source screen` (display composited output)

### Testing
- [ ] Unit tests for scene graph operations (add/remove/reorder)
- [ ] Integration tests with synthetic frame sources
- [ ] Benchmark: compositor throughput at 1080p/4K

---

## v0.6.0 — Multi-Source & Hardware Encoding

### Multi-source capture
- [ ] Concurrent capture from multiple sources (screen + camera + media)
- [ ] Per-source frame clock with independent capture rates
- [ ] Source hot-plug: add/remove sources while compositing is live

### Camera capture
- [ ] V4L2 camera source (webcam, capture cards)
- [ ] Device enumeration and capability querying
- [ ] Auto-detect resolution, framerate, pixel format

### Hardware-accelerated encoding
- [ ] ai-hwaccel integration for encoder selection
- [ ] NVENC encoding path (via tarang)
- [ ] VA-API encoding path (Intel/AMD)
- [ ] QSV encoding path (Intel Quick Sync)
- [ ] Automatic fallback: hw → sw when GPU unavailable

### Audio capture
- [ ] PipeWire audio source (system audio, mic, per-app)
- [ ] Per-source volume control
- [ ] Basic mixer: sum sources with gain

---

## v0.7.0 — Overlays, Transitions & Scene Switching

### Overlays
- [ ] Text overlay with font rendering (position, size, color, background)
- [ ] Image watermark with alpha channel
- [ ] Animated overlays (fade in/out, scroll)
- [ ] Clock / timer overlay

### Transitions
- [ ] Cut (instant scene switch)
- [ ] Crossfade (alpha blend between scenes)
- [ ] Slide (push/reveal direction)
- [ ] Configurable transition duration

### Scene switching API
- [ ] Scene presets: named collections of layers + layout
- [ ] Switch between scenes with transition
- [ ] IPC command interface for external controllers (stream deck, agnoshi)
- [ ] `aethersafta switch --scene camera-only --transition fade --duration 500ms`

---

## v0.8.0 — Streaming Output

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

## v0.9.0 — Audio DSP & Performance

### Audio DSP
- [ ] Noise gate (silence detection, configurable threshold)
- [ ] Compressor (dynamic range control)
- [ ] Equalizer (parametric EQ, per-source)
- [ ] Noise suppression (RNNoise or similar)

### Latency tracking
- [ ] Per-stage timing: capture → composite → encode → output
- [ ] `LatencyBudget` with configurable target (e.g. 33ms for 30fps)
- [ ] Alert when pipeline exceeds budget
- [ ] Nazar integration for monitoring dashboard

### Performance
- [ ] Zero-copy frame path (compositor → encoder without memcpy)
- [ ] GPU-accelerated compositing via Vulkan compute (optional)
- [ ] Memory pool for frame buffers (eliminate per-frame allocation)
- [ ] Benchmark regression CI (fail on >10% regression)

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

### Platform support
- [ ] macOS CoreMedia capture (alternative to wlr-screencopy)
- [ ] Windows Desktop Duplication API
- [ ] Headless mode (no display server, for server-side compositing)

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
