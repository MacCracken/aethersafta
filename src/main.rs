//! aethersafta CLI — record, preview, and stream composited scenes.
//!
//! Usage:
//!   aethersafta record --source screen --output recording.h264
//!   aethersafta record --source image:bg.png --output recording.h264 --audio default
//!   aethersafta preview --source screen
//!   aethersafta info
//!   aethersafta --version

use std::collections::HashMap;
use std::time::Instant;

use clap::{Parser, Subcommand};
use tracing::{error, info};

use aethersafta::encode::{EncodePipeline, EncoderConfig, VideoCodec};
use aethersafta::output::OutputSink;
use aethersafta::output::file::FileOutput;
use aethersafta::scene::compositor::Compositor;
use aethersafta::scene::{Layer, LayerContent, SceneGraph};
use aethersafta::source::Source;
use aethersafta::source::image::ImageSource;
use aethersafta::timing::FrameClock;

#[derive(Parser)]
#[command(
    name = "aethersafta",
    version,
    about = "Real-time media compositing engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show system info (available sources, encoders, hardware)
    Info,
    /// Record a composited scene to file
    Record {
        /// Source: "screen", "image:<path>", or "color:<RRGGBBAA>"
        #[arg(long, default_value = "screen")]
        source: String,
        /// Output file path (.h264 for raw H.264, other for raw frames)
        #[arg(short, long)]
        output: String,
        /// Duration in seconds (0 = until stopped)
        #[arg(long, default_value = "0")]
        duration: u64,
        /// Target framerate
        #[arg(long, default_value = "30")]
        fps: u32,
        /// Output width
        #[arg(long, default_value = "1920")]
        width: u32,
        /// Output height
        #[arg(long, default_value = "1080")]
        height: u32,
        /// Encoding bitrate in kbps
        #[arg(long, default_value = "6000")]
        bitrate: u32,
        /// Audio source: "default", "none", or a PipeWire device ID
        #[arg(long, default_value = "none")]
        audio: String,
        /// Audio gain in dB (0 = unity)
        #[arg(long, default_value = "0")]
        audio_gain: f32,
    },
    /// Preview composited output (display only, no recording)
    Preview {
        /// Source: "screen", "image:<path>", or "color:<RRGGBBAA>"
        #[arg(long, default_value = "screen")]
        source: String,
        /// Target framerate
        #[arg(long, default_value = "30")]
        fps: u32,
        /// Number of frames to preview (0 = until stopped)
        #[arg(long, default_value = "60")]
        frames: u64,
    },
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Info => cmd_info(),
        Commands::Record {
            source,
            output,
            duration,
            fps,
            width,
            height,
            bitrate,
            audio,
            audio_gain,
        } => {
            if let Err(e) = cmd_record(
                &source, &output, duration, fps, width, height, bitrate, &audio, audio_gain,
            ) {
                error!("{e:#}");
                std::process::exit(1);
            }
        }
        Commands::Preview {
            source,
            fps,
            frames,
        } => {
            if let Err(e) = cmd_preview(&source, fps, frames) {
                error!("{e:#}");
                std::process::exit(1);
            }
        }
    }
}

fn cmd_info() {
    println!("aethersafta v{}", env!("CARGO_PKG_VERSION"));
    println!();

    // Hardware accelerators
    #[cfg(feature = "hwaccel")]
    {
        let registry =
            ai_hwaccel::DiskCachedRegistry::new(std::time::Duration::from_secs(60)).get();
        let profiles = registry.all_profiles();
        if profiles.is_empty() {
            println!("Hardware: no accelerators detected");
        } else {
            println!("Hardware:");
            for p in profiles {
                let mem_gb = p.memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                println!("  {} ({:.1} GB)", p.accelerator, mem_gb);
            }
        }
        println!();
    }

    // Encoding
    let best = aethersafta::encode::detect_best_encoder(VideoCodec::H264);
    println!("Encoding:");
    println!("  H.264: {best}");
    println!();

    // Audio
    println!("Audio:");
    println!("  Mixer: dhvani (DSP, metering, mixing)");
    #[cfg(feature = "pipewire")]
    {
        println!("  Capture: PipeWire (via dhvani)");
        match dhvani::capture::enumerate_devices() {
            Ok(devices) => {
                if devices.is_empty() {
                    println!("  No audio devices detected");
                } else {
                    for dev in &devices {
                        let kind = match dev.device_type {
                            dhvani::capture::DeviceType::Source => "in",
                            dhvani::capture::DeviceType::Sink => "out",
                            _ => "??",
                        };
                        println!(
                            "    [{kind}] {} (id={}, {}ch, {}Hz)",
                            dev.name, dev.id, dev.channels, dev.sample_rate
                        );
                    }
                }
            }
            Err(e) => println!("  PipeWire not available: {e}"),
        }
    }
    #[cfg(not(feature = "pipewire"))]
    println!("  Capture: (disabled — build with --features pipewire)");
    println!();

    println!("Supported outputs: file (raw H.264 bitstream)");
    #[cfg(feature = "rtmp")]
    println!("  + RTMP streaming");
    #[cfg(feature = "srt")]
    println!("  + SRT streaming");
}

