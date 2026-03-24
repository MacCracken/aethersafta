//! Static image source (PNG/JPEG).
//!
//! Loads an image file and converts it to an ARGB8888 [`RawFrame`].
//! Returns the same frame on every [`Source::capture_frame`] call.

use std::path::{Path, PathBuf};

use bytes::Bytes;
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

        // Convert RGBA → ARGB via ranga
        let pixels = rgba.into_raw();
        let rgba_buf =
            ranga::pixel::PixelBuffer::new(pixels, width, height, ranga::pixel::PixelFormat::Rgba8)
                .map_err(|e| anyhow::anyhow!("pixel buffer: {e}"))?;
        let argb_buf = ranga::convert::rgba8_to_argb8(&rgba_buf)
            .map_err(|e| anyhow::anyhow!("RGBA→ARGB: {e}"))?;
        let argb = argb_buf.data;

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
                data: Bytes::from(argb),
                format: PixelFormat::Argb8888,
                width,
                height,
                pts_us: 0,
            },
        })
    }

    /// Create an ImageSource directly from ARGB8888 pixel data (for testing).
    #[must_use]
    pub fn from_raw(name: impl Into<String>, width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            path: PathBuf::new(),
            frame: RawFrame {
                data: Bytes::from(data),
                format: PixelFormat::Argb8888,
                width,
                height,
                pts_us: 0,
            },
        }
    }

    /// The file path this image was loaded from.
    #[must_use]
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

    #[test]
    fn open_valid_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("red.png");
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        img.save(&path).unwrap();

        let src = ImageSource::open(&path).expect("should load valid PNG");
        assert_eq!(src.name(), "red.png");
        assert_eq!(src.resolution(), (4, 4));
        assert_eq!(src.path(), path.as_path());
        assert!(!src.is_live());

        let frame = src.capture_frame().unwrap().unwrap();
        assert_eq!(frame.format, PixelFormat::Argb8888);
        assert!(frame.is_valid());
        assert_eq!(frame.width, 4);
        assert_eq!(frame.height, 4);
        // ARGB: 4*4 pixels * 4 bytes
        assert_eq!(frame.data.len(), 4 * 4 * 4);
    }

    #[test]
    fn open_valid_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blue.jpg");
        // JPEG requires RGB (no alpha channel support)
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 255]));
        img.save(&path).unwrap();

        let src = ImageSource::open(&path).expect("should load valid JPEG");
        assert_eq!(src.name(), "blue.jpg");
        assert_eq!(src.resolution(), (8, 8));

        let frame = src.capture_frame().unwrap().unwrap();
        assert_eq!(frame.format, PixelFormat::Argb8888);
        assert!(frame.is_valid());
    }

    #[test]
    fn open_corrupt_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.png");
        std::fs::write(&path, b"not a valid image").unwrap();

        assert!(ImageSource::open(&path).is_err());
    }

    #[test]
    fn open_png_argb_pixel_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pixel.png");
        // Single red pixel: RGBA = [255, 0, 0, 255]
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        img.save(&path).unwrap();

        let src = ImageSource::open(&path).unwrap();
        let frame = src.capture_frame().unwrap().unwrap();
        // After RGBA→ARGB conversion: [A, R, G, B] = [255, 255, 0, 0]
        assert_eq!(frame.data.len(), 4);
        assert_eq!(frame.data[0], 255); // A
        assert_eq!(frame.data[1], 255); // R
        assert_eq!(frame.data[2], 0); // G
        assert_eq!(frame.data[3], 0); // B
    }

    #[test]
    fn path_accessor_from_raw() {
        let src = ImageSource::from_raw("test", 1, 1, vec![0u8; 4]);
        assert_eq!(src.path(), Path::new(""));
    }

    #[test]
    fn id_is_unique() {
        let a = ImageSource::from_raw("a", 1, 1, vec![0u8; 4]);
        let b = ImageSource::from_raw("b", 1, 1, vec![0u8; 4]);
        assert_ne!(a.id(), b.id());
    }
}
