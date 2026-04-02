# Changelog

All notable changes to aethersafta are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Pre-1.0 versioning uses `0.D.M` (day.month) SemVer.

---

## [Unreleased]

Multi-source capture, dependency upgrade to AGNOS 1.0 ecosystem, performance hardening, and three new capture source types.

### Breaking (downstream note)

- **`build_scene()` returns `VideoCaptureManager`** — the CLI function now returns `(SceneGraph, VideoCaptureManager)` instead of `(SceneGraph, Option<Box<dyn Source>>)`. `capture_source_frames()` replaced by `VideoCaptureManager::capture_all()` + `collect_layer_frames()`.
- **`--source` CLI arg now accepts multiple values** — `--source screen --source camera:/dev/video0` layers sources bottom-to-top by order.

### Added

- **`VideoCaptureManager`** (`source::manager`) — multi-source orchestrator with per-source frame clocks, `SourceEvent` hot-plug notifications, and `collect_layer_frames()` compositor bridge
- **V4L2 camera source** (`source::camera`, `camera` feature) — `CameraSource::open()` with auto-detect format negotiation (MJPEG > YUYV > NV12), `enumerate_cameras()` device discovery, YUYV→ARGB BT.601 and MJPEG→ARGB conversion
- **Wayland screen capture** (`source::screen`, `wayland` feature) — `ScreenSource::open()` via `wlr-screencopy-unstable-v1` protocol with persistent shm buffer reuse, `enumerate_screens()` output discovery, XRGB→ARGB conversion with stride handling
- **Media file source** (`source::media`, `openh264-dec` feature) — `MediaFileSource::open()` with tarang MP4 demuxer + OpenH264 decoder, loop playback, YUV420p→ARGB BT.709 conversion
- **Encode buffer reuse** — `argb_to_yuv420p_into()`, `argb_to_nv12_into()` zero-alloc `_into` variants; `EncodePipeline` reuses YUV scratch buffer across frames via `Bytes::try_into_mut()` reclaim
- **Compositor scaling scratch** — `Compositor` reuses a `scale_scratch` buffer for the resize path, eliminating per-frame `.to_vec()` allocation
- **GPU compositor readback buffer** — `GpuCompositor` reuses `argb_scratch` for RGBA→ARGB conversion; `reclaim_buffer()` for frame data recovery
- **Encode→decode roundtrip test** — H.264 encode via OpenH264, decode back, pixel comparison with lossy tolerance
- **Buffer reuse benchmarks** — `_into` vs allocating variants at 1080p in `benches/convert.rs`
- **RSS stability test** — 600 frames at 1080p30 with leak detection via `/proc/self/status` VmRSS
- **Multi-source E2E test** — two synthetic sources at different FPS composited over 60 frames
- **Benchmark regression CI gate** — `bench-regression` job in CI compares PR vs base via `critcmp`, fails on >10% regression
- **`openh264-dec` feature** — enables tarang H.264 decoding for media file source and roundtrip tests
- **`camera` feature** — gates `v4l` dependency for V4L2 camera capture
- **`wayland` feature** — gates `wayland-client` + `wayland-protocols-wlr` + `libc` for screen capture
- **Camera/screen enumeration in `aethersafta info`** — lists available V4L2 devices and Wayland outputs

### Changed

