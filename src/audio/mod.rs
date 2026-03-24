//! Audio capture, mixing, and DSP pipeline.
//!
//! Delegates to [`dhvani`] for all audio processing:
//! - Capture via PipeWire ([`dhvani::capture`])
//! - Mixing via [`dhvani::buffer::mix`]
//! - DSP effects via [`dhvani::dsp`]
//! - Metering via [`dhvani::meter`]
//! - A/V sync via [`dhvani::clock`]
//! - Graph-based routing via [`dhvani::graph`]
//!
//! # Architecture
//!
//! The audio pipeline supports two modes:
//!
//! **Manual mixer** ([`AudioMixer`]): Simple per-source DSP → mix → master chain.
//! Good for basic use cases with a fixed number of sources.
//!
//! **Graph pipeline** ([`AudioPipeline`]): Node-based routing via [`dhvani::graph`].
//! Supports dynamic source add/remove with real-time safe graph swaps.
//!
//! ```text
//! PipeWire ──► AudioCaptureManager ──► per-source DSP ──► Mixer ──► master DSP ──► output
//!              (multi-device)           (gain, EQ,         (graph     (compressor,
//!                                        compressor)       node)      limiter)
//! ```

pub mod capture;
pub mod graph;
pub mod mixer;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "pipewire")]
pub use capture::AudioCaptureManager;
pub use graph::AudioPipeline;
pub use mixer::AudioMixer;

/// Unique identifier for an audio source.
pub type AudioSourceId = Uuid;

/// Configuration for an audio source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSourceConfig {
    /// Human-readable name (e.g. "Desktop Audio", "Microphone").
    pub name: String,
    /// PipeWire device ID. `None` = default device.
    pub device_id: Option<u32>,
    /// Gain in dB applied before mixing (0.0 = unity).
    pub gain_db: f32,
    /// Whether this source is muted.
    pub muted: bool,
    /// Stereo pan: -1.0 (left) to 1.0 (right), 0.0 = center.
    pub pan: f32,
}

impl Default for AudioSourceConfig {
    fn default() -> Self {
        Self {
            name: "Audio".into(),
            device_id: None,
            gain_db: 0.0,
            muted: false,
            pan: 0.0,
        }
    }
}

impl AudioSourceConfig {
    /// Create a config with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
}

/// Configuration for the audio mixer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioMixerConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Output channels (1 = mono, 2 = stereo).
    pub channels: u32,
    /// Master gain in dB.
    pub master_gain_db: f32,
    /// Whether to apply a limiter on the master bus.
    pub master_limiter: bool,
}

impl Default for AudioMixerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            master_gain_db: 0.0,
            master_limiter: true,
        }
    }
}

/// Per-source DSP effect type, used with [`AudioMixer::clear_source_effect`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceEffect {
    /// Parametric EQ.
    Eq,
    /// 10-band graphic EQ.
    GraphicEq,
    /// Dynamic range compressor.
    Compressor,
    /// Sibilance reduction.
    DeEsser,
    /// Reverb (Schroeder/Freeverb).
    Reverb,
    /// Fixed delay with feedback.
    Delay,
    /// Noise gate (amplitude threshold).
    NoiseGate,
}

/// A managed audio source with its ID and configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSourceEntry {
    /// Unique source ID.
    pub id: AudioSourceId,
    /// Source configuration.
    pub config: AudioSourceConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_config_default() {
        let cfg = AudioSourceConfig::default();
        assert_eq!(cfg.gain_db, 0.0);
        assert!(!cfg.muted);
        assert_eq!(cfg.pan, 0.0);
        assert!(cfg.device_id.is_none());
    }

    #[test]
    fn source_config_new() {
        let cfg = AudioSourceConfig::new("Desktop Audio");
        assert_eq!(cfg.name, "Desktop Audio");
    }

    #[test]
    fn mixer_config_default() {
        let cfg = AudioMixerConfig::default();
        assert_eq!(cfg.sample_rate, 48000);
        assert_eq!(cfg.channels, 2);
        assert!(cfg.master_limiter);
    }

    #[test]
    fn source_config_serde() {
        let cfg = AudioSourceConfig {
            name: "Mic".into(),
            device_id: Some(42),
            gain_db: -3.0,
            muted: false,
            pan: 0.5,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AudioSourceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "Mic");
        assert_eq!(back.device_id, Some(42));
        assert!((back.gain_db - (-3.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn mixer_config_serde() {
        let cfg = AudioMixerConfig {
            sample_rate: 44100,
            channels: 1,
            master_gain_db: -6.0,
            master_limiter: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AudioMixerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sample_rate, 44100);
        assert!(!back.master_limiter);
    }
}
