//! Wayland screen capture via the `wlr-screencopy-unstable-v1` protocol.
//!
//! Connects to the Wayland compositor, binds `zwlr_screencopy_manager_v1`,
//! and captures frames from a `wl_output` using shared memory buffers.
//!
//! Requires the `wayland` feature and a compositor supporting wlr-screencopy
//! (sway, wlroots-based compositors, Hyprland, etc.).

use std::os::fd::AsFd;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use tracing::{debug, info, warn};
use wayland_client::globals::{self, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

use super::{PixelFormat, RawFrame, Source, SourceId};

/// Information about an available screen/output.
#[derive(Debug, Clone)]
pub struct ScreenInfo {
    /// Output index (order of wl_output globals).
    pub index: usize,
    /// Output name (if reported by compositor).
    pub name: Option<String>,
    /// Output description.
    pub description: Option<String>,
    /// Width in pixels (0 if not yet reported).
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Enumerate available Wayland outputs (screens).
///
/// # Errors
///
/// Returns an error if the Wayland display is unavailable.
pub fn enumerate_screens() -> anyhow::Result<Vec<ScreenInfo>> {
    let conn = Connection::connect_to_env().with_context(|| "no Wayland display")?;
    let (global_list, mut queue) =
        globals::registry_queue_init::<EnumState>(&conn).with_context(|| "registry init")?;

    let mut state = EnumState {
        outputs: Vec::new(),
    };
    queue.roundtrip(&mut state).ok();

    // Bind outputs and collect info
    let qh = queue.handle();
    let mut screens = Vec::new();
    for (i, global) in global_list.contents().clone_list().iter().enumerate() {
        if global.interface == "wl_output" {
            let output: wl_output::WlOutput =
                global_list
                    .registry()
                    .bind(global.name, global.version.min(4), &qh, ());
            queue.roundtrip(&mut state).ok();

            let info = state
                .outputs
                .iter()
                .find(|o| o.output == output)
                .map(|o| ScreenInfo {
                    index: i,
                    name: o.name.clone(),
                    description: o.description.clone(),
                    width: o.width,
                    height: o.height,
                })
                .unwrap_or(ScreenInfo {
                    index: i,
                    name: None,
                    description: None,
                    width: 0,
                    height: 0,
                });
            screens.push(info);
        }
    }

    Ok(screens)
}

struct OutputEntry {
    output: wl_output::WlOutput,
    name: Option<String>,
    description: Option<String>,
    width: u32,
    height: u32,
}

struct EnumState {
    outputs: Vec<OutputEntry>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for EnumState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for EnumState {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Find or create entry for this output
        let idx = state
            .outputs
            .iter()
            .position(|o| o.output == *proxy)
            .unwrap_or_else(|| {
                state.outputs.push(OutputEntry {
                    output: proxy.clone(),
                    name: None,
                    description: None,
                    width: 0,
                    height: 0,
                });
                state.outputs.len() - 1
            });

        match event {
            wl_output::Event::Name { name } => {
                state.outputs[idx].name = Some(name);
            }
            wl_output::Event::Description { description } => {
                state.outputs[idx].description = Some(description);
            }
            wl_output::Event::Mode { width, height, .. } => {
                state.outputs[idx].width = width as u32;
                state.outputs[idx].height = height as u32;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// ScreenSource
// ---------------------------------------------------------------------------

/// Wayland screen capture source.
///
/// Captures frames from a Wayland output using the `wlr-screencopy-unstable-v1`
/// protocol with shared memory buffers.
pub struct ScreenSource {
    id: SourceId,
    name: String,
    width: u32,
    height: u32,
    inner: Mutex<ScreenInner>,
    frame_count: AtomicU64,
}

struct ScreenInner {
    #[allow(dead_code)] // Kept alive — the EventQueue borrows from it
    conn: Connection,
    queue: EventQueue<CaptureState>,
    shm: wl_shm::WlShm,
    manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    output: wl_output::WlOutput,
}

/// Internal state for capture dispatch.
struct CaptureState {
    /// Buffer parameters from the frame's `buffer` event.
    shm_format: Option<wl_shm::Format>,
    shm_width: u32,
    shm_height: u32,
    shm_stride: u32,
    /// Whether the frame is ready to read.
    ready: bool,
    /// Whether the capture failed.
    failed: bool,
    /// Whether buffer info has been received.
    buffer_info: bool,
    /// Y-invert flag.
    y_invert: bool,
}

impl CaptureState {
    fn new() -> Self {
        Self {
            shm_format: None,
            shm_width: 0,
            shm_height: 0,
            shm_stride: 0,
            ready: false,
            failed: false,
            buffer_info: false,
            y_invert: false,
        }
    }
}

impl ScreenSource {
    /// Open a screen capture source for the given output index.
    ///
    /// `monitor` is the output index (0 = first). If `None`, captures the first output.
    ///
    /// # Errors
    ///
    /// Returns an error if the Wayland display is unavailable, the compositor
    /// doesn't support wlr-screencopy, or no outputs are available.
    pub fn open(monitor: Option<u32>) -> anyhow::Result<Self> {
        let conn = Connection::connect_to_env().with_context(|| "no Wayland display")?;

        let (global_list, mut queue) = globals::registry_queue_init::<CaptureState>(&conn)
            .with_context(|| "registry init failed")?;

        let qh = queue.handle();

        // Bind wl_shm
        let shm: wl_shm::WlShm = global_list
            .bind(&qh, 1..=1, ())
            .with_context(|| "compositor does not support wl_shm")?;

        // Bind screencopy manager
        let manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 = global_list
            .bind(&qh, 1..=3, ())
            .with_context(|| "compositor does not support wlr-screencopy-unstable-v1")?;

        // Find the target output
        let monitor_idx = monitor.unwrap_or(0) as usize;
        let output_globals: Vec<_> = global_list
            .contents()
            .clone_list()
            .into_iter()
            .filter(|g| g.interface == "wl_output")
            .collect();

        let output_global = output_globals
            .get(monitor_idx)
            .ok_or_else(|| anyhow::anyhow!("no output at index {monitor_idx}"))?;

        let output: wl_output::WlOutput =
            global_list
                .registry()
                .bind(output_global.name, output_global.version.min(4), &qh, ());

        // Roundtrip to get output geometry
        let mut state = CaptureState::new();
        queue.roundtrip(&mut state).ok();

        // Do a test capture to get dimensions
        let frame = manager.capture_output(1, &output, &qh, ());
        queue.roundtrip(&mut state).ok();

        // Dispatch until we get buffer info
        for _ in 0..10 {
            if state.buffer_info {
                break;
            }
            queue.roundtrip(&mut state).ok();
        }

        let width = if state.shm_width > 0 {
            state.shm_width
        } else {
            1920
        };
        let height = if state.shm_height > 0 {
            state.shm_height
        } else {
            1080
        };

        // Destroy the test frame
        frame.destroy();
        queue.roundtrip(&mut state).ok();

        info!(
            monitor = monitor_idx,
            width, height, "screen capture initialized"
        );

        Ok(Self {
            id: uuid::Uuid::new_v4(),
            name: format!("Screen {monitor_idx}"),
            width,
            height,
            inner: Mutex::new(ScreenInner {
                conn,
                queue,
                shm,
                manager,
                output,
            }),
            frame_count: AtomicU64::new(0),
        })
    }
}

impl Source for ScreenSource {
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
            .map_err(|_| anyhow::anyhow!("screen capture lock poisoned"))?;

        let qh = inner.queue.handle();

        // Reset state for this capture
        let mut state = CaptureState::new();

        // Request a new frame
        let frame = inner.manager.capture_output(1, &inner.output, &qh, ());

        // Wait for buffer info
        for _ in 0..20 {
            inner.queue.roundtrip(&mut state).ok();
            if state.buffer_info || state.failed {
                break;
            }
        }

        let shm_format = match (state.failed, state.buffer_info, state.shm_format) {
            (true, _, _) | (_, false, _) | (_, _, None) => {
                frame.destroy();
                inner.queue.roundtrip(&mut state).ok();
                return Ok(None);
            }
            (false, true, Some(fmt)) => fmt,
        };

        let buf_size = (state.shm_stride as usize) * (state.shm_height as usize);
        // Sanity: reject buffers > 256MB (8K@32bpp with stride padding)
        if buf_size == 0 || buf_size > 256 * 1024 * 1024 {
            frame.destroy();
            inner.queue.roundtrip(&mut state).ok();
            return Ok(None);
        }

        // Create shm buffer
        let memfd = create_memfd(buf_size)?;
        let pool = inner
            .shm
            .create_pool(memfd.as_fd(), buf_size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            state.shm_width as i32,
            state.shm_height as i32,
            state.shm_stride as i32,
            shm_format,
            &qh,
            (),
        );

        // Send copy request
        frame.copy(&buffer);
        inner.queue.flush().ok();

        // Wait for ready
        for _ in 0..50 {
            inner.queue.roundtrip(&mut state).ok();
            if state.ready || state.failed {
                break;
            }
        }

        // Read the pixel data
        let argb = if state.ready {
            let mmap = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    buf_size,
                    libc::PROT_READ,
                    libc::MAP_SHARED,
                    std::os::fd::AsRawFd::as_raw_fd(&memfd),
                    0,
                )
            };

            if mmap == libc::MAP_FAILED {
                warn!("mmap failed for screen capture buffer");
                None
            } else {
                let slice = unsafe { std::slice::from_raw_parts(mmap as *const u8, buf_size) };
                let data = xrgb_to_argb(slice, state.shm_width, state.shm_height, state.shm_stride);
                unsafe {
                    libc::munmap(mmap, buf_size);
                }
                Some(data)
            }
        } else {
            None
        };

        // Cleanup protocol objects
        buffer.destroy();
        pool.destroy();
        frame.destroy();
        inner.queue.roundtrip(&mut state).ok();

        match argb {
            Some(data) => {
                let frame_num = self.frame_count.fetch_add(1, Ordering::Relaxed);
                let pts_us = frame_num * 33_333; // ~30fps default

                debug!(
                    pts_us,
                    w = self.width,
                    h = self.height,
                    "screen frame captured"
                );

                Ok(Some(RawFrame {
                    data: data.into(),
                    format: PixelFormat::Argb8888,
                    width: self.width,
                    height: self.height,
                    pts_us,
                }))
            }
            None => Ok(None),
        }
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_live(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Wayland dispatch implementations
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for CaptureState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                // format is WEnum<wl_shm::Format>
                if let wayland_client::WEnum::Value(fmt) = format {
                    state.shm_format = Some(fmt);
                }
                state.shm_width = width;
                state.shm_height = height;
                state.shm_stride = stride;
                state.buffer_info = true;
            }
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: wayland_client::WEnum::Value(f),
            } => {
                state.y_invert = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.ready = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.failed = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(CaptureState: ignore wl_shm::WlShm);
delegate_noop!(CaptureState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(CaptureState: ignore wl_buffer::WlBuffer);
delegate_noop!(CaptureState: ignore wl_output::WlOutput);
delegate_noop!(CaptureState: ignore zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an anonymous memory-backed file descriptor.
fn create_memfd(size: usize) -> anyhow::Result<std::fs::File> {
    use std::ffi::CString;
    let name = CString::new("aethersafta-screencopy").expect("CString");
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        anyhow::bail!("memfd_create failed: {}", std::io::Error::last_os_error());
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(size as u64)
        .with_context(|| "ftruncate on memfd")?;
    Ok(file)
}

use std::os::fd::FromRawFd;

/// Convert XRGB8888 (or ARGB8888) shm buffer to our ARGB8888 format.
///
/// Handles stride padding (stride may be > width * 4).
/// Sets alpha to 0xFF for XRGB (where X is undefined).
fn xrgb_to_argb(data: &[u8], width: u32, height: u32, stride: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let s = stride as usize;
    let out_size = w * h * 4;
    let mut argb = Vec::with_capacity(out_size);

    for y in 0..h {
        let row_start = y * s;
        let row_end = row_start + w * 4;
        if row_end > data.len() {
            // Pad remaining rows with black
            argb.resize(out_size, 0);
            return argb;
        }
        let row = &data[row_start..row_end];
        // Wayland XRGB8888 is [B, G, R, X] in memory (little-endian ARGB)
        // Our ARGB8888 is [A, R, G, B] in memory
        for px in row.chunks_exact(4) {
            argb.push(255); // A (set alpha to opaque)
            argb.push(px[2]); // R
            argb.push(px[1]); // G
            argb.push(px[0]); // B
        }
    }

    argb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xrgb_to_argb_basic() {
        // XRGB8888 little-endian: [B, G, R, X]
        let xrgb = vec![
            0x00, 0x00, 0xFF, 0x00, // Red pixel
            0xFF, 0x00, 0x00, 0x00, // Blue pixel
        ];
        let argb = xrgb_to_argb(&xrgb, 2, 1, 8);
        assert_eq!(argb.len(), 8);
        // Pixel 0: should be A=255, R=255, G=0, B=0
        assert_eq!(argb[0], 255); // A
        assert_eq!(argb[1], 0xFF); // R
        assert_eq!(argb[2], 0x00); // G
        assert_eq!(argb[3], 0x00); // B
        // Pixel 1: should be A=255, R=0, G=0, B=255
        assert_eq!(argb[4], 255); // A
        assert_eq!(argb[5], 0x00); // R
        assert_eq!(argb[6], 0x00); // G
        assert_eq!(argb[7], 0xFF); // B
    }

    #[test]
    fn xrgb_to_argb_with_stride_padding() {
        // 1 pixel wide, stride = 8 (4 bytes padding per row)
        let data = vec![
            0x00, 0xFF, 0x00, 0x00, // Green pixel row 0
            0xAA, 0xBB, 0xCC, 0xDD, // Stride padding
            0xFF, 0x00, 0x00, 0x00, // Blue pixel row 1
            0x00, 0x00, 0x00, 0x00, // Stride padding
        ];
        let argb = xrgb_to_argb(&data, 1, 2, 8);
        assert_eq!(argb.len(), 8);
        // Row 0: Green → A=255, R=0, G=255, B=0
        assert_eq!(argb[0..4], [255, 0, 255, 0]);
        // Row 1: Blue → A=255, R=0, G=0, B=255
        assert_eq!(argb[4..8], [255, 0, 0, 255]);
    }

    #[test]
    fn xrgb_to_argb_empty() {
        let argb = xrgb_to_argb(&[], 0, 0, 0);
        assert!(argb.is_empty());
    }

    #[test]
    fn xrgb_to_argb_truncated_input() {
        // Input too short for 2x2 — should pad with black
        let data = vec![0xFF; 8]; // only 2 pixels, need 4
        let argb = xrgb_to_argb(&data, 2, 2, 8);
        assert_eq!(argb.len(), 16);
    }

    #[test]
    fn enumerate_screens_does_not_panic() {
        // May fail if no Wayland display — that's fine
        let _ = enumerate_screens();
    }
}
