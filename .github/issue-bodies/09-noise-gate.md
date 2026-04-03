## Parent PRD

#6

## What to build

Add a noise gate to the microphone capture pipeline with a 6-state machine: CLOSED → OPENING → OPEN → HOLDING → CLOSING → CLOSED.

**Key implementation details:**
- New module: `audio_processing/noise_gate.rs`
- When RNNoise is active, the gate is driven by VAD probability (0.0-1.0) instead of raw RMS thresholding — RNNoise's VAD is specifically trained for speech detection
- When `--no-denoise` is used, falls back to dB threshold using existing `audio_utils.rs` RMS functions
- Smooth attack/release ramps to prevent clicks
- Hold time prevents rapid gate cycling (chattering)
- Parameters: open threshold, close threshold, attack time, hold time, release time

**CLI flags:**
- `--no-noise-gate` — disable noise gate (ON by default)
- `--gate-threshold <value>` — VAD probability (0.0-1.0) when RNNoise active, dB threshold when `--no-denoise`

See PRD section: *Noise Gate (Phase 3)*
See Eng Review: Issue 4 (pipeline order — RNNoise before gate)
See Plan: Phase 5

## Acceptance criteria

- [ ] New `audio_processing/noise_gate.rs` module
- [ ] 6-state machine: CLOSED → OPENING → OPEN → HOLDING → CLOSING → CLOSED
- [ ] Gate driven by VAD probability when RNNoise active
- [ ] Falls back to dB threshold when `--no-denoise` is used
- [ ] Smooth attack/release ramps (no clicks)
- [ ] Hold time prevents rapid gate cycling (chattering)
- [ ] `--no-noise-gate` flag disables noise gate
- [ ] `--gate-threshold` configures gate sensitivity
- [ ] Gate ON by default
- [ ] Unit tests: all 6 state transitions, hold time, attack/release ramps, VAD-driven mode
- [ ] CLI tests: `--no-noise-gate`, `--gate-threshold` flag parsing

## Blocked by

- Blocked by #14 (RNNoise Real-Time Noise Suppression)

## User stories addressed

- User story 7: noise gate on microphone
- User story 8: `--no-noise-gate` disable flag
- User story 16: configurable gate threshold
- User story 22: quality features work together seamlessly
