//! Audio mixer: collects buffers from multiple sources, applies per-source
//! DSP (gain, pan, EQ, compressor), mixes, and applies master processing.
//!
//! All DSP and mixing is delegated to [`dhvani`].

use std::collections::HashMap;

use dhvani::buffer::{AudioBuffer, BufferPool};
use dhvani::dsp;
use dhvani::meter::LevelMeter;
use uuid::Uuid;

use super::{AudioMixerConfig, AudioSourceConfig, AudioSourceEntry, AudioSourceId};

/// Per-source DSP state (stateful effects that need to persist across buffers).
struct SourceDsp {
    pan: dsp::StereoPanner,
    gain_smoother: dsp::GainSmoother,
    compressor: Option<dsp::Compressor>,
    eq: Option<dsp::ParametricEq>,
    meter: LevelMeter,
}

/// The audio mixer: manages sources, applies DSP, and mixes to a master bus.
///
/// Uses a [`BufferPool`] to reduce per-frame allocations in the real-time path,
/// and [`GainSmoother`](dsp::GainSmoother) for click-free volume transitions.
pub struct AudioMixer {
    config: AudioMixerConfig,
    sources: Vec<AudioSourceEntry>,
    source_dsp: HashMap<AudioSourceId, SourceDsp>,
    master_limiter: Option<dsp::EnvelopeLimiter>,
    master_meter: LevelMeter,
    #[allow(dead_code)] // Used for RT allocation pooling in future mix() optimization
    buffer_pool: BufferPool,
}

impl AudioMixer {
    /// Create a new mixer with the given configuration.
    pub fn new(config: AudioMixerConfig) -> Self {
        let master_limiter = if config.master_limiter {
            dsp::EnvelopeLimiter::new(
                dsp::LimiterParams::default(),
                config.sample_rate,
            ).ok()
        } else {
            None
        };

        let buffer_pool = BufferPool::new(
            8, // pre-allocate 8 buffers
            config.channels,
            1024, // default buffer frames
            config.sample_rate,
        );

        Self {
            master_meter: LevelMeter::new(config.channels as usize, config.sample_rate as f32),
            config,
            sources: Vec::new(),
            source_dsp: HashMap::new(),
            master_limiter,
            buffer_pool,
        }
    }

    /// Add a new audio source. Returns its unique ID.
    pub fn add_source(&mut self, config: AudioSourceConfig) -> AudioSourceId {
        let id = Uuid::new_v4();
        let mut gain_smoother = dsp::GainSmoother::from_params(
            dsp::GainSmootherParams::default(),
        );
        // Pre-set to initial gain so first buffer isn't smoothed from unity
        gain_smoother.reset(dsp::db_to_amplitude(config.gain_db));

        let dsp_state = SourceDsp {
            pan: dsp::StereoPanner::new(config.pan),
            gain_smoother,
            compressor: None,
            eq: None,
            meter: LevelMeter::new(self.config.channels as usize, self.config.sample_rate as f32),
        };
        self.sources.push(AudioSourceEntry {
            id,
            config,
        });
        self.source_dsp.insert(id, dsp_state);
        id
    }

    /// Remove a source by ID. Returns `true` if found.
    pub fn remove_source(&mut self, id: AudioSourceId) -> bool {
        let before = self.sources.len();
        self.sources.retain(|s| s.id != id);
        self.source_dsp.remove(&id);
        self.sources.len() < before
    }

