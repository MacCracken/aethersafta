//! Multi-source audio capture manager.
//!
//! Manages concurrent [`dhvani::capture::PwCapture`] instances for capturing
//! audio from multiple PipeWire devices simultaneously (system audio, mic, per-app).

use std::collections::HashMap;

#[cfg(feature = "pipewire")]
use dhvani::capture::{CaptureConfig, CaptureEvent, PwCapture};

use super::{AudioSourceConfig, AudioSourceId};

/// State for a single capture source.
#[cfg(feature = "pipewire")]
struct CaptureSource {
    capture: PwCapture,
    config: AudioSourceConfig,
    running: bool,
}

/// Manages concurrent audio capture from multiple PipeWire devices.
///
/// Each source gets its own [`PwCapture`] instance with independent
/// sample rates and buffer sizes. Buffers are collected per-source
/// and fed into the [`AudioMixer`](super::AudioMixer) for mixing.
#[cfg(feature = "pipewire")]
pub struct AudioCaptureManager {
    sources: HashMap<AudioSourceId, CaptureSource>,
    default_sample_rate: u32,
    default_buffer_frames: u32,
}

#[cfg(feature = "pipewire")]
impl AudioCaptureManager {
    /// Create a new capture manager.
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            default_sample_rate: 48000,
            default_buffer_frames: 1024,
        }
    }

    /// Create with custom defaults.
    pub fn with_defaults(sample_rate: u32, buffer_frames: u32) -> Self {
        Self {
            sources: HashMap::new(),
            default_sample_rate: sample_rate,
            default_buffer_frames: buffer_frames,
        }
    }

    /// Add and start capturing from a new audio source.
    ///
    /// Returns the source ID on success.
    pub fn add_source(
        &mut self,
        id: AudioSourceId,
        config: AudioSourceConfig,
    ) -> anyhow::Result<()> {
        let capture_config = CaptureConfig {
            device_id: config.device_id,
            sample_rate: self.default_sample_rate,
            channels: 2,
            buffer_frames: self.default_buffer_frames,
        };

        let mut capture = PwCapture::new(capture_config)?;
        capture.start()?;

        self.sources.insert(id, CaptureSource {
            capture,
            config,
            running: true,
        });

        Ok(())
    }

    /// Remove a capture source. Stops capture automatically.
    pub fn remove_source(&mut self, id: AudioSourceId) -> bool {
        if let Some(mut source) = self.sources.remove(&id) {
            if source.running {
                let _ = source.capture.stop();
            }
            true
        } else {
            false
        }
    }

    /// Collect all available audio buffers from all sources.
    ///
    /// Returns a map of source ID → latest audio buffer, suitable for
    /// passing directly to [`AudioMixer::mix`](super::AudioMixer::mix).
    pub fn drain_buffers(
        &self,
    ) -> HashMap<AudioSourceId, dhvani::buffer::AudioBuffer> {
        let mut buffers = HashMap::new();

        for (&id, source) in &self.sources {
            if !source.running {
                continue;
            }
            // Drain all pending buffers, keep only the latest
            let mut latest = None;
            while let Some(buf) = source.capture.try_recv() {
                latest = Some(buf);
            }
            if let Some(buf) = latest {
                buffers.insert(id, buf);
            }
        }

        buffers
    }

    /// Collect hot-plug events from all sources.
    pub fn drain_events(&self) -> Vec<(AudioSourceId, CaptureEvent)> {
        let mut events = Vec::new();
        for (&id, source) in &self.sources {
            while let Some(event) = source.capture.try_recv_event() {
                events.push((id, event));
            }
        }
        events
    }

    /// Number of active capture sources.
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Whether a specific source is currently capturing.
    pub fn is_running(&self, id: AudioSourceId) -> bool {
        self.sources
            .get(&id)
            .is_some_and(|s| s.running && s.capture.is_running())
    }

    /// Stop all captures.
    pub fn stop_all(&mut self) {
        for source in self.sources.values_mut() {
            if source.running {
                let _ = source.capture.stop();
                source.running = false;
            }
        }
    }

    /// Get the config for a source.
    pub fn get_config(&self, id: AudioSourceId) -> Option<&AudioSourceConfig> {
        self.sources.get(&id).map(|s| &s.config)
    }
}

#[cfg(feature = "pipewire")]
impl Default for AudioCaptureManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "pipewire")]
impl Drop for AudioCaptureManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn capture_manager_requires_pipewire_feature() {
        // Compile-time check: AudioCaptureManager only available with pipewire feature.
        // This test just verifies the module compiles cleanly.
    }
}
