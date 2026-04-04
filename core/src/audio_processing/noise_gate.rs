//! Noise gate with 6-state machine for microphone audio.
//!
//! When RNNoise is active, the gate is driven by VAD probability.
//! When --no-denoise is used, falls back to dB threshold.
//! Smooth attack/release ramps prevent clicks.

/// Gate states in the 6-state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    Closed,
    Opening,
    Open,
    Holding,
    Closing,
    // Note: transitions back to Closed from Closing
}

/// Configuration for the noise gate
#[derive(Debug, Clone)]
pub struct NoiseGateConfig {
    /// Threshold to open the gate (VAD probability 0.0-1.0 when RNNoise active, dB otherwise)
    pub open_threshold: f64,
    /// Threshold to close the gate (should be lower than open_threshold for hysteresis)
    pub close_threshold: f64,
    /// Attack time in samples (ramp from 0.0 to 1.0)
    pub attack_samples: u32,
    /// Hold time in samples (keep gate open after signal drops below threshold)
    pub hold_samples: u32,
    /// Release time in samples (ramp from 1.0 to 0.0)
    pub release_samples: u32,
    /// Whether the gate is enabled
    pub enabled: bool,
    /// Whether to use VAD-driven mode (true) or dB threshold mode (false)
    pub vad_driven: bool,
}

impl Default for NoiseGateConfig {
    fn default() -> Self {
        Self {
            open_threshold: 0.5,
            close_threshold: 0.3,
            attack_samples: 480,    // ~10ms at 48kHz
            hold_samples: 24000,    // 500ms at 48kHz
            release_samples: 2400,  // ~50ms at 48kHz
            enabled: true,
            vad_driven: true,
        }
    }
}

/// Noise gate processor with 6-state machine
pub struct NoiseGate {
    config: NoiseGateConfig,
    state: GateState,
    /// Current gain (0.0 = fully closed, 1.0 = fully open)
    gain: f32,
    /// Counter for current state (attack/hold/release progress)
    state_counter: u32,
}

impl NoiseGate {
    pub fn new(config: NoiseGateConfig) -> Self {
        Self {
            config,
            state: GateState::Closed,
            gain: 0.0,
            state_counter: 0,
        }
    }

    /// Get the current gate state
    pub fn state(&self) -> GateState {
        self.state
    }

    /// Get the current gain value (0.0 to 1.0)
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Check if the gate is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Update the gate state based on input level.
    /// `level` is VAD probability (0.0-1.0) in VAD mode, or dB in threshold mode.
    pub fn update(&mut self, level: f64) {
        if !self.config.enabled {
            self.gain = 1.0;
            self.state = GateState::Open;
            return;
        }

        let above_open = level >= self.config.open_threshold;
        let above_close = level >= self.config.close_threshold;

        match self.state {
            GateState::Closed => {
                if above_open {
                    self.state = GateState::Opening;
                    self.state_counter = 0;
                }
            }
            GateState::Opening => {
                self.state_counter += 1;
                if self.config.attack_samples > 0 {
                    self.gain = (self.state_counter as f32 / self.config.attack_samples as f32).min(1.0);
                } else {
                    self.gain = 1.0;
                }
                if self.gain >= 1.0 {
                    self.state = GateState::Open;
                    self.gain = 1.0;
                }
            }
            GateState::Open => {
                if !above_close {
                    self.state = GateState::Holding;
                    self.state_counter = 0;
                }
                // Re-trigger: if above_open while open, stay open (no-op)
            }
            GateState::Holding => {
                self.state_counter += 1;
                if above_open {
                    // Re-trigger: go back to Open
                    self.state = GateState::Open;
                    self.state_counter = 0;
                } else if self.state_counter >= self.config.hold_samples {
                    self.state = GateState::Closing;
                    self.state_counter = 0;
                }
            }
            GateState::Closing => {
                if above_open {
                    // Re-trigger during release: go back to Opening
                    self.state = GateState::Opening;
                    // Keep current gain, ramp up from here
                    let current_progress = self.gain * self.config.attack_samples as f32;
                    self.state_counter = current_progress as u32;
                    return;
                }
                self.state_counter += 1;
                if self.config.release_samples > 0 {
                    self.gain = (1.0 - self.state_counter as f32 / self.config.release_samples as f32).max(0.0);
                } else {
                    self.gain = 0.0;
                }
                if self.gain <= 0.0 {
                    self.state = GateState::Closed;
                    self.gain = 0.0;
                }
            }
        }
    }

    /// Apply the current gate gain to audio samples (in-place)
    pub fn apply(&self, samples: &mut [f32]) {
        if !self.config.enabled {
            return; // passthrough
        }
        let g = self.gain;
        for sample in samples.iter_mut() {
            *sample *= g;
        }
    }

