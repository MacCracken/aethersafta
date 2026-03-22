//! Encoding pipeline: hardware selection via ai-hwaccel, encoding via tarang.
//!
//! When the `hwaccel` feature is enabled, [`EncodePipeline::init`] probes for
//! GPU hardware via [`ai_hwaccel::AcceleratorRegistry`] and selects the best
//! available encoder:
//!
//! 1. **VA-API** (feature `vaapi`) — hardware H.264 on Intel/AMD GPUs
//! 2. **OpenH264** (feature `openh264-enc`) — software H.264 fallback
//!
//! Uses ranga for ARGB8888 → YUV420p / NV12 color space conversion.

use serde::{Deserialize, Serialize};

use crate::output::EncodedPacket;
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

/// Which encoder backend is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderBackend {
    /// VA-API hardware encoder (Intel/AMD GPU).
    Vaapi,
    /// OpenH264 software encoder.
    OpenH264,
    /// No encoder available.
    None,
}

impl std::fmt::Display for EncoderBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vaapi => write!(f, "VA-API (hardware)"),
            Self::OpenH264 => write!(f, "OpenH264 (software)"),
            Self::None => write!(f, "none"),
        }
    }
}

/// The encode pipeline consumes composited frames and produces encoded packets.
///
/// Selects the best available encoder backend based on hardware detection
/// and enabled features. Use [`backend`] to check which encoder is active.
pub struct EncodePipeline {
    pub config: EncoderConfig,
    encoder: EncoderInner,
    frames_encoded: u64,
}

enum EncoderInner {
    Uninitialised,
    #[cfg(feature = "vaapi")]
    Vaapi(tarang::video::VaapiEncoder),
    #[cfg(feature = "openh264-enc")]
    OpenH264(tarang::video::OpenH264Encoder),
}

impl EncodePipeline {
    /// Create a new encode pipeline. Call [`init`] before encoding.
    pub fn new(config: EncoderConfig) -> Self {
        Self {
            config,
            encoder: EncoderInner::Uninitialised,
            frames_encoded: 0,
        }
    }

    /// Initialise the encoder for the given resolution and framerate.
    ///
    /// When `prefer_hardware` is true and the `hwaccel` feature is enabled,
    /// probes for GPU hardware and selects VA-API if available. Falls back
    /// to OpenH264 software encoding otherwise.
    pub fn init(&mut self, width: u32, height: u32, fps: u32) -> anyhow::Result<()> {
        let bitrate_bps = self.config.bitrate_kbps * 1000;

        // Try hardware encoder first
        if self.config.prefer_hardware
            && let Some(encoder) = self.try_hw_encoder(width, height, fps, bitrate_bps)
        {
            self.encoder = encoder;
            return Ok(());
        }

        // Fall back to software encoder
        self.encoder = self.try_sw_encoder(width, height, fps, bitrate_bps)?;
        Ok(())
    }

    /// Attempt hardware encoder selection via ai-hwaccel.
    fn try_hw_encoder(
        &self,
        width: u32,
        height: u32,
        fps: u32,
        bitrate_bps: u32,
    ) -> Option<EncoderInner> {
        let _ = (width, height, fps, bitrate_bps);

        #[cfg(all(feature = "hwaccel", feature = "vaapi"))]
        {
            if self.config.codec != VideoCodec::H264 {
                tracing::debug!("VA-API only supports H.264, skipping hw encoder");
                return None;
            }

            // Use disk-cached registry to avoid re-probing hardware every run.
            // Cache persists to $XDG_CACHE_HOME/ai-hwaccel/registry.json with 60s TTL.
            let registry =
                ai_hwaccel::DiskCachedRegistry::new(std::time::Duration::from_secs(60)).get();
            let has_gpu = registry
                .all_profiles()
                .iter()
                .any(|p| matches!(p.family, ai_hwaccel::AcceleratorFamily::Gpu));

            if !has_gpu {
                tracing::debug!("no GPU detected, skipping VA-API");
                return None;
            }

            let vaapi_config = tarang::video::VaapiEncoderConfig {
                codec: tarang::core::VideoCodec::H264,
                width,
                height,
                bitrate_bps,
                frame_rate_num: fps,
                frame_rate_den: 1,
                device: None, // auto-detect
            };

            match tarang::video::VaapiEncoder::new(&vaapi_config) {
                Ok(enc) => {
                    tracing::info!("encoder: VA-API H.264 ({})", enc.driver_name());
                    return Some(EncoderInner::Vaapi(enc));
                }
                Err(e) => {
                    tracing::debug!("VA-API init failed, falling back to software: {e}");
                }
            }
        }

        None
    }

    /// Attempt software encoder initialisation.
    fn try_sw_encoder(
        &self,
        width: u32,
        height: u32,
        fps: u32,
        bitrate_bps: u32,
    ) -> anyhow::Result<EncoderInner> {
        let _ = (width, height, fps, bitrate_bps);

        #[cfg(feature = "openh264-enc")]
        {
            if self.config.codec == VideoCodec::H264 {
                let enc_config = tarang::video::OpenH264EncoderConfig {
                    width,
                    height,
                    bitrate_bps,
                    frame_rate_num: fps,
                    frame_rate_den: 1,
                };
                let enc = tarang::video::OpenH264Encoder::new(&enc_config)?;
                tracing::info!("encoder: OpenH264 (software)");
                return Ok(EncoderInner::OpenH264(enc));
            }
        }

        anyhow::bail!(
            "no encoder available for {:?} — build with --features openh264-enc or --features vaapi",
            self.config.codec
        );
    }

