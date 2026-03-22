//! MP4 container output via tarang's muxer.
//!
//! Writes encoded video (and optionally audio) into a proper MP4 container
//! instead of a raw bitstream. Supports video-only or audio+video modes.

use std::fs::File;
use std::path::{Path, PathBuf};

use tarang::demux::{Mp4Muxer, MuxConfig, Muxer, VideoMuxConfig};

use super::EncodedPacket;

/// Writes encoded video packets into an MP4 container.
pub struct Mp4Output {
    path: PathBuf,
    muxer: Mp4Muxer<File>,
    packets_written: u64,
    bytes_written: u64,
}

impl Mp4Output {
    /// Create a video-only MP4 output.
    pub fn create_video_only(
        path: impl AsRef<Path>,
        codec: tarang::core::VideoCodec,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let file = File::create(path)?;
        let video_config = VideoMuxConfig {
            codec,
            width,
            height,
        };
        // tarang 0.20.3 requires audio config; use dummy config for video-only.
        let audio_config = MuxConfig {
            codec: tarang::core::AudioCodec::Aac,
            sample_rate: 48000,
            channels: 2,
            bits_per_sample: 16,
        };
        let mut muxer = Mp4Muxer::new_with_video(file, audio_config, video_config);
        muxer.write_header()?;

        Ok(Self {
            path: path.to_path_buf(),
            muxer,
            packets_written: 0,
            bytes_written: 0,
        })
    }

    /// Create an audio+video MP4 output.
    pub fn create_with_audio(
        path: impl AsRef<Path>,
        video_codec: tarang::core::VideoCodec,
        width: u32,
        height: u32,
        audio_config: MuxConfig,
    ) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let file = File::create(path)?;
        let video_config = VideoMuxConfig {
            codec: video_codec,
            width,
            height,
        };
        let mut muxer = Mp4Muxer::new_with_video(file, audio_config, video_config);
        muxer.write_header()?;

        Ok(Self {
            path: path.to_path_buf(),
            muxer,
            packets_written: 0,
            bytes_written: 0,
        })
    }

    /// Write a video packet to the MP4 container.
    pub fn write_video(&mut self, packet: &EncodedPacket) -> anyhow::Result<()> {
        self.muxer.write_video_packet(&packet.data)?;
        self.packets_written += 1;
        self.bytes_written += packet.data.len() as u64;
        Ok(())
    }

    /// Write an audio packet to the MP4 container.
    pub fn write_audio(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.muxer.write_packet(data)?;
        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Finalize the MP4 container (write moov atom, fix headers).
    pub fn finalize(&mut self) -> anyhow::Result<()> {
        self.muxer.finalize()?;
        Ok(())
    }

    /// The output file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Total video packets written.
    pub fn packets_written(&self) -> u64 {
        self.packets_written
    }

    /// Total bytes written to the container.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_only_mp4_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mp4");

        let mut out = Mp4Output::create_video_only(
            &path,
            tarang::core::VideoCodec::H264,
            320,
            240,
        )
        .unwrap();

        // Write some fake video packets
        for i in 0..5 {
            out.write_video(&EncodedPacket {
                data: vec![0x00, 0x00, 0x00, 0x01, 0x67, i as u8],
                pts_us: i * 33333,
                dts_us: i * 33333,
                is_keyframe: i == 0,
            })
            .unwrap();
        }

        out.finalize().unwrap();

        assert_eq!(out.packets_written(), 5);
        assert!(out.bytes_written() > 0);

        let file_size = std::fs::metadata(&path).unwrap().len();
        assert!(file_size > 0);
    }

    #[test]
    fn audio_video_mp4_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("av_test.mp4");

        let audio_config = MuxConfig {
            codec: tarang::core::AudioCodec::Aac,
            sample_rate: 48000,
            channels: 2,
            bits_per_sample: 16,
        };

        let mut out = Mp4Output::create_with_audio(
            &path,
            tarang::core::VideoCodec::H264,
            640,
            480,
            audio_config,
        )
        .unwrap();

        // Write audio packets first (MP4 muxer expects audio before video)
        for i in 0..10 {
            out.write_audio(&vec![0xFFu8; 128 + i]).unwrap();
        }

        // Write video packets
        for i in 0..5 {
            out.write_video(&EncodedPacket {
                data: vec![0x65u8; 256],
                pts_us: i * 33333,
                dts_us: i * 33333,
                is_keyframe: i == 0,
            })
            .unwrap();
        }

        out.finalize().unwrap();
        assert!(out.bytes_written() > 0);
    }
}
