//! Input sources: screen capture, camera, media files, images.

pub mod image;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique source identifier.
pub type SourceId = Uuid;

/// A raw uncompressed frame from a source.
#[derive(Debug, Clone)]
pub struct RawFrame {
    /// Frame data in ARGB8888 format.
    pub data: Vec<u8>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Presentation timestamp in microseconds.
    pub pts_us: u64,
}

impl RawFrame {
    /// Expected byte length for the given dimensions (4 bytes per pixel).
    pub fn expected_size(width: u32, height: u32) -> usize {
        width as usize * height as usize * 4
    }

    /// Whether the data length matches the expected size.
    pub fn is_valid(&self) -> bool {
        self.data.len() == Self::expected_size(self.width, self.height)
    }
}

/// The `Source` trait: anything that can produce frames.
pub trait Source: Send + Sync {
    /// Unique identifier for this source.
    fn id(&self) -> SourceId;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Capture the current frame. Returns `None` if no frame is available.
    fn capture_frame(&self) -> anyhow::Result<Option<RawFrame>>;

    /// Native resolution of this source.
    fn resolution(&self) -> (u32, u32);

    /// Whether this source is currently producing frames.
    fn is_live(&self) -> bool;
}

/// Source type descriptor (for serialisation / scene saving).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceConfig {
    /// Full screen or specific monitor.
    Screen { monitor: Option<u32> },
    /// Camera device.
    Camera { device: String },
    /// Media file playback.
    MediaFile { path: String },
    /// Static image.
    Image { path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_frame_valid() {
        let frame = RawFrame {
            data: vec![0u8; 1920 * 1080 * 4],
            width: 1920,
            height: 1080,
            pts_us: 0,
        };
        assert!(frame.is_valid());
    }

    #[test]
    fn raw_frame_invalid_size() {
        let frame = RawFrame {
            data: vec![0u8; 100],
            width: 1920,
            height: 1080,
            pts_us: 0,
        };
        assert!(!frame.is_valid());
    }

    #[test]
    fn expected_size() {
        assert_eq!(RawFrame::expected_size(1920, 1080), 1920 * 1080 * 4);
        assert_eq!(RawFrame::expected_size(3840, 2160), 3840 * 2160 * 4);
    }
}