/// Build a scene graph with a single layer from the --source argument.
fn build_scene(
    source_str: &str,
    width: u32,
    height: u32,
    fps: u32,
) -> anyhow::Result<(SceneGraph, Option<Box<dyn Source>>)> {
    let mut scene = SceneGraph::new(width, height, fps);

    if let Some(path) = source_str.strip_prefix("image:") {
        let src = ImageSource::open(path)?;
        let mut layer = Layer::new(
            src.name(),
            LayerContent::Source {
                source_id: src.id(),
            },
        );
        layer.size = Some((width, height));
        scene.add_layer(layer);
        Ok((scene, Some(Box::new(src))))
    } else if let Some(hex) = source_str.strip_prefix("color:") {
        let color = parse_hex_color(hex)?;
        scene.add_layer(Layer::new("fill", LayerContent::ColorFill { color }));
        Ok((scene, None))
    } else if source_str == "screen" {
        anyhow::bail!("screen capture not yet implemented — use image:<path> or color:<RRGGBBAA>");
    } else {
        anyhow::bail!("unknown source: {source_str}");
    }
}

/// Capture frames from source for all matching layers in the scene.
fn capture_source_frames(
    scene: &SceneGraph,
    source: &Option<Box<dyn Source>>,
) -> HashMap<aethersafta::scene::LayerId, aethersafta::source::RawFrame> {
    let mut frames = HashMap::new();
    if let Some(src) = source {
        for layer in scene.layers() {
            if let LayerContent::Source { source_id } = &layer.content
                && *source_id == src.id()
                && let Ok(Some(f)) = src.capture_frame()
            {
                frames.insert(layer.id, f);
            }
        }
    }
    frames
}

fn parse_hex_color(hex: &str) -> anyhow::Result<[u8; 4]> {
    if hex.len() != 8 {
        anyhow::bail!("color must be 8 hex chars (RRGGBBAA), got: {hex}");
    }
    let r = u8::from_str_radix(&hex[0..2], 16)?;
    let g = u8::from_str_radix(&hex[2..4], 16)?;
    let b = u8::from_str_radix(&hex[4..6], 16)?;
    let a = u8::from_str_radix(&hex[6..8], 16)?;
    Ok([r, g, b, a])
}

/// Set up audio capture via dhvani PipeWire, if requested.
///
/// Returns the audio mixer, capture manager, and source ID.
/// Supports multiple audio sources via `AudioCaptureManager`.
#[cfg(feature = "pipewire")]
fn setup_audio(
    audio_arg: &str,
    gain_db: f32,
) -> anyhow::Result<
    Option<(
        aethersafta::audio::AudioMixer,
        aethersafta::audio::AudioCaptureManager,
        aethersafta::audio::AudioSourceId,
    )>,
> {
    if audio_arg == "none" {
        return Ok(None);
    }

    let device_id = if audio_arg == "default" {
        None
    } else {
        Some(audio_arg.parse::<u32>().map_err(|_| {
            anyhow::anyhow!("--audio must be \"default\", \"none\", or a device ID (integer)")
        })?)
    };

    let mut mixer =
        aethersafta::audio::AudioMixer::new(aethersafta::audio::AudioMixerConfig::default());
    let mut src_config = aethersafta::audio::AudioSourceConfig::new("PipeWire Capture");
    src_config.gain_db = gain_db;
    src_config.device_id = device_id;
    let src_id = mixer.add_source(src_config.clone());

    let mut capture_mgr = aethersafta::audio::AudioCaptureManager::new();
    capture_mgr.add_source(src_id, src_config)?;

    info!(
        "audio capture: PipeWire (device={}, 48kHz stereo)",
        device_id.map_or("default".into(), |id| id.to_string())
    );

    Ok(Some((mixer, capture_mgr, src_id)))
}

