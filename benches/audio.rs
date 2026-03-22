//! Audio pipeline benchmarks: mixer throughput, DSP chain, metering.

use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use dhvani::buffer::AudioBuffer;

use aethersafta::audio::{AudioMixer, AudioMixerConfig, AudioSourceConfig};

fn test_buffer(value: f32, frames: usize) -> AudioBuffer {
    AudioBuffer::from_interleaved(vec![value; frames * 2], 2, 48000).unwrap()
}

// ---------------------------------------------------------------------------
// Mixer throughput
// ---------------------------------------------------------------------------

fn bench_mix_single_source(c: &mut Criterion) {
    let mut group = c.benchmark_group("mix_single_source");

    for &frames in &[256, 1024, 4096] {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Source"));

        group.bench_with_input(BenchmarkId::new("frames", frames), &frames, |b, &frames| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, frames));
                mixer.mix(&mut buffers)
            })
        });
    }
    group.finish();
}

fn bench_mix_multi_source(c: &mut Criterion) {
    let mut group = c.benchmark_group("mix_multi_source");

    for &n_sources in &[2, 4, 8, 16] {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let ids: Vec<_> = (0..n_sources)
            .map(|i| mixer.add_source(AudioSourceConfig::new(format!("Src-{i}"))))
            .collect();

        group.bench_with_input(
            BenchmarkId::new("1024_frames", format!("{n_sources}_sources")),
            &(),
            |b, _| {
                b.iter(|| {
                    let mut buffers = HashMap::new();
                    for &id in &ids {
                        buffers.insert(id, test_buffer(0.3, 1024));
                    }
                    mixer.mix(&mut buffers)
                })
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// DSP chain
// ---------------------------------------------------------------------------

fn bench_mix_with_dsp(c: &mut Criterion) {
    let mut group = c.benchmark_group("mix_with_dsp");

    // Baseline: no DSP
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Plain"));

        group.bench_function("no_dsp", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // EQ only
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("EQ"));
        mixer.set_source_eq(
            id,
            vec![
                dhvani::dsp::EqBandConfig {
                    band_type: dhvani::dsp::BandType::HighPass,
                    freq_hz: 80.0,
                    gain_db: 0.0,
                    q: 0.707,
                    enabled: true,
                },
                dhvani::dsp::EqBandConfig {
                    band_type: dhvani::dsp::BandType::Peaking,
                    freq_hz: 3000.0,
                    gain_db: 3.0,
                    q: 1.0,
                    enabled: true,
                },
            ],
        );

        group.bench_function("eq_2band", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // Compressor only
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Comp"));
        mixer.set_source_compressor(
            id,
            dhvani::dsp::CompressorParams {
                threshold_db: -20.0,
                ratio: 4.0,
                attack_ms: 5.0,
                release_ms: 50.0,
                makeup_gain_db: 0.0,
                knee_db: 6.0,
            },
        );

        group.bench_function("compressor", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.8, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // Full chain: EQ + compressor + limiter
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: true,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Full"));
        mixer.set_source_eq(
            id,
            vec![dhvani::dsp::EqBandConfig {
                band_type: dhvani::dsp::BandType::HighPass,
                freq_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
                enabled: true,
            }],
        );
        mixer.set_source_compressor(
            id,
            dhvani::dsp::CompressorParams {
                threshold_db: -18.0,
                ratio: 3.0,
                attack_ms: 10.0,
                release_ms: 100.0,
                makeup_gain_db: 3.0,
                knee_db: 6.0,
            },
        );

        group.bench_function("full_chain_eq_comp_limiter", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.7, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Master limiter
// ---------------------------------------------------------------------------

fn bench_master_limiter(c: &mut Criterion) {
    let mut group = c.benchmark_group("master_limiter");

    for &enabled in &[false, true] {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: enabled,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Src"));

        let label = if enabled { "enabled" } else { "disabled" };
        group.bench_function(label, |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(1.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Metering overhead
// ---------------------------------------------------------------------------

fn bench_metering(c: &mut Criterion) {
    let mut mixer = AudioMixer::new(AudioMixerConfig {
        master_limiter: false,
        ..Default::default()
    });
    let id = mixer.add_source(AudioSourceConfig::new("Metered"));

    c.bench_function("mix_1024_with_metering", |b| {
        b.iter(|| {
            let mut buffers = HashMap::new();
            buffers.insert(id, test_buffer(0.5, 1024));
            mixer.mix(&mut buffers);
            // Read meters (simulates UI polling)
            let _ = mixer.source_peak_db(id, 0);
            let _ = mixer.source_rms_db(id, 0);
            let _ = mixer.master_peak_db(0);
            let _ = mixer.master_rms_db(0);
            let _ = mixer.master_lufs();
        })
    });
}

criterion_group!(
    benches,
    bench_mix_single_source,
    bench_mix_multi_source,
    bench_mix_with_dsp,
    bench_master_limiter,
    bench_metering,
);
criterion_main!(benches);
