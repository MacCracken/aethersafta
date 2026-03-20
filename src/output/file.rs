//! File output sink: writes encoded packets to a local file.
//!
//! Currently writes raw H.264 bitstream (`.h264`). MP4 container
//! muxing will be added when tarang supports video muxing.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{EncodedPacket, OutputSink};

/// Writes encoded video packets to a file.
pub struct FileOutput {
    path: PathBuf,
    file: File,
    bytes_written: u64,
    packets_written: u64,
}

impl FileOutput {
    /// Create a new file output sink.
    pub fn create(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let file = File::create(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            bytes_written: 0,
            packets_written: 0,
        })
    }

    /// The output file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Total bytes written.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Total packets written.
    pub fn packets_written(&self) -> u64 {
        self.packets_written
    }
}

impl OutputSink for FileOutput {
    fn write_packet(&mut self, packet: &EncodedPacket) -> anyhow::Result<()> {
        self.file.write_all(&packet.data)?;
        self.bytes_written += packet.data.len() as u64;
        self.packets_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        self.file.flush()?;
        Ok(())
    }

    fn close(&mut self) -> anyhow::Result<()> {
        self.file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.h264");

        let mut out = FileOutput::create(&path).unwrap();
        let packet = EncodedPacket {
            data: vec![0x00, 0x00, 0x00, 0x01, 0x67], // fake NAL unit
            pts_us: 0,
            dts_us: 0,
            is_keyframe: true,
        };
        out.write_packet(&packet).unwrap();
        out.flush().unwrap();

        assert_eq!(out.packets_written(), 1);
        assert_eq!(out.bytes_written(), 5);

        let contents = std::fs::read(&path).unwrap();
        assert_eq!(contents, vec![0x00, 0x00, 0x00, 0x01, 0x67]);
    }

    #[test]
    fn multiple_packets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.h264");

        let mut out = FileOutput::create(&path).unwrap();
        for i in 0..5 {
            out.write_packet(&EncodedPacket {
                data: vec![i as u8; 10],
                pts_us: i * 33000,
                dts_us: i * 33000,
                is_keyframe: i == 0,
            })
            .unwrap();
        }
        out.close().unwrap();

        assert_eq!(out.packets_written(), 5);
        assert_eq!(out.bytes_written(), 50);
    }
}
