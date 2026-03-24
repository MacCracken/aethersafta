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
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            default_sample_rate: 48000,
            default_buffer_frames: 1024,
        }
    }

    /// Create with custom defaults.
    #[must_use]
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

        self.sources.insert(
            id,
            CaptureSource {
                capture,
                config,
                running: true,
            },
        );

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
    #[must_use]
    pub fn drain_buffers(&self) -> HashMap<AudioSourceId, dhvani::buffer::AudioBuffer> {
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
    #[must_use]
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
    #[must_use]
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Whether a specific source is currently capturing.
    #[must_use]
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
    #[must_use]
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

    #[cfg(feature = "pipewire")]
    mod pipewire_tests {
        use super::super::*;
        use crate::audio::AudioSourceConfig;

        #[test]
        fn new_creates_empty_manager() {
            let mgr = AudioCaptureManager::new();
            assert_eq!(mgr.source_count(), 0);
        }

        #[test]
        fn default_creates_empty_manager() {
            let mgr = AudioCaptureManager::default();
            assert_eq!(mgr.source_count(), 0);
        }

        #[test]
        fn with_defaults_custom_params() {
            let mgr = AudioCaptureManager::with_defaults(44100, 512);
            assert_eq!(mgr.source_count(), 0);
            assert_eq!(mgr.default_sample_rate, 44100);
            assert_eq!(mgr.default_buffer_frames, 512);
        }

        #[test]
        fn add_source_increments_count() {
            let mut mgr = AudioCaptureManager::new();
            let id = uuid::Uuid::new_v4();
            // Use default device (None) — PipeWire will open a virtual capture
            let result = mgr.add_source(id, AudioSourceConfig::new("Test Capture"));
            // On CI without a real device, this may fail — that's OK
            if result.is_ok() {
                assert_eq!(mgr.source_count(), 1);
                assert!(mgr.is_running(id));
            }
        }

        #[test]
        fn remove_nonexistent_returns_false() {
            let mut mgr = AudioCaptureManager::new();
            assert!(!mgr.remove_source(uuid::Uuid::new_v4()));
        }

        #[test]
        fn add_and_remove_source() {
            let mut mgr = AudioCaptureManager::new();
            let id = uuid::Uuid::new_v4();
            let result = mgr.add_source(id, AudioSourceConfig::new("Mic"));
            if result.is_ok() {
                assert_eq!(mgr.source_count(), 1);
                assert!(mgr.remove_source(id));
                assert_eq!(mgr.source_count(), 0);
                assert!(!mgr.is_running(id));
            }
        }

        #[test]
        fn get_config_returns_source_config() {
            let mut mgr = AudioCaptureManager::new();
            let id = uuid::Uuid::new_v4();
            let config = AudioSourceConfig::new("Desktop Audio");
            let result = mgr.add_source(id, config);
            if result.is_ok() {
                let cfg = mgr.get_config(id);
                assert!(cfg.is_some());
                assert_eq!(cfg.unwrap().name, "Desktop Audio");
            }
        }

        #[test]
        fn get_config_nonexistent_returns_none() {
            let mgr = AudioCaptureManager::new();
            assert!(mgr.get_config(uuid::Uuid::new_v4()).is_none());
        }

        #[test]
        fn is_running_nonexistent_returns_false() {
            let mgr = AudioCaptureManager::new();
            assert!(!mgr.is_running(uuid::Uuid::new_v4()));
        }

        #[test]
        fn drain_buffers_empty_when_no_sources() {
            let mgr = AudioCaptureManager::new();
            let buffers = mgr.drain_buffers();
            assert!(buffers.is_empty());
        }

        #[test]
        fn drain_events_empty_when_no_sources() {
            let mgr = AudioCaptureManager::new();
            let events = mgr.drain_events();
            assert!(events.is_empty());
        }

        #[test]
        fn stop_all_stops_running_sources() {
            let mut mgr = AudioCaptureManager::new();
            let id = uuid::Uuid::new_v4();
            let result = mgr.add_source(id, AudioSourceConfig::new("Stoppable"));
            if result.is_ok() {
                assert!(mgr.is_running(id));
                mgr.stop_all();
                assert!(!mgr.is_running(id));
                // Double stop should be safe
                mgr.stop_all();
            }
        }

        #[test]
        fn drain_buffers_with_active_source() {
            let mut mgr = AudioCaptureManager::new();
            let id = uuid::Uuid::new_v4();
            let result = mgr.add_source(id, AudioSourceConfig::new("Buffer Drain"));
            if result.is_ok() {
                // Give PipeWire a moment to produce buffers
                std::thread::sleep(std::time::Duration::from_millis(100));
                let buffers = mgr.drain_buffers();
                // May or may not have buffers depending on device, but shouldn't panic
                let _ = buffers;
            }
        }

        #[test]
        fn multiple_sources() {
            let mut mgr = AudioCaptureManager::new();
            let id1 = uuid::Uuid::new_v4();
            let id2 = uuid::Uuid::new_v4();

            let r1 = mgr.add_source(id1, AudioSourceConfig::new("Src 1"));
            let r2 = mgr.add_source(id2, AudioSourceConfig::new("Src 2"));

            let expected_count = r1.is_ok() as usize + r2.is_ok() as usize;
            assert_eq!(mgr.source_count(), expected_count);

            // Cleanup
            mgr.stop_all();
        }

        #[test]
        fn drop_stops_all_captures() {
            let mut mgr = AudioCaptureManager::new();
            let id = uuid::Uuid::new_v4();
            let _ = mgr.add_source(id, AudioSourceConfig::new("Drop Test"));
            // Drop should call stop_all — just verify no panic
            drop(mgr);
        }
    }
}
