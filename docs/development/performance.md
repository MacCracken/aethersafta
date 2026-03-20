# Aethersafta Performance

> Benchmark results from v0.20.3 on x86_64 Linux.
>
> Run with: `cargo bench --bench compose --no-default-features`
> and: `cargo bench --bench encode --no-default-features --features openh264-enc`

---

## Pipeline budget (1080p @ 30fps)

Target: **33.3 ms** per frame.

| Stage | Time | % budget |
|-------|------|----------|
| Composite (1 layer, SIMD) | 1.2 ms | 3.6% |
| ARGB→YUV420p conversion | 4.0 ms | 12.0% |
| H.264 encode (openh264) | 7.5 ms | 22.5% |
| **Total** | **12.7 ms** | **38.1%** |
| Headroom | 20.6 ms | 61.9% |

At 60fps (16.6 ms budget), single-layer 1080p compositing + encode fits in ~12.7 ms with 3.9 ms to spare.

---

## Compositor

All benchmarks use ARGB8888 frames with per-layer opacity (alpha blending active).

### Color fill

| Resolution | Time |
|------------|------|
| 1080p (1920×1080) | 1.1 ms |
| 4K (3840×2160) | 6.1 ms |

### Source layer blending (1:1 scale, opacity=0.8)

SSE2 SIMD path — processes 2 ARGB pixels per iteration via 128-bit registers.

| Layers | Time | Per-layer |
|--------|------|-----------|
| 1 | 1.2 ms | 1.2 ms |
| 3 | 3.4 ms | 1.1 ms |
| 5 | 5.7 ms | 1.1 ms |

### Scaled compositing

Nearest-neighbour scaling uses the scalar path (gather pattern not SIMD-friendly).

| Operation | Time |
|-----------|------|
| 480p → 1080p | 12.2 ms |

### SIMD impact

| Benchmark | Scalar | SSE2 | Speedup |
|-----------|--------|------|---------|
| 1 source layer 1080p | 11.2 ms | 1.2 ms | **9.3×** |
| 3 source layers 1080p | 32.5 ms | 3.4 ms | **9.6×** |
| 5 source layers 1080p | 53.6 ms | 5.7 ms | **9.4×** |

---

## Color space conversion

BT.601 fixed-point integer math (no floats).

| Resolution | ARGB→YUV420p | ARGB→NV12 |
|------------|-------------|-----------|
| 1080p | 4.0 ms | ~4.0 ms |
| 4K | 16.6 ms | ~16.6 ms |

---

## H.264 encoding (tarang / openh264)

Software encoding via openh264, default bitrate 6 Mbps.

| Resolution | Single frame | 30-frame burst | Per-frame avg |
|------------|-------------|----------------|---------------|
| 240p (320×240) | 229 µs | — | — |
| 720p (1280×720) | 2.9 ms | 96 ms | 3.2 ms |
| 1080p (1920×1080) | 7.5 ms | — | — |

### Full pipeline (compose → encode → write)

| Resolution | 10 frames | Per-frame |
|------------|-----------|-----------|
| 480p (640×480) | 12.2 ms | 1.2 ms |

---

## Optimization history

| Version | Change | Impact |
|---------|--------|--------|
| 0.20.3 | Pre-computed clip rects | Color fill 10× (11ms→1.1ms) |
| 0.20.3 | Fixed-point opacity (no float per pixel) | ~1.1× across compositor |
| 0.20.3 | Row-level memcpy for opaque layers | 1:1 opaque blit near memcpy speed |
| 0.20.3 | Fixed-point BT.601 (no f32) | YUV conversion 1.3× (5.3ms→4.0ms) |
| 0.20.3 | SSE2 SIMD alpha blending | Source blend 9.4× (11ms→1.2ms) |

---

## Future targets

| Optimization | Expected impact | Version |
|-------------|-----------------|---------|
| AVX2 alpha blending (4 pixels/iter) | ~2× over SSE2 | 0.9.0 |
| SIMD color conversion (YUV/NV12) | ~2–4× | 0.9.0 |
| GPU compositing (Vulkan compute) | 10–100× for multi-layer | 0.9.0 |
| Zero-copy frame pipeline | Eliminate per-frame alloc | 0.9.0 |
| Memory pool for frame buffers | Reduce allocator pressure | 0.9.0 |
| SIMD scaled compositing | ~5× for scaled layers | post-1.0 |

---

## Running benchmarks

```bash
# Compositor + color conversion (no system deps)
cargo bench --bench compose --no-default-features

# H.264 encoding (requires openh264)
cargo bench --bench encode --no-default-features --features openh264-enc

# All benchmarks
cargo bench --no-default-features --features openh264-enc
```

HTML reports are generated in `target/criterion/`.
