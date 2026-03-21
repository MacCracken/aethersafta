//! Aethersafta — Real-time media compositing engine.
//!
//! Multi-source capture, scene graph compositing, hardware-accelerated encoding,
//! and streaming output. Built on [`tarang`] for encoding/muxing,
//! [`ranga`] for image processing, [`dhvani`] for audio capture/mixing/DSP,
//! and [`ai_hwaccel`] for hardware encoder selection.
//!
//! # Architecture
//!
//! ```text
//! Video Sources (screen, camera, media, image)
//!     │
//!     ▼
//! Scene Graph (layers with z-order, transforms, opacity)
//!     │
//!     ▼
//! Compositor (alpha blend, crop, scale → composited frame via ranga)
//!     │
//!     ▼                                    Audio Sources (PipeWire via dhvani)
//! Encode Pipeline                              │
//! (ai-hwaccel → tarang encodes)                ▼
//!     │                                    AudioMixer (per-source DSP, mix, master bus)
//!     ▼                                        │
//! Output Sinks (file, RTMP, SRT) ◄─────────────┘
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

pub mod audio;
pub mod encode;
pub mod output;
pub mod scene;
pub mod source;
pub mod timing;

pub use audio::{AudioMixer, AudioMixerConfig, AudioSourceConfig, AudioSourceId};
pub use encode::{EncodePipeline, EncoderConfig};
pub use output::file::FileOutput;
pub use output::{OutputConfig, OutputSink};
pub use scene::compositor::Compositor;
pub use scene::{Layer, LayerId, SceneGraph};
pub use source::image::ImageSource;
pub use source::synthetic::SyntheticSource;
pub use source::{PixelFormat, RawFrame, Source, SourceId};
pub use timing::{FrameClock, LatencyBudget};

#[cfg(test)]
mod tests;
