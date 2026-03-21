//! Encoding pipeline: hardware selection via ai-hwaccel, encoding via tarang.
//!
//! Uses ranga for ARGB8888 → YUV420p / NV12 color space conversion.
//! Software H.264 encoding via tarang's openh264 backend (behind `openh264-enc` feature).

use serde::{Deserialize, Serialize};

#[cfg(feature = "openh264-enc")]
use crate::output::EncodedPacket;
#[cfg(feature = "openh264-enc")]
use crate::source::RawFrame;

/// Encoder configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderConfig {
    /// Target codec.
    pub codec: VideoCodec,
    /// Target bitrate in kbps.
    pub bitrate_kbps: u32,
    /// Keyframe interval in frames.
    pub keyframe_interval: u32,
    /// Prefer hardware encoding when available.
    pub prefer_hardware: bool,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            codec: VideoCodec::H264,
            bitrate_kbps: 6000,
            keyframe_interval: 60,
            prefer_hardware: true,
        }
    }
}

/// Supported video codecs for output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    H264,
    H265,
    VP9,
    AV1,
}

/// The encode pipeline consumes composited frames and produces encoded packets.
pub struct EncodePipeline {
    pub config: EncoderConfig,
    #[cfg(feature = "openh264-enc")]
    encoder: Option<tarang::video::OpenH264Encoder>,
    frames_encoded: u64,
}

impl EncodePipeline {
    /// Create a new encode pipeline. Call [`init`] before encoding.
    pub fn new(config: EncoderConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "openh264-enc")]
            encoder: None,
            frames_encoded: 0,
        }
    }

    /// Initialise the encoder for the given resolution and framerate.
    #[cfg(feature = "openh264-enc")]
    pub fn init(&mut self, width: u32, height: u32, fps: u32) -> anyhow::Result<()> {
        let enc_config = tarang::video::OpenH264EncoderConfig {
            width,
            height,
            bitrate_bps: self.config.bitrate_kbps * 1000,
            frame_rate_num: fps,
            frame_rate_den: 1,
        };
        self.encoder = Some(tarang::video::OpenH264Encoder::new(&enc_config)?);
        Ok(())
    }

    /// Encode a composited ARGB8888 frame into an H.264 packet.
    #[cfg(feature = "openh264-enc")]
    pub fn encode_frame(&mut self, frame: &RawFrame) -> anyhow::Result<EncodedPacket> {
        let encoder = self
            .encoder
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("encoder not initialised — call init() first"))?;

        let yuv = argb_to_yuv420p(&frame.data, frame.width, frame.height);
        let video_frame = tarang::core::VideoFrame {
            data: bytes::Bytes::from(yuv),
            pixel_format: tarang::core::PixelFormat::Yuv420p,
            width: frame.width,
            height: frame.height,
            timestamp: std::time::Duration::from_micros(frame.pts_us),
        };

        let nal_data = encoder.encode(&video_frame)?;
        self.frames_encoded += 1;

        Ok(EncodedPacket {
            data: nal_data,
            pts_us: frame.pts_us,
            dts_us: frame.pts_us,
            is_keyframe: self.frames_encoded % self.config.keyframe_interval as u64 == 1,
        })
    }

    /// Number of frames encoded so far.
    pub fn frames_encoded(&self) -> u64 {
        self.frames_encoded
    }
}

/// Convert an ARGB8888 buffer to YUV420p (planar Y, U, V) via ranga.
///
/// Converts ARGB→RGBA, then delegates to ranga's BT.601 fixed-point conversion.
pub fn argb_to_yuv420p(argb: &[u8], width: u32, height: u32) -> Vec<u8> {
    // Build a ranga PixelBuffer from ARGB data by converting to RGBA first
    let argb_buf = ranga::pixel::PixelBuffer::new(
        argb.to_vec(),
        width,
        height,
        ranga::pixel::PixelFormat::Argb8,
    )
    .expect("ARGB buffer size mismatch");
    let rgba_buf = ranga::convert::argb8_to_rgba8(&argb_buf).expect("ARGB→RGBA conversion");
    let yuv_buf = ranga::convert::rgba_to_yuv420p(&rgba_buf).expect("RGBA→YUV420p conversion");
    yuv_buf.data
}

