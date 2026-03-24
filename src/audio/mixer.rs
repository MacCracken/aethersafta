//! Audio mixer: collects buffers from multiple sources, applies per-source
//! DSP (gain, pan, EQ, compressor), mixes, and applies master processing.
//!
//! All DSP and mixing is delegated to [`dhvani`].

use std::collections::HashMap;

use dhvani::buffer::{AudioBuffer, BufferPool};
use dhvani::dsp;
use dhvani::meter::LevelMeter;
use tracing;
use uuid::Uuid;

use super::{AudioMixerConfig, AudioSourceConfig, AudioSourceEntry, AudioSourceId};

/// Per-source DSP state (stateful effects that need to persist across buffers).
struct SourceDsp {
    pan: dsp::StereoPanner,
    gain_smoother: dsp::GainSmoother,
    compressor: Option<dsp::Compressor>,
    eq: Option<dsp::ParametricEq>,
    graphic_eq: Option<dsp::GraphicEq>,
    deesser: Option<dsp::DeEsser>,
    reverb: Option<dsp::Reverb>,
    delay: Option<dsp::DelayLine>,
    noise_gate_threshold: Option<f32>,
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
    #[must_use]
    pub fn new(config: AudioMixerConfig) -> Self {
        let master_limiter = if config.master_limiter {
            dsp::EnvelopeLimiter::new(dsp::LimiterParams::default(), config.sample_rate).ok()
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
        let mut gain_smoother = dsp::GainSmoother::from_params(dsp::GainSmootherParams::default());
        // Pre-set to initial gain so first buffer isn't smoothed from unity
        gain_smoother.reset(dsp::db_to_amplitude(config.gain_db));

        let dsp_state = SourceDsp {
            pan: dsp::StereoPanner::new(config.pan),
            gain_smoother,
            compressor: None,
            eq: None,
            graphic_eq: None,
            deesser: None,
            reverb: None,
            delay: None,
            noise_gate_threshold: None,
            meter: LevelMeter::new(
                self.config.channels as usize,
                self.config.sample_rate as f32,
            ),
        };
        tracing::debug!(source_id = %id, name = %config.name, "mixer: source added");
        self.sources.push(AudioSourceEntry { id, config });
        self.source_dsp.insert(id, dsp_state);
        id
    }

    /// Remove a source by ID. Returns `true` if found.
    pub fn remove_source(&mut self, id: AudioSourceId) -> bool {
        let before = self.sources.len();
        self.sources.retain(|s| s.id != id);
        self.source_dsp.remove(&id);
        let removed = self.sources.len() < before;
        if removed {
            tracing::debug!(source_id = %id, "mixer: source removed");
        }
        removed
    }

    /// Get source configuration by ID.
    #[must_use]
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
            dsp_state.compressor = dsp::Compressor::new(params, self.config.sample_rate).ok();
            true
        } else {
            false
        }
    }

    /// Enable per-source EQ with the given bands.
    pub fn set_source_eq(&mut self, id: AudioSourceId, bands: Vec<dsp::EqBandConfig>) -> bool {
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

    /// Enable per-source graphic EQ with the given settings.
    pub fn set_source_graphic_eq(
        &mut self,
        id: AudioSourceId,
        settings: dsp::GraphicEqSettings,
    ) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            let mut geq = dsp::GraphicEq::new(self.config.sample_rate, self.config.channels);
            geq.set_settings(settings);
            dsp_state.graphic_eq = Some(geq);
            true
        } else {
            false
        }
    }

    /// Enable per-source de-esser with the given parameters.
    pub fn set_source_deesser(&mut self, id: AudioSourceId, params: dsp::DeEsserParams) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            dsp_state.deesser =
                dsp::DeEsser::new(params, self.config.sample_rate, self.config.channels).ok();
            true
        } else {
            false
        }
    }

    /// Enable per-source reverb with the given parameters.
    pub fn set_source_reverb(&mut self, id: AudioSourceId, params: dsp::ReverbParams) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            dsp_state.reverb = dsp::Reverb::new(params, self.config.sample_rate).ok();
            true
        } else {
            false
        }
    }

    /// Enable per-source delay with the given parameters.
    ///
    /// `delay_ms` is the delay time (clamped to `0.1..=5000.0`), `feedback`
    /// controls echo repetition (clamped to `0.0..=0.95` to prevent infinite
    /// buildup), `mix` controls dry/wet balance (clamped to `0.0..=1.0`).
    pub fn set_source_delay(
        &mut self,
        id: AudioSourceId,
        delay_ms: f32,
        feedback: f32,
        mix: f32,
    ) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            let delay_ms = delay_ms.clamp(0.1, 5000.0);
            let feedback = feedback.clamp(0.0, 0.95);
            let mix = mix.clamp(0.0, 1.0);
            dsp_state.delay = Some(dsp::DelayLine::new(
                delay_ms,
                delay_ms * 2.0,
                feedback,
                mix,
                self.config.sample_rate,
                self.config.channels,
            ));
            true
        } else {
            false
        }
    }

    /// Enable per-source noise gate. Samples below the threshold (in linear
    /// amplitude, e.g. 0.01) are silenced. Threshold is clamped to `0.0..=1.0`.
    pub fn set_source_noise_gate(&mut self, id: AudioSourceId, threshold: f32) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            dsp_state.noise_gate_threshold = Some(threshold.clamp(0.0, 1.0));
            true
        } else {
            false
        }
    }

    /// Disable a specific per-source effect. Pass the effect name:
    /// `"compressor"`, `"eq"`, `"graphic_eq"`, `"deesser"`, `"reverb"`,
    /// `"delay"`, `"noise_gate"`.
    pub fn clear_source_effect(&mut self, id: AudioSourceId, effect: &str) -> bool {
        if let Some(dsp_state) = self.source_dsp.get_mut(&id) {
            match effect {
                "compressor" => dsp_state.compressor = None,
                "eq" => dsp_state.eq = None,
                "graphic_eq" => dsp_state.graphic_eq = None,
                "deesser" => dsp_state.deesser = None,
                "reverb" => dsp_state.reverb = None,
                "delay" => dsp_state.delay = None,
                "noise_gate" => dsp_state.noise_gate_threshold = None,
                _ => return false,
            }
            true
        } else {
            false
        }
    }

    /// All registered sources.
    #[must_use]
    pub fn sources(&self) -> &[AudioSourceEntry] {
        &self.sources
    }

    /// Number of sources.
    #[must_use]
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Current mixer configuration.
    #[must_use]
    pub fn config(&self) -> &AudioMixerConfig {
        &self.config
    }

    /// Per-source peak level for a channel in dB (post-DSP, pre-mix).
    #[must_use]
    pub fn source_peak_db(&self, id: AudioSourceId, channel: usize) -> Option<f32> {
        self.source_dsp
            .get(&id)
            .map(|dsp| dsp.meter.peak_db(channel))
    }

    /// Per-source RMS level for a channel in dB (post-DSP, pre-mix).
    #[must_use]
    pub fn source_rms_db(&self, id: AudioSourceId, channel: usize) -> Option<f32> {
        self.source_dsp
            .get(&id)
            .map(|dsp| dsp.meter.rms_db(channel))
    }

    /// Master peak level for a channel in dB.
    #[must_use]
    pub fn master_peak_db(&self, channel: usize) -> f32 {
        self.master_meter.peak_db(channel)
    }

    /// Master RMS level for a channel in dB.
    #[must_use]
    pub fn master_rms_db(&self, channel: usize) -> f32 {
        self.master_meter.rms_db(channel)
    }

    /// Integrated LUFS of the master bus.
    #[must_use]
    pub fn master_lufs(&self) -> f32 {
        self.master_meter.lufs
    }

    /// Mix the given per-source audio buffers into a single master buffer.
    ///
    /// `source_buffers` maps source IDs to their captured audio buffers.
    /// Sources not in the map (or muted) are treated as silence.
    /// Returns `None` if no sources contributed audio.
    #[inline]
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

            // Apply per-source DSP chain:
            //   gain → EQ → noise gate → compressor → de-esser
            //   → delay → reverb → pan → sanitize → meter
            //
            // EQ before gate so HP filter removes rumble that would hold gate open.
            if let Some(dsp_state) = self.source_dsp.get_mut(&entry.id) {
                // Smoothed gain (click-free volume transitions)
                let target_gain = dsp::db_to_amplitude(entry.config.gain_db);
                let smoothed_gain = dsp_state.gain_smoother.smooth(target_gain);
                if (smoothed_gain - 1.0).abs() > f32::EPSILON {
                    buf.apply_gain(smoothed_gain);
                }

                // EQ (parametric or graphic — parametric takes priority if both set)
                if let Some(eq) = &mut dsp_state.eq {
                    eq.process(&mut buf);
                } else if let Some(geq) = &mut dsp_state.graphic_eq {
                    geq.process(&mut buf);
                }

                // Noise gate after EQ (HP filter removes rumble first)
                if let Some(threshold) = dsp_state.noise_gate_threshold {
                    dsp::noise_gate(&mut buf, threshold);
                }

                // Dynamics
                if let Some(comp) = &mut dsp_state.compressor {
                    comp.process(&mut buf);
                }
                if let Some(deesser) = &mut dsp_state.deesser {
                    deesser.process(&mut buf);
                }

                // Time-based effects
                if let Some(delay) = &mut dsp_state.delay {
                    delay.process(&mut buf);
                }
                if let Some(reverb) = &mut dsp_state.reverb {
                    reverb.process(&mut buf);
                }

                // Spatial
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

        mixer.set_source_eq(
            id,
            vec![dsp::EqBandConfig {
                band_type: dsp::BandType::HighPass,
                freq_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
                enabled: true,
            }],
        );

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

        mixer.set_source_compressor(
            id,
            dsp::CompressorParams {
                threshold_db: -20.0,
                ratio: 4.0,
                attack_ms: 5.0,
                release_ms: 50.0,
                makeup_gain_db: 0.0,
                knee_db: 0.0,
                mix: 1.0,
            },
        );

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
            mix: 1.0,
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
        assert_eq!(
            result.channels(),
            2,
            "single source should preserve 2 channels"
        );

        // Multiple sources mix
        let mut buffers = HashMap::new();
        buffers.insert(id1, test_buffer(0.3, 512));
        buffers.insert(id2, test_buffer(0.4, 512));
        let result = mixer.mix(&mut buffers).unwrap();
        assert_eq!(
            result.channels(),
            2,
            "multi source should preserve 2 channels"
        );
    }

    #[test]
    fn noise_gate_silences_below_threshold() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Gated"));
        mixer.set_source_noise_gate(id, 0.3);

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.1, 1024)); // below threshold
        let result = mixer.mix(&mut buffers).unwrap();
        // All samples should be silenced
        assert!(
            result.peak() < 0.01,
            "noise gate should silence signal below threshold, peak={}",
            result.peak()
        );
    }

    #[test]
    fn deesser_processes_without_error() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("DeEssed"));
        assert!(mixer.set_source_deesser(
            id,
            dsp::DeEsserParams {
                freq_hz: 6000.0,
                threshold_db: -20.0,
                reduction_db: 6.0,
                q: 1.0,
            },
        ));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
    }

    #[test]
    fn graphic_eq_processes_without_error() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("GEQ"));
        let settings = dsp::GraphicEqSettings {
            enabled: true,
            bands: [3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -3.0],
        };
        assert!(mixer.set_source_graphic_eq(id, settings));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
    }

    #[test]
    fn reverb_processes_without_error() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Reverbed"));
        assert!(mixer.set_source_reverb(
            id,
            dsp::ReverbParams {
                room_size: 0.8,
                damping: 0.5,
                mix: 0.3,
            },
        ));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
    }

    #[test]
    fn delay_processes_without_error() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Delayed"));
        assert!(mixer.set_source_delay(id, 50.0, 0.3, 0.5));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        let result = mixer.mix(&mut buffers);
        assert!(result.is_some());
    }

    #[test]
    fn clear_source_effect_works() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Fx"));
        mixer.set_source_noise_gate(id, 0.1);
        assert!(mixer.clear_source_effect(id, "noise_gate"));
        assert!(!mixer.clear_source_effect(id, "nonexistent"));
        assert!(!mixer.clear_source_effect(Uuid::new_v4(), "noise_gate"));
    }

    #[test]
    fn full_dsp_chain() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: true,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Full Chain"));

        // Enable everything
        mixer.set_source_noise_gate(id, 0.01);
        mixer.set_source_eq(
            id,
            vec![dsp::EqBandConfig {
                band_type: dsp::BandType::HighPass,
                freq_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
                enabled: true,
            }],
        );
        mixer.set_source_compressor(
            id,
            dsp::CompressorParams {
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
            dsp::DeEsserParams {
                freq_hz: 6000.0,
                threshold_db: -20.0,
                reduction_db: 6.0,
                q: 1.0,
            },
        );
        mixer.set_source_delay(id, 10.0, 0.2, 0.3);
        mixer.set_source_reverb(
            id,
            dsp::ReverbParams {
                room_size: 0.5,
                damping: 0.5,
                mix: 0.2,
            },
        );

        // Run multiple mix cycles to exercise stateful effects
        for _ in 0..5 {
            let mut buffers = HashMap::new();
            buffers.insert(id, test_buffer(0.5, 1024));
            let result = mixer.mix(&mut buffers);
            assert!(result.is_some());
            let mixed = result.unwrap();
            // No NaN/Inf from the full chain
            for &s in mixed.samples() {
                assert!(!s.is_nan(), "full chain produced NaN");
                assert!(!s.is_infinite(), "full chain produced Inf");
            }
        }
    }

    #[test]
    fn sanitize_prevents_inf() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Inf source"));

        let mut buf = test_buffer(0.5, 512);
        for sample in buf.samples_mut() {
            *sample = f32::INFINITY;
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
    fn eq_mutual_exclusion() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("EQ Test"));

        // Set both parametric and graphic EQ
        mixer.set_source_eq(
            id,
            vec![dsp::EqBandConfig {
                band_type: dsp::BandType::HighPass,
                freq_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
                enabled: true,
            }],
        );
        mixer.set_source_graphic_eq(
            id,
            dsp::GraphicEqSettings {
                enabled: true,
                bands: [6.0; 10],
            },
        );

        // Both set — should still process without error (parametric wins)
        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        assert!(mixer.mix(&mut buffers).is_some());

        // Clear parametric — graphic should take over
        mixer.clear_source_effect(id, "eq");
        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        assert!(mixer.mix(&mut buffers).is_some());
    }

    #[test]
    fn effect_toggle_clear_and_readd() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Toggle"));

        // Add reverb, mix, clear, mix, re-add, mix
        mixer.set_source_reverb(
            id,
            dsp::ReverbParams {
                room_size: 0.8,
                damping: 0.5,
                mix: 0.5,
            },
        );
        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        assert!(mixer.mix(&mut buffers).is_some());

        mixer.clear_source_effect(id, "reverb");
        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        assert!(mixer.mix(&mut buffers).is_some());

        // Re-add with different params — should be fresh state
        mixer.set_source_reverb(
            id,
            dsp::ReverbParams {
                room_size: 0.3,
                damping: 0.8,
                mix: 0.2,
            },
        );
        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        let result = mixer.mix(&mut buffers).unwrap();
        for &s in result.samples() {
            assert!(!s.is_nan(), "re-added reverb produced NaN");
        }
    }

    #[test]
    fn compressor_reduces_loud_signal() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Comp Check"));
        mixer.set_source_compressor(
            id,
            dsp::CompressorParams {
                threshold_db: -20.0,
                ratio: 8.0,
                attack_ms: 0.1,
                release_ms: 10.0,
                makeup_gain_db: 0.0,
                knee_db: 0.0,
                mix: 1.0,
            },
        );

        // Run several cycles so compressor converges
        let mut last_peak = 0.0_f32;
        for _ in 0..10 {
            let mut buffers = HashMap::new();
            buffers.insert(id, test_buffer(0.9, 1024));
            let result = mixer.mix(&mut buffers).unwrap();
            last_peak = result.peak();
        }
        // 0.9 input → after pan (~0.636) → above -20dB threshold → compressed
        // Should be measurably less than uncompressed peak
        assert!(
            last_peak < 0.7,
            "compressor should reduce peak, got {last_peak}"
        );
    }

    #[test]
    fn delay_feedback_clamped() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Delay Clamp"));
        // feedback > 1.0 should be clamped to 0.95
        mixer.set_source_delay(id, 10.0, 5.0, 0.5);

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        let result = mixer.mix(&mut buffers).unwrap();
        // Should not blow up despite extreme feedback input
        assert!(
            result.peak() < 2.0,
            "clamped feedback should prevent blowup"
        );
    }

    #[test]
    fn master_gain_applied() {
        let mut mixer = AudioMixer::new(AudioMixerConfig {
            master_limiter: false,
            master_gain_db: -6.0,
            ..Default::default()
        });
        let id = mixer.add_source(AudioSourceConfig::new("Src"));

        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.8, 1024));
        let result = mixer.mix(&mut buffers).unwrap();
        let peak = result.peak();
        // -6dB master gain should attenuate to ~half, plus pan factor
        assert!(peak < 0.5, "master gain should attenuate, peak={peak}");
    }

    #[test]
    fn clear_all_effect_types() {
        let mut mixer = AudioMixer::new(AudioMixerConfig::default());
        let id = mixer.add_source(AudioSourceConfig::new("AllFx"));

        // Set all effects
        mixer.set_source_noise_gate(id, 0.01);
        mixer.set_source_eq(
            id,
            vec![dsp::EqBandConfig {
                band_type: dsp::BandType::HighPass,
                freq_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
                enabled: true,
            }],
        );
        mixer.set_source_graphic_eq(
            id,
            dsp::GraphicEqSettings {
                enabled: true,
                bands: [0.0; 10],
            },
        );
        mixer.set_source_compressor(
            id,
            dsp::CompressorParams {
                threshold_db: -20.0,
                ratio: 4.0,
                attack_ms: 5.0,
                release_ms: 50.0,
                makeup_gain_db: 0.0,
                knee_db: 0.0,
                mix: 1.0,
            },
        );
        mixer.set_source_deesser(
            id,
            dsp::DeEsserParams {
                freq_hz: 6000.0,
                threshold_db: -20.0,
                reduction_db: 6.0,
                q: 1.0,
            },
        );
        mixer.set_source_delay(id, 10.0, 0.2, 0.3);
        mixer.set_source_reverb(
            id,
            dsp::ReverbParams {
                room_size: 0.5,
                damping: 0.5,
                mix: 0.2,
            },
        );

        // Clear all individually
        for effect in &[
            "noise_gate",
            "eq",
            "graphic_eq",
            "compressor",
            "deesser",
            "delay",
            "reverb",
        ] {
            assert!(
                mixer.clear_source_effect(id, effect),
                "clearing {effect} should succeed"
            );
        }

        // Mix should still work with all effects cleared
        let mut buffers = HashMap::new();
        buffers.insert(id, test_buffer(0.5, 1024));
        assert!(mixer.mix(&mut buffers).is_some());
    }
}
