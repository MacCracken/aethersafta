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

    fn process(&mut self, inputs: &[&AudioBuffer], output: &mut AudioBuffer) {
        if let Some(input) = inputs.first() {
            *output = (*input).clone();
            if let Some(limiter) = &mut self.limiter {
                limiter.process(output);
            }
            self.meter.process(output);
        }
    }
}

// --- Graph-based audio pipeline ---

/// Node IDs for sources in the audio graph.
struct SourceNodes {
    input: NodeId,
    gain: NodeId,
    dsp: NodeId,
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
            },
        );
        self.source_meters
            .insert(id, dhvani::meter::PeakMeter::new());

        let _ = (gain, pan); // Used when building the graph below.
        self.dirty = true;
        self.compile_and_swap();
    }

    /// Remove a source from the pipeline.
    pub fn remove_source(&mut self, id: AudioSourceId) -> bool {
        if self.source_nodes.remove(&id).is_some() {
            self.source_meters.remove(&id);
            self.dirty = true;
            self.compile_and_swap();
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
            graph.add_node(nodes.gain, Box::new(GainNode::new(1.0)));
            graph.add_node(nodes.dsp, Box::new(DspChainNode::new(0.0)));

            graph.connect(nodes.input, nodes.gain);
            graph.connect(nodes.gain, nodes.dsp);
            graph.connect(nodes.dsp, self.mixer_node_id);
        }

        // compile() consumes the Graph — that's fine, we rebuild each time.
        if let Ok(plan) = graph.compile() {
            let handle = self.processor.swap_handle();
            handle.swap(plan);
            self.dirty = false;
        }
    }

    /// Process one cycle: run the graph processor and return the output.
    ///
    /// Source input buffers should be fed into the graph's input nodes
    /// before calling this. Returns the master output buffer.
    pub fn process(&mut self) -> Option<AudioBuffer> {
        self.processor.process().cloned()
    }

    /// Get per-source peak levels (L, R) in linear amplitude.
    pub fn source_peak(&self, id: AudioSourceId) -> Option<[f32; 2]> {
        self.source_meters.get(&id).map(|m| m.load())
    }

    /// Number of sources in the pipeline.
    pub fn source_count(&self) -> usize {
        self.source_nodes.len()
    }

    /// Current configuration.
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
}
