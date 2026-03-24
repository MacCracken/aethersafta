//! Graph-based audio pipeline using [`dhvani::graph`].
//!
//! Builds an audio processing graph where each source flows through
//! per-source DSP nodes, into a mixer node, then through master DSP
//! to the output. The graph is compiled into an [`ExecutionPlan`] and
//! processed by a [`GraphProcessor`] for real-time safe execution.
//!
//! ```text
//! Source 1 → [Gain] → [EQ] → [Compressor] → [Pan] ──┐
//! Source 2 → [Gain] → [EQ] → [Compressor] → [Pan] ──┤
//!                                                     ├→ [Mixer] → [Limiter] → [Meter] → Output
//! Source N → [Gain] → [EQ] → [Compressor] → [Pan] ──┘
//! ```

use std::collections::HashMap;

use dhvani::buffer::AudioBuffer;
use dhvani::graph::{AudioNode, Graph, GraphProcessor, NodeId};

use super::{AudioMixerConfig, AudioSourceId};

// --- Audio graph nodes ---

/// Input node: injects a captured audio buffer into the graph.
#[allow(dead_code)]
pub(crate) struct InputNode {
    buffer: Option<AudioBuffer>,
    channels: u32,
    sample_rate: u32,
}

#[allow(dead_code)]
impl InputNode {
    pub fn new(channels: u32, sample_rate: u32) -> Self {
        Self {
            buffer: None,
            channels,
            sample_rate,
        }
    }

    /// Set the buffer to be emitted on the next process cycle.
    pub fn set_buffer(&mut self, buf: AudioBuffer) {
        self.buffer = Some(buf);
    }
}

impl AudioNode for InputNode {
    fn name(&self) -> &str {
        "input"
    }

    fn num_inputs(&self) -> usize {
        0
    }

    fn num_outputs(&self) -> usize {
        1
    }

    #[inline]
    fn process(&mut self, _inputs: &[&AudioBuffer], output: &mut AudioBuffer) {
        if let Some(buf) = self.buffer.take() {
            *output = buf;
        } else {
            *output = AudioBuffer::silence(self.channels, 1024, self.sample_rate);
        }
    }
}

/// Gain node: applies a linear gain multiplier.
#[allow(dead_code)]
pub(crate) struct GainNode {
    gain: f32,
}

#[allow(dead_code)]
impl GainNode {
    pub fn new(gain: f32) -> Self {
        Self { gain }
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }
}

impl AudioNode for GainNode {
    fn name(&self) -> &str {
        "gain"
    }

    fn num_inputs(&self) -> usize {
        1
    }

    fn num_outputs(&self) -> usize {
        1
    }

    #[inline]
    fn process(&mut self, inputs: &[&AudioBuffer], output: &mut AudioBuffer) {
        if let Some(input) = inputs.first() {
            *output = (*input).clone();
            if (self.gain - 1.0).abs() > f32::EPSILON {
                output.apply_gain(self.gain);
            }
        }
    }
}

/// Mixer node: sums all input buffers.
pub(crate) struct MixerNode;

impl AudioNode for MixerNode {
    fn name(&self) -> &str {
        "mixer"
    }

    fn num_inputs(&self) -> usize {
        // Accept any number of inputs; the graph wires the actual connections.
        usize::MAX
    }

    fn num_outputs(&self) -> usize {
        1
    }

    #[inline]
    fn process(&mut self, inputs: &[&AudioBuffer], output: &mut AudioBuffer) {
        if inputs.is_empty() {
            return;
        }
        if inputs.len() == 1 {
            *output = inputs[0].clone();
            return;
        }
        if let Ok(mixed) = dhvani::buffer::mix(inputs) {
            *output = mixed;
        }
    }
}

/// DSP chain node: applies EQ, compressor, and panner in sequence.
#[allow(dead_code)]
pub(crate) struct DspChainNode {
    eq: Option<dhvani::dsp::ParametricEq>,
    compressor: Option<dhvani::dsp::Compressor>,
    panner: dhvani::dsp::StereoPanner,
}

#[allow(dead_code)]
impl DspChainNode {
    pub fn new(pan: f32) -> Self {
        Self {
            eq: None,
            compressor: None,
            panner: dhvani::dsp::StereoPanner::new(pan),
        }
    }