/// Convert an NV12 buffer to ARGB8888 via ranga.
pub fn nv12_to_argb(nv12: &[u8], width: u32, height: u32) -> Vec<u8> {
    let nv12_buf = ranga::pixel::PixelBuffer::new(
        nv12.to_vec(),
        width,
        height,
        ranga::pixel::PixelFormat::Nv12,
    )
    .expect("NV12 buffer size mismatch");
    let rgba_buf = ranga::convert::nv12_to_rgba(&nv12_buf).expect("NV12→RGBA conversion");
    let argb_buf = ranga::convert::rgba8_to_argb8(&rgba_buf).expect("RGBA→ARGB conversion");
    argb_buf.data
}

/// Convert an ARGB8888 buffer to NV12 via ranga.
pub fn argb_to_nv12(argb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let argb_buf = ranga::pixel::PixelBuffer::new(
        argb.to_vec(),
        width,
        height,
        ranga::pixel::PixelFormat::Argb8,
    )
    .expect("ARGB buffer size mismatch");
    let nv12_buf = ranga::convert::argb_to_nv12(&argb_buf).expect("ARGB→NV12 conversion");
    nv12_buf.data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = EncoderConfig::default();
        assert_eq!(cfg.codec, VideoCodec::H264);
        assert_eq!(cfg.bitrate_kbps, 6000);
        assert!(cfg.prefer_hardware);
    }

    #[test]
    fn serde_roundtrip() {
        let cfg = EncoderConfig {
            codec: VideoCodec::AV1,
            bitrate_kbps: 8000,
            keyframe_interval: 120,
            prefer_hardware: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: EncoderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.codec, VideoCodec::AV1);
        assert_eq!(back.bitrate_kbps, 8000);
    }

    #[test]
    fn argb_to_yuv_dimensions() {
        let mut argb = vec![0u8; 4 * 4 * 4];
        for chunk in argb.chunks_exact_mut(4) {
            chunk[0] = 255; // A
        }
        let yuv = argb_to_yuv420p(&argb, 4, 4);
        assert_eq!(yuv.len(), 24);
    }

    #[test]
    fn argb_white_to_yuv() {
        let argb = vec![255u8; 2 * 2 * 4];
        let yuv = argb_to_yuv420p(&argb, 2, 2);
        for &y in &yuv[..4] {
            assert!(y > 250, "Y should be near 255, got {y}");
        }
        assert!((yuv[4] as i16 - 128).unsigned_abs() < 5);
        assert!((yuv[5] as i16 - 128).unsigned_abs() < 5);
    }

    #[test]
    fn pipeline_creation() {
        let pipe = EncodePipeline::new(EncoderConfig::default());
        assert_eq!(pipe.frames_encoded(), 0);
    }

    #[test]
    fn argb_to_nv12_dimensions() {
        let mut argb = vec![0u8; 4 * 4 * 4];
        for chunk in argb.chunks_exact_mut(4) {
            chunk[0] = 255;
        }
        let nv12 = argb_to_nv12(&argb, 4, 4);
        assert_eq!(nv12.len(), 24);
    }

    #[test]
    fn nv12_to_argb_dimensions() {
        let nv12 = vec![128u8; 4 * 4 + 4 * 2];
        let argb = nv12_to_argb(&nv12, 4, 4);
        assert_eq!(argb.len(), 4 * 4 * 4);
        for chunk in argb.chunks_exact(4) {
            assert_eq!(chunk[0], 255);
        }
    }

    #[test]
    fn nv12_roundtrip_white() {
        let argb = vec![255u8; 2 * 2 * 4];
        let nv12 = argb_to_nv12(&argb, 2, 2);
        let back = nv12_to_argb(&nv12, 2, 2);
        for chunk in back.chunks_exact(4) {
            assert_eq!(chunk[0], 255, "alpha must be 255");
            assert!(chunk[1] > 250, "R should be near 255, got {}", chunk[1]);
            assert!(chunk[2] > 250, "G should be near 255, got {}", chunk[2]);
            assert!(chunk[3] > 250, "B should be near 255, got {}", chunk[3]);
        }
    }

    #[test]
    fn nv12_roundtrip_black() {
        let mut argb = vec![0u8; 2 * 2 * 4];
        for chunk in argb.chunks_exact_mut(4) {
            chunk[0] = 255;
        }
        let nv12 = argb_to_nv12(&argb, 2, 2);
        let back = nv12_to_argb(&nv12, 2, 2);
        for chunk in back.chunks_exact(4) {
            assert_eq!(chunk[0], 255);
            assert!(chunk[1] < 5, "R should be near 0, got {}", chunk[1]);
            assert!(chunk[2] < 5, "G should be near 0, got {}", chunk[2]);
            assert!(chunk[3] < 5, "B should be near 0, got {}", chunk[3]);
        }
    }
}
