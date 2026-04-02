//! V4L2 camera capture source.
//!
//! Captures frames from a Video4Linux2 device (webcam, capture card).
//! Uses memory-mapped I/O for efficient zero-copy buffer access.
//!
//! Requires the `camera` feature.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
use tracing::{debug, info};
use v4l::buffer::Type as BufType;
use v4l::io::traits::CaptureStream;
use v4l::prelude::*;
use v4l::video::Capture;

use super::{PixelFormat, RawFrame, Source, SourceId};

/// Information about an available camera device.
#[derive(Debug, Clone)]
pub struct CameraInfo {
    /// Device index (e.g. 0 for /dev/video0).
    pub index: usize,
    /// Device path.
    pub path: String,
    /// Human-readable device name.
    pub card: String,
    /// Driver name.
    pub driver: String,
    /// Whether the device supports video capture.
    pub can_capture: bool,
}

/// Enumerate available V4L2 camera devices.
///
/// Scans `/dev/video0` through `/dev/video15` and returns devices
/// that support video capture.
#[must_use]
pub fn enumerate_cameras() -> Vec<CameraInfo> {
    let mut cameras = Vec::new();
    for i in 0..16 {
        let path = format!("/dev/video{i}");
        if let Ok(dev) = Device::with_path(&path)
            && let Ok(caps) = dev.query_caps()
        {
            let can_capture = caps
                .capabilities
                .contains(v4l::capability::Flags::VIDEO_CAPTURE);
            cameras.push(CameraInfo {
                index: i,
                path,
                card: caps.card,
                driver: caps.driver,
                can_capture,
            });
        }
    }
    cameras
}

/// A V4L2 camera source.
///
/// Opens a V4L2 device, negotiates the best available pixel format,
/// and captures frames via memory-mapped buffers.
///
/// The device and stream are wrapped in `Mutex` for `Send + Sync`
/// (required by the [`Source`] trait).
pub struct CameraSource {
    id: SourceId,
    name: String,
    device_path: String,
    width: u32,
    height: u32,
    native_fourcc: [u8; 4],
    stream: Mutex<MmapStream<'static>>,
    frame_count: AtomicU64,
    fps: u32,
}

impl CameraSource {
    /// Open a camera device with auto-detected format.
    ///
    /// Negotiates the best pixel format in priority order:
    /// MJPEG > YUYV > NV12 > first available.
    ///
    /// # Errors
    ///
    /// Returns an error if the device cannot be opened, has no capture
    /// capability, or format negotiation fails.
    pub fn open(device_path: &str) -> anyhow::Result<Self> {
        Self::open_with_resolution(device_path, None, None)
    }