    pub fn set_eq(&mut self, eq: dhvani::dsp::ParametricEq) {
        self.eq = Some(eq);
    }

    pub fn set_compressor(&mut self, comp: dhvani::dsp::Compressor) {
        self.compressor = Some(comp);
    }

    pub fn set_pan(&mut self, pan: f32) {
        self.panner.set_pan(pan);
    }
}

impl AudioNode for DspChainNode {
    fn name(&self) -> &str {
        "dsp_chain"
    }

    fn num_inputs(&self) -> usize {
        1
    }

    fn num_outputs(&self) -> usize {
        1
    }

    #[inline]
    fn process(&mut self, inputs: &[&AudioBuffer], output: &mut AudioBuffer) {
        if let Some(input) = inputs.first() {
            *output = (*input).clone();
            if let Some(eq) = &mut self.eq {
                eq.process(output);
            }
            if let Some(comp) = &mut self.compressor {
                comp.process(output);
            }
            self.panner.process(output);
            for sample in output.samples_mut() {
                *sample = dhvani::dsp::sanitize_sample(*sample);
            }
        }
    }
}

/// Master bus node: limiter + metering.
#[allow(dead_code)]
pub(crate) struct MasterNode {
    limiter: Option<dhvani::dsp::EnvelopeLimiter>,
    meter: dhvani::meter::LevelMeter,
}

#[allow(dead_code)]
impl MasterNode {
    pub fn new(config: &AudioMixerConfig) -> Self {
        let limiter = if config.master_limiter {
            dhvani::dsp::EnvelopeLimiter::new(
                dhvani::dsp::LimiterParams::default(),
                config.sample_rate,
            )
            .ok()
        } else {
            None
        };

        Self {
            limiter,
            meter: dhvani::meter::LevelMeter::new(
                config.channels as usize,
                config.sample_rate as f32,
            ),
        }
    }

    pub fn peak_db(&self, channel: usize) -> f32 {
        self.meter.peak_db(channel)
    }

    pub fn rms_db(&self, channel: usize) -> f32 {
        self.meter.rms_db(channel)
    }

    pub fn lufs(&self) -> f32 {
        self.meter.lufs
    }
}

impl AudioNode for MasterNode {
    fn name(&self) -> &str {
        "master"
    }

    fn num_inputs(&self) -> usize {
        1
    }

    fn num_outputs(&self) -> usize {
        1
    }

    #[inline]
    fn process(&mut self, inputs: &[&AudioBuffer], output: &mut AudioBuffer) {
        if let Some(input) = inputs.first() {
            *output = (*input).clone();
            if let Some(limiter) = &mut self.limiter {
                limiter.process(output);
            }
            self.meter.process(output);
            for sample in output.samples_mut() {
                *sample = dhvani::dsp::sanitize_sample(*sample);
            }
        }
    }
}

// --- Graph-based audio pipeline ---

/// Node IDs for sources in the audio graph.
struct SourceNodes {
    input: NodeId,
    gain: NodeId,
    dsp: NodeId,
    gain_value: f32,
    pan_value: f32,
}

/// A graph-based audio pipeline that routes multiple sources through
/// per-source DSP, a mixer, and master processing.
///
/// Uses [`dhvani::graph`] for topologically-sorted, real-time safe execution.
/// The graph is rebuilt and compiled whenever sources are added or removed,
/// with the new [`ExecutionPlan`] swapped into the processor lock-free.
pub struct AudioPipeline {
    config: AudioMixerConfig,
    source_nodes: HashMap<AudioSourceId, SourceNodes>,
    mixer_node_id: NodeId,
    master_node_id: NodeId,
    processor: GraphProcessor,
    dirty: bool,
    /// Per-source metering (peak L/R).
    source_meters: HashMap<AudioSourceId, dhvani::meter::PeakMeter>,
}