#[allow(clippy::too_many_arguments)]
fn cmd_record(
    source_str: &str,
    output_path: &str,
    duration_secs: u64,
    fps: u32,
    width: u32,
    height: u32,
    bitrate: u32,
    audio_arg: &str,
    audio_gain: f32,
) -> anyhow::Result<()> {
    let (scene, source) = build_scene(source_str, width, height, fps)?;
    let mut compositor = Compositor::new(width, height);
    let mut clock = FrameClock::new(fps);

    let total_frames = if duration_secs == 0 {
        info!("recording until interrupted (Ctrl+C)");
        u64::MAX
    } else {
        let n = duration_secs * fps as u64;
        info!("recording {duration_secs}s ({n} frames) to {output_path}");
        n
    };

    // Init encoder (hw auto-detect via ai-hwaccel, sw fallback)
    let mut encoder = EncodePipeline::new(EncoderConfig {
        codec: VideoCodec::H264,
        bitrate_kbps: bitrate,
        ..Default::default()
    });
    encoder.init(width, height, fps)?;
    info!("encoder: {} @ {bitrate} kbps", encoder.backend());

    // Use MP4 container for .mp4 files, raw bitstream otherwise
    let is_mp4 = output_path.ends_with(".mp4");
    let mut mp4_out = if is_mp4 {
        Some(aethersafta::output::mp4::Mp4Output::create_video_only(
            output_path,
            tarang::core::VideoCodec::H264,
            width,
            height,
        )?)
    } else {
        None
    };
    let mut file_out = if !is_mp4 {
        Some(FileOutput::create(output_path)?)
    } else {
        None
    };
    if is_mp4 {
        info!("output: MP4 container (via tarang muxer)");
    }

    // Audio setup (PipeWire capture → mixer)
    #[cfg(feature = "pipewire")]
    let mut audio_state = setup_audio(audio_arg, audio_gain)?;
    #[cfg(not(feature = "pipewire"))]
    let audio_state: Option<()> = {
        if audio_arg != "none" {
            anyhow::bail!("audio capture requires --features pipewire");
        }
        None
    };

    let start = Instant::now();
    for frame_num in 0..total_frames {
        clock.tick();

        // Video: capture → composite → encode → output
        let frames = capture_source_frames(&scene, &source);
        let composited = compositor.compose(&scene, &frames, clock.current_pts_us());
        let packet = encoder.encode_frame(&composited)?;
        if let Some(ref mut mp4) = mp4_out {
            mp4.write_video(&packet)?;
        } else if let Some(ref mut raw) = file_out {
            raw.write_packet(&packet)?;
        }

        // Audio: drain capture → mix (audio data is collected but not yet muxed
        // into the output file — requires tarang container muxing, tracked in roadmap v0.22.0)
        #[cfg(feature = "pipewire")]
        if let Some((ref mut mixer, ref capture_mgr, _)) = audio_state {
            let mut audio_buffers = capture_mgr.drain_buffers();
            if !audio_buffers.is_empty() {
                let _mixed = mixer.mix(&mut audio_buffers);
                // TODO: mux audio into container via tarang::demux::Mp4Muxer
            }
        }

        if (frame_num + 1) % (fps as u64) == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let bytes = mp4_out
                .as_ref()
                .map(|m| m.bytes_written())
                .or_else(|| file_out.as_ref().map(|f| f.bytes_written()))
                .unwrap_or(0);

            #[cfg(feature = "pipewire")]
            if let Some((ref mixer, _, _)) = audio_state {
                let [l, r] = [mixer.master_peak_db(0), mixer.master_peak_db(1)];
                info!(
                    "frame {}: {:.1}s, {} bytes, audio L={:.1}dB R={:.1}dB",
                    frame_num + 1,
                    elapsed,
                    bytes,
                    l,
                    r,
                );
            } else {
                info!("frame {}: {:.1}s, {} bytes", frame_num + 1, elapsed, bytes);
            }
            #[cfg(not(feature = "pipewire"))]
            info!("frame {}: {:.1}s, {} bytes", frame_num + 1, elapsed, bytes);
        }
    }

    // Shut down audio capture
    #[cfg(feature = "pipewire")]
    if let Some((_, ref mut capture_mgr, _)) = audio_state {
        capture_mgr.stop_all();
    }

    // Finalize output
    let total_bytes;
    if let Some(ref mut mp4) = mp4_out {
        mp4.finalize()?;
        total_bytes = mp4.bytes_written();
    } else if let Some(ref mut raw) = file_out {
        raw.close()?;
        total_bytes = raw.bytes_written();
    } else {
        total_bytes = 0;
    }
    info!(
        "done: {} frames, {} bytes written to {}",
        encoder.frames_encoded(),
        total_bytes,
        output_path
    );

    Ok(())
}

fn cmd_preview(source_str: &str, fps: u32, max_frames: u64) -> anyhow::Result<()> {
    let width = 1920;
    let height = 1080;
    let (scene, source) = build_scene(source_str, width, height, fps)?;
    let mut compositor = Compositor::new(width, height);
    let mut clock = FrameClock::new(fps);

    let total = if max_frames == 0 {
        u64::MAX
    } else {
        max_frames
    };

    info!("preview: compositing {fps}fps (no display backend — logging frame stats)");

    let start = Instant::now();
    for frame_num in 0..total {
        clock.tick();

        let frames = capture_source_frames(&scene, &source);

        let composited = compositor.compose(&scene, &frames, clock.current_pts_us());

        if (frame_num + 1) % (fps as u64) == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let actual_fps = (frame_num + 1) as f64 / elapsed;
            info!(
                "frame {}: pts={}µs, {:.0} fps, behind={}",
                frame_num + 1,
                composited.pts_us,
                actual_fps,
                clock.is_behind()
            );
        }
    }

    let elapsed = start.elapsed();
    info!(
        "preview done: {} frames in {:.2}s ({:.1} fps)",
        total.min(max_frames),
        elapsed.as_secs_f64(),
        total.min(max_frames) as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}