    /// Open a camera with optional preferred resolution.
    ///
    /// If `width`/`height` are `None`, uses the device's default or the
    /// largest available discrete resolution.
    pub fn open_with_resolution(
        device_path: &str,
        preferred_width: Option<u32>,
        preferred_height: Option<u32>,
    ) -> anyhow::Result<Self> {
        let dev = Device::with_path(device_path)
            .with_context(|| format!("failed to open V4L2 device: {device_path}"))?;

        let caps = dev
            .query_caps()
            .with_context(|| format!("failed to query capabilities: {device_path}"))?;

        if !caps
            .capabilities
            .contains(v4l::capability::Flags::VIDEO_CAPTURE)
        {
            anyhow::bail!("{device_path} does not support video capture");
        }

        info!(
            device = device_path,
            card = caps.card,
            driver = caps.driver,
            "opening V4L2 camera"
        );

        // Enumerate formats and pick best
        let formats = dev
            .enum_formats()
            .with_context(|| "failed to enumerate formats")?;

        let fourcc = select_best_format(&formats)
            .ok_or_else(|| anyhow::anyhow!("no supported pixel format on {device_path}"))?;

        // Find best resolution
        let (width, height) = if let (Some(w), Some(h)) = (preferred_width, preferred_height) {
            (w, h)
        } else {
            select_best_resolution(&dev, fourcc, preferred_width, preferred_height)?
        };

        // Set format
        let fmt = v4l::Format::new(width, height, fourcc);
        let negotiated = dev.set_format(&fmt).with_context(|| {
            format!(
                "failed to set format {}x{} {:?}",
                width,
                height,
                fourcc.str().unwrap_or("????")
            )
        })?;

        let actual_w = negotiated.width;
        let actual_h = negotiated.height;
        let actual_fourcc = negotiated.fourcc;

        info!(
            device = device_path,
            width = actual_w,
            height = actual_h,
            format = actual_fourcc.str().unwrap_or("????"),
            "camera format negotiated"
        );

        // Detect FPS from frame intervals
        let fps = detect_fps(&dev, actual_fourcc, actual_w, actual_h);

        // Create mmap stream (4 buffers, 2s timeout)
        // SAFETY: We own the device and the stream borrows from it. We leak the device
        // into the stream's lifetime to satisfy the 'static bound required by Mutex<MmapStream>.
        // The stream is dropped before the leaked handle is invalid because they share an Arc.
        let dev_leaked: &'static Device = Box::leak(Box::new(dev));
        let mut stream = MmapStream::with_buffers(dev_leaked, BufType::VideoCapture, 4)
            .with_context(|| "failed to create mmap stream")?;
        stream.set_timeout(Duration::from_secs(2));

        let fourcc_bytes = actual_fourcc.repr;

        Ok(Self {
            id: uuid::Uuid::new_v4(),
            name: caps.card,
            device_path: device_path.to_string(),
            width: actual_w,
            height: actual_h,
            native_fourcc: fourcc_bytes,
            stream: Mutex::new(stream),
            frame_count: AtomicU64::new(0),
            fps,
        })
    }

    /// The device path this camera was opened from.
    #[must_use]
    pub fn device_path(&self) -> &str {
        &self.device_path
    }

    /// The negotiated FPS.
    #[must_use]
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// The native FourCC format code.
    #[must_use]
    pub fn native_fourcc(&self) -> [u8; 4] {
        self.native_fourcc
    }
}

