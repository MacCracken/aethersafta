//! Synthetic frame source for testing and benchmarking.
//!
//! Generates deterministic pattern frames (gradient, solid, checkerboard)
//! without any hardware or file dependencies.

use std::sync::atomic::{AtomicU64, Ordering};

use uuid::Uuid;

use super::{PixelFormat, RawFrame, Source, SourceId};

/// Pattern type for synthetic frame generation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum Pattern {
    /// Solid color fill (ARGB).
    Solid([u8; 4]),
    /// Horizontal RGB gradient.
    Gradient,
    /// Checkerboard with configurable block size.
    Checkerboard(u32),
}

/// A source that generates deterministic test frames.
pub struct SyntheticSource {
    id: SourceId,
    name: String,
    width: u32,
    height: u32,
    pattern: Pattern,
    frame_count: AtomicU64,
    fps: u32,
}

impl SyntheticSource {
    /// Create a new synthetic source.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        width: u32,
        height: u32,
        fps: u32,
        pattern: Pattern,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            width,
            height,
            pattern,
            frame_count: AtomicU64::new(0),
            fps,
        }
    }

    /// Generate a frame with the configured pattern.
    fn generate_frame(&self, pts_us: u64) -> RawFrame {
        let size = RawFrame::expected_size(self.width, self.height);
        let mut data = vec![0u8; size];

        match self.pattern {
            Pattern::Solid(argb) => {
                for chunk in data.chunks_exact_mut(4) {
                    chunk.copy_from_slice(&argb);
                }
            }
            Pattern::Gradient => {
                for (i, chunk) in data.chunks_exact_mut(4).enumerate() {
                    let x = (i as u32) % self.width;
                    let y = (i as u32) / self.width;
                    chunk[0] = 255; // A
                    chunk[1] = (x * 255 / self.width.max(1)) as u8; // R
                    chunk[2] = (y * 255 / self.height.max(1)) as u8; // G
                    chunk[3] = ((x + y) * 255 / (self.width + self.height).max(1)) as u8; // B
                }
            }
            Pattern::Checkerboard(block) => {
                let block = block.max(1);
                for (i, chunk) in data.chunks_exact_mut(4).enumerate() {
                    let x = (i as u32) % self.width;
                    let y = (i as u32) / self.width;
                    let white = ((x / block) + (y / block)).is_multiple_of(2);
                    let val = if white { 255 } else { 0 };
                    chunk[0] = 255; // A (always opaque)
                    chunk[1] = val;
                    chunk[2] = val;
                    chunk[3] = val;
                }
            }
        }

        RawFrame {
            data,
            format: PixelFormat::Argb8888,
            width: self.width,
            height: self.height,
            pts_us,
        }
    }
}

impl Source for SyntheticSource {
    fn id(&self) -> SourceId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capture_frame(&self) -> anyhow::Result<Option<RawFrame>> {
        let n = self.frame_count.fetch_add(1, Ordering::Relaxed);
        let frame_duration_us = 1_000_000u64 / self.fps.max(1) as u64;
        let pts_us = n * frame_duration_us;
        Ok(Some(self.generate_frame(pts_us)))
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_live(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_pattern() {
        let src = SyntheticSource::new("solid", 4, 4, 30, Pattern::Solid([255, 128, 64, 32]));
        let frame = src.capture_frame().unwrap().unwrap();
        assert!(frame.is_valid());
        assert_eq!(frame.format, PixelFormat::Argb8888);
        // Every pixel should be [255, 128, 64, 32]
        for chunk in frame.data.chunks_exact(4) {
            assert_eq!(chunk, [255, 128, 64, 32]);
        }
    }

    #[test]
    fn gradient_pattern() {
        let src = SyntheticSource::new("grad", 8, 8, 30, Pattern::Gradient);
        let frame = src.capture_frame().unwrap().unwrap();
        assert!(frame.is_valid());
        // All pixels should be opaque
        for chunk in frame.data.chunks_exact(4) {
            assert_eq!(chunk[0], 255);
        }
    }

    #[test]
    fn checkerboard_pattern() {
        let src = SyntheticSource::new("check", 4, 4, 30, Pattern::Checkerboard(2));
        let frame = src.capture_frame().unwrap().unwrap();
        assert!(frame.is_valid());
        // (0,0) block → white, (2,0) block → black
        assert_eq!(&frame.data[0..4], [255, 255, 255, 255]);
        let idx = 2 * 4; // pixel (2,0)
        assert_eq!(&frame.data[idx..idx + 4], [255, 0, 0, 0]);
    }

    #[test]
    fn frame_count_increments() {
        let src = SyntheticSource::new("count", 2, 2, 30, Pattern::Solid([255, 0, 0, 0]));
        let f1 = src.capture_frame().unwrap().unwrap();
        let f2 = src.capture_frame().unwrap().unwrap();
        assert!(f2.pts_us > f1.pts_us);
    }

    #[test]
    fn zero_fps_no_panic() {
        let src = SyntheticSource::new("zero", 2, 2, 0, Pattern::Solid([255, 0, 0, 0]));
        let f = src.capture_frame().unwrap().unwrap();
        assert!(f.is_valid());
        assert_eq!(f.pts_us, 0);
        // Second frame should also work (not div-by-zero)
        let f2 = src.capture_frame().unwrap().unwrap();
        // fps=0 treated as 1fps: second frame gets pts = 1_000_000
        assert_eq!(f2.pts_us, 1_000_000);
    }

    #[test]
    fn is_live() {
        let src = SyntheticSource::new("live", 2, 2, 30, Pattern::Gradient);
        assert!(src.is_live());
    }
}
