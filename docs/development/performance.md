# Aethersafta Performance

> Benchmark history on x86_64 Linux. All times are criterion median values.
>
> Run with: `cargo bench`

---

## v0.21.3 Baseline (2026-03-21)

### Pipeline budget (1080p @ 30fps)

Target: **33.3 ms** per frame.

| Stage | Time | % budget |
|-------|------|----------|
| Composite (1 layer, SIMD) | 5.8 ms | 17.4% |
| ARGB→YUV420p BT.709 | 15.0 ms | 45.0% |
| H.264 encode (openh264) | 7.5 ms | 22.5% |
| **Total** | **28.3 ms** | **84.9%** |
| Headroom | 5.0 ms | 15.1% |

Note: BT.709 conversion is ~4× slower than the v0.20.3 BT.601 path (15.0 ms vs 3.6 ms) due to ranga's BT.709 implementation not yet having fixed-point optimisation. This is the primary optimisation target.

### Compositor

All benchmarks use ARGB8888 frames with per-layer opacity (alpha blending active).

#### Color fill

| Resolution | Time |
|------------|------|
| 1080p (1920×1080) | 1.3 ms |
| 4K (3840×2160) | 7.3 ms |

#### Source layer blending (1:1 scale, opacity=0.8)

| Layers | Time | Per-layer |
|--------|------|-----------|
| 1 | 5.8 ms | 5.8 ms |
| 3 | 16.0 ms | 5.3 ms |
| 5 | 26.8 ms | 5.4 ms |

#### Scaled compositing (bicubic)

Upgraded from nearest-neighbour to Catmull-Rom bicubic in v0.21.3.

| Operation | Time |
|-----------|------|
| 480p → 1080p (single layer) | 247.8 ms |
| 480p → 540p × 2 layers | 131.9 ms |
| 480p → 540p × 4 layers | 233.0 ms |

#### 4K compositing

| Operation | Time |
|-----------|------|
| 4K bg fill + 4K source layer | 25.9 ms |

### Color space conversion

BT.709 for YUV420p (HD video standard), fixed-point for NV12.

| Resolution | ARGB→YUV420p (BT.709) | ARGB→NV12 | NV12→ARGB | NV12 roundtrip |
|------------|----------------------|-----------|-----------|----------------|
| 480p | 0.7 ms | 0.8 ms | 1.6 ms | 2.4 ms |
| 720p | 6.5 ms | 2.7 ms | 5.2 ms | — |
| 1080p | 15.0 ms | 6.3 ms | 11.3 ms | 18.2 ms |
| 4K | 70.7 ms | 29.0 ms | 79.6 ms | — |

### H.264 encoding (tarang / openh264)

Software encoding via openh264, default bitrate 6 Mbps.

| Resolution | Single frame | 30-frame burst | Per-frame avg |
|------------|-------------|----------------|---------------|
| 240p (320×240) | 213 µs | — | — |
| 720p (1280×720) | 2.9 ms | 95.7 ms | 3.2 ms |
| 1080p (1920×1080) | 7.5 ms | — | — |

#### Full pipeline (compose → encode → write)

| Resolution | 10 frames | Per-frame |
|------------|-----------|-----------|
| 480p (640×480) | 11.1 ms | 1.1 ms |

### Audio mixer

1024-frame stereo buffers at 48 kHz (21.3 ms of audio per buffer).

#### Mix throughput

| Frames | Time |
|--------|------|
| 256 | 2.9 µs |
| 1024 | 11.2 µs |
| 4096 | 43.7 µs |

#### Multi-source mix (1024 frames)

| Sources | Time | Per-source |
|---------|------|------------|
| 2 | 17.4 µs | 8.7 µs |
| 4 | 30.3 µs | 7.6 µs |
| 8 | 56.5 µs | 7.1 µs |
| 16 | 120.2 µs | 7.5 µs |

#### DSP chain cost (1024 frames, single source)

| Configuration | Time | Overhead vs bare |
|---------------|------|-----------------|
| No DSP | 11.3 µs | — |
| 2-band EQ | 22.9 µs | +11.7 µs |
| Compressor | 37.5 µs | +26.2 µs |
| Full chain (EQ + comp + limiter) | 44.9 µs | +33.6 µs |

#### Master limiter

| State | Time |
|-------|------|
| Disabled | 11.1 µs |
| Enabled | 33.4 µs |

#### Metering overhead

| Operation | Time |
|-----------|------|
| Mix + read all meters | 11.1 µs |

Metering is effectively free — the `LevelMeter` processing is amortised into the mix pass.

### Scene graph operations

| Operation | Time |
|-----------|------|
| Add 100 layers | 17.7 µs |
| Filter 50 layers (visible_layers) | 203 ns |

---

## v0.20.3 Baseline (2026-03-20)

### Pipeline budget (1080p @ 30fps)

| Stage | Time | % budget |
|-------|------|----------|
| Composite (1 layer, SIMD) | 1.2 ms | 3.6% |
| ARGB→YUV420p BT.601 | 4.0 ms | 12.0% |
| H.264 encode (openh264) | 7.5 ms | 22.5% |
| **Total** | **12.7 ms** | **38.1%** |
| Headroom | 20.6 ms | 61.9% |

