//! Static image source (PNG/JPEG).
//!
//! Loads an image file and converts it to an ARGB8888 [`RawFrame`].
//! Returns the same frame on every [`capture_frame`] call.

use std::path::{Path, PathBuf};

use image::ImageReader;
use uuid::Uuid;

use super::{PixelFormat, RawFrame, Source, SourceId};

/// A source that produces frames from a static image file.
#[derive(Debug, Clone)]
pub struct ImageSource {
    id: SourceId,
    name: String,
    path: PathBuf,
    frame: RawFrame,
}

impl ImageSource {
    /// Load an image from the given path and convert to ARGB8888.
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let img = ImageReader::open(path)?.decode()?;
        let rgba = img.into_rgba8();
        let (width, height) = rgba.dimensions();

        // Convert RGBA → ARGB (swap alpha to front)
        let pixels = rgba.into_raw();
        let mut argb = Vec::with_capacity(pixels.len());
        for chunk in pixels.chunks_exact(4) {
            argb.push(chunk[3]); // A
            argb.push(chunk[0]); // R
            argb.push(chunk[1]); // G
            argb.push(chunk[2]); // B
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image")
            .to_string();

        Ok(Self {
            id: Uuid::new_v4(),
            name,
            path: path.to_path_buf(),
            frame: RawFrame {
                data: argb,
                format: PixelFormat::Argb8888,
                width,
                height,
                pts_us: 0,
            },
        })
    }

    /// Create an ImageSource directly from ARGB8888 pixel data (for testing).
    pub fn from_raw(name: impl Into<String>, width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            path: PathBuf::new(),
            frame: RawFrame {
                data,
                format: PixelFormat::Argb8888,
                width,
                height,
                pts_us: 0,
            },
        }
    }

    /// The file path this image was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Source for ImageSource {
    fn id(&self) -> SourceId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capture_frame(&self) -> anyhow::Result<Option<RawFrame>> {
        Ok(Some(self.frame.clone()))
    }

    fn resolution(&self) -> (u32, u32) {
        (self.frame.width, self.frame.height)
    }

    fn is_live(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_raw_valid() {
        let data = vec![255u8; 10 * 10 * 4]; // 10x10 white opaque
        let src = ImageSource::from_raw("test", 10, 10, data);
        assert_eq!(src.name(), "test");
        assert_eq!(src.resolution(), (10, 10));
        assert!(!src.is_live());

        let frame = src.capture_frame().unwrap().unwrap();
        assert!(frame.is_valid());
    }

    #[test]
    fn capture_returns_same_frame() {
        let data = vec![0u8; 4 * 4 * 4];
        let src = ImageSource::from_raw("repeat", 4, 4, data);
        let f1 = src.capture_frame().unwrap().unwrap();
        let f2 = src.capture_frame().unwrap().unwrap();
        assert_eq!(f1.data, f2.data);
    }

    #[test]
    fn open_nonexistent_fails() {
        assert!(ImageSource::open("/nonexistent/image.png").is_err());
    }
}