- **AGNOS crates upgraded to 1.0** — ranga 0.24.3→1.0.0, tarang 0.21.3→1.0.0, ai-hwaccel 0.23.3→1.0.0, dhvani 0.22.4→1.1.0, soorat 0.24.3→1.0.0
- **wgpu 24→29** — via mabda 1.0.0 (soorat's GPU foundation layer)
- **Non-exhaustive API migration** — all dhvani DSP params use builders/`Default` + mutate (e.g. `CompressorParams::new().with_threshold(-20.0)`), ranga `PixelBuffer.data` → `.data()`/`.into_data()`, dhvani `LevelMeter.lufs` → `.lufs()`
- **`deny.toml`** — added `GPL-3.0-only` license (AGNOS 1.0 relicensed), removed stale `encoding_rs` clarify
- **Compositor blend simplification** — removed redundant `all_opaque` row scan; ranga SIMD handles per-pixel fast paths internally
- **Latency bench uses buffer reuse** — `benches/latency.rs` uses `argb_to_yuv420p_into` matching real pipeline

### Fixed

- **Input bounds guards** — `argb_to_yuv420p_into`, `argb_to_nv12_into`, `nv12_to_argb` early-return on undersized input instead of panic
- **GPU compositor unwrap** — `texture_cache.get().unwrap()` replaced with defensive `let Some(...) else { continue }`
- **`double_must_use` clippy lint** — `acquire_buffer()` `#[must_use]` given descriptive message for Rust 1.93 compatibility

### Dependencies

| Crate | Old | New | Notes |
|-------|-----|-----|-------|
| ranga | 0.24.3 | 1.0.0 | 1.0 stable, `PixelBuffer` fields now private |
| tarang | 0.21.3 | 1.0.0 | 1.0 stable, `VideoFrame::new()` constructor |
| dhvani | 0.22.4 | 1.1.0 | 1.0 stable, builder APIs for DSP params |
| ai-hwaccel | 0.23.3 | 1.0.0 | 1.0 stable |
| soorat | 0.24.3 | 1.0.0 | 1.0 stable, uses mabda for GPU |
| wgpu | 24 | 29 | via mabda 1.0.0 |
| v4l | — | 0.14 | new optional dep (camera feature) |
| wayland-client | — | 0.31 | new optional dep (wayland feature) |
| wayland-protocols-wlr | — | 0.3 | new optional dep (wayland feature) |
| libc | — | 0.2 | new optional dep (wayland feature, for memfd/mmap) |

### Performance

| Metric | Before | After | Change |
|--------|--------|-------|--------|
| Compose 1080p (1 src) | ~29ms | ~27ms | **-10%** |
| Audio mix 4096 frames | ~57µs | ~49µs | **-13%** |
| Latency p99 pipeline | — | 20.9ms | 12.4ms headroom @ 30fps |
| RSS growth (500 frames) | — | 0 MB | no leaks detected |

### Code Quality

- **259 tests** (unit + integration + proptest + doc-tests + fuzz targets)
- **`#[non_exhaustive]`** on all new public structs: `VideoCaptureManager`, `SourceEvent`, `CameraInfo`, `ScreenInfo`
- **`cargo audit`** clean, **`cargo deny`** clean, **`cargo clippy`** clean
- All `unsafe` blocks in screen capture documented with `// SAFETY:` comments

---

## [0.24.3] — 2026-03-24

Code audit and scaffold hardening release: full DSP chain in graph pipeline, compositor buffer reuse, dependency cleanup, Architecture Decision Records, and comprehensive benchmark expansion.

### Breaking (downstream note)

- **`RawFrame.data` is now `bytes::Bytes` instead of `Vec<u8>`** — enables O(1) clone for static sources (e.g. `ImageSource`). Downstream code that writes to `frame.data` directly must call `.to_vec()` first. Read access via indexing/slicing is unchanged (`Bytes` derefs to `[u8]`). `Compositor::reclaim_buffer()` now accepts `Bytes` and recovers the underlying `Vec` when possible.

### Changed

- **Graph pipeline full DSP chain** — `DspChainNode` now supports all 7 per-source effects matching `AudioMixer::mix()` order: EQ (parametric or graphic) → noise gate → compressor → de-esser → delay → reverb → pan → sanitize → meter
- **Graph source metering fixed** — per-source `PeakMeter` is now wired into graph execution via `Arc` sharing between `DspChainNode` and `AudioPipeline`, so `source_peak()` returns live values instead of always-zero
- **BufferPool activated in mixer** — removed `#[allow(dead_code)]`, processed buffers are returned to the pool after mixing for reuse; public `acquire_buffer()`/`release_buffer()` API for callers to participate in pool allocation
- **Compositor buffer reuse** — `Compositor` now stores a reusable scratch buffer; `compose()` takes `&mut self` and reuses the buffer across frames via `reclaim_buffer()` to eliminate per-frame heap allocation
- **NV12/ARGB conversions unified to BT.709** — `argb_to_nv12` and `nv12_to_argb` now use BT.709 coefficients (matching `argb_to_yuv420p`), correct for HD video. Previously used BT.601.
- **`serde_json` moved to dev-dependencies** — only used in tests, not library code
- **Compositor hot path** — `visible_layers()` Vec allocation replaced with inline iterator; `#[inline]` added to `RawFrame::expected_size`, `expected_size_for`, `is_valid`
- **Audio delay params** — out-of-range values now log via `tracing::debug!` when clamped

### Fixed

- **`SyntheticSource` division by zero** — `fps=0` no longer panics in `capture_frame()` (guarded with `.max(1)`)
- **`parse_hex_color` UTF-8 panic** — CLI color parsing now filters to ASCII hex digits before slicing, preventing panic on multi-byte input
- **Preview frame count** — `cmd_preview` now reports correct frame count when `--frames 0` (was reporting 0 instead of actual)
- **Buffer reclaim wired in CLI** — both `cmd_record` and `cmd_preview` now call `compositor.reclaim_buffer()` after each frame, eliminating ~8MB/frame allocation at 1080p
- **`SceneGraph::new` validation** — debug-asserts that width, height, and fps are non-zero

### Removed

- **`tokio`** dependency — unused (no async code yet), removes 36+ transitive crates
- **`chrono`** dependency — unused, removes timezone/date transitive crates
- **`thiserror`** dependency — unused (all errors use `anyhow`), removes derive macro crate
- **`tokio-test`** dev-dependency — unused

### Added

- **Architecture Decision Records** (`docs/decisions/`):
  - ADR-001: ARGB8888 as internal pixel format (vs NV12)
  - ADR-002: SIMD delegation to ranga (vs inline intrinsics)
  - ADR-003: tarang for encoding (vs direct FFI)
- **Property-based tests** — `proptest` for compositor: random layers/positions/opacities never panic, hidden/zero-opacity invariants, opaque fill coverage
- **Fuzz targets** — `fuzz/` crate with `libfuzzer-sys`: `fuzz_compose` (random scene graphs), `fuzz_frame_validation` (arbitrary frame data), `fuzz_color_convert` (YUV/NV12 roundtrip)
- **Benchmark expansion** — 7 bench targets, 36 functions:
  - `compose`: buffer reclaim, opaque vs blend paths, resolution scaling (480p–4K), many color fills
  - `audio`: graph pipeline (1/4/8 sources), buffer pool, mix buffer sizes (64–4096 frames)
  - `convert`: YUV420p forward, odd dimensions (1x1 to 1921x1081)
  - `output`: file write throughput (1KB–1MB), MP4 write throughput
  - `latency`: p50/p95/p99 per-frame at 1080p30 (1000 frames)
- **`scripts/bench-history.sh`** — runs all criterion benchmarks and appends results to `benchmarks/history.csv`
- **Memory stability test** — 300 frames at 1080p30 with buffer reclaim, verifies no growth

### Dependencies

| Crate | Old | New | Notes |
|-------|-----|-----|-------|
| ranga | 0.21.4 | 0.24.3 | GPU compute (GpuChain, transitions, geometry), Oklab/Oklch, BT.2020, perspective transforms, ICC profiles, histogram equalization, div255 precision fix, SIMD brightness/grayscale, cache-aware blur |
| proptest | — | 1 | dev-dependency for property-based testing |

### Inherited Fixes (via ranga 0.24.3)

- **Compositing precision** — div255 rounding replaces `>> 8`, eliminating ~0.4% cumulative brightness loss per blend pass
- **BT.709 Y coefficient** — white correctly maps to Y=255 (coefficient sum 255→256)
- **YUV420p odd-dimension sizing** — `div_ceil(2)` for chroma planes, fixing undersized buffers
- **Auto white balance** — clamped scale factors prevent extreme corrections
- **NEON brightness OOB read** — fixed on aarch64

### Latency Baseline (1080p30, 1 source + bg fill)

| Metric | Compose | Full Pipeline |
|--------|---------|---------------|
| p50 | 3.4ms | 18.9ms |
| p95 | 6.2ms | 24.6ms |
| p99 | 6.4ms | 25.0ms |
| Headroom | — | 8.3ms |

### Code Quality

- **Zero `unsafe` in src/** — confirmed during audit, no `unsafe` blocks in library or binary code
- **Zero `unwrap()`/`expect()` in library code** — confirmed during audit, all in test code only
- **217 tests** (unit + integration + proptest + doc-tests)
- **`cargo audit`** clean, **`cargo deny`** clean, **`cargo clippy`** clean
- Removed `#[allow(dead_code)]` from graph node structs/impls (now used or annotated intentionally)

---

## [0.23.3] — 2026-03-23

Audit and hardening release: ecosystem dependency upgrades, security fixes, performance optimization of color conversion hot paths, and project-wide annotation sweep.

### Security

- **Integer overflow in compositor ClipRect** — layer position + size arithmetic now uses i64 to prevent overflow that could cause out-of-bounds writes with crafted layer positions
- **Opacity threshold fix** — corrected the fully-opaque fast path threshold from 255 to 256 (fixed-point Q8), preventing layers at 99.6% opacity from skipping alpha blending
- **Opacity clamping** — layer opacity values are now clamped to `0.0..=1.0` at entry points, preventing undefined behavior from NaN or negative opacity values

### Performance

- **Single-pass color conversions** — `argb_to_yuv420p`, `argb_to_nv12`, and `nv12_to_argb` rewritten as direct single-pass implementations operating on `&[u8]` slices, eliminating intermediate buffer copies and format conversions through ranga
  - `argb_to_yuv420p` 1080p: 13.5ms → 4.1ms (**-69%**)
  - `argb_to_nv12` 720p: 3.4ms → 1.0ms (**-70%**, was +20% regression from ranga delegation)
  - `nv12_to_argb` 4K: 79.4ms → 34.7ms (**-56%**)
  - NV12 roundtrip 1080p: 18.9ms → 11.6ms (**-39%**)
- **Buffered file I/O** — `FileOutput` now wraps `File` in `BufWriter` with 256 KB buffer, reducing syscalls from one-per-packet to amortized batches
- **Box large enum variant** — `EncoderInner::OpenH264` boxed to reduce enum size from 7352 to 144 bytes

### Fixed

- **Audio graph gain/pan ignored** — `AudioPipeline::add_source()` now stores and applies the gain and pan parameters instead of silently discarding them
- **FrameClock u32 truncation** — `is_behind()` arithmetic widened from u32 to u64, preventing wrap-around after ~39.7 hours at 30fps
- **Mp4Output corrupt on drop** — added `Drop` impl that calls `finalize()` as a safety net, preventing corrupt MP4 files (missing moov atom) when dropped without explicit finalize
- **CompressorParams missing field** — added `mix: 1.0` to all `CompressorParams` literals for dhvani 0.22.4 compatibility

### Added

- **`#[non_exhaustive]`** on all 7 public enums: `LayerContent`, `VideoCodec`, `EncoderBackend`, `OutputConfig`, `SourceConfig`, `PixelFormat`, `Pattern`
- **`#[must_use]`** on 61 pure functions across all modules
- **`#[inline]`** on hot-path functions: `ClipRect::compute`, `blend_color_fill`, `blend_frame`, `blend_row_alpha`, `make_video_frame`, `make_packet`, color conversion functions

### Dependencies

| Crate | Old | New |
|-------|-----|-----|
| ai-hwaccel | 0.21.3 | 0.23.3 |
| dhvani | 0.21.4 | 0.22.4 |
| criterion | 0.5 | 0.8 |

New transitive dependency: `abaco` 0.22.4 (shared DSP math, via dhvani).

---

## [0.21.3] — 2026-03-21

Hardening release: project infrastructure, deeper crate integration, expanded test and benchmark coverage.

### Added

- **Project docs** — LICENSE (GPL-3.0-only), CONTRIBUTING.md, CODE_OF_CONDUCT.md (Contributor Covenant 2.1), SECURITY.md (threat model, reporting process)
- **CI hardening** — doc verification job, `cargo-semver-checks`, `cargo-vet` with Mozilla imports, coverage threshold 85%+, fuzz job (30s/target on main push)
- **`codecov.yml`** — project target 85%, patch target 80%, ignores benches/fuzz/examples
- **Example binaries** — `examples/compose.rs`, `examples/encode.rs`, `examples/record.rs`
- **Audio graph pipeline** (`audio::graph`) — node-based audio routing via `dhvani::graph` with `InputNode`, `GainNode`, `DspChainNode`, `MixerNode`, `MasterNode`; real-time safe plan swapping via `GraphProcessor`
- **Multi-source audio capture** (`audio::capture`) — `AudioCaptureManager` manages concurrent `PwCapture` instances for multi-device PipeWire capture with `drain_buffers()` and hot-plug event collection
- **Per-source audio metering** — `source_peak_db()` and `source_rms_db()` on `AudioMixer` for post-DSP, pre-mix level monitoring per source
- **GainSmoother integration** — per-source volume changes smoothed via `dhvani::dsp::GainSmoother` to eliminate audible clicks on gain transitions
- **NaN/Inf sanitization** — `dhvani::dsp::sanitize_sample()` applied after each per-source DSP chain to guard against filter instability
- **BufferPool** — `dhvani::buffer::BufferPool` pre-allocated in `AudioMixer` for future RT allocation reduction
- **Audio benchmarks** (`benches/audio.rs`) — mixer throughput (256–4096 frames, 2–16 sources), DSP chain comparison (none/EQ/compressor/full), master limiter, metering overhead
- **Color conversion benchmarks** (`benches/convert.rs`) — ARGB→YUV420p BT.709, ARGB→NV12, NV12→ARGB, NV12 roundtrip at 480p/720p/1080p/4K
- **Compositor benchmarks** — multi-layer bicubic scaling (2/4 layers), 4K background+source compositing
- **35 new tests** (87→122) — compositor scaling/transparency/edge cases, audio metering/NaN safety/gain smoother, BT.709 color validation, latency budget, timing, source edge cases, integration pipelines

### Changed

- **Bicubic resize** — compositor layer scaling upgraded from bilinear to `ranga::transform::ScaleFilter::Bicubic` (Catmull-Rom) for higher quality
- **BT.709 color conversion** — encode pipeline uses `rgba_to_yuv420p_bt709()` instead of BT.601, correct for HD video (H.264 assumes BT.709 for >= 720p)
- **Cached hardware detection** — `ai_hwaccel::DiskCachedRegistry` replaces `AcceleratorRegistry::detect()` with 60s disk-persisted cache, avoiding re-probing nvidia-smi/vulkaninfo every run
- **CLI audio capture** — `cmd_record` migrated from single `PwCapture` to `AudioCaptureManager` for multi-source support
- **MP4 output** — fixed for tarang 0.20.3 API compatibility (uses `new_with_video` with dummy audio config for video-only mode)

### Dependencies

| Crate | Old | New |
|-------|-----|-----|
| dhvani | 0.20 | 0.21.4 |
| ai-hwaccel | 0.20 | 0.21.3 |
| ranga | 0.21.3 | 0.21.4 |

---

## [0.20.3] — 2026-03-20

First functional release. Full compositing pipeline from source to encoded file output.

### Added

- **Scene graph** — `SceneGraph` with ordered `Layer` list, z-index sorting, position/scale/crop/opacity per layer, serde roundtrip
- **Source trait** — `Source` trait with `capture_frame()` → `RawFrame`, `SourceId`, `SourceConfig`
- **ImageSource** — loads PNG/JPEG via `image` crate, converts to ARGB8888
- **SyntheticSource** — generates gradient, solid, and checkerboard test patterns
- **Pixel formats** — `PixelFormat` enum (ARGB8888, NV12), format-aware `RawFrame::expected_size_for()`
- **Compositor** — alpha-blends visible layers bottom-to-top with per-pixel alpha and per-layer opacity
  - Pre-computed clip rects (eliminate per-pixel bounds checks)
  - Row-level memcpy fast path for fully opaque layers
  - **SSE2 SIMD** alpha blending on x86_64 — 9.4× speedup (11ms → 1.2ms per 1080p layer)
  - Nearest-neighbour scaling for size mismatches
- **Color conversion** — `argb_to_yuv420p`, `argb_to_nv12`, `nv12_to_argb` with fixed-point BT.601 (no floats)
- **H.264 encoding** — software encode via tarang/openh264 (`openh264-enc` feature), `EncodePipeline` with init/encode_frame API
- **File output** — `FileOutput` sink writes encoded packets to raw H.264 bitstream files
- **Frame timing** — `FrameClock` (PTS calculation, behind-schedule detection), `LatencyBudget` (per-stage tracking)
- **CLI** — `aethersafta record`, `preview`, `info` subcommands with clap
  - `--source image:<path>` and `--source color:<RRGGBBAA>` inputs
  - H.264 encode when `openh264-enc` is enabled, raw ARGB fallback otherwise
- **Benchmarks** — criterion benchmarks for compositor (color fill, source layers, scaling), color conversion (1080p/4K), H.264 encode (240p–1080p), full pipeline
- **CI** — GitHub Actions: fmt, clippy, test (Linux + macOS), MSRV 1.89, cargo-audit, cargo-deny, coverage
- **cargo-deny** — `deny.toml` with allowed licenses for full dependency tree

### Performance (v0.20.3 baseline)

| Operation | 1080p | 4K |
|-----------|-------|----|
| Composite 1 source layer (SSE2) | 1.2 ms | — |
| Composite 5 source layers (SSE2) | 5.7 ms | — |
| Color fill | 1.1 ms | 6.1 ms |
| ARGB→YUV420p | 4.0 ms | 16.6 ms |
| H.264 encode (openh264) | 7.5 ms | — |
| Full pipeline (compose+encode+write) | 12.7 ms | — |