    /// Encode a composited ARGB8888 frame into an encoded packet.
    #[allow(unused_variables)]
    pub fn encode_frame(&mut self, frame: &RawFrame) -> anyhow::Result<EncodedPacket> {
        match &mut self.encoder {
            #[cfg(feature = "vaapi")]
            EncoderInner::Vaapi(enc) => {
                let video_frame = make_video_frame(frame);
                let nal_data = enc.encode(&video_frame)?;
                self.frames_encoded += 1;
                Ok(make_packet(
                    nal_data,
                    frame.pts_us,
                    self.frames_encoded,
                    self.config.keyframe_interval,
                ))
            }
            #[cfg(feature = "openh264-enc")]
            EncoderInner::OpenH264(enc) => {
                let video_frame = make_video_frame(frame);
                let nal_data = enc.encode(&video_frame)?;
                self.frames_encoded += 1;
                Ok(make_packet(
                    nal_data,
                    frame.pts_us,
                    self.frames_encoded,
                    self.config.keyframe_interval,
                ))
            }
            EncoderInner::Uninitialised => {
                anyhow::bail!("encoder not initialised — call init() first");
            }
        }
    }

    /// Which encoder backend is active.
    pub fn backend(&self) -> EncoderBackend {
        match &self.encoder {
            #[cfg(feature = "vaapi")]
            EncoderInner::Vaapi(_) => EncoderBackend::Vaapi,
            #[cfg(feature = "openh264-enc")]
            EncoderInner::OpenH264(_) => EncoderBackend::OpenH264,
            EncoderInner::Uninitialised => EncoderBackend::None,
        }
    }

    /// Number of frames encoded so far.
    pub fn frames_encoded(&self) -> u64 {
        self.frames_encoded
    }
}

/// Detect and describe the best available encoder without initialising it.
///
/// Useful for `aethersafta info` to show what encoding is available.
pub fn detect_best_encoder(codec: VideoCodec) -> EncoderBackend {
    #[cfg(all(feature = "hwaccel", feature = "vaapi"))]
    {
        if codec == VideoCodec::H264 {
            let registry =
                ai_hwaccel::DiskCachedRegistry::new(std::time::Duration::from_secs(60)).get();
            let has_gpu = registry
                .all_profiles()
                .iter()
                .any(|p| matches!(p.family, ai_hwaccel::AcceleratorFamily::Gpu));
            if has_gpu {
                return EncoderBackend::Vaapi;
            }
        }
    }

    #[cfg(feature = "openh264-enc")]
    if codec == VideoCodec::H264 {
        return EncoderBackend::OpenH264;
    }

    let _ = codec;
    EncoderBackend::None
}

#[cfg(any(feature = "vaapi", feature = "openh264-enc"))]
fn make_video_frame(frame: &RawFrame) -> tarang::core::VideoFrame {
    let yuv = argb_to_yuv420p(&frame.data, frame.width, frame.height);
    tarang::core::VideoFrame {
        data: bytes::Bytes::from(yuv),
        pixel_format: tarang::core::PixelFormat::Yuv420p,
        width: frame.width,
        height: frame.height,
        timestamp: std::time::Duration::from_micros(frame.pts_us),
    }
}

#[cfg(any(feature = "vaapi", feature = "openh264-enc"))]
fn make_packet(
    data: Vec<u8>,
    pts_us: u64,
    frames_encoded: u64,
    keyframe_interval: u32,
) -> EncodedPacket {
    EncodedPacket {
        data,
        pts_us,
        dts_us: pts_us,
        is_keyframe: frames_encoded % keyframe_interval as u64 == 1,
    }
}

/// Convert an ARGB8888 buffer to YUV420p (planar Y, U, V) via ranga.
///
/// Converts ARGB→RGBA, then delegates to ranga's BT.709 fixed-point conversion
/// (correct for HD video, H.264 assumes BT.709 for >= 720p).
pub fn argb_to_yuv420p(argb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let argb_buf = ranga::pixel::PixelBuffer::new(
        argb.to_vec(),
        width,
        height,
        ranga::pixel::PixelFormat::Argb8,
    )
    .expect("ARGB buffer size mismatch");
    let rgba_buf = ranga::convert::argb8_to_rgba8(&argb_buf).expect("ARGB→RGBA conversion");
    let yuv_buf =
        ranga::convert::rgba_to_yuv420p_bt709(&rgba_buf).expect("RGBA→YUV420p BT.709 conversion");
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
        assert_eq!(pipe.backend(), EncoderBackend::None);
    }

    #[test]
    fn detect_best_encoder_returns_something() {
        let best = detect_best_encoder(VideoCodec::H264);
        // Should return OpenH264 or Vaapi depending on features, or None
        println!("best encoder: {best}");
    }

    #[test]
    fn encoder_backend_display() {
        assert_eq!(EncoderBackend::Vaapi.to_string(), "VA-API (hardware)");
        assert_eq!(EncoderBackend::OpenH264.to_string(), "OpenH264 (software)");
        assert_eq!(EncoderBackend::None.to_string(), "none");
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