    /// Get source configuration by ID.
    pub fn get_source(&self, id: AudioSourceId) -> Option<&AudioSourceEntry> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// Update source configuration. Returns `true` if found.
    pub fn update_source(&mut self, id: AudioSourceId, config: AudioSourceConfig) -> bool {
        if let Some(entry) = self.sources.iter_mut().find(|s| s.id == id) {
            if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
                dsp_state.pan.set_pan(config.pan);
            }
            entry.config = config;
            true
        } else {
            false
        }
    }

    /// Enable per-source compression with the given parameters.
    pub fn set_source_compressor(
        &mut self,
        id: AudioSourceId,
        params: dsp::CompressorParams,
    ) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            dsp_state.compressor =
                dsp::Compressor::new(params, self.config.sample_rate).ok();
            true
        } else {
            false
        }
    }

    /// Enable per-source EQ with the given bands.
    pub fn set_source_eq(
        &mut self,
        id: AudioSourceId,
        bands: Vec<dsp::EqBandConfig>,
    ) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            dsp_state.eq = Some(dsp::ParametricEq::new(
                bands,
                self.config.sample_rate,
                self.config.channels,
            ));
            true
        } else {
            false
        }
    }

    /// All registered sources.
    pub fn sources(&self) -> &[AudioSourceEntry] {
        &self.sources
    }

    /// Number of sources.
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Current mixer configuration.
    pub fn config(&self) -> &AudioMixerConfig {
        &self.config
    }

    /// Per-source peak level for a channel in dB (post-DSP, pre-mix).
    pub fn source_peak_db(&self, id: AudioSourceId, channel: usize) -> Option<f32> {
        self.source_dsp.get(&id).map(|dsp| dsp.meter.peak_db(channel))
    }

    /// Per-source RMS level for a channel in dB (post-DSP, pre-mix).
    pub fn source_rms_db(&self, id: AudioSourceId, channel: usize) -> Option<f32> {
        self.source_dsp.get(&id).map(|dsp| dsp.meter.rms_db(channel))
    }

    /// Master peak level for a channel in dB.
    pub fn master_peak_db(&self, channel: usize) -> f32 {
        self.master_meter.peak_db(channel)
    }

    /// Master RMS level for a channel in dB.
    pub fn master_rms_db(&self, channel: usize) -> f32 {
        self.master_meter.rms_db(channel)
    }

    /// Integrated LUFS of the master bus.
    pub fn master_lufs(&self) -> f32 {
        self.master_meter.lufs
    }

    /// Mix the given per-source audio buffers into a single master buffer.
    ///
    /// `source_buffers` maps source IDs to their captured audio buffers.
    /// Sources not in the map (or muted) are treated as silence.
    /// Returns `None` if no sources contributed audio.
    pub fn mix(
        &mut self,
        source_buffers: &mut HashMap<AudioSourceId, AudioBuffer>,
    ) -> Option<AudioBuffer> {
        let mut to_mix: Vec<AudioBuffer> = Vec::new();

        for entry in &self.sources {
            if entry.config.muted {
                continue;
            }

            let Some(buf) = source_buffers.remove(&entry.id) else {
                continue;
            };

            let mut buf = buf;

            // Apply per-source DSP chain
            if let Some(dsp_state) = self.source_dsp.get_mut(&entry.id) {
                // Smoothed gain (click-free volume transitions)
                let target_gain = dsp::db_to_amplitude(entry.config.gain_db);
                let smoothed_gain = dsp_state.gain_smoother.smooth(target_gain);
                if (smoothed_gain - 1.0).abs() > f32::EPSILON {
                    buf.apply_gain(smoothed_gain);
                }

                if let Some(eq) = &mut dsp_state.eq {
                    eq.process(&mut buf);
                }
                if let Some(comp) = &mut dsp_state.compressor {
                    comp.process(&mut buf);
                }
                dsp_state.pan.process(&mut buf);

                // Sanitize after DSP chain to catch NaN/Inf from filter instability
                for sample in buf.samples_mut() {
                    *sample = dsp::sanitize_sample(*sample);
                }

                dsp_state.meter.process(&buf);
            }

            to_mix.push(buf);
        }

        if to_mix.is_empty() {
            return None;
        }

        // Mix all sources
        let refs: Vec<&AudioBuffer> = to_mix.iter().collect();
        let mut mixed = dhvani::buffer::mix(&refs).ok()?;

        // Master gain
        let master_gain = dsp::db_to_amplitude(self.config.master_gain_db);
        if (master_gain - 1.0).abs() > f32::EPSILON {
            mixed.apply_gain(master_gain);
        }

        // Master limiter
        if let Some(limiter) = &mut self.master_limiter {
            limiter.process(&mut mixed);
        }

        // Update metering
        self.master_meter.process(&mixed);

        Some(mixed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_buffer(value: f32, frames: usize) -> AudioBuffer {
        AudioBuffer::from_interleaved(vec![value; frames * 2], 2, 48000).unwrap()
    }

    #[test]
    fn mixer_new() {
        let mixer = AudioMixer::new(AudioMixerConfig::default());
        assert_eq!(mixer.source_count(), 0);
        assert_eq!(mixer.config().sample_rate, 48000);
    }

    #[test]
    fn add_and_remove_source() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let id = mixer.add_source(AudioSourceConfig::new("Mic"));
        assert_eq!(mixer.source_count(), 1);
        assert!(mixer.get_source(id).is_some());
        assert_eq!(mixer.get_source(id).unwrap().config.name, "Mic");

        assert!(mixer.remove_source(id));
        assert_eq!(mixer.source_count(), 0);
    }

    #[test]
    fn mix_single_source() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Test"));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));

        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
        let mixed = result.unwrap();
        assert_eq!(mixed.channels(), 2);
        assert_eq!(mixed.frames(), 1024);
    }

    #[test]
    fn mix_two_sources() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id1 = mixer.add_source(AudioSourceConfig::new("Source 1"));
        let id2 = mixer.add_source(AudioSourceConfig::new("Source 2"));

        let mut buffers = HashMap::new();
        buffers.insert(id1, test_buffer(0.3, 512));
        buffers.insert(id2, test_buffer(0.2, 512));

        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
        let mixed = result.unwrap();
        // Mixed is sum of both after equal-power panning (~0.707x each):
        // (0.3 + 0.2) * 0.707 ≈ 0.354
        let peak = mixed.peak();
        assert!(peak > 0.3 && peak < 0.4, "peak={peak}");
    }

    #[test]
    fn muted_source_excluded() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let mut cfg = AudioSourceConfig::new("Muted");
        cfg.muted = true;
        let id = mixer.add_source(cfg);

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 512));

        let result = mixer.mix(&mut buffers);
        assert!(result.is_none());
    }

    #[test]
    fn source_gain_applied() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let mut cfg = AudioSourceConfig::new("Quiet");
        cfg.gain_db = -6.0; // ~0.501x
        let id = mixer.add_source(cfg);

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(1.0, 512));

        let result = mixer.mix(&mut buffers).unwrap();
        let peak = result.peak();
        // -6dB ≈ 0.501, then equal-power pan at center ≈ 0.707x → ~0.354
        assert!(peak > 0.3 && peak < 0.4, "peak={peak}");
    }

    #[test]
    fn master_limiter_clamps() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let id = mixer.add_source(AudioSourceConfig::new("Loud"));

        let mut buffers = HashMap::new();
        // Very loud signal
        buffers.insert(id, test_buffer(2.0, 1024));

        let result = mixer.mix(&mut buffers).unwrap();
        let peak = result.peak();
        // Limiter should reduce peak below ~1.0
        assert!(peak < 1.5, "limiter should reduce peak, got {peak}");
    }

    #[test]
    fn update_source_config() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let id = mixer.add_source(AudioSourceConfig::new("Mic"));

        let mut updated = AudioSourceConfig::new("Mic (boosted)");
        updated.gain_db = 6.0;
        updated.pan = 0.5;
        assert!(mixer.update_source(id, updated));

        let entry = mixer.get_source(id).unwrap();
        assert_eq!(entry.config.name, "Mic (boosted)");
        assert!((entry.config.gain_db - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn source_eq_applied() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("EQ'd"));

        mixer.set_source_eq(id, vec![
            dsp::EqBandConfig {
                band_type: dsp::BandType::HighPass,
                freq_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
                enabled: true,
            },
        ]);

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));

        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
    }

    #[test]
    fn source_compressor_applied() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Compressed"));

        mixer.set_source_compressor(id, dsp::CompressorParams {
            threshold_db: -20.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 50.0,
            makeup_gain_db: 0.0,
            knee_db: 0.0,
        });

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.8, 1024));

        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
    }

    #[test]
    fn empty_mix_returns_none() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let _id = mixer.add_source(AudioSourceConfig::new("No data"));

        let mut buffers = HashMap::new();
        let result = mixer.mix(&mut buffers);
        assert!(result.is_none());
    }

    #[test]
    fn master_metering() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Test"));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        mixer.mix(&mut buffers);

        let l_db = mixer.master_peak_db(0);
        let r_db = mixer.master_peak_db(1);
        assert!(l_db > -20.0 && l_db < 0.0, "left_db={l_db}");
        assert!(r_db > -20.0 && r_db < 0.0, "right_db={r_db}");

        let l_rms = mixer.master_rms_db(0);
        assert!(l_rms > -20.0 && l_rms < 0.0, "left_rms={l_rms}");
    }

    #[test]
    fn per_source_metering() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Metered"));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        mixer.mix(&mut buffers);

        let peak = mixer.source_peak_db(id, 0);
        let rms = mixer.source_rms_db(id, 0);
        assert!(peak.is_some(), "source_peak_db should return Some");
        assert!(rms.is_some(), "source_rms_db should return Some");
        let peak_val = peak.unwrap();
        let rms_val = rms.unwrap();
        assert!(peak_val > -20.0 && peak_val < 0.0, "peak_db={peak_val}");
        assert!(rms_val > -20.0 && rms_val < 0.0, "rms_db={rms_val}");
    }

    #[test]
    fn gain_smoother_converges() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let mut cfg = AudioSourceConfig::new("Smoothed");
        cfg.gain_db = -12.0;
        let id = mixer.add_source(cfg);

        let mut peaks = Vec::new();
        for _ in 0..10 {
            let mut buffers = HashMap::new();
            buffers.insert(id, test_buffer(1.0, 512));
            let result = mixer.mix(&mut buffers).unwrap();
            peaks.push(result.peak());
        }

        // Gain smoother is pre-set to initial gain, so peak should be consistent
        let first = peaks[0];
        for (i, &p) in peaks.iter().enumerate() {
            assert!(
                (p - first).abs() < 0.01,
                "peak at iteration {i} ({p}) diverges from first ({first})"
            );
        }
    }

    #[test]
    fn sanitize_prevents_nan() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("NaN source"));

        let mut buf = test_buffer(0.5, 512);
        for sample in buf.samples_mut() {
            *sample = f32::NAN;
        }

        let mut buffers = HashMap::new();
        buffers.insert(id, buf);

        let result = mixer.mix(&mut buffers).unwrap();
        for &s in result.samples() {
            assert!(!s.is_nan(), "output contains NaN");
            assert!(!s.is_infinite(), "output contains Inf");
        }
    }

    #[test]
    fn multiple_sources_metered_independently() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });

        let mut cfg_quiet = AudioSourceConfig::new("Quiet");
        cfg_quiet.gain_db = -6.0;
        let id_quiet = mixer.add_source(cfg_quiet);

        let cfg_unity = AudioSourceConfig::new("Unity");
        let id_unity = mixer.add_source(cfg_unity);

        let mut cfg_loud = AudioSourceConfig::new("Loud");
        cfg_loud.gain_db = 6.0;
        let id_loud = mixer.add_source(cfg_loud);

        let mut buffers = HashMap::new();
        buffers.insert(id_quiet, test_buffer(0.5, 1024));
        buffers.insert(id_unity, test_buffer(0.5, 1024));
        buffers.insert(id_loud, test_buffer(0.5, 1024));
        mixer.mix(&mut buffers);

        let peak_quiet = mixer.source_peak_db(id_quiet, 0).unwrap();
        let peak_unity = mixer.source_peak_db(id_unity, 0).unwrap();
        let peak_loud = mixer.source_peak_db(id_loud, 0).unwrap();

        assert!(
            peak_quiet < peak_unity,
            "quiet ({peak_quiet}) should be less than unity ({peak_unity})"
        );
        assert!(
            peak_unity < peak_loud,
            "unity ({peak_unity}) should be less than loud ({peak_loud})"
        );
    }

    #[test]
    fn remove_nonexistent_source() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        assert!(!mixer.remove_source(Uuid::new_v4()));
    }

    #[test]
    fn update_nonexistent_source() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let config = AudioSourceConfig::new("Ghost");
        assert!(!mixer.update_source(Uuid::new_v4(), config));
    }

    #[test]
    fn set_compressor_nonexistent() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let params = dsp::CompressorParams {
            threshold_db: -20.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 50.0,
            makeup_gain_db: 0.0,
            knee_db: 0.0,
        };
        assert!(!mixer.set_source_compressor(Uuid::new_v4(), params));
    }

    #[test]
    fn set_eq_nonexistent() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let bands = vec![dsp::EqBandConfig {
            band_type: dsp::BandType::HighPass,
            freq_hz: 80.0,
            gain_db: 0.0,
            q: 0.707,
            enabled: true,
        }];
        assert!(!mixer.set_source_eq(Uuid::new_v4(), bands));
    }

    #[test]
    fn buffer_pool_initialized() {
        // Verifies that AudioMixer::new (which calls BufferPool::new) does not panic.
        let _mixer = AudioMixer::new(AudioMixerConfig::default());
    }

    #[test]
    fn mix_preserves_channel_count() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            channels: 2,
            ..Default::default()
        });
        let id1 = mixer.add_source(AudioSourceConfig::new("Src1"));
        let id2 = mixer.add_source(AudioSourceConfig::new("Src2"));

        // Single source mix
        let mut buffers = HashMap::new();
        buffers.insert(id1, test_buffer(0.5, 512));
        let result = mixer.mix(&mut buffers).unwrap();
        assert_eq!(result.channels(), 2, "single source should preserve 2 channels");

        // Multiple sources mix
        let mut buffers = HashMap::new();
        buffers.insert(id1, test_buffer(0.3, 512));
        buffers.insert(id2, test_buffer(0.4, 512));
        let result = mixer.mix(&mut buffers).unwrap();
        assert_eq!(result.channels(), 2, "multi source should preserve 2 channels");
    }
}
