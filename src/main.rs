//! aethersafta CLI — record, preview, and stream composited scenes.
//!
//! Usage:
//!   aethersafta record --source screen --output recording.mp4
//!   aethersafta preview --source screen
//!   aethersafta info
//!   aethersafta --version

use clap::{Parser, Subcommand};

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
        /// Source type: screen, camera, media
        #[arg(long, default_value = "screen")]
        source: String,
        /// Output file path
        #[arg(short, long)]
        output: String,
        /// Duration in seconds (0 = until stopped)
        #[arg(long, default_value = "0")]
        duration: u64,
        /// Target framerate
        #[arg(long, default_value = "30")]
        fps: u32,
    },
    /// Preview composited output (display only, no recording)
    Preview {
        /// Source type
        #[arg(long, default_value = "screen")]
        source: String,
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
        Commands::Info => {
            println!("aethersafta v{}", env!("CARGO_PKG_VERSION"));
            println!();

            #[cfg(feature = "hwaccel")]
            {
                let registry = ai_hwaccel::AcceleratorRegistry::detect();
                println!("Hardware accelerators:");
                for p in registry.all_profiles() {
                    let mem_gb = p.memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    println!("  {} ({:.1} GB)", p.accelerator, mem_gb);
                }
                println!();
            }

            println!("Supported outputs: file (MP4/MKV/WebM)");
            #[cfg(feature = "rtmp")]
            println!("  + RTMP streaming");
            #[cfg(feature = "srt")]
            println!("  + SRT streaming");
        }
        Commands::Record {
            source,
            output,
            duration,
            fps,
        } => {
            println!(
                "Recording: source={}, output={}, fps={}, duration={}s",
                source,
                output,
                fps,
                if duration == 0 {
                    "unlimited".to_string()
                } else {
                    duration.to_string()
                }
            );
            println!("(not yet implemented — scaffold only)");
        }
        Commands::Preview { source } => {
            println!("Preview: source={}", source);
            println!("(not yet implemented — scaffold only)");
        }
    }
}
