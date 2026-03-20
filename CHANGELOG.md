# Changelog

All notable changes to aethersafta are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Pre-1.0 versioning uses `0.D.M` (day.month) SemVer.

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
