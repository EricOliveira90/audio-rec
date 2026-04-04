//! Real-time soft audio limiter.
//!
//! Limits samples above a threshold with a fast attack envelope.
//! Samples below the threshold pass through unchanged.

/// Configuration for the real-time limiter
#[derive(Debug, Clone)]
pub struct LimiterConfig {
    /// Threshold above which samples are limited (0.0 to 1.0)
    pub threshold: f32,
    /// Attack time in samples (how quickly the limiter engages)
    pub attack_samples: u32,
    /// Whether the limiter is enabled
    pub enabled: bool,
}

impl Default for LimiterConfig {
    fn default() -> Self {
        Self {
            threshold: 0.95,
            attack_samples: 48, // ~1ms at 48kHz
            enabled: true,
        }
    }
}

/// Real-time soft limiter
pub struct Limiter {
    config: LimiterConfig,
    /// Current gain reduction (1.0 = no reduction, < 1.0 = limiting)
    gain: f32,
}

impl Limiter {
    pub fn new(config: LimiterConfig) -> Self {
        Self {
            config,
            gain: 1.0,
        }
    }

    /// Check if the limiter is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the current gain value
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Limit a single sample. Returns the limited sample value.
    #[inline]
    pub fn limit_sample(&mut self, sample: f32) -> f32 {
        if !self.config.enabled {
            return sample;
        }

        let abs_sample = sample.abs();

        if abs_sample > self.config.threshold {
            // Calculate required gain to bring sample to threshold
            let required_gain = self.config.threshold / abs_sample;

            // Fast attack: immediately reduce gain if needed
            if required_gain < self.gain {
                self.gain = required_gain;
            }

            sample * self.gain
        } else {
            // Slow release: gradually return gain to 1.0
            if self.gain < 1.0 {
                let release_rate = if self.config.attack_samples > 0 {
                    1.0 / (self.config.attack_samples as f32 * 10.0) // Release 10x slower than attack
                } else {
                    0.1
                };
                self.gain = (self.gain + release_rate).min(1.0);
            }

            sample * self.gain
        }
    }

    /// Process a buffer of samples through the limiter (in-place)
    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.config.enabled {
            return;
        }
        for sample in samples.iter_mut() {
            *sample = self.limit_sample(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_passes_through_quiet_samples() {
        let mut limiter = Limiter::new(LimiterConfig::default());
        let sample = 0.5f32;
        let result = limiter.limit_sample(sample);
        assert!((result - 0.5).abs() < 0.01);
    }

    #[test]
    fn limiter_reduces_loud_samples() {
        let mut limiter = Limiter::new(LimiterConfig::default());
        let sample = 1.5f32; // Way above threshold of 0.95
        let result = limiter.limit_sample(sample);
        assert!(result.abs() <= 0.95 + 0.01);
    }

    #[test]
    fn limiter_threshold_boundary() {
        let mut limiter = Limiter::new(LimiterConfig {
            threshold: 0.95,
            ..Default::default()
        });
        // Exactly at threshold should pass through
        let result = limiter.limit_sample(0.95);
        assert!((result - 0.95).abs() < 0.01);
    }

    #[test]
    fn limiter_handles_negative_samples() {
        let mut limiter = Limiter::new(LimiterConfig::default());
        let result = limiter.limit_sample(-1.5);
        assert!(result.abs() <= 0.95 + 0.01);
        assert!(result < 0.0); // Should preserve sign
    }

    #[test]
    fn limiter_handles_silence() {
        let mut limiter = Limiter::new(LimiterConfig::default());
        let result = limiter.limit_sample(0.0);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn limiter_disabled_passes_through() {
        let mut limiter = Limiter::new(LimiterConfig {
            enabled: false,
            ..Default::default()
        });
        let result = limiter.limit_sample(2.0);
        assert_eq!(result, 2.0);
    }

    #[test]
    fn limiter_process_buffer() {
        let mut limiter = Limiter::new(LimiterConfig {
            threshold: 0.5,
            ..Default::default()
        });
        let mut samples = vec![0.3f32, 0.8, -0.9, 0.2, 1.0];
        limiter.process(&mut samples);

        // First sample below threshold
        assert!((samples[0] - 0.3).abs() < 0.05);
        // Loud samples should be limited
        assert!(samples[1].abs() <= 0.55);
        assert!(samples[2].abs() <= 0.55);
        // Last sample should also be limited
        assert!(samples[4].abs() <= 0.55);
    }

    #[test]
    fn limiter_gain_reduces_on_loud_input() {
        let mut limiter = Limiter::new(LimiterConfig::default());
        assert_eq!(limiter.gain(), 1.0);
        limiter.limit_sample(2.0); // Very loud
        assert!(limiter.gain() < 1.0);
    }

    #[test]
    fn limiter_gain_recovers_after_quiet_input() {
        let mut limiter = Limiter::new(LimiterConfig {
            threshold: 0.5,
            attack_samples: 10,
            ..Default::default()
        });
        // Drive gain down
        limiter.limit_sample(2.0);
        let reduced_gain = limiter.gain();
        assert!(reduced_gain < 1.0);

        // Feed quiet samples to let gain recover
        for _ in 0..1000 {
            limiter.limit_sample(0.1);
        }
        assert!(limiter.gain() > reduced_gain);
    }

    #[test]
    fn limiter_is_enabled_by_default() {
        let limiter = Limiter::new(LimiterConfig::default());
        assert!(limiter.is_enabled());
    }
}
