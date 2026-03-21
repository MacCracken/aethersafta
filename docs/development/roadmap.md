# Aethersafta Roadmap

> **Principle**: Compositing pipeline correctness first, then performance, then protocol breadth.

Completed items are in [CHANGELOG.md](../../CHANGELOG.md).

---

## v0.20.3 — Core Compositing + Hardening

Code quality, correctness, and performance passes before expanding scope.

### Project infrastructure (adopted from tarang/ai-hwaccel)

Missing documentation and CI parity with sibling projects.

- [ ] LICENSE file (AGPL-3.0-only, currently only in Cargo.toml)
- [ ] CONTRIBUTING.md — system deps, dev workflow, project layout, commit conventions, PR guidelines
- [ ] CODE_OF_CONDUCT.md (Contributor Covenant 2.1)
- [ ] SECURITY.md — threat model (scene graph injection, GPU memory, streaming auth, FFI safety), supported versions, reporting process
- [ ] CI: doc verification job (verify README, LICENSE, CHANGELOG, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, VERSION all exist)
- [ ] CI: `cargo-semver-checks` to prevent accidental API breaks
- [ ] CI: `cargo-vet` supply chain auditing with Mozilla imports
- [ ] CI: coverage threshold enforcement (85%+, fail on drop)
- [ ] CI: fuzz job on main pushes (30s per target, nightly toolchain)
- [ ] `codecov.yml` with project target 85%, patch target 80%, ignore benches/fuzz/examples
- [ ] Example binaries: `examples/compose.rs`, `examples/encode.rs`, `examples/record.rs`
- [ ] Architecture Decision Records (`docs/decisions/`) for key choices (ARGB vs NV12 internal format, SSE2 vs portable SIMD, tarang vs direct FFI)

### Code audit (3–5 rounds)

Each round: read every module, fix issues found, run full CI.

- [ ] Round 1: `unsafe` review — verify all SIMD/FFI blocks, add `// SAFETY:` comments, fuzz edge cases (zero-size frames, odd dimensions, empty scenes)
- [ ] Round 2: Error handling — replace `unwrap()`/`expect()` in non-test code with proper `Result` propagation, add error context with `anyhow::Context`
- [ ] Round 3: API surface — audit public types for consistency (naming, builder patterns, `#[must_use]`), ensure `Send`/`Sync` bounds are correct, review serde representations, add `#[serde(deny_unknown_fields)]` where appropriate
- [ ] Round 4: Bounds & overflow — check all `as` casts for truncation, verify clip rect arithmetic with `i32::MAX`/`u32::MAX` inputs, add property-based tests (proptest)
- [ ] Round 5: Dependency hygiene — remove unused deps, minimize feature flags, audit transitive `unsafe` with `cargo-geiger`, finalize `cargo-vet` supply chain

### Performance & memory optimization (3–5 rounds)

Each round: profile, optimize hotspot, benchmark before/after.

- [ ] Round 1: Allocations — profile with DHAT, eliminate per-frame `Vec` allocations in compositor (reuse output buffer), pool `RawFrame` buffers
- [ ] Round 2: Color conversion — SIMD `argb_to_yuv420p` and `argb_to_nv12` (currently scalar, ~4ms at 1080p), target <1ms
- [ ] Round 3: Compositor scaling — SIMD path for nearest-neighbour scaled blending (currently 12ms scalar at 480p→1080p), reduce gather overhead
- [ ] Round 4: Encode pipeline — avoid ARGB→YUV copy when source is already NV12, direct NV12 compositing path for single-layer capture
- [ ] Round 5: Cache & prefetch — optimize memory access patterns for L2 cache locality, benchmark with `perf stat` for cache miss rates

### Benchmarking infrastructure

- [ ] Establish v0.20.3 baselines as golden numbers in `docs/development/performance.md`
- [ ] Benchmark regression CI gate (fail on >10% regression from baseline)
- [ ] Add end-to-end pipeline benchmark: source → composite → encode → file (1080p30, 5s)
- [ ] Add multi-layer benchmark matrix: 1/3/5/10 layers × 720p/1080p/4K
- [ ] Add memory benchmark: peak RSS during 10s recording at 1080p30
- [ ] Latency percentile tracking: p50/p95/p99 per-frame times over 1000-frame runs
- [ ] HTML benchmark dashboard via criterion (auto-generated)
- [ ] Compare across feature configs: `--no-default-features` vs `--features openh264-enc` vs `--features full`

### Testing hardening

- [ ] Fuzz targets: scene graph composition, NV12/YUV conversion, ARGB frame validation (`fuzz/` crate with libfuzzer-sys)
- [ ] Property-based tests for compositor (proptest: random layers, positions, opacities, dimensions)
- [ ] Roundtrip tests: encode → decode → pixel comparison (when tarang decode available)
- [ ] Coverage target: 85%+ line coverage
- [ ] Integration tests: multi-source composition, error recovery, feature-gated paths

### Remaining from core compositing

Deferred to v0.6.0:
- [ ] Screen capture via Wayland `wlr-screencopy-unstable-v1` protocol
- [ ] Media file source (video playback via tarang decode)

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

### Per-layer color correction
- [ ] Brightness / contrast / saturation per layer (real-time, composited inline)
- [ ] Color temperature shift (warm/cool)
- [ ] SIMD color correction (apply during blend pass to avoid extra full-frame pass)
- [ ] Color LUT support (1D/3D LUT applied per layer)
- [ ] White balance auto-correct (histogram-based, same approach as tazama `auto_color_correct`)

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
- [ ] AVX2 alpha blending (4 pixels/iter, ~2× over current SSE2)
- [ ] NEON alpha blending (aarch64)
- [ ] Zero-copy frame path (compositor → encoder without memcpy)
- [ ] GPU-accelerated compositing via Vulkan compute (optional)
- [ ] Memory pool for frame buffers (eliminate per-frame allocation)

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
- [ ] Blur/sharpen filter per layer
- [ ] Vignette overlay effect

### Platform support
- [ ] macOS CoreMedia capture (alternative to wlr-screencopy)
- [ ] Windows Desktop Duplication API
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
