## Parent PRD

#6

## What to build

Wire the full real-time audio processing pipeline and inject it into the microphone capture thread. Add a real-time soft limiter as the final stage.

**New modules:**
- `audio_processing/limiter.rs` — real-time soft limiter (limit at 0.95, 1ms attack)
- `audio_processing/pipeline.rs` — chains all stages together

**Pipeline order:** Resample to 48kHz → RNNoise (denoise + VAD) → Noise Gate (VAD-driven) → Limiter → Resample to native rate → WAV writer

**Key implementation details:**
- Each stage is independently toggleable via CLI flags (`--no-denoise`, `--no-noise-gate`, `--no-limiter`)
- Pipeline configuration determined at startup from CLI flags and passed to the capture thread
- Pipeline injected into the microphone capture thread via the injection point created in the Capture Engine Refactor

See PRD section: *Audio Processing Pipeline (Phase 3)*
See Eng Review: Issue 4 (pipeline order)
See Plan: Phase 5

## Acceptance criteria

- [ ] New `audio_processing/limiter.rs` — real-time soft limiter
- [ ] New `audio_processing/pipeline.rs` — chains stages together
- [ ] Full pipeline: Resample → RNNoise → Gate → Limiter → Resample → WAV writer
- [ ] Each stage independently toggleable via CLI flags
- [ ] Pipeline injected into mic capture thread
- [ ] Limiter limits samples above 0.95 threshold with 1ms attack
- [ ] Samples below threshold pass through unchanged
- [ ] Unit tests: limiter threshold behavior, passthrough, attack envelope
- [ ] Unit tests: full pipeline chain, each stage independently disabled, config from flags

## Blocked by

- Blocked by #14 (RNNoise Real-Time Noise Suppression)
- Blocked by #15 (Noise Gate VAD-Driven)

## User stories addressed

- User story 22: all quality features work together seamlessly
