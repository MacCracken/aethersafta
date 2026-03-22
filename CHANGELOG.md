# Changelog

All notable changes to aethersafta are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Pre-1.0 versioning uses `0.D.M` (day.month) SemVer.

---

## [0.21.3] — 2026-03-21

Hardening release: project infrastructure, deeper crate integration, expanded test and benchmark coverage.

### Added

- **Project docs** — LICENSE (AGPL-3.0-only), CONTRIBUTING.md, CODE_OF_CONDUCT.md (Contributor Covenant 2.1), SECURITY.md (threat model, reporting process)
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
