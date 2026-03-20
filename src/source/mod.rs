//! Input sources: screen capture, camera, media files, images.

pub mod image;
pub mod synthetic;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique source identifier.
pub type SourceId = Uuid;

/// Pixel format of a raw frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    /// 4 bytes per pixel: [A, R, G, B].
    Argb8888,
    /// Semi-planar YUV 4:2:0: Y plane (w*h) + interleaved UV plane (w*h/2).
    Nv12,
}

/// A raw uncompressed frame from a source.
#[derive(Debug, Clone)]
pub struct RawFrame {
    /// Raw pixel data.
    pub data: Vec<u8>,
    /// Pixel format.
    pub format: PixelFormat,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Presentation timestamp in microseconds.
    pub pts_us: u64,
}

impl RawFrame {
    /// Expected byte length for ARGB8888 at the given dimensions.
    pub fn expected_size(width: u32, height: u32) -> usize {
        Self::expected_size_for(PixelFormat::Argb8888, width, height)
    }

    /// Expected byte length for a given format and dimensions.
    pub fn expected_size_for(format: PixelFormat, width: u32, height: u32) -> usize {
        match format {
            PixelFormat::Argb8888 => width as usize * height as usize * 4,
            // Y plane: w*h, UV plane: w*(h/2) interleaved
            PixelFormat::Nv12 => {
                let w = width as usize;
                let h = height as usize;
                w * h + w * (h / 2)
            }
        }
    }

    /// Whether the data length matches the expected size for this frame's format.
    pub fn is_valid(&self) -> bool {
        self.data.len() == Self::expected_size_for(self.format, self.width, self.height)
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
            format: PixelFormat::Argb8888,
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
            format: PixelFormat::Argb8888,
            width: 1920,
            height: 1080,
            pts_us: 0,
        };
        assert!(!frame.is_valid());
    }

    #[test]
    fn nv12_expected_size() {
        // 1920x1080 NV12: Y=1920*1080 + UV=1920*540 = 2073600 + 1036800 = 3110400
        assert_eq!(
            RawFrame::expected_size_for(PixelFormat::Nv12, 1920, 1080),
            1920 * 1080 + 1920 * 540
        );
    }

    #[test]
    fn nv12_frame_valid() {
        let size = RawFrame::expected_size_for(PixelFormat::Nv12, 4, 4);
        let frame = RawFrame {
            data: vec![128u8; size],
            format: PixelFormat::Nv12,
            width: 4,
            height: 4,
            pts_us: 0,
        };
        assert!(frame.is_valid());
    }

    #[test]
    fn expected_size() {
        assert_eq!(RawFrame::expected_size(1920, 1080), 1920 * 1080 * 4);
        assert_eq!(RawFrame::expected_size(3840, 2160), 3840 * 2160 * 4);
    }
}
