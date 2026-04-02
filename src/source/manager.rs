//! Multi-source video capture manager.
//!
//! Manages concurrent video sources with independent frame clocks.
//! Each source runs at its own target FPS; [`VideoCaptureManager::capture_all`]
//! polls only sources whose frame interval has elapsed.
//!
//! Follows the same pattern as [`crate::audio::AudioCaptureManager`].

use std::collections::HashMap;

use tracing::{debug, trace};

use super::{RawFrame, Source, SourceConfig, SourceId};
use crate::LayerId;
use crate::scene::{LayerContent, SceneGraph};

/// Events emitted by managed sources.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum SourceEvent {
    /// Source started producing frames.
    Started,
    /// Source stopped producing frames.
    Stopped,
    /// Source resolution changed.
    ResolutionChanged { width: u32, height: u32 },
}

/// A source with its frame clock and cached state.
struct ManagedSource {
    source: Box<dyn Source>,
    config: SourceConfig,
    target_fps: u32,
    frame_interval_us: u64,
    last_capture_us: u64,
    /// Last successfully captured frame — repeated when source FPS < compositor FPS.
    last_frame: Option<RawFrame>,
}

/// Manages multiple video sources with per-source frame clocks.
///
/// Each source is polled at its own target FPS. The compositor runs at its
/// own rate; sources that haven't reached their interval repeat their last frame.
///
/// # Examples
///
/// ```rust,no_run
/// use aethersafta::source::Source;
/// use aethersafta::source::manager::VideoCaptureManager;
/// use aethersafta::source::synthetic::{SyntheticSource, Pattern};
/// use aethersafta::source::SourceConfig;
///
/// let mut mgr = VideoCaptureManager::new();
/// let src = SyntheticSource::new("test", 1920, 1080, 30, Pattern::Gradient);
/// let id = src.id();
/// mgr.add_source(Box::new(src), SourceConfig::Screen { monitor: None }, 30);
/// let frames = mgr.capture_all(0);
/// assert!(frames.contains_key(&id));
/// ```
pub struct VideoCaptureManager {
    sources: HashMap<SourceId, ManagedSource>,
    events: Vec<(SourceId, SourceEvent)>,
}

impl VideoCaptureManager {
    /// Create an empty manager with no sources.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Add a source with its configuration and target FPS.
    ///
    /// Returns the source's ID.
    pub fn add_source(
        &mut self,
        source: Box<dyn Source>,
        config: SourceConfig,
        target_fps: u32,
    ) -> SourceId {
        let id = source.id();
        let fps = target_fps.max(1);
        let interval = 1_000_000u64 / fps as u64;
        debug!(source_id = %id, name = source.name(), fps, "added video source");
        self.events.push((id, SourceEvent::Started));
        self.sources.insert(
            id,
            ManagedSource {
                source,
                config,
                target_fps: fps,
                frame_interval_us: interval,
                last_capture_us: 0,
                last_frame: None,
            },
        );
        id
    }

    /// Remove a source. Returns `true` if it existed.
    pub fn remove_source(&mut self, id: SourceId) -> bool {
        if self.sources.remove(&id).is_some() {
            debug!(source_id = %id, "removed video source");
            self.events.push((id, SourceEvent::Stopped));
            true
        } else {
            false
        }
    }

    /// Capture frames from all sources whose interval has elapsed.
    ///
    /// `now_us` is the current compositor PTS in microseconds. Sources whose
    /// `frame_interval_us` hasn't elapsed since the last capture return their
    /// cached frame (frame repeat for sources slower than the compositor).
    #[must_use]
    pub fn capture_all(&mut self, now_us: u64) -> HashMap<SourceId, RawFrame> {
        let mut frames = HashMap::with_capacity(self.sources.len());

        for (&id, managed) in &mut self.sources {
            let elapsed = now_us.saturating_sub(managed.last_capture_us);
            let should_capture =
                elapsed >= managed.frame_interval_us || managed.last_frame.is_none();

            if should_capture {
                match managed.source.capture_frame() {
                    Ok(Some(frame)) => {
                        trace!(source_id = %id, pts = frame.pts_us, "captured frame");
                        managed.last_frame = Some(frame.clone());
                        managed.last_capture_us = now_us;
                        frames.insert(id, frame);
                    }
                    Ok(None) => {
                        // No frame available — use cached if we have one
                        if let Some(ref cached) = managed.last_frame {
                            frames.insert(id, cached.clone());
                        }
                    }
                    Err(e) => {
                        debug!(source_id = %id, error = %e, "capture error");
                        if let Some(ref cached) = managed.last_frame {
                            frames.insert(id, cached.clone());
                        }
                    }
                }
            } else if let Some(ref cached) = managed.last_frame {
                // Interval not elapsed — repeat last frame
                frames.insert(id, cached.clone());
            }
        }

        frames
    }