### Compositor

| Benchmark | Time |
|-----------|------|
| Color fill 1080p | 1.1 ms |
| Color fill 4K | 6.1 ms |
| 1 source layer 1080p | 1.2 ms |
| 3 source layers 1080p | 3.4 ms |
| 5 source layers 1080p | 5.7 ms |
| Scaled 480p→1080p (nearest) | 12.2 ms |

### SIMD impact

| Benchmark | Scalar | SSE2 | Speedup |
|-----------|--------|------|---------|
| 1 source layer 1080p | 11.2 ms | 1.2 ms | **9.3×** |
| 3 source layers 1080p | 32.5 ms | 3.4 ms | **9.6×** |
| 5 source layers 1080p | 53.6 ms | 5.7 ms | **9.4×** |

---

## Version comparison

| Benchmark | v0.20.3 | v0.21.3 | Change | Notes |
|-----------|---------|---------|--------|-------|
| Color fill 1080p | 1.1 ms | 1.3 ms | +18% | Within noise |
| 1 source layer 1080p | 1.2 ms | 5.8 ms | +383% | Needs investigation — ranga 0.21.4 blend path |
| 5 source layers 1080p | 5.7 ms | 26.8 ms | +370% | Same cause as above |
| ARGB→YUV420p 1080p | 4.0 ms (BT.601) | 15.0 ms (BT.709) | +275% | BT.709 not yet optimised in ranga |
| Scaled 480p→1080p | 12.2 ms (nearest) | 247.8 ms (bicubic) | +1931% | Expected — bicubic is much higher quality |
| H.264 encode 1080p | 7.5 ms | 7.5 ms | 0% | Unchanged (tarang 0.20.3) |

### Key observations

1. **Source layer blending regression** — 1:1 source blending is ~5× slower in v0.21.3. This is likely a ranga 0.21.4 blend path change that needs investigation. The v0.20.3 SSE2 fast path may not be activating correctly with the new ranga version.
2. **BT.709 conversion is the new bottleneck** — 15 ms at 1080p vs 4 ms for BT.601. Ranga's BT.709 path needs SIMD optimisation (tracked upstream).
3. **Bicubic scaling is expensive but correct** — 248 ms for 480p→1080p is too slow for real-time. This path should be used for static overlays, not per-frame scaling. For live sources, pre-scale or use the GPU path (future).
4. **Audio mixer is extremely fast** — full DSP chain (EQ + compressor + limiter) at 45 µs for 1024 frames is well under the 21.3 ms audio buffer duration. 16 sources mixed in 120 µs.
5. **Encoding is stable** — no change between versions (same tarang 0.20.3).

---

## Optimization history

| Version | Change | Impact |
|---------|--------|--------|
| 0.20.3 | Pre-computed clip rects | Color fill 10× (11ms→1.1ms) |
| 0.20.3 | Fixed-point opacity (no float per pixel) | ~1.1× across compositor |
| 0.20.3 | Row-level memcpy for opaque layers | 1:1 opaque blit near memcpy speed |
| 0.20.3 | Fixed-point BT.601 (no f32) | YUV conversion 1.3× (5.3ms→4.0ms) |
| 0.20.3 | SSE2 SIMD alpha blending | Source blend 9.4× (11ms→1.2ms) |
| 0.21.3 | Bicubic resize (Catmull-Rom) | Higher quality, ~20× slower than nearest |
| 0.21.3 | BT.709 color conversion | Correct for HD video, ~4× slower than BT.601 |
| 0.21.3 | GainSmoother for audio | Click-free volume, negligible overhead |
| 0.21.3 | NaN sanitization in DSP | <1 µs overhead per mix cycle |
| 0.21.3 | DiskCachedRegistry | Hardware detection cached to disk (60s TTL) |

---

## Future targets

| Optimization | Expected impact | Version |
|-------------|-----------------|---------|
| Investigate ranga 0.21.4 blend regression | Restore ~1.2 ms/layer | 0.21.x |
| SIMD BT.709 conversion (ranga upstream) | ~3–4× (15ms→4ms) | 0.22.0 |
| AVX2 alpha blending (4 pixels/iter) | ~2× over SSE2 | 0.22.0 |
| GPU compositing (Vulkan compute) | 10–100× for multi-layer | 0.25.0 |
| Zero-copy frame pipeline | Eliminate per-frame alloc | 0.25.0 |
| Pre-scaled layer cache | Avoid per-frame bicubic | 0.23.0 |

---

## Running benchmarks

```bash
# Compositor (no system deps needed)
cargo bench --bench compose

# Audio mixer
cargo bench --bench audio

# Color conversion
cargo bench --bench convert

# H.264 encoding (requires openh264)
cargo bench --bench encode --features openh264-enc

# All benchmarks
cargo bench --features openh264-enc
```

HTML reports are generated in `target/criterion/`.
