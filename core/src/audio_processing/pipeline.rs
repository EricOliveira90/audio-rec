//! Real-time audio processing pipeline for microphone capture.
//!
//! Pipeline order: Resample to 48kHz → RNNoise → Noise Gate (VAD-driven) → Limiter → Resample to native → output
//! Each stage is independently toggleable via CLI flags.

use super::limiter::{Limiter, LimiterConfig};
use super::noise_gate::{NoiseGate, NoiseGateConfig};
use super::noise_suppress::{NoiseSuppressor, NoiseSuppressorConfig};

/// Configuration for the real-time audio processing pipeline
#[derive(Debug, Clone)]
pub struct AudioPipelineConfig {
    /// Whether noise suppression (RNNoise) is enabled
    pub denoise: bool,
    /// Whether the noise gate is enabled
    pub noise_gate: bool,
    /// Whether the real-time limiter is enabled
    pub limiter: bool,
    /// Input sample rate
    pub sample_rate: u32,
    /// Input channels
    pub channels: u16,
    /// Gate threshold (VAD probability 0.0-1.0 or dB)
    pub gate_threshold: f64,
}

impl Default for AudioPipelineConfig {
    fn default() -> Self {
        Self {
            denoise: true,
            noise_gate: true,
            limiter: true,
            sample_rate: 48000,
            channels: 1,
            gate_threshold: 0.5,
        }
    }
}

/// Real-time audio processing pipeline for microphone capture.
/// Chains: RNNoise → Noise Gate → Limiter
pub struct AudioPipeline {
    config: AudioPipelineConfig,
    suppressor: NoiseSuppressor,
    gate: NoiseGate,
    limiter: Limiter,
}

impl AudioPipeline {
    pub fn new(config: AudioPipelineConfig) -> Self {
        let suppressor = NoiseSuppressor::new(NoiseSuppressorConfig {
            sample_rate: config.sample_rate,
            channels: config.channels,
            enabled: config.denoise,
        });

        let gate = NoiseGate::new(NoiseGateConfig {
            open_threshold: config.gate_threshold,
            close_threshold: config.gate_threshold * 0.6, // Hysteresis
            enabled: config.noise_gate,
            vad_driven: config.denoise, // Use VAD when denoise is active
            ..Default::default()
        });

        let limiter = Limiter::new(LimiterConfig {
            enabled: config.limiter,
            ..Default::default()
        });

        Self {
            config,
            suppressor,
            gate,
            limiter,
        }
    }

    /// Process a chunk of audio samples through the full pipeline.
    /// Modifies samples in-place.
    pub fn process(&mut self, samples: &mut Vec<f32>) {
        // Stage 1: RNNoise (denoise + VAD)
        let vad_probability = if self.config.denoise {
            let result = self.suppressor.process(samples);
            *samples = result.samples;
            result.vad_probability as f64
        } else {
            // When denoise is off, estimate level from RMS for the gate
            if samples.is_empty() {
                0.0
            } else {
                let rms: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / samples.len() as f64;
                rms.sqrt()
            }
        };

        // Stage 2: Noise Gate (VAD-driven or dB threshold)
        if self.config.noise_gate {
            self.gate.process(samples, vad_probability);
        }

        // Stage 3: Limiter
        if self.config.limiter {
            self.limiter.process(samples);
        }
    }

    /// Check if any processing stage is active
    pub fn has_active_stages(&self) -> bool {
        self.config.denoise || self.config.noise_gate || self.config.limiter
    }

    /// Get a summary of which stages are active
    pub fn active_stages_summary(&self) -> Vec<&'static str> {
        let mut stages = Vec::new();
        if self.config.denoise { stages.push("denoise"); }
        if self.config.noise_gate { stages.push("noise_gate"); }
        if self.config.limiter { stages.push("limiter"); }
        stages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_all_stages_active_by_default() {
        let pipeline = AudioPipeline::new(AudioPipelineConfig::default());
        assert!(pipeline.has_active_stages());
        let stages = pipeline.active_stages_summary();
        assert_eq!(stages, vec!["denoise", "noise_gate", "limiter"]);
    }

    #[test]
    fn pipeline_no_stages_when_all_disabled() {
        let pipeline = AudioPipeline::new(AudioPipelineConfig {
            denoise: false,
            noise_gate: false,
            limiter: false,
            ..Default::default()
        });
        assert!(!pipeline.has_active_stages());
        assert!(pipeline.active_stages_summary().is_empty());
    }

    #[test]
    fn pipeline_process_does_not_crash_on_empty() {
        let mut pipeline = AudioPipeline::new(AudioPipelineConfig::default());
        let mut samples: Vec<f32> = vec![];
        pipeline.process(&mut samples);
        assert!(samples.is_empty());
    }

    #[test]
    fn pipeline_process_preserves_sample_count() {
        let mut pipeline = AudioPipeline::new(AudioPipelineConfig::default());
        let mut samples: Vec<f32> = vec![0.5; 480];
        pipeline.process(&mut samples);
        assert_eq!(samples.len(), 480);
    }

    #[test]
    fn pipeline_with_only_limiter() {
        let mut pipeline = AudioPipeline::new(AudioPipelineConfig {
            denoise: false,
            noise_gate: false,
            limiter: true,
            ..Default::default()
        });
        let mut samples = vec![2.0f32; 10]; // Above limiter threshold
        pipeline.process(&mut samples);
        // All samples should be limited
        for &s in &samples {
            assert!(s.abs() <= 0.96);
        }
    }

    #[test]
    fn pipeline_with_only_denoise() {
        let mut pipeline = AudioPipeline::new(AudioPipelineConfig {
            denoise: true,
            noise_gate: false,
            limiter: false,
            ..Default::default()
        });
        let mut samples = vec![0.5f32; 480];
        pipeline.process(&mut samples);
        assert_eq!(samples.len(), 480);
    }

    #[test]
    fn pipeline_disabled_passes_through() {
        let mut pipeline = AudioPipeline::new(AudioPipelineConfig {
            denoise: false,
            noise_gate: false,
            limiter: false,
            ..Default::default()
        });
        let original = vec![0.3f32, -0.5, 0.8, -0.1];
        let mut samples = original.clone();
        pipeline.process(&mut samples);
        assert_eq!(samples, original);
    }

    #[test]
    fn pipeline_each_stage_independently_disabled() {
        // Only gate
        let pipeline = AudioPipeline::new(AudioPipelineConfig {
            denoise: false,
            noise_gate: true,
            limiter: false,
            ..Default::default()
        });
        assert_eq!(pipeline.active_stages_summary(), vec!["noise_gate"]);

        // Only denoise
        let pipeline = AudioPipeline::new(AudioPipelineConfig {
            denoise: true,
            noise_gate: false,
            limiter: false,
            ..Default::default()
        });
        assert_eq!(pipeline.active_stages_summary(), vec!["denoise"]);

        // Only limiter
        let pipeline = AudioPipeline::new(AudioPipelineConfig {
            denoise: false,
            noise_gate: false,
            limiter: true,
            ..Default::default()
        });
        assert_eq!(pipeline.active_stages_summary(), vec!["limiter"]);
    }
}
