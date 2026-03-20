//! Encoding pipeline: hardware selection via ai-hwaccel, encoding via tarang.
//!
//! Includes ARGB8888 → YUV420p color space conversion and software
//! H.264 encoding via tarang's openh264 backend (behind `openh264-enc` feature).

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

/// Convert an ARGB8888 buffer to YUV420p (planar Y, U, V).
///
/// Uses BT.601 coefficients with fixed-point integer math (no floats).
/// Dimensions must be even.
pub fn argb_to_yuv420p(argb: &[u8], width: u32, height: u32) -> Vec<u8> {
    // BT.601 coefficients scaled by 256 (fixed-point Q8)
    const YR: i32 = 77; // 0.299 * 256
    const YG: i32 = 150; // 0.587 * 256
    const YB: i32 = 29; // 0.114 * 256
    const UR: i32 = -43; // -0.169 * 256
    const UG: i32 = -85; // -0.331 * 256
    const UB: i32 = 128; // 0.500 * 256
    const VR: i32 = 128; // 0.500 * 256
    const VG: i32 = -107; // -0.419 * 256
    const VB: i32 = -21; // -0.081 * 256

    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let chroma_w = w / 2;
    let chroma_h = h / 2;
    let total = y_size + 2 * chroma_w * chroma_h;
    let mut yuv = vec![0u8; total];

    let (y_plane, chroma) = yuv.split_at_mut(y_size);
    let (u_plane, v_plane) = chroma.split_at_mut(chroma_w * chroma_h);

    for row in 0..h {
        let row_offset = row * w;
        let is_chroma_row = row % 2 == 0;

        for col in 0..w {
            let px = (row_offset + col) * 4;
            // ARGB: [A, R, G, B]
            let r = argb[px + 1] as i32;
            let g = argb[px + 2] as i32;
            let b = argb[px + 3] as i32;

            // Y = (77*R + 150*G + 29*B) >> 8, clamped to [0, 255]
            y_plane[row_offset + col] = ((YR * r + YG * g + YB * b) >> 8).clamp(0, 255) as u8;

            // Subsample chroma 2x2 (top-left pixel of each block)
            if is_chroma_row && col % 2 == 0 {
                let ci = (row / 2) * chroma_w + (col / 2);
                u_plane[ci] = ((UR * r + UG * g + UB * b + 128 * 256) >> 8).clamp(0, 255) as u8;
                v_plane[ci] = ((VR * r + VG * g + VB * b + 128 * 256) >> 8).clamp(0, 255) as u8;
            }
        }
    }

    yuv
}

/// Convert an NV12 buffer to ARGB8888.
///
/// NV12 layout: Y plane (w*h bytes) + interleaved UV plane (w * h/2 bytes).
/// Uses BT.601 inverse with fixed-point integer math.
pub fn nv12_to_argb(nv12: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let mut argb = vec![0u8; w * h * 4];

    let y_plane = &nv12[..y_size];
    let uv_plane = &nv12[y_size..];

    for row in 0..h {
        for col in 0..w {
            let y_val = y_plane[row * w + col] as i32;
            let uv_idx = (row / 2) * w + (col & !1);
            let u_val = uv_plane[uv_idx] as i32 - 128;
            let v_val = uv_plane[uv_idx + 1] as i32 - 128;

            // BT.601 inverse (fixed-point Q8):
            // R = Y + 1.402*V  ≈ Y + (359*V >> 8)
            // G = Y - 0.344*U - 0.714*V  ≈ Y - (88*U + 183*V) >> 8
            // B = Y + 1.772*U  ≈ Y + (454*U >> 8)
            let r = (y_val + ((359 * v_val) >> 8)).clamp(0, 255) as u8;
            let g = (y_val - ((88 * u_val + 183 * v_val) >> 8)).clamp(0, 255) as u8;
            let b = (y_val + ((454 * u_val) >> 8)).clamp(0, 255) as u8;

            let out = (row * w + col) * 4;
            argb[out] = 255; // A
            argb[out + 1] = r;
            argb[out + 2] = g;
            argb[out + 3] = b;
        }
    }

    argb
}

/// Convert an ARGB8888 buffer to NV12 (semi-planar YUV 4:2:0).
///
/// NV12 layout: Y plane (w*h) followed by interleaved UV pairs (w * h/2).
/// Uses BT.601 coefficients with fixed-point integer math.
pub fn argb_to_nv12(argb: &[u8], width: u32, height: u32) -> Vec<u8> {
    const YR: i32 = 77;
    const YG: i32 = 150;
    const YB: i32 = 29;
    const UR: i32 = -43;
    const UG: i32 = -85;
    const UB: i32 = 128;
    const VR: i32 = 128;
    const VG: i32 = -107;
    const VB: i32 = -21;

    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = w * (h / 2);
    let mut nv12 = vec![0u8; y_size + uv_size];

    let (y_plane, uv_plane) = nv12.split_at_mut(y_size);

    for row in 0..h {
        let row_offset = row * w;
        let is_chroma_row = row % 2 == 0;

        for col in 0..w {
            let px = (row_offset + col) * 4;
            let r = argb[px + 1] as i32;
            let g = argb[px + 2] as i32;
            let b = argb[px + 3] as i32;

            y_plane[row_offset + col] = ((YR * r + YG * g + YB * b) >> 8).clamp(0, 255) as u8;

            if is_chroma_row && col % 2 == 0 {
                let uv_idx = (row / 2) * w + col;
                uv_plane[uv_idx] =
                    ((UR * r + UG * g + UB * b + 128 * 256) >> 8).clamp(0, 255) as u8;
                uv_plane[uv_idx + 1] =
                    ((VR * r + VG * g + VB * b + 128 * 256) >> 8).clamp(0, 255) as u8;
            }
        }
    }

    nv12
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
        // 4x4 black frame (ARGB: [255, 0, 0, 0])
        let mut argb = vec![0u8; 4 * 4 * 4];
        for chunk in argb.chunks_exact_mut(4) {
            chunk[0] = 255; // A
        }
        let yuv = argb_to_yuv420p(&argb, 4, 4);
        // Y: 4*4=16, U: 2*2=4, V: 2*2=4 → total 24
        assert_eq!(yuv.len(), 24);
    }

    #[test]
    fn argb_white_to_yuv() {
        // 2x2 white frame: ARGB [255, 255, 255, 255]
        let argb = vec![255u8; 2 * 2 * 4];
        let yuv = argb_to_yuv420p(&argb, 2, 2);
        // White in BT.601: Y≈255, U≈128, V≈128
        for &y in &yuv[..4] {
            assert!(y > 250, "Y should be near 255, got {y}");
        }
        // U and V should be near 128
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
            chunk[0] = 255; // A
        }
        let nv12 = argb_to_nv12(&argb, 4, 4);
        // Y: 4*4=16, UV: 4*2=8 → total 24
        assert_eq!(nv12.len(), 24);
    }

    #[test]
    fn nv12_to_argb_dimensions() {
        let nv12 = vec![128u8; 4 * 4 + 4 * 2]; // 4x4 NV12
        let argb = nv12_to_argb(&nv12, 4, 4);
        assert_eq!(argb.len(), 4 * 4 * 4);
        // All pixels should be opaque
        for chunk in argb.chunks_exact(4) {
            assert_eq!(chunk[0], 255);
        }
    }

    #[test]
    fn nv12_roundtrip_white() {
        // White ARGB → NV12 → ARGB should stay close to white
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
        // Black ARGB [255, 0, 0, 0] → NV12 → ARGB
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
