# ADR-001: ARGB8888 as Internal Pixel Format

**Status**: Accepted
**Date**: 2026-03-20 (v0.20.3)

## Context

The compositor needs a single canonical pixel format for the compositing buffer. The two candidates are:

- **ARGB8888** — 4 bytes per pixel, alpha in the high byte, direct CPU access
- **NV12** — planar Y + interleaved UV, half chroma resolution, native encoder input

## Decision

Use **ARGB8888** as the internal compositing format. Convert to NV12/YUV420p only at the encode boundary.

## Rationale

1. **Alpha blending requires alpha channel.** NV12 has no alpha — compositing in NV12 would require a separate alpha plane or pre-multiplied blending tricks that add complexity and reduce quality.

2. **Source input is ARGB.** Image sources (PNG, JPEG) decode to RGBA/ARGB. Screen capture (wlr-screencopy, PipeWire DMA-BUF) provides ARGB/XRGB. Converting sources to NV12 for compositing, then losing chroma resolution, then re-compositing — wastes quality.

3. **Compositor operations are simpler in ARGB.** Per-pixel alpha blending, opacity, color fills, and row-level SIMD all operate naturally on 4-byte aligned pixels. NV12's planar layout requires separate Y and UV passes.

4. **Single conversion point.** ARGB→YUV420p happens exactly once, in `EncodePipeline::encode_frame()`, after compositing is complete. This keeps the conversion cost isolated and measurable.

## Trade-offs

- **Conversion cost.** ARGB→YUV420p adds ~4ms at 1080p (BT.709). This is significant in a 33ms frame budget but acceptable given the compositing quality benefit.

- **Memory.** ARGB8888 is 4 bytes/pixel vs NV12's 1.5 bytes/pixel. A 1080p frame is 8.3MB vs 3.1MB. Acceptable for a compositor that holds 1-2 frames in flight.

- **GPU path.** When GPU compositing arrives (wgpu), the internal format may shift to GPU-native (RGBA8 or NV12 via compute shader). This ADR covers the CPU path.

## Alternatives Considered

- **NV12 internal**: Rejected — no alpha channel makes compositing impractical without a separate alpha surface.
- **RGBA8888**: Considered — functionally equivalent but ARGB matches ranga's blend primitives and x86 SIMD lane ordering. ranga provides ARGB↔RGBA conversion when needed (e.g., for resize via `ranga::transform`).
