//! Encoding pipeline: hardware selection via ai-hwaccel, encoding via tarang.

use serde::{Deserialize, Serialize};

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
}

impl EncodePipeline {
    pub fn new(config: EncoderConfig) -> Self {
        Self { config }
    }
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
}