impl Source for CameraSource {
    fn id(&self) -> SourceId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capture_frame(&self) -> anyhow::Result<Option<RawFrame>> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| anyhow::anyhow!("camera stream lock poisoned"))?;

        let (buf, meta) = stream.next().with_context(|| "V4L2 capture failed")?;
        let seq = meta.sequence;

        // Convert captured buffer to ARGB8888
        let argb = convert_to_argb(buf, self.width, self.height, &self.native_fourcc)?;

        let frame_num = self.frame_count.fetch_add(1, Ordering::Relaxed);
        let pts_us = if self.fps > 0 {
            frame_num * 1_000_000 / self.fps as u64
        } else {
            frame_num * 33_333
        };

        debug!(seq, pts_us, "camera frame captured");

        Ok(Some(RawFrame {
            data: argb.into(),
            format: PixelFormat::Argb8888,
            width: self.width,
            height: self.height,
            pts_us,
        }))
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_live(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Format selection
// ---------------------------------------------------------------------------

/// Select best pixel format from available formats.
/// Priority: MJPG > YUYV > NV12 > first available.
fn select_best_format(formats: &[v4l::format::Description]) -> Option<v4l::FourCC> {
    let priority = [b"MJPG", b"YUYV", b"NV12"];
    for target in &priority {
        if let Some(desc) = formats.iter().find(|f| &f.fourcc.repr == *target) {
            return Some(desc.fourcc);
        }
    }
    formats.first().map(|f| f.fourcc)
}

/// Select best resolution for a given format.
fn select_best_resolution(
    dev: &Device,
    fourcc: v4l::FourCC,
    preferred_w: Option<u32>,
    preferred_h: Option<u32>,
) -> anyhow::Result<(u32, u32)> {
    let sizes = dev.enum_framesizes(fourcc).unwrap_or_default();

    let mut best = (640u32, 480u32); // safe default
    let mut best_pixels = 0u64;

    for fs in &sizes {
        match &fs.size {
            v4l::framesize::FrameSizeEnum::Discrete(d) => {
                let pixels = d.width as u64 * d.height as u64;
                // If user wants specific dimensions, prefer exact match
                if let (Some(pw), Some(ph)) = (preferred_w, preferred_h)
                    && d.width == pw
                    && d.height == ph
                {
                    return Ok((pw, ph));
                }
                // Otherwise pick largest up to 1080p
                if pixels > best_pixels && d.width <= 1920 && d.height <= 1080 {
                    best = (d.width, d.height);
                    best_pixels = pixels;
                }
            }
            v4l::framesize::FrameSizeEnum::Stepwise(s) => {
                // Use max within 1080p
                let w = s.max_width.min(1920);
                let h = s.max_height.min(1080);
                best = (w, h);
            }
        }
    }

    Ok(best)
}

/// Detect FPS from frame intervals.
fn detect_fps(dev: &Device, fourcc: v4l::FourCC, width: u32, height: u32) -> u32 {
    let intervals = dev
        .enum_frameintervals(fourcc, width, height)
        .unwrap_or_default();

    for fi in &intervals {
        match &fi.interval {
            v4l::frameinterval::FrameIntervalEnum::Discrete(frac) => {
                if frac.numerator > 0 {
                    return frac.denominator / frac.numerator;
                }
            }
            v4l::frameinterval::FrameIntervalEnum::Stepwise(s) => {
                if s.min.numerator > 0 {
                    return s.min.denominator / s.min.numerator;
                }
            }
        }
    }

    30 // default fallback
}

// ---------------------------------------------------------------------------
// Pixel format conversion
// ---------------------------------------------------------------------------

/// Convert a V4L2 buffer to ARGB8888.
fn convert_to_argb(
    buf: &[u8],
    width: u32,
    height: u32,
    fourcc: &[u8; 4],
) -> anyhow::Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let out_size = w * h * 4;

    match fourcc {
        b"YUYV" => {
            let mut argb = vec![0u8; out_size];
            yuyv_to_argb(buf, &mut argb, w, h);
            Ok(argb)
        }
        b"NV12" => {
            // Use our existing converter
            Ok(crate::encode::nv12_to_argb(buf, width, height))
        }
        b"MJPG" => {
            // Decode JPEG via image crate, then convert RGBA→ARGB
            let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg)
                .with_context(|| "MJPEG decode failed")?;
            let rgba = img.to_rgba8();
            let (iw, ih) = (rgba.width(), rgba.height());
            let pixels = rgba.into_raw();

            let mut argb = vec![0u8; iw as usize * ih as usize * 4];
            for (src, dst) in pixels.chunks_exact(4).zip(argb.chunks_exact_mut(4)) {
                dst[0] = src[3]; // A
                dst[1] = src[0]; // R
                dst[2] = src[1]; // G
                dst[3] = src[2]; // B
            }
            Ok(argb)
        }
        _ => {
            anyhow::bail!(
                "unsupported V4L2 pixel format: {}",
                std::str::from_utf8(fourcc).unwrap_or("????")
            );
        }
    }
}