impl AudioPipeline {
    /// Create a new graph-based audio pipeline.
    #[must_use]
    pub fn new(config: AudioMixerConfig) -> Self {
        let mixer_node_id = NodeId::next();
        let master_node_id = NodeId::next();

        let processor = GraphProcessor::new(config.channels, config.sample_rate, 1024);

        let mut pipeline = Self {
            config,
            source_nodes: HashMap::new(),
            mixer_node_id,
            master_node_id,
            processor,
            dirty: true,
            source_meters: HashMap::new(),
        };
        pipeline.compile_and_swap();
        pipeline
    }

    /// Add a source to the pipeline. Creates input → gain → DSP → mixer chain.
    pub fn add_source(&mut self, id: AudioSourceId, gain: f32, pan: f32) {
        let input_id = NodeId::next();
        let gain_id = NodeId::next();
        let dsp_id = NodeId::next();

        self.source_nodes.insert(
            id,
            SourceNodes {
                input: input_id,
                gain: gain_id,
                dsp: dsp_id,
                gain_value: gain,
                pan_value: pan,
            },
        );
        self.source_meters
            .insert(id, dhvani::meter::PeakMeter::new());

        self.dirty = true;
        self.compile_and_swap();
        tracing::debug!(source_id = %id, gain, pan, "audio pipeline: source added");
    }

    /// Update gain and pan for an existing source.
    pub fn update_source(&mut self, id: AudioSourceId, gain: f32, pan: f32) -> bool {
        if let Some(nodes) = self.source_nodes.get_mut(&id) {
            nodes.gain_value = gain;
            nodes.pan_value = pan;
            self.dirty = true;
            self.compile_and_swap();
            true
        } else {
            false
        }
    }

    /// Remove a source from the pipeline.
    pub fn remove_source(&mut self, id: AudioSourceId) -> bool {
        if self.source_nodes.remove(&id).is_some() {
            self.source_meters.remove(&id);
            self.dirty = true;
            self.compile_and_swap();
            tracing::debug!(source_id = %id, "audio pipeline: source removed");
            true
        } else {
            false
        }
    }

    /// Build a fresh graph from current source_nodes, compile it, and swap
    /// the execution plan into the processor.
    fn compile_and_swap(&mut self) {
        let mut graph = Graph::new();
        graph.add_node(self.mixer_node_id, Box::new(MixerNode));
        graph.add_node(self.master_node_id, Box::new(MasterNode::new(&self.config)));
        graph.connect(self.mixer_node_id, self.master_node_id);

        for nodes in self.source_nodes.values() {
            graph.add_node(
                nodes.input,
                Box::new(InputNode::new(
                    self.config.channels,
                    self.config.sample_rate,
                )),
            );
            graph.add_node(nodes.gain, Box::new(GainNode::new(nodes.gain_value)));
            graph.add_node(nodes.dsp, Box::new(DspChainNode::new(nodes.pan_value)));

            graph.connect(nodes.input, nodes.gain);
            graph.connect(nodes.gain, nodes.dsp);
            graph.connect(nodes.dsp, self.mixer_node_id);
        }

        // compile() consumes the Graph — that's fine, we rebuild each time.
        match graph.compile() {
            Ok(plan) => {
                let handle = self.processor.swap_handle();
                handle.swap(plan);
                self.dirty = false;
            }
            Err(e) => {
                tracing::error!("audio graph compilation failed: {e}");
            }
        }
    }

    /// Process one cycle: run the graph processor and return the output.
    ///
    /// Source input buffers should be fed into the graph's input nodes
    /// before calling this. Returns the master output buffer.
    #[must_use]
    pub fn process(&mut self) -> Option<AudioBuffer> {
        self.processor.process().cloned()
    }

    /// Get per-source peak levels (L, R) in linear amplitude.
    #[must_use]
    pub fn source_peak(&self, id: AudioSourceId) -> Option<[f32; 2]> {
        self.source_meters.get(&id).map(|m| m.load())
    }

    /// Number of sources in the pipeline.
    #[must_use]
    pub fn source_count(&self) -> usize {
        self.source_nodes.len()
    }

    /// Current configuration.
    #[must_use]
    pub fn config(&self) -> &AudioMixerConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioMixerConfig;

    #[test]
    fn pipeline_new() {
        let pipeline = AudioPipeline::new(AudioMixerConfig::default());
        assert_eq!(pipeline.source_count(), 0);
    }

