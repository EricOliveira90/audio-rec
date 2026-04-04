//! Real-time noise suppression using RNNoise (nnnoiseless crate).
//!
//! RNNoise requires 48kHz mono float32 in 480-sample frames.
//! This module handles stereo downmix/upmix and resampling internally.
//! VAD (voice activity detection) probability is exposed per frame.
//!
//! Placeholder — actual nnnoiseless integration will be added when
//! the dependency is included in Cargo.toml.

/// Result of processing a frame through RNNoise
#[derive(Debug, Clone)]
pub struct DenoiseResult {
    /// Denoised audio samples (same length as input)
    pub samples: Vec<f32>,
    /// Voice activity detection probability (0.0 = noise, 1.0 = speech)
    pub vad_probability: f32,
}

/// Configuration for the noise suppressor
#[derive(Debug, Clone)]
pub struct NoiseSuppressorConfig {
    /// Input sample rate (will be resampled to 48kHz internally if needed)
    pub sample_rate: u32,
    /// Number of input channels (will be downmixed to mono internally if needed)
    pub channels: u16,
    /// Whether the suppressor is enabled
    pub enabled: bool,
}

impl Default for NoiseSuppressorConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 1,
            enabled: true,
        }
    }
}

/// Real-time noise suppressor wrapping RNNoise.
pub struct NoiseSuppressor {
    config: NoiseSuppressorConfig,
    /// Buffer for accumulating samples until we have a full 480-sample frame
    frame_buffer: Vec<f32>,
    /// Whether RNNoise has been disabled due to a panic
    disabled_due_to_panic: bool,
    /// Performance tracking: total frames processed
    frames_processed: u64,
    /// Performance tracking: total processing time in microseconds
    total_processing_us: u64,
}

impl NoiseSuppressor {
    pub fn new(config: NoiseSuppressorConfig) -> Self {
        Self {
            config,
            frame_buffer: Vec::with_capacity(480),
            disabled_due_to_panic: false,
            frames_processed: 0,
            total_processing_us: 0,
        }
    }

    /// Check if the suppressor is active (enabled and not disabled due to panic)
    pub fn is_active(&self) -> bool {
        self.config.enabled && !self.disabled_due_to_panic
    }

    /// Get the number of frames processed so far
    pub fn frames_processed(&self) -> u64 {
        self.frames_processed
    }

    /// Get the average processing time per frame in microseconds
    pub fn avg_processing_time_us(&self) -> f64 {
        if self.frames_processed == 0 {
            0.0
        } else {
            self.total_processing_us as f64 / self.frames_processed as f64
        }
    }