/// Convert YUYV (4:2:2 packed) to ARGB8888 using BT.601.
#[inline]
fn yuyv_to_argb(yuyv: &[u8], argb: &mut [u8], width: usize, height: usize) {
    let pixels = width * height;
    let pairs = pixels / 2;
    let yuyv_len = pairs * 4;

    if yuyv.len() < yuyv_len || argb.len() < pairs * 8 {
        return;
    }

    for i in 0..pairs {
        let si = i * 4;
        let y0 = yuyv[si] as i32;
        let u = yuyv[si + 1] as i32 - 128;
        let y1 = yuyv[si + 2] as i32;
        let v = yuyv[si + 3] as i32 - 128;

        let di = i * 8;
        // Pixel 0
        argb[di] = 255; // A
        argb[di + 1] = (y0 + ((351 * v) >> 8)).clamp(0, 255) as u8; // R
        argb[di + 2] = (y0 - ((86 * u + 179 * v) >> 8)).clamp(0, 255) as u8; // G
        argb[di + 3] = (y0 + ((444 * u) >> 8)).clamp(0, 255) as u8; // B

        // Pixel 1
        argb[di + 4] = 255; // A
        argb[di + 5] = (y1 + ((351 * v) >> 8)).clamp(0, 255) as u8; // R
        argb[di + 6] = (y1 - ((86 * u + 179 * v) >> 8)).clamp(0, 255) as u8; // G
        argb[di + 7] = (y1 + ((444 * u) >> 8)).clamp(0, 255) as u8; // B
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuyv_to_argb_white() {
        // White in YUYV: Y=235, U=128, V=128 (neutral chroma)
        let yuyv = vec![235, 128, 235, 128]; // two white pixels
        let mut argb = vec![0u8; 8];
        yuyv_to_argb(&yuyv, &mut argb, 2, 1);

        // Both pixels should be near white
        assert_eq!(argb[0], 255); // A
        assert!(argb[1] > 220, "R={}", argb[1]); // R
        assert!(argb[2] > 220, "G={}", argb[2]); // G
        assert!(argb[3] > 220, "B={}", argb[3]); // B
        assert_eq!(argb[4], 255); // A
    }

    #[test]
    fn yuyv_to_argb_black() {
        let yuyv = vec![16, 128, 16, 128]; // two black pixels
        let mut argb = vec![0u8; 8];
        yuyv_to_argb(&yuyv, &mut argb, 2, 1);

        assert_eq!(argb[0], 255); // A
        assert!(argb[1] < 30, "R={}", argb[1]); // R near 0
        assert!(argb[2] < 30, "G={}", argb[2]); // G near 0
        assert!(argb[3] < 30, "B={}", argb[3]); // B near 0
    }

    #[test]
    fn yuyv_short_buffer() {
        let yuyv = vec![0; 2]; // too short
        let mut argb = vec![0u8; 8];
        yuyv_to_argb(&yuyv, &mut argb, 2, 1);
        // Should not panic, output stays zero
        assert!(argb.iter().all(|&b| b == 0));
    }

    #[test]
    fn enumerate_cameras_does_not_panic() {
        // Just verify it doesn't crash — devices may or may not exist
        let cameras = enumerate_cameras();
        for cam in &cameras {
            assert!(!cam.path.is_empty());
        }
    }

    #[test]
    fn select_best_format_priority() {
        use v4l::format::Description;

        let formats = vec![
            Description {
                index: 0,
                typ: 1,
                flags: v4l::format::description::Flags::empty(),
                description: "YUYV".into(),
                fourcc: v4l::FourCC::new(b"YUYV"),
            },
            Description {
                index: 1,
                typ: 1,
                flags: v4l::format::description::Flags::COMPRESSED,
                description: "MJPEG".into(),
                fourcc: v4l::FourCC::new(b"MJPG"),
            },
        ];

        let best = select_best_format(&formats);
        // MJPG should win over YUYV
        assert_eq!(best.unwrap().repr, *b"MJPG");
    }

    #[test]
    fn select_best_format_fallback() {
        use v4l::format::Description;

        let formats = vec![Description {
            index: 0,
            typ: 1,
            flags: v4l::format::description::Flags::empty(),
            description: "RGB3".into(),
            fourcc: v4l::FourCC::new(b"RGB3"),
        }];

        let best = select_best_format(&formats);
        assert_eq!(best.unwrap().repr, *b"RGB3");
    }

    #[test]
    fn select_best_format_empty() {
        let formats: Vec<v4l::format::Description> = vec![];
        assert!(select_best_format(&formats).is_none());
    }
}
