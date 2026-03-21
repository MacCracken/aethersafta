//! Output sinks: file recording, RTMP streaming, SRT streaming.

pub mod file;
pub mod mp4;

use serde::{Deserialize, Serialize};

/// An encoded packet ready for output.
#[derive(Debug, Clone)]
pub struct EncodedPacket {
    /// Compressed data.
    pub data: Vec<u8>,
    /// Presentation timestamp in microseconds.
    pub pts_us: u64,
    /// Decode timestamp in microseconds.
    pub dts_us: u64,
    /// Whether this is a keyframe.
    pub is_keyframe: bool,
}

/// Output configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputConfig {
    /// Record to a local file.
    File { path: String },
    /// Stream via RTMP.
    Rtmp { url: String, stream_key: String },
    /// Stream via SRT.
    Srt {
        address: String,
        passphrase: Option<String>,
    },
}

impl OutputConfig {
    /// Convenience: file output.
    pub fn file(path: impl Into<String>) -> Self {
        Self::File { path: path.into() }
    }

    /// Convenience: RTMP output.
    pub fn rtmp(url: impl Into<String>, key: impl Into<String>) -> Self {
        Self::Rtmp {
            url: url.into(),
            stream_key: key.into(),
        }
    }
}

/// The `OutputSink` trait: anything that can consume encoded packets.
pub trait OutputSink: Send + Sync {
    /// Write an encoded packet to the output.
    fn write_packet(&mut self, packet: &EncodedPacket) -> anyhow::Result<()>;

    /// Flush buffered data.
    fn flush(&mut self) -> anyhow::Result<()>;

    /// Close the output and finalize (e.g. write container trailer).
    fn close(&mut self) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_config_file() {
        let cfg = OutputConfig::file("test.mp4");
        match cfg {
            OutputConfig::File { path } => assert_eq!(path, "test.mp4"),
            _ => panic!("expected File"),
        }
    }

    #[test]
    fn output_config_rtmp() {
        let cfg = OutputConfig::rtmp("rtmp://live.twitch.tv/app", "my_key");
        match cfg {
            OutputConfig::Rtmp { url, stream_key } => {
                assert!(url.contains("twitch"));
                assert_eq!(stream_key, "my_key");
            }
            _ => panic!("expected Rtmp"),
        }
    }

    #[test]
    fn serde_roundtrip() {
        let cfg = OutputConfig::Srt {
            address: "srt://host:9000".into(),
            passphrase: Some("secret".into()),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: OutputConfig = serde_json::from_str(&json).unwrap();
        match back {
            OutputConfig::Srt {
                address,
                passphrase,
            } => {
                assert!(address.contains("9000"));
                assert_eq!(passphrase.unwrap(), "secret");
            }
            _ => panic!("expected Srt"),
        }
    }
}
