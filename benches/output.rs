//! Output throughput benchmarks.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use aethersafta::output::file::FileOutput;
use aethersafta::output::mp4::Mp4Output;
use aethersafta::output::{EncodedPacket, OutputSink};

fn make_packets(count: usize, size: usize) -> Vec<EncodedPacket> {
    (0..count)
        .map(|i| EncodedPacket {
            data: vec![0xABu8; size],
            pts_us: i as u64 * 33333,
            dts_us: i as u64 * 33333,
            is_keyframe: i == 0,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// FileOutput write throughput
// ---------------------------------------------------------------------------

fn bench_file_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_write");
    for &(size, label) in &[
        (1_024, "1KB"),
        (10_240, "10KB"),
        (102_400, "100KB"),
        (1_048_576, "1MB"),
    ] {
        let packets = make_packets(100, size);
        group.bench_with_input(
            BenchmarkId::new("throughput", label),
            &packets,
            |b, pkts| {
                b.iter(|| {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("bench.h264");
                    let mut out = FileOutput::create(&path).unwrap();
                    for pkt in pkts {
                        out.write_packet(pkt).unwrap();
                    }
                    out.flush().unwrap();
                })
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Mp4Output video-only write throughput
// ---------------------------------------------------------------------------

fn bench_mp4_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("mp4_write");
    for &(size, label) in &[(1_024, "1KB"), (10_240, "10KB")] {
        let packets = make_packets(100, size);
        group.bench_with_input(
            BenchmarkId::new("video_only", label),
            &packets,
            |b, pkts| {
                b.iter(|| {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("bench.mp4");
                    let mut out = Mp4Output::create_video_only(
                        &path,
                        tarang::core::VideoCodec::H264,
                        1920,
                        1080,
                    )
                    .unwrap();
                    for pkt in pkts {
                        out.write_video(pkt).unwrap();
                    }
                    out.finalize().unwrap();
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_file_write, bench_mp4_write);
criterion_main!(benches);
