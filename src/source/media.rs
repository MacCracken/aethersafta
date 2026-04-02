//! Media file playback source.
//!
//! Demuxes a video file (MP4, MKV) via tarang and decodes frames using
//! OpenH264. Each [`capture_frame()`](Source::capture_frame) call advances
//! one frame; the [`VideoCaptureManager`](super::manager::VideoCaptureManager)
//! throttles calls to match the file's native FPS.
//!
//! Requires the `openh264-dec` feature.

use std::io::{Read, Seek};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use tarang::core::{StreamInfo, TarangError};
use tarang::demux::Demuxer;
use tracing::{debug, info, warn};

use super::{PixelFormat, RawFrame, Source, SourceId};

/// A media file playback source.
///
/// Opens a video file, probes for a video stream, and decodes frames
/// sequentially. Supports loop playback via seek-to-zero on EOF.
pub struct MediaFileSource {
    id: SourceId,
    name: String,
    width: u32,
    height: u32,
    fps: f64,
    looping: bool,
    inner: Mutex<MediaInner>,
}

struct MediaInner {
    demuxer: Box<dyn DemuxerObj>,
    decoder: tarang::video::OpenH264Decoder,
    video_stream_index: usize,
    eof: bool,
    frames_decoded: u64,
}

/// Object-safe wrapper for `Demuxer` (which is generic over `R: Read + Seek`).
trait DemuxerObj: Send {
    fn next_packet(&mut self) -> tarang::core::Result<tarang::demux::Packet>;
    fn seek(&mut self, ts: Duration) -> tarang::core::Result<()>;
}

impl<R: Read + Seek + Send> DemuxerObj for tarang::demux::Mp4Demuxer<R> {
    fn next_packet(&mut self) -> tarang::core::Result<tarang::demux::Packet> {
        Demuxer::next_packet(self)
    }
    fn seek(&mut self, ts: Duration) -> tarang::core::Result<()> {
        Demuxer::seek(self, ts)
    }
}

impl MediaFileSource {
    /// Open a media file for playback.
    ///
    /// Probes the file for video streams and initializes the decoder.
    /// Currently supports MP4 containers with H.264 video.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be opened, has no video stream,
    /// or the decoder fails to initialize.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        Self::open_with_options(path, true)
    }

    /// Open a media file with loop control.
    pub fn open_with_options(path: &str, looping: bool) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path).with_context(|| format!("failed to open: {path}"))?;

        let mut demuxer = tarang::demux::Mp4Demuxer::new(file);
        let info = demuxer
            .probe()
            .map_err(|e| anyhow::anyhow!("probe failed: {e}"))?;

        // Find first video stream
        let (video_idx, video_info) = info
            .streams
            .iter()
            .enumerate()
            .find_map(|(i, s)| match s {
                StreamInfo::Video(v) => Some((i, v.clone())),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("no video stream in {path}"))?;

        let decoder = tarang::video::OpenH264Decoder::new()
            .map_err(|e| anyhow::anyhow!("OpenH264 decoder init failed: {e}"))?;

        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("media")
            .to_string();

        info!(
            path,
            width = video_info.width,
            height = video_info.height,
            fps = video_info.frame_rate,
            codec = ?video_info.codec,
            looping,
            "media file source opened"
        );

        Ok(Self {
            id: uuid::Uuid::new_v4(),
            name,
            width: video_info.width,
            height: video_info.height,
            fps: video_info.frame_rate,
            looping,
            inner: Mutex::new(MediaInner {
                demuxer: Box::new(demuxer),
                decoder,
                video_stream_index: video_idx,
                eof: false,
                frames_decoded: 0,
            }),
        })
    }

    /// The file's native FPS.
    #[must_use]
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// Whether the source will loop on EOF.
    #[must_use]
    pub fn looping(&self) -> bool {
        self.looping
    }
}

impl Source for MediaFileSource {
    fn id(&self) -> SourceId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capture_frame(&self) -> anyhow::Result<Option<RawFrame>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("media source lock poisoned"))?;

        if inner.eof && !self.looping {
            return Ok(None);
        }

        // Try to decode the next video frame.
        // Limit iterations to prevent infinite spin if all packets fail decode.
        const MAX_PACKETS: usize = 500;
        for _ in 0..MAX_PACKETS {
            match inner.demuxer.next_packet() {
                Ok(packet) => {
                    if packet.stream_index != inner.video_stream_index {
                        continue; // skip audio/subtitle packets
                    }

                    match inner.decoder.decode(&packet.data, packet.timestamp) {
                        Ok(Some(video_frame)) => {
                            inner.frames_decoded += 1;
                            let argb = video_frame_to_argb(&video_frame)?;
                            let pts_us = video_frame.timestamp.as_micros() as u64;

                            debug!(pts_us, frame = inner.frames_decoded, "media frame decoded");

                            return Ok(Some(RawFrame {
                                data: argb.into(),
                                format: PixelFormat::Argb8888,
                                width: self.width,
                                height: self.height,
                                pts_us,
                            }));
                        }
                        Ok(None) => continue, // decoder needs more data
                        Err(e) => {
                            warn!(error = %e, "decode error, skipping packet");
                            continue;
                        }
                    }
                }
                Err(TarangError::EndOfStream) => {
                    // Flush remaining frames from decoder
                    if let Ok(remaining) = inner.decoder.flush()
                        && let Some(video_frame) = remaining.into_iter().next()
                    {
                        inner.frames_decoded += 1;
                        let argb = video_frame_to_argb(&video_frame)?;
                        let pts_us = video_frame.timestamp.as_micros() as u64;
                        return Ok(Some(RawFrame {
                            data: argb.into(),
                            format: PixelFormat::Argb8888,
                            width: self.width,
                            height: self.height,
                            pts_us,
                        }));
                    }

                    if self.looping {
                        // Seek back to start
                        if let Err(e) = inner.demuxer.seek(Duration::ZERO) {
                            warn!(error = %e, "seek to start failed");
                            inner.eof = true;
                            return Ok(None);
                        }
                        // Recreate decoder for clean state
                        match tarang::video::OpenH264Decoder::new() {
                            Ok(dec) => inner.decoder = dec,
                            Err(e) => {
                                warn!(error = %e, "decoder reinit failed on loop");
                                inner.eof = true;
                                return Ok(None);
                            }
                        }
                        inner.frames_decoded = 0;
                        debug!("media source looped");
                        continue;
                    }

                    inner.eof = true;
                    return Ok(None);
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("demux error: {e}"));
                }
            }
        }

        // Exhausted packet budget without producing a frame
        warn!("exceeded {MAX_PACKETS} packets without a decoded frame");
        Ok(None)
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_live(&self) -> bool {
        false
    }
}

