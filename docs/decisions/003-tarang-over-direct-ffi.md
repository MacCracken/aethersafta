# ADR-003: tarang for Encoding Instead of Direct FFI

**Status**: Accepted
**Date**: 2026-03-20 (v0.20.3)

## Context

H.264 encoding requires calling into native codec libraries — OpenH264 (software), VA-API (Intel/AMD hardware), NVENC (NVIDIA). The options are:

1. Direct FFI bindings to each codec library from aethersafta
2. Use the `tarang` crate which wraps these codecs behind a safe Rust API

## Decision

Use **tarang** for all encoding and muxing. Aethersafta never calls codec FFI directly. The encode pipeline (`EncodePipeline`) selects a backend via `ai-hwaccel` and passes frames to tarang's encoder API.

## Rationale

1. **Own the stack.** tarang is an AGNOS crate that owns the encoding/muxing domain. Centralising codec FFI in tarang means one crate handles the `unsafe` bindings, ABI compatibility, and codec quirks (NAL unit formatting, PTS/DTS management, flush semantics).

2. **Backend abstraction.** `EncoderInner` is an enum over `tarang::video::VaapiEncoder` and `tarang::video::OpenH264Encoder`. Adding a new backend (NVENC, QSV) means adding a tarang feature and a new enum variant — no FFI work in aethersafta.

3. **Muxing included.** tarang provides MP4 muxing (`Mp4Muxer`) alongside encoding. Direct FFI would require separate muxing bindings (mp4parse, minimp4, or manual ISO BMFF writing).

4. **Feature gating.** Optional backends are feature-gated: `openh264-enc`, `vaapi`. Consumers pull only the codecs they need. tarang handles the conditional compilation.

5. **Safety.** tarang's safe API prevents common FFI mistakes: dangling encoder handles, use-after-free on codec contexts, incorrect buffer lifetime management. aethersafta's encode module has zero `unsafe`.

## Trade-offs

- **Version coupling.** tarang API changes require aethersafta updates (e.g., v0.21.3 `Mp4Muxer::new_with_video` change). Pinned to `0.21.3` to avoid surprise breakage.

- **Abstraction overhead.** tarang's `EncodedPacket` wraps `bytes::Bytes` which adds a reference count. Negligible for encoded data (small relative to raw frames).

- **Feature surface.** tarang exposes a subset of each codec's parameters. If aethersafta needs a codec-specific option not in tarang's API, tarang must be extended first.

## Alternatives Considered

- **Direct openh264-sys FFI**: Considered for v0.20.3 prototype. Rejected: requires manual NAL parsing, encoder lifecycle management, and `unsafe` blocks in aethersafta.
- **ffmpeg-next / ffmpeg-sys**: Rejected: GPL dependency, massive transitive dep tree, configuration complexity. tarang provides exactly the codecs we need without the ffmpeg surface area.
- **gstreamer-rs**: Rejected: heavyweight runtime, pipeline model doesn't match aethersafta's pull-based frame model, adds GLib dependency.