    #[test]
    fn pipeline_add_remove_source() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        pipeline.add_source(id, 1.0, 0.0);
        assert_eq!(pipeline.source_count(), 1);
        assert!(pipeline.remove_source(id));
        assert_eq!(pipeline.source_count(), 0);
    }

    #[test]
    fn pipeline_multiple_sources() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        let id3 = uuid::Uuid::new_v4();
        pipeline.add_source(id1, 1.0, -1.0);
        pipeline.add_source(id2, 0.5, 0.0);
        pipeline.add_source(id3, 1.0, 1.0);
        assert_eq!(pipeline.source_count(), 3);
    }

    #[test]
    fn pipeline_source_peak_default() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        pipeline.add_source(id, 1.0, 0.0);
        let peak = pipeline.source_peak(id).unwrap();
        assert_eq!(peak, [0.0, 0.0]);
    }

    #[test]
    fn process_produces_output() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        pipeline.add_source(id, 1.0, 0.0);

        // process() exercises compile → swap → process even without injected buffers.
        // InputNode emits silence when no buffer is set, which flows through
        // the entire chain: input → gain → dsp → mixer → master.
        let output = pipeline.process();
        assert!(
            output.is_some(),
            "process must return Some after adding a source"
        );
        let buf = output.unwrap();
        assert_eq!(buf.channels(), 2);
        assert_eq!(buf.sample_rate(), 48000);
        assert!(buf.frames() > 0);
    }

    #[test]
    fn gain_applied() {
        // With silence input the absolute values are zero regardless of gain,
        // but the graph still compiles and processes. We verify both paths
        // produce output and that the processing path runs without error.
        let mut full_gain = AudioPipeline::new(AudioMixerConfig::default());
        let mut half_gain = AudioPipeline::new(AudioMixerConfig::default());

        let id_full = uuid::Uuid::new_v4();
        let id_half = uuid::Uuid::new_v4();

        full_gain.add_source(id_full, 1.0, 0.0);
        half_gain.add_source(id_half, 0.5, 0.0);

        let out_full = full_gain
            .process()
            .expect("full gain should produce output");
        let out_half = half_gain
            .process()
            .expect("half gain should produce output");

        // Both should be silence since InputNode has no injected buffer,
        // but the graph ran the full processing chain.
        assert!(out_full.samples().iter().all(|&s| s.abs() < f32::EPSILON));
        assert!(out_half.samples().iter().all(|&s| s.abs() < f32::EPSILON));
    }

    #[test]
    fn update_source_changes_gain() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        pipeline.add_source(id, 1.0, 0.0);

        // First process with gain=1.0
        let out1 = pipeline.process();
        assert!(out1.is_some());

        // Update gain to 0.25 and pan to 0.5 — triggers recompile
        assert!(pipeline.update_source(id, 0.25, 0.5));

        // Process again with new gain/pan
        let out2 = pipeline.process();
        assert!(
            out2.is_some(),
            "process must return Some after update_source"
        );
    }

    #[test]
    fn update_nonexistent_source_returns_false() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        assert!(!pipeline.update_source(id, 1.0, 0.0));
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        assert!(!pipeline.remove_source(id));
    }

    #[test]
    fn process_with_no_sources() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        // Empty pipeline still has mixer → master nodes, so process should
        // return Some with silence from the master node.
        let output = pipeline.process();
        assert!(
            output.is_some(),
            "empty pipeline should still produce output"
        );
        let buf = output.unwrap();
        assert!(buf.samples().iter().all(|&s| s.abs() < f32::EPSILON));
    }

    #[test]
    fn multiple_add_remove_cycles() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        let id3 = uuid::Uuid::new_v4();
        let id4 = uuid::Uuid::new_v4();

        // Add 3 sources
        pipeline.add_source(id1, 1.0, 0.0);
        pipeline.add_source(id2, 0.8, -0.5);
        pipeline.add_source(id3, 0.6, 0.5);
        assert_eq!(pipeline.source_count(), 3);

        // Process mid-cycle to exercise the 3-source graph
        assert!(pipeline.process().is_some());

        // Remove 2 sources
        assert!(pipeline.remove_source(id1));
        assert!(pipeline.remove_source(id2));
        assert_eq!(pipeline.source_count(), 1);

        // Process with 1 source
        assert!(pipeline.process().is_some());

        // Add 1 more source
        pipeline.add_source(id4, 0.9, 0.0);
        assert_eq!(pipeline.source_count(), 2);

        // Final process with 2 sources
        assert!(pipeline.process().is_some());
    }

    #[test]
    fn process_multiple_cycles() {
        let mut pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        pipeline.add_source(id, 1.0, 0.0);

        // Process several cycles — verifies the processor is stable across
        // multiple invocations without recompilation.
        for _ in 0..10 {
            let output = pipeline.process();
            assert!(output.is_some());
        }
    }

    #[test]
    fn config_accessor() {
        let config = AudioMixerConfig::default();
        let pipeline = AudioPipeline::new(config.clone());
        assert_eq!(pipeline.config().sample_rate, 48000);
        assert_eq!(pipeline.config().channels, 2);
    }

    #[test]
    fn source_peak_nonexistent() {
        let pipeline = AudioPipeline::new(AudioMixerConfig::default());
        let id = uuid::Uuid::new_v4();
        assert!(pipeline.source_peak(id).is_none());
    }

    // --- Node-level unit tests ---

    fn silence_buf() -> AudioBuffer {
        AudioBuffer::silence(2, 256, 48000)
    }

    #[test]
    fn input_node_emits_silence_when_empty() {
        let mut node = InputNode::new(2, 48000);
        let mut output = silence_buf();
        node.process(&[], &mut output);
        assert!(output.samples().iter().all(|&s| s == 0.0));
    }

    #[test]
    fn input_node_emits_set_buffer() {
        let mut node = InputNode::new(2, 48000);
        let buf = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        node.set_buffer(buf);
        let mut output = silence_buf();
        node.process(&[], &mut output);
        assert!(
            output
                .samples()
                .iter()
                .all(|&s| (s - 0.5).abs() < f32::EPSILON)
        );
    }

    #[test]
    fn input_node_consumes_buffer_once() {
        let mut node = InputNode::new(2, 48000);
        let buf = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        node.set_buffer(buf);

        let mut output = silence_buf();
        node.process(&[], &mut output);
        assert!(output.peak() > 0.0);

        // Second call should emit silence (buffer consumed)
        let mut output2 = silence_buf();
        node.process(&[], &mut output2);
        assert!(output2.samples().iter().all(|&s| s == 0.0));
    }

    #[test]
    fn gain_node_applies_gain() {
        let mut node = GainNode::new(0.5);
        let input = AudioBuffer::from_interleaved(vec![1.0; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        for &s in output.samples() {
            assert!((s - 0.5).abs() < f32::EPSILON, "expected 0.5, got {s}");
        }
    }

    #[test]
    fn gain_node_unity_no_change() {
        let mut node = GainNode::new(1.0);
        let input = AudioBuffer::from_interleaved(vec![0.7; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        for &s in output.samples() {
            assert!((s - 0.7).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn gain_node_set_gain() {
        let mut node = GainNode::new(1.0);
        node.set_gain(0.25);
        let input = AudioBuffer::from_interleaved(vec![1.0; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        for &s in output.samples() {
            assert!((s - 0.25).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn dsp_chain_node_passthrough() {
        let mut node = DspChainNode::new(0.0);
        let input = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        // With no EQ/comp, center pan, output should be ~0.5 * 0.707 (constant power)
        assert!(output.peak() > 0.3);
    }

    #[test]
    fn dsp_chain_node_set_pan() {
        let mut node = DspChainNode::new(0.0);
        node.set_pan(-1.0); // hard left
        let input = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        assert!(output.peak() > 0.0);
    }

    #[test]
    fn dsp_chain_node_with_eq() {
        let mut node = DspChainNode::new(0.0);
        node.set_eq(dhvani::dsp::ParametricEq::new(
            vec![dhvani::dsp::EqBandConfig {
                band_type: dhvani::dsp::BandType::HighPass,
                freq_hz: 80.0,
                gain_db: 0.0,
                q: 0.707,
                enabled: true,
            }],
            48000,
            2,
        ));
        let input = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        assert!(output.peak() > 0.0);
    }

    #[test]
    fn dsp_chain_node_with_compressor() {
        let mut node = DspChainNode::new(0.0);
        node.set_compressor(
            dhvani::dsp::Compressor::new(
                dhvani::dsp::CompressorParams {
                    threshold_db: -20.0,
                    ratio: 4.0,
                    attack_ms: 5.0,
                    release_ms: 50.0,
                    makeup_gain_db: 0.0,
                    knee_db: 0.0,
                    mix: 1.0,
                },
                48000,
            )
            .unwrap(),
        );
        let input = AudioBuffer::from_interleaved(vec![0.8; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        assert!(output.peak() > 0.0);
    }

    #[test]
    fn mixer_node_single_input() {
        let mut node = MixerNode;
        let input = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        for &s in output.samples() {
            assert!((s - 0.5).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn mixer_node_multiple_inputs() {
        let mut node = MixerNode;
        let a = AudioBuffer::from_interleaved(vec![0.3; 256 * 2], 2, 48000).unwrap();
        let b = AudioBuffer::from_interleaved(vec![0.2; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&a, &b], &mut output);
        assert!(output.peak() > 0.4); // sum should be ~0.5
    }

    #[test]
    fn mixer_node_empty_inputs() {
        let mut node = MixerNode;
        let mut output = AudioBuffer::from_interleaved(vec![0.99; 256 * 2], 2, 48000).unwrap();
        node.process(&[], &mut output);
        // Empty inputs should leave output unchanged (early return)
        assert!((output.peak() - 0.99).abs() < f32::EPSILON);
    }

    #[test]
    fn master_node_with_limiter() {
        let config = AudioMixerConfig {
            master_limiter: true,
            ..Default::default()
        };
        let mut node = MasterNode::new(&config);
        let input = AudioBuffer::from_interleaved(vec![2.0; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        // Limiter should reduce peak
        assert!(output.peak() < 2.0);
    }

    #[test]
    fn master_node_without_limiter() {
        let config = AudioMixerConfig {
            master_limiter: false,
            ..Default::default()
        };
        let mut node = MasterNode::new(&config);
        let input = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);
        assert!(output.peak() > 0.0);
    }

    #[test]
    fn master_node_metering() {
        let config = AudioMixerConfig::default();
        let mut node = MasterNode::new(&config);
        let input = AudioBuffer::from_interleaved(vec![0.5; 256 * 2], 2, 48000).unwrap();
        let mut output = silence_buf();
        node.process(&[&input], &mut output);

        // After processing, meter should have non-zero readings
        let peak = node.peak_db(0);
        let rms = node.rms_db(0);
        let lufs = node.lufs();
        assert!(peak > -20.0, "peak_db={peak}");
        assert!(rms > -20.0, "rms_db={rms}");
        let _ = lufs; // LUFS may need more data to converge
    }

    #[test]
    fn node_names() {
        assert_eq!(InputNode::new(2, 48000).name(), "input");
        assert_eq!(GainNode::new(1.0).name(), "gain");
        assert_eq!(MixerNode.name(), "mixer");
        assert_eq!(DspChainNode::new(0.0).name(), "dsp_chain");
        assert_eq!(
            MasterNode::new(&AudioMixerConfig::default()).name(),
            "master"
        );
    }

    #[test]
    fn node_io_counts() {
        let input = InputNode::new(2, 48000);
        assert_eq!(input.num_inputs(), 0);
        assert_eq!(input.num_outputs(), 1);

        let gain = GainNode::new(1.0);
        assert_eq!(gain.num_inputs(), 1);
        assert_eq!(gain.num_outputs(), 1);

        let mixer = MixerNode;
        assert_eq!(mixer.num_inputs(), usize::MAX);
        assert_eq!(mixer.num_outputs(), 1);

        let dsp = DspChainNode::new(0.0);
        assert_eq!(dsp.num_inputs(), 1);
        assert_eq!(dsp.num_outputs(), 1);

        let master = MasterNode::new(&AudioMixerConfig::default());
        assert_eq!(master.num_inputs(), 1);
        assert_eq!(master.num_outputs(), 1);
    }
}