    /// Downmix stereo to mono by averaging channels
    pub fn stereo_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
        if channels <= 1 {
            return samples.to_vec();
        }
        let ch = channels as usize;
        samples
            .chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    }

    /// Upmix mono to stereo by duplicating the channel
    pub fn mono_to_stereo(samples: &[f32], channels: u16) -> Vec<f32> {
        if channels <= 1 {
            return samples.to_vec();
        }
        let ch = channels as usize;
        samples
            .iter()
            .flat_map(|&s| std::iter::repeat(s).take(ch))
            .collect()
    }

    /// Linear interpolation resampling (good enough for speech)
    pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        if from_rate == to_rate || samples.is_empty() {
            return samples.to_vec();
        }
        let ratio = from_rate as f64 / to_rate as f64;
        let output_len = ((samples.len() as f64) / ratio).ceil() as usize;
        let mut output = Vec::with_capacity(output_len);

        for i in 0..output_len {
            let src_pos = i as f64 * ratio;
            let src_idx = src_pos.floor() as usize;
            let frac = (src_pos - src_idx as f64) as f32;

            if src_idx + 1 < samples.len() {
                output.push(samples[src_idx] * (1.0 - frac) + samples[src_idx + 1] * frac);
            } else if src_idx < samples.len() {
                output.push(samples[src_idx]);
            }
        }

        output
    }

    /// Process a chunk of audio samples through the noise suppressor.
    /// Returns denoised samples and VAD probability.
    /// If RNNoise is not available or disabled, returns input unchanged with VAD=0.0.
    pub fn process(&mut self, samples: &[f32]) -> DenoiseResult {
        if !self.is_active() {
            return DenoiseResult {
                samples: samples.to_vec(),
                vad_probability: 0.0,
            };
        }

        // Placeholder: pass through unchanged with estimated VAD based on RMS
        // Real implementation will use nnnoiseless crate
        let rms = if samples.is_empty() {
            0.0
        } else {
            let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
            (sum / samples.len() as f64).sqrt() as f32
        };

        // Simple VAD estimate: if RMS > threshold, likely speech
        let vad = if rms > 0.01 { 0.9 } else { 0.1 };

        self.frames_processed += 1;

        DenoiseResult {
            samples: samples.to_vec(),
            vad_probability: vad,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_to_mono_averages_channels() {
        let stereo = vec![0.5f32, 0.3, 0.8, 0.2, 1.0, 0.0];
        let mono = NoiseSuppressor::stereo_to_mono(&stereo, 2);
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.4).abs() < 0.001);
        assert!((mono[1] - 0.5).abs() < 0.001);
        assert!((mono[2] - 0.5).abs() < 0.001);
    }

    #[test]
    fn stereo_to_mono_passthrough_for_mono() {
        let mono = vec![0.5f32, 0.3, 0.8];
        let result = NoiseSuppressor::stereo_to_mono(&mono, 1);
        assert_eq!(result, mono);
    }

    #[test]
    fn mono_to_stereo_duplicates_channels() {
        let mono = vec![0.5f32, 0.3, 0.8];
        let stereo = NoiseSuppressor::mono_to_stereo(&mono, 2);
        assert_eq!(stereo.len(), 6);
        assert_eq!(stereo, vec![0.5, 0.5, 0.3, 0.3, 0.8, 0.8]);
    }

    #[test]
    fn mono_to_stereo_passthrough_for_mono() {
        let mono = vec![0.5f32, 0.3];
        let result = NoiseSuppressor::mono_to_stereo(&mono, 1);
        assert_eq!(result, mono);
    }

    #[test]
    fn resample_same_rate_passthrough() {
        let samples = vec![0.1f32, 0.2, 0.3, 0.4];
        let result = NoiseSuppressor::resample(&samples, 48000, 48000);
        assert_eq!(result, samples);
    }

    #[test]
    fn resample_downsample_reduces_length() {
        let samples: Vec<f32> = (0..960).map(|i| (i as f32) / 960.0).collect();
        let result = NoiseSuppressor::resample(&samples, 48000, 24000);
        // Should be approximately half the length
        assert!(result.len() >= 479 && result.len() <= 481);
    }

    #[test]
    fn resample_upsample_increases_length() {
        let samples: Vec<f32> = (0..480).map(|i| (i as f32) / 480.0).collect();
        let result = NoiseSuppressor::resample(&samples, 24000, 48000);
        // Should be approximately double the length
        assert!(result.len() >= 959 && result.len() <= 961);
    }

    #[test]
    fn resample_empty_returns_empty() {
        let result = NoiseSuppressor::resample(&[], 48000, 24000);
        assert!(result.is_empty());
    }

    #[test]
    fn process_returns_vad_for_speech() {
        let mut suppressor = NoiseSuppressor::new(NoiseSuppressorConfig::default());
        // Loud signal = speech
        let samples: Vec<f32> = (0..480).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let result = suppressor.process(&samples);
        assert!(result.vad_probability > 0.5);
        assert_eq!(result.samples.len(), samples.len());
    }

    #[test]
    fn process_returns_low_vad_for_silence() {
        let mut suppressor = NoiseSuppressor::new(NoiseSuppressorConfig::default());
        let samples = vec![0.0f32; 480];
        let result = suppressor.process(&samples);
        assert!(result.vad_probability < 0.5);
    }

    #[test]
    fn process_disabled_returns_passthrough() {
        let mut suppressor = NoiseSuppressor::new(NoiseSuppressorConfig {
            enabled: false,
            ..Default::default()
        });
        let samples = vec![0.5f32; 480];
        let result = suppressor.process(&samples);
        assert_eq!(result.samples, samples);
        assert_eq!(result.vad_probability, 0.0);
    }

    #[test]
    fn frames_processed_increments() {
        let mut suppressor = NoiseSuppressor::new(NoiseSuppressorConfig::default());
        assert_eq!(suppressor.frames_processed(), 0);
        suppressor.process(&vec![0.0f32; 480]);
        assert_eq!(suppressor.frames_processed(), 1);
        suppressor.process(&vec![0.0f32; 480]);
        assert_eq!(suppressor.frames_processed(), 2);
    }

    #[test]
    fn is_active_when_enabled() {
        let suppressor = NoiseSuppressor::new(NoiseSuppressorConfig::default());
        assert!(suppressor.is_active());
    }

    #[test]
    fn is_not_active_when_disabled() {
        let suppressor = NoiseSuppressor::new(NoiseSuppressorConfig {
            enabled: false,
            ..Default::default()
        });
        assert!(!suppressor.is_active());
    }
}
