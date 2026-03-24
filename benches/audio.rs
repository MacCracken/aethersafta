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
                mix: 1.0,
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
                mix: 1.0,
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

    // Noise gate only
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Gate"));
        mixer.set_source_noise_gate(id, 0.01);

        group.bench_function("noise_gate", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // De-esser only
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("DeEss"));
        mixer.set_source_deesser(
            id,
            dhvani::dsp::DeEsserParams {
                freq_hz: 6000.0,
                threshold_db: -20.0,
                reduction_db: 6.0,
                q: 1.0,
            },
        );

        group.bench_function("deesser", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // Graphic EQ only
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("GEQ"));
        mixer.set_source_graphic_eq(
            id,
            dhvani::dsp::GraphicEqSettings {
                enabled: true,
                bands: [3.0, 1.0, 0.0, -1.0, 0.0, 2.0, 0.0, -2.0, 1.0, -3.0],
            },
        );

        group.bench_function("graphic_eq_10band", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // Reverb only
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Rev"));
        mixer.set_source_reverb(
            id,
            dhvani::dsp::ReverbParams {
                room_size: 0.8,
                damping: 0.5,
                mix: 0.3,
            },
        );

        group.bench_function("reverb", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // Delay only
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Dly"));
        mixer.set_source_delay(id, 50.0, 0.3, 0.5);

        group.bench_function("delay", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
                mixer.mix(&mut buffers)
            })
        });
    }

    // Full expanded chain: gate + EQ + comp + deesser + delay + reverb + limiter
    {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: true,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("AllFx"));
        mixer.set_source_noise_gate(id, 0.01);
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
                threshold_db: -20.0,
                ratio: 4.0,
                attack_ms: 5.0,
                release_ms: 50.0,
                makeup_gain_db: 0.0,
                knee_db: 6.0,
                mix: 1.0,
            },
        );
        mixer.set_source_deesser(
            id,
            dhvani::dsp::DeEsserParams {
                freq_hz: 6000.0,
                threshold_db: -20.0,
                reduction_db: 6.0,
                q: 1.0,
            },
        );
        mixer.set_source_delay(id, 10.0, 0.2, 0.3);
        mixer.set_source_reverb(
            id,
            dhvani::dsp::ReverbParams {
                room_size: 0.5,
                damping: 0.5,
                mix: 0.2,
            },
        );

        group.bench_function("full_chain_all_effects", |b| {
            b.iter(|| {
                let mut buffers = HashMap::new();
                buffers.insert(id, test_buffer(0.5, 1024));
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

// ---------------------------------------------------------------------------
// Graph-based AudioPipeline
// ---------------------------------------------------------------------------

fn bench_graph_pipeline(c: &mut Criterion) {
    use aethersafta::audio::AudioPipeline;

    let mut group = c.benchmark_group("graph_pipeline");

    for &n_sources in &[1, 4, 8] {
        group.bench_function(format!("{n_sources}_source"), |b| {
            let mut pipeline = AudioPipeline::new(AudioMixerConfig {
                master_limiter: false,
                ..Default::default()
            });
            for i in 0..n_sources {
                let id = uuid::Uuid::from_u128(i as u128);
                pipeline.add_source(id, 1.0, 0.0);
            }

            b.iter(|| pipeline.process())
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Buffer pool: acquire / fill / mix / release cycle
// ---------------------------------------------------------------------------

fn bench_buffer_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("buffer_pool");

    group.bench_function("acquire_release_cycle", |b| {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Src"));

        b.iter(|| {
            let mut buffers = HashMap::new();
            buffers.insert(id, test_buffer(0.5, 1024));
            let output = mixer.mix(&mut buffers);
            drop(output);
        })
    });

    group.bench_function("acquire_release_8_sources", |b| {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let ids: Vec<_> = (0..8)
            .map(|i| mixer.add_source(AudioSourceConfig::new(format!("Src-{i}"))))
            .collect();

        b.iter(|| {
            let mut buffers = HashMap::new();
            for &id in &ids {
                buffers.insert(id, test_buffer(0.5, 1024));
            }
            let output = mixer.mix(&mut buffers);
            drop(output);
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Mix throughput across buffer sizes
// ---------------------------------------------------------------------------

fn bench_mix_buffer_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("mix_buffer_sizes");

    for &frames in &[64, 128, 256, 512, 1024, 2048, 4096] {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Src"));

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

criterion_group!(
    benches,
    bench_mix_single_source,
    bench_mix_multi_source,
    bench_mix_with_dsp,
    bench_master_limiter,
    bench_metering,
    bench_graph_pipeline,
    bench_buffer_pool,
    bench_mix_buffer_sizes,
);
criterion_main!(benches);
