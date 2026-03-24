//! Encoding benchmarks — requires `openh264-enc` feature.
//!
//! Run with: cargo bench --bench encode --features openh264-enc

#[cfg(feature = "openh264-enc")]
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

#[cfg(feature = "openh264-enc")]
fn make_argb_frame(width: u32, height: u32, pts_us: u64) -> aethersafta::source::RawFrame {
    let size = (width * height * 4) as usize;
    let mut data = vec![0u8; size];
    for (i, chunk) in data.chunks_exact_mut(4).enumerate() {
        let x = (i as u32) % width;
        let y = (i as u32) / width;
        chunk[0] = 255; // A
        chunk[1] = (x % 256) as u8; // R
        chunk[2] = (y % 256) as u8; // G
        chunk[3] = ((x + y) % 256) as u8; // B
    }
    aethersafta::source::RawFrame {
        data,
        format: aethersafta::source::PixelFormat::Argb8888,
        width,
        height,
        pts_us,
    }
}

#[cfg(feature = "openh264-enc")]
fn bench_encode_single_frame(c: &mut Criterion) {
    use aethersafta::encode::{EncodePipeline, EncoderConfig};

    let mut group = c.benchmark_group("encode_h264_single_frame");

    for &(w, h, label) in &[
        (320, 240, "240p"),
        (1280, 720, "720p"),
        (1920, 1080, "1080p"),
    ] {
        let mut pipeline = EncodePipeline::new(EncoderConfig::default());
        pipeline.init(w, h, 30).unwrap();
        let frame = make_argb_frame(w, h, 0);

        group.bench_with_input(BenchmarkId::new("encode", label), &(), |b, _| {
            b.iter(|| pipeline.encode_frame(&frame).unwrap())
        });
    }
    group.finish();
}

#[cfg(feature = "openh264-enc")]
fn bench_encode_throughput(c: &mut Criterion) {
    use aethersafta::encode::{EncodePipeline, EncoderConfig};

    let mut group = c.benchmark_group("encode_h264_throughput");
    group.sample_size(10);

    let w = 1280;
    let h = 720;
    let n_frames = 30;

    let mut pipeline = EncodePipeline::new(EncoderConfig::default());
    pipeline.init(w, h, 30).unwrap();

    let frames: Vec<_> = (0..n_frames)
        .map(|i| make_argb_frame(w, h, i * 33333))
        .collect();

    group.bench_function("720p_30_frames", |b| {
        b.iter(|| {
            for frame in &frames {
                pipeline.encode_frame(frame).unwrap();
            }
        })
    });
    group.finish();
}

#[cfg(feature = "openh264-enc")]
fn bench_full_pipeline(c: &mut Criterion) {
    use std::collections::HashMap;

    use aethersafta::encode::{EncodePipeline, EncoderConfig};
    use aethersafta::output::OutputSink;
    use aethersafta::output::file::FileOutput;
    use aethersafta::scene::compositor::Compositor;
    use aethersafta::scene::{Layer, LayerContent, SceneGraph};

    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);

    let w = 640;
    let h = 480;
    let mut comp = Compositor::new(w, h);
    let mut scene = SceneGraph::new(w, h, 30);
    scene.add_layer(Layer::new(
        "bg",
        LayerContent::ColorFill {
            color: [64, 128, 255, 255],
        },
    ));

    let mut pipeline = EncodePipeline::new(EncoderConfig::default());
    pipeline.init(w, h, 30).unwrap();

    group.bench_function("480p_compose_encode_write_10frames", |b| {
        b.iter(|| {
            let dir = tempfile::tempdir().unwrap();
            let mut out = FileOutput::create(dir.path().join("bench.h264")).unwrap();

            for i in 0..10u64 {
                let composited = comp.compose(&scene, &HashMap::new(), i * 33333);
                let packet = pipeline.encode_frame(&composited).unwrap();
                out.write_packet(&packet).unwrap();
            }
            out.close().unwrap();
        })
    });
    group.finish();
}

#[cfg(feature = "openh264-enc")]
criterion_group!(
    benches,
    bench_encode_single_frame,
    bench_encode_throughput,
    bench_full_pipeline,
);

#[cfg(feature = "openh264-enc")]
criterion_main!(benches);

#[cfg(not(feature = "openh264-enc"))]
fn main() {
    eprintln!("encode benchmarks require --features openh264-enc");
}