    /// Number of managed sources.
    #[must_use]
    #[inline]
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Whether a source exists in the manager.
    #[must_use]
    #[inline]
    pub fn has_source(&self, id: SourceId) -> bool {
        self.sources.contains_key(&id)
    }

    /// Get a source's configuration.
    #[must_use]
    pub fn get_config(&self, id: SourceId) -> Option<&SourceConfig> {
        self.sources.get(&id).map(|s| &s.config)
    }

    /// Get a source's target FPS.
    #[must_use]
    pub fn get_fps(&self, id: SourceId) -> Option<u32> {
        self.sources.get(&id).map(|s| s.target_fps)
    }

    /// Drain pending events (hot-plug notifications, resolution changes).
    pub fn drain_events(&mut self) -> Vec<(SourceId, SourceEvent)> {
        std::mem::take(&mut self.events)
    }

    /// Get all source IDs.
    #[must_use]
    pub fn source_ids(&self) -> Vec<SourceId> {
        self.sources.keys().copied().collect()
    }
}

impl Default for VideoCaptureManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Map captured source frames to layer IDs for the compositor.
///
/// Iterates scene layers, finds those with `LayerContent::Source { source_id }`,
/// and maps matching frames from the capture output.
#[must_use]
pub fn collect_layer_frames(
    scene: &SceneGraph,
    source_frames: &HashMap<SourceId, RawFrame>,
) -> HashMap<LayerId, RawFrame> {
    let mut layer_frames = HashMap::new();
    for layer in scene.layers() {
        if let LayerContent::Source { source_id } = &layer.content
            && let Some(frame) = source_frames.get(source_id)
        {
            layer_frames.insert(layer.id, frame.clone());
        }
    }
    layer_frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::synthetic::{Pattern, SyntheticSource};

    fn make_source(name: &str, fps: u32) -> SyntheticSource {
        SyntheticSource::new(name, 64, 64, fps, Pattern::Solid([255, 128, 64, 255]))
    }

    #[test]
    fn empty_manager() {
        let mut mgr = VideoCaptureManager::new();
        assert_eq!(mgr.source_count(), 0);
        let frames = mgr.capture_all(0);
        assert!(frames.is_empty());
    }

    #[test]
    fn add_and_remove_source() {
        let mut mgr = VideoCaptureManager::new();
        let src = make_source("test", 30);
        let id = src.id();
        mgr.add_source(Box::new(src), SourceConfig::Screen { monitor: None }, 30);
        assert_eq!(mgr.source_count(), 1);
        assert!(mgr.has_source(id));
        assert!(mgr.remove_source(id));
        assert_eq!(mgr.source_count(), 0);
        assert!(!mgr.has_source(id));
    }

    #[test]
    fn capture_all_returns_frames() {
        let mut mgr = VideoCaptureManager::new();
        let src = make_source("a", 30);
        let id = src.id();
        mgr.add_source(Box::new(src), SourceConfig::Screen { monitor: None }, 30);
        let frames = mgr.capture_all(0);
        assert_eq!(frames.len(), 1);
        assert!(frames.contains_key(&id));
        let frame = &frames[&id];
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 64);
        assert!(frame.is_valid());
    }

    #[test]
    fn multi_source_capture() {
        let mut mgr = VideoCaptureManager::new();
        let src_a = make_source("a", 30);
        let src_b = make_source("b", 60);
        let id_a = src_a.id();
        let id_b = src_b.id();
        mgr.add_source(Box::new(src_a), SourceConfig::Screen { monitor: None }, 30);
        mgr.add_source(
            Box::new(src_b),
            SourceConfig::Camera {
                device: "/dev/video0".into(),
            },
            60,
        );
        assert_eq!(mgr.source_count(), 2);
        let frames = mgr.capture_all(0);
        assert_eq!(frames.len(), 2);
        assert!(frames.contains_key(&id_a));
        assert!(frames.contains_key(&id_b));
    }

    #[test]
    fn per_source_fps_throttling() {
        let mut mgr = VideoCaptureManager::new();
        let src = make_source("slow", 10); // 10fps → 100000µs interval
        let id = src.id();
        mgr.add_source(Box::new(src), SourceConfig::Screen { monitor: None }, 10);

        // First capture at t=0
        let f1 = mgr.capture_all(0);
        assert!(f1.contains_key(&id));

        // At t=50ms — within interval, should repeat cached frame
        let f2 = mgr.capture_all(50_000);
        assert!(f2.contains_key(&id));

        // At t=100ms — interval elapsed, should capture fresh
        let f3 = mgr.capture_all(100_000);
        assert!(f3.contains_key(&id));
    }

    #[test]
    fn events_lifecycle() {
        let mut mgr = VideoCaptureManager::new();
        let src = make_source("events", 30);
        let id = src.id();
        mgr.add_source(Box::new(src), SourceConfig::Screen { monitor: None }, 30);

        let events = mgr.drain_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].1, SourceEvent::Started));
        assert_eq!(events[0].0, id);

        // Second drain should be empty
        assert!(mgr.drain_events().is_empty());

        mgr.remove_source(id);
        let events = mgr.drain_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].1, SourceEvent::Stopped));
    }

    #[test]
    fn remove_nonexistent() {
        let mut mgr = VideoCaptureManager::new();
        assert!(!mgr.remove_source(uuid::Uuid::new_v4()));
    }

    #[test]
    fn get_config_and_fps() {
        let mut mgr = VideoCaptureManager::new();
        let src = make_source("cfg", 24);
        let id = src.id();
        mgr.add_source(
            Box::new(src),
            SourceConfig::MediaFile {
                path: "test.mp4".into(),
            },
            24,
        );
        assert_eq!(mgr.get_fps(id), Some(24));
        assert!(matches!(
            mgr.get_config(id),
            Some(SourceConfig::MediaFile { .. })
        ));
    }

    #[test]
    fn collect_layer_frames_maps_correctly() {
        let mut scene = SceneGraph::new(64, 64, 30);
        let src = make_source("map", 30);
        let id = src.id();

        use crate::scene::Layer;
        let mut layer = Layer::new("src", LayerContent::Source { source_id: id });
        layer.size = Some((64, 64));
        let lid = layer.id;
        scene.add_layer(layer);

        // Also add a color fill — should not appear in output
        scene.add_layer(Layer::new(
            "bg",
            LayerContent::ColorFill {
                color: [0, 0, 0, 255],
            },
        ));

        let mut source_frames = HashMap::new();
        source_frames.insert(
            id,
            RawFrame {
                data: vec![0u8; 64 * 64 * 4].into(),
                format: crate::source::PixelFormat::Argb8888,
                width: 64,
                height: 64,
                pts_us: 0,
            },
        );

        let layer_frames = collect_layer_frames(&scene, &source_frames);
        assert_eq!(layer_frames.len(), 1);
        assert!(layer_frames.contains_key(&lid));
    }

    #[test]
    fn collect_layer_frames_missing_source() {
        let mut scene = SceneGraph::new(64, 64, 30);
        use crate::scene::Layer;
        let layer = Layer::new(
            "missing",
            LayerContent::Source {
                source_id: uuid::Uuid::new_v4(),
            },
        );
        scene.add_layer(layer);

        let source_frames = HashMap::new();
        let layer_frames = collect_layer_frames(&scene, &source_frames);
        assert!(layer_frames.is_empty());
    }

    #[test]
    fn multiple_layers_same_source() {
        let mut scene = SceneGraph::new(64, 64, 30);
        let src = make_source("shared", 30);
        let id = src.id();

        use crate::scene::Layer;
        let layer1 = Layer::new("pip1", LayerContent::Source { source_id: id });
        let layer2 = Layer::new("pip2", LayerContent::Source { source_id: id });
        let lid1 = layer1.id;
        let lid2 = layer2.id;
        scene.add_layer(layer1);
        scene.add_layer(layer2);

        let mut source_frames = HashMap::new();
        source_frames.insert(
            id,
            RawFrame {
                data: vec![0u8; 64 * 64 * 4].into(),
                format: crate::source::PixelFormat::Argb8888,
                width: 64,
                height: 64,
                pts_us: 0,
            },
        );

        let layer_frames = collect_layer_frames(&scene, &source_frames);
        assert_eq!(layer_frames.len(), 2);
        assert!(layer_frames.contains_key(&lid1));
        assert!(layer_frames.contains_key(&lid2));
    }
}