    /// Process: update state and apply gain in one step
    pub fn process(&mut self, samples: &mut [f32], level: f64) {
        self.update(level);
        self.apply(samples);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_gate() -> NoiseGate {
        NoiseGate::new(NoiseGateConfig::default())
    }

    #[test]
    fn gate_starts_closed() {
        let gate = default_gate();
        assert_eq!(gate.state(), GateState::Closed);
        assert_eq!(gate.gain(), 0.0);
    }

    #[test]
    fn gate_opens_on_high_level() {
        let mut gate = default_gate();
        gate.update(0.8); // Above open_threshold (0.5)
        assert_eq!(gate.state(), GateState::Opening);
    }

    #[test]
    fn gate_stays_closed_on_low_level() {
        let mut gate = default_gate();
        gate.update(0.1); // Below open_threshold
        assert_eq!(gate.state(), GateState::Closed);
        assert_eq!(gate.gain(), 0.0);
    }

    #[test]
    fn gate_reaches_open_after_attack() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            attack_samples: 10,
            ..Default::default()
        });
        // Trigger opening
        for _ in 0..11 {
            gate.update(0.8);
        }
        assert_eq!(gate.state(), GateState::Open);
        assert_eq!(gate.gain(), 1.0);
    }

    #[test]
    fn gate_enters_holding_when_level_drops() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            attack_samples: 1,
            ..Default::default()
        });
        // Open the gate
        gate.update(0.8);
        gate.update(0.8); // Now Open
        assert_eq!(gate.state(), GateState::Open);

        // Drop below close threshold
        gate.update(0.1);
        assert_eq!(gate.state(), GateState::Holding);
    }

    #[test]
    fn gate_hold_prevents_chattering() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            attack_samples: 1,
            hold_samples: 100,
            ..Default::default()
        });
        // Open the gate
        gate.update(0.8);
        gate.update(0.8);
        // Drop below threshold
        gate.update(0.1);
        assert_eq!(gate.state(), GateState::Holding);

        // During hold, gate stays at gain 1.0
        assert_eq!(gate.gain(), 1.0);

        // After 50 updates (less than hold_samples), still holding
        for _ in 0..50 {
            gate.update(0.1);
        }
        assert_eq!(gate.state(), GateState::Holding);
    }

    #[test]
    fn gate_retriggers_from_holding() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            attack_samples: 1,
            hold_samples: 100,
            ..Default::default()
        });
        // Open -> Holding
        gate.update(0.8);
        gate.update(0.8);
        gate.update(0.1);
        assert_eq!(gate.state(), GateState::Holding);

        // Re-trigger
        gate.update(0.8);
        assert_eq!(gate.state(), GateState::Open);
    }

    #[test]
    fn gate_closes_after_hold_and_release() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            attack_samples: 1,
            hold_samples: 5,
            release_samples: 5,
            ..Default::default()
        });
        // Open the gate
        gate.update(0.8);
        gate.update(0.8);
        // Drop below threshold -> Holding
        gate.update(0.1);
        // Exhaust hold time
        for _ in 0..5 {
            gate.update(0.1);
        }
        assert_eq!(gate.state(), GateState::Closing);

        // Exhaust release time
        for _ in 0..6 {
            gate.update(0.1);
        }
        assert_eq!(gate.state(), GateState::Closed);
        assert_eq!(gate.gain(), 0.0);
    }

    #[test]
    fn gate_attack_ramp_is_smooth() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            attack_samples: 4,
            ..Default::default()
        });
        // update 1: Closed -> Opening (counter=0, gain unchanged at 0.0)
        gate.update(0.8);
        assert_eq!(gate.state(), GateState::Opening);
        assert_eq!(gate.gain(), 0.0);

        // update 2: Opening, counter=1, gain=1/4=0.25
        gate.update(0.8);
        assert!((gate.gain() - 0.25).abs() < 0.01);

        // update 3: counter=2, gain=2/4=0.5
        gate.update(0.8);
        assert!((gate.gain() - 0.5).abs() < 0.01);

        // update 4: counter=3, gain=3/4=0.75
        gate.update(0.8);
        assert!((gate.gain() - 0.75).abs() < 0.01);

        // update 5: counter=4, gain=4/4=1.0 -> transitions to Open
        gate.update(0.8);
        assert_eq!(gate.state(), GateState::Open);
        assert_eq!(gate.gain(), 1.0);
    }

    #[test]
    fn gate_release_ramp_is_smooth() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            attack_samples: 1,
            hold_samples: 1,
            release_samples: 4,
            ..Default::default()
        });
        // Open the gate
        gate.update(0.8);
        gate.update(0.8);
        // Holding
        gate.update(0.1);
        // Exhaust hold
        gate.update(0.1);
        // Now Closing
        assert_eq!(gate.state(), GateState::Closing);
        gate.update(0.1); // counter=1
        assert!((gate.gain() - 0.75).abs() < 0.01);
        gate.update(0.1); // counter=2
        assert!((gate.gain() - 0.5).abs() < 0.01);
    }

    #[test]
    fn gate_apply_multiplies_samples() {
        let mut gate = default_gate();
        gate.gain = 0.5;
        let mut samples = vec![1.0f32, -1.0, 0.5, -0.5];
        gate.apply(&mut samples);
        assert_eq!(samples, vec![0.5, -0.5, 0.25, -0.25]);
    }

    #[test]
    fn gate_disabled_passes_through() {
        let mut gate = NoiseGate::new(NoiseGateConfig {
            enabled: false,
            ..Default::default()
        });
        gate.update(0.0);
        assert_eq!(gate.gain(), 1.0);
        assert_eq!(gate.state(), GateState::Open);
    }
}