/// Convert a tarang VideoFrame to ARGB8888.
fn video_frame_to_argb(frame: &tarang::core::VideoFrame) -> anyhow::Result<Vec<u8>> {
    match frame.pixel_format {
        tarang::core::PixelFormat::Yuv420p => {
            Ok(yuv420p_to_argb(&frame.data, frame.width, frame.height))
        }
        tarang::core::PixelFormat::Nv12 => Ok(crate::encode::nv12_to_argb(
            &frame.data,
            frame.width,
            frame.height,
        )),
        other => anyhow::bail!("unsupported decoded pixel format: {other:?}"),
    }
}

/// Convert YUV420p (planar) to ARGB8888 using BT.709.
///
/// Same coefficients as [`crate::encode::nv12_to_argb`]:
/// R = Y + 1.5748*V ≈ Y + (403*V)>>8
/// G = Y - 0.1873*U - 0.4681*V ≈ Y - (48*U + 120*V)>>8
/// B = Y + 1.8556*U ≈ Y + (475*U)>>8
#[must_use]
fn yuv420p_to_argb(yuv: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    let y_size = w * h;
    let u_off = y_size;
    let v_off = u_off + cw * ch;
    let expected = v_off + cw * ch;
    if yuv.len() < expected {
        return vec![0u8; w * h * 4]; // black frame on invalid input
    }

    let mut argb = vec![0u8; w * h * 4];

    for y in 0..h {
        let cy = (y / 2).min(ch.saturating_sub(1));
        for x in 0..w {
            let cx = (x / 2).min(cw.saturating_sub(1));
            let yi = yuv[y * w + x] as i16;
            let u = yuv[u_off + cy * cw + cx] as i16 - 128;
            let v = yuv[v_off + cy * cw + cx] as i16 - 128;
            let oi = (y * w + x) * 4;
            argb[oi] = 255; // A
            argb[oi + 1] = (yi + ((403 * v) >> 8)).clamp(0, 255) as u8; // R
            argb[oi + 2] = (yi - ((48 * u + 120 * v) >> 8)).clamp(0, 255) as u8; // G
            argb[oi + 3] = (yi + ((475 * u) >> 8)).clamp(0, 255) as u8; // B
        }
    }

    argb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuv420p_to_argb_white() {
        // White: Y=235, U=128, V=128
        let w = 2u32;
        let h = 2u32;
        let mut yuv = vec![235u8; 4]; // Y plane
        yuv.push(128); // U
        yuv.push(128); // V
        let argb = yuv420p_to_argb(&yuv, w, h);
        assert_eq!(argb.len(), 16);
        for px in argb.chunks_exact(4) {
            assert_eq!(px[0], 255, "alpha");
            assert!(px[1] > 220, "R={}", px[1]);
            assert!(px[2] > 220, "G={}", px[2]);
            assert!(px[3] > 220, "B={}", px[3]);
        }
    }

    #[test]
    fn yuv420p_to_argb_black() {
        let w = 2u32;
        let h = 2u32;
        let mut yuv = vec![16u8; 4]; // Y plane (broadcast black)
        yuv.push(128); // U
        yuv.push(128); // V
        let argb = yuv420p_to_argb(&yuv, w, h);
        for px in argb.chunks_exact(4) {
            assert_eq!(px[0], 255);
            assert!(px[1] < 30, "R={}", px[1]);
            assert!(px[2] < 30, "G={}", px[2]);
            assert!(px[3] < 30, "B={}", px[3]);
        }
    }

    #[test]
    fn yuv420p_to_argb_empty() {
        assert!(yuv420p_to_argb(&[], 0, 0).is_empty());
    }

    #[test]
    fn yuv420p_to_argb_undersized() {
        // Undersized buffer — should return black frame
        let argb = yuv420p_to_argb(&[0; 2], 4, 4);
        assert_eq!(argb.len(), 64);
        assert!(argb.iter().all(|&b| b == 0));
    }

    #[test]
    fn media_source_missing_file() {
        let result = MediaFileSource::open("/nonexistent/video.mp4");
        assert!(result.is_err());
    }
}
