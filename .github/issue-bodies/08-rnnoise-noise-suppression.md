## Parent PRD

#6

## What to build

Add real-time neural network noise suppression to the microphone capture pipeline using the `nnnoiseless` crate (pure Rust RNNoise port, no C dependencies).

**Key implementation details:**
- New module: `audio_processing/noise_suppress.rs`
- RNNoise requires 48kHz mono float32 in 480-sample frames
- The module handles stereo downmix/upmix internally
- If mic sample rate ≠ 48kHz, the module handles resampling internally (linear interpolation — good enough for speech)
- VAD (voice activity detection) probability output per frame — used by the noise gate (follow-up issue)
- Wrap `denoise_frame()` in `std::panic::catch_unwind` — if RNNoise panics on malformed input, disable it for the rest of the recording and log a warning
- Add `nnnoiseless = "0.5"` dependency to `core/Cargo.toml`

**Performance:**
- Benchmark test: process 1000 frames, assert average < 2ms
- Runtime CPU budget check: if first 100 frames average > 5ms, log performance warning

**CLI flag:** `--no-denoise` — disable noise suppression (ON by default)

See PRD section: *RNNoise Noise Suppression (Phase 3)*
See Eng Review: Issue 11 (CPU budget), Issue 12 (resampling), Failure Modes (RNNoise panic)
See Plan: Phase 5

## Acceptance criteria

- [ ] `nnnoiseless` dependency added to `core/Cargo.toml`
- [ ] New `audio_processing/noise_suppress.rs` module
- [ ] RNNoise processes mic audio in real-time — keyboard typing and fan noise removed from recordings
- [ ] Handles stereo downmix/upmix internally
- [ ] Handles non-48kHz resampling internally (linear interpolation)
- [ ] VAD probability output exposed per frame (for noise gate)
- [ ] RNNoise panic caught via `catch_unwind` — disables denoise for rest of recording with warning
- [ ] `--no-denoise` flag disables noise suppression
- [ ] Denoise ON by default
- [ ] Benchmark: 1000 RNNoise frames < 2ms average
- [ ] Runtime: if first 100 frames > 5ms average, log performance warning
- [ ] Unit tests: stereo downmix, resampling, noise reduction (output RMS lower for noise-only), VAD output, frame buffering
- [ ] CLI tests: `--no-denoise` flag parsing

## Blocked by

- Blocked by #7 (Capture Engine Refactor)

## User stories addressed

- User story 6: real-time noise suppression on microphone
- User story 8: `--no-denoise` disable flag
- User story 22: quality features work together seamlessly
- User story 24: works regardless of mic sample rate and channel count
