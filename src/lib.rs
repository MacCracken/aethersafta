//! Aethersafta — Real-time media compositing engine.
//!
//! Multi-source capture, scene graph compositing, hardware-accelerated encoding,
//! and streaming output. Built on [`tarang`] for encoding/muxing and
//! [`ai_hwaccel`] for hardware encoder selection.
//!
//! # Architecture
//!
//! ```text
//! Sources (screen, camera, media, image)
//!     │
//!     ▼
//! Scene Graph (layers with z-order, transforms, opacity)
//!     │
//!     ▼
//! Compositor (alpha blend, crop, scale → composited frame)
//!     │
//!     ▼
//! Encode Pipeline (ai-hwaccel selects encoder → tarang encodes)
//!     │
//!     ▼
//! Output Sinks (file, RTMP, SRT)
//! ```
//!
//! # Quick start
//!
//! ```rust,no_run
//! use aethersafta::{SceneGraph, Layer, OutputConfig};
//!
//! // Create a scene with a screen capture source
//! let mut scene = SceneGraph::new(1920, 1080, 30);
//! scene.add_layer(Layer::screen_capture());
//!
//! // Record to file
//! let config = OutputConfig::file("recording.mp4");
//! // scene.start(config)?;
//! ```

pub mod scene;
pub mod source;
pub mod encode;
pub mod output;
pub mod timing;

pub use scene::{Layer, LayerId, SceneGraph};
pub use source::{RawFrame, Source, SourceId};
pub use encode::{EncodePipeline, EncoderConfig};
pub use output::{OutputConfig, OutputSink};
pub use timing::{FrameClock, LatencyBudget};

#[cfg(test)]
mod tests;
