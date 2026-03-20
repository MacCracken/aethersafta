# Aethersafta Architecture

> Real-time media compositing engine for the AGNOS ecosystem.
>
> **Name**: Aethersafta — Greek *aether* (upper sky) + Arabic *safta* (clarity/purity).
> Extracted from the aethersafha desktop compositor as a standalone, reusable crate.

---

## Design Principles

1. **Zero-copy where possible** — frame data flows through the pipeline via `Bytes` references, not allocations
2. **Hardware-first encoding** — ai-hwaccel selects the best available encoder (NVENC > VA-API > QSV > AMF > software)
3. **Tarang for all encoding** — no direct FFI to codecs; tarang owns the encode/mux pipeline
4. **Latency-aware** — every pipeline stage tracks its time budget; nazar integration for alerting
5. **Compositor-agnostic** — works with aethersafha, sway, any Wayland compositor, or headless

---

## Pipeline Architecture

```
                    ┌──────────────────────────────────────────┐
                    │            Scene Graph                    │
                    │  ┌─────────┐ ┌─────────┐ ┌────────────┐│
  Sources ─────────▶│  │ Screen  │ │ Camera  │ │  Overlay   ││
                    │  │ Capture │ │ V4L2    │ │  Text/Img  ││
                    │  └────┬────┘ └────┬────┘ └─────┬──────┘│
                    │       │           │             │        │
                    │  ┌────▼───────────▼─────────────▼──────┐│
                    │  │         Compositor Layer             ││
                    │  │   Z-order, alpha blend, transform    ││
                    │  └──────────────┬──────────────────────┘│
                    └─────────────────┼────────────────────────┘
                                      │
                    ┌─────────────────▼────────────────────────┐
                    │           Encode Pipeline                 │
                    │  ┌──────────┐  ┌──────────┐             │
                    │  │ ai-hwaccel│  │  tarang  │             │
                    │  │ (select) │─▶│ (encode) │             │
                    │  └──────────┘  └────┬─────┘             │
                    └──────────────────────┼───────────────────┘
                                           │
                    ┌──────────────────────▼───────────────────┐
                    │            Output Sinks                   │
                    │  ┌────────┐ ┌───────┐ ┌───────────────┐ │
                    │  │  File  │ │  RTMP │ │     SRT       │ │
                    │  │ MP4/MKV│ │Stream │ │  Low-latency  │ │
                    │  └────────┘ └───────┘ └───────────────┘ │
                    └──────────────────────────────────────────┘
```

---

## Audio Pipeline

```
  PipeWire ──▶ Per-source capture ──▶ Mixer ──▶ tarang audio encode
                                        ▲
                                        │
              DSP (noise gate,    ◀─────┘
              compression, EQ)
```

Audio and video pipelines run independently and are muxed at the output stage by tarang. Clock synchronisation uses PTS (presentation timestamps) aligned to a shared monotonic clock.

---

## Module Structure

```
src/
├── lib.rs              Public API re-exports
├── main.rs             CLI binary (preview, record, stream)
├── scene/              Scene graph and layer management
│   ├── mod.rs          SceneGraph, Layer, LayerId
│   ├── compositor.rs   Alpha blending, z-ordering, transforms
│   └── transition.rs   Cut, fade, slide transitions
├── source/             Input sources
│   ├── mod.rs          Source trait, SourceId
│   ├── screen.rs       Wayland screen capture (wlr-screencopy / ext-image-copy)
│   ├── camera.rs       V4L2 camera capture
│   ├── media.rs        Media file playback (via tarang decode)
│   └── image.rs        Static image / overlay source
├── audio/              Audio capture and mixing
│   ├── mod.rs          AudioMixer, AudioSource
│   ├── pipewire.rs     PipeWire capture backend
│   └── dsp.rs          Noise gate, compressor, EQ
├── encode/             Encoding pipeline
│   ├── mod.rs          EncodePipeline, EncoderConfig
│   ├── hw.rs           Hardware encoder selection via ai-hwaccel
│   └── sw.rs           Software fallback (tarang pure-Rust encoders)
├── output/             Output sinks
│   ├── mod.rs          OutputSink trait
│   ├── file.rs         File recording (MP4, MKV, WebM via tarang muxers)
│   ├── rtmp.rs         RTMP streaming output
│   └── srt.rs          SRT low-latency streaming
├── timing/             Latency tracking and frame scheduling
│   ├── mod.rs          FrameClock, LatencyBudget
│   └── stats.rs        Per-stage timing metrics
└── tests/              Test modules
```

---

## Key Types

### SceneGraph
The central data structure. Owns an ordered list of `Layer`s, each referencing a `Source`. The compositor reads the scene graph on every frame tick and produces a composited frame buffer.

### Source (trait)
```rust
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;
    fn capture_frame(&self) -> Result<RawFrame>;
    fn resolution(&self) -> (u32, u32);
    fn is_live(&self) -> bool;
}
```

### EncodePipeline
Consumes composited frames + mixed audio, selects encoder via ai-hwaccel, encodes through tarang, and pushes to output sinks.

### OutputSink (trait)
```rust
pub trait OutputSink: Send + Sync {
    fn write_packet(&mut self, packet: &EncodedPacket) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}
```

---

## Dependencies

| Crate | Role |
|-------|------|
| **tarang** | Video/audio encoding, container muxing (MP4/MKV/WebM) |
| **ai-hwaccel** | Hardware encoder detection (NVENC, VA-API, QSV, AMF) |
| **pipewire** | Audio capture from system audio graph |
| **tokio** | Async runtime for concurrent capture + encode + output |

---

## Consumers

| Project | Usage |
|---------|-------|
| **aethersafha** | Built-in screen recording + compositor capture |
| **Streaming app** | OBS-like live broadcast production |
| **tazama** | Real-time preview during video editing |
| **SecureYeoman** | Sandbox session recording + security overlay annotations |
| **Video conferencing** | Call compositing (camera + screen share + overlays) |
| **selah** | Screenshot with overlay annotations (simple single-frame path) |
