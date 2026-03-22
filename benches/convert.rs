//! Color conversion and format interchange benchmarks.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use aethersafta::encode::{argb_to_nv12, argb_to_yuv420p, nv12_to_argb};

fn make_argb(width: u32, height: u32) -> Vec<u8> {
    let size = (width * height * 4) as usize;
    let mut data = vec![0u8; size];
    for (i, chunk) in data.chunks_exact_mut(4).enumerate() {
        chunk[0] = 255; // A
        chunk[1] = (i % 256) as u8; // R
        chunk[2] = ((i * 3) % 256) as u8; // G
        chunk[3] = ((i * 7) % 256) as u8; // B
    }
    data
}

fn make_nv12(width: u32, height: u32) -> Vec<u8> {
    let y_size = (width * height) as usize;
    let uv_size = (width * (height / 2)) as usize;
    vec![128u8; y_size + uv_size]
}

// ---------------------------------------------------------------------------
// ARGB → YUV420p (BT.709)
// ---------------------------------------------------------------------------

fn bench_argb_to_yuv420p(c: &mut Criterion) {
    let mut group = c.benchmark_group("argb_to_yuv420p_bt709");
    for &(w, h, label) in &[
        (640, 480, "480p"),
        (1280, 720, "720p"),
        (1920, 1080, "1080p"),
        (3840, 2160, "4K"),
    ] {
        let argb = make_argb(w, h);
        group.bench_with_input(BenchmarkId::new("convert", label), &(), |b, _| {
            b.iter(|| argb_to_yuv420p(&argb, w, h))
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// ARGB → NV12
// ---------------------------------------------------------------------------

fn bench_argb_to_nv12(c: &mut Criterion) {
    let mut group = c.benchmark_group("argb_to_nv12");
    for &(w, h, label) in &[
        (640, 480, "480p"),
        (1280, 720, "720p"),
        (1920, 1080, "1080p"),
        (3840, 2160, "4K"),
    ] {
        let argb = make_argb(w, h);
        group.bench_with_input(BenchmarkId::new("convert", label), &(), |b, _| {
            b.iter(|| argb_to_nv12(&argb, w, h))
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// NV12 → ARGB
// ---------------------------------------------------------------------------

fn bench_nv12_to_argb(c: &mut Criterion) {
    let mut group = c.benchmark_group("nv12_to_argb");
    for &(w, h, label) in &[
        (640, 480, "480p"),
        (1280, 720, "720p"),
        (1920, 1080, "1080p"),
        (3840, 2160, "4K"),
    ] {
        let nv12 = make_nv12(w, h);
        group.bench_with_input(BenchmarkId::new("convert", label), &(), |b, _| {
            b.iter(|| nv12_to_argb(&nv12, w, h))
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Roundtrip: ARGB → NV12 → ARGB
// ---------------------------------------------------------------------------

fn bench_nv12_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("nv12_roundtrip");
    for &(w, h, label) in &[(640, 480, "480p"), (1920, 1080, "1080p")] {
        let argb = make_argb(w, h);
        group.bench_with_input(BenchmarkId::new("argb_nv12_argb", label), &(), |b, _| {
            b.iter(|| {
                let nv12 = argb_to_nv12(&argb, w, h);
                nv12_to_argb(&nv12, w, h)
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_argb_to_yuv420p,
    bench_argb_to_nv12,
    bench_nv12_to_argb,
    bench_nv12_roundtrip,
);
criterion_main!(benches);
