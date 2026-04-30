## Parent PRD

#20

## What to build

Add a pure function `calculate_mic_gain_db(loopback_lufs: f64, mic_lufs: f64) -> f64` to `core/src/ffmpeg_encoder.rs` that calculates the gain adjustment (in dB) to apply to the microphone stream so it matches the system audio loudness level.

This is a pure computation with no FFmpeg dependency — ideal for TDD with fast unit tests.

### Function signature

```rust
/// Calculate the gain adjustment (in dB) to apply to the microphone stream
/// so it matches the system audio loudness level.
pub fn calculate_mic_gain_db(loopback_lufs: f64, mic_lufs: f64) -> f64
```

### Logic

- If `mic_lufs < -50.0` → return `0.0` (silent mic guard — avoid amplifying noise floor)
- If `loopback_lufs < -50.0` → return `0.0` (silent system audio — nothing to match)
- `gain = loopback_lufs - mic_lufs`
- Clamp to range `[-6.0, 20.0]`
- If `gain.abs() < 1.0` → return `0.0` (already close enough)

Reference: PRD Step 2 from #20.

### Unit tests

All tests are pure functions — no FFmpeg needed, run instantly:

- Mic quieter than system (e.g., loopback=-12, mic=-30) → positive gain (18.0 dB)
- Mic louder than system (e.g., loopback=-20, mic=-8) → negative gain (capped at -6.0 dB)
- Similar levels within 1 dB (e.g., loopback=-14, mic=-14.5) → 0.0
- Silent mic (< -50 LUFS) → 0.0
- Silent system (< -50 LUFS) → 0.0
- Extreme difference (e.g., loopback=-10, mic=-40) → capped at +20.0 dB

Reference: PRD Step 4 (unit tests) from #20.

## Acceptance criteria

- [ ] `calculate_mic_gain_db()` is a `pub` function in `core/src/ffmpeg_encoder.rs`
- [ ] Returns 0.0 for silent mic (< -50 LUFS)
- [ ] Returns 0.0 for silent system (< -50 LUFS)
- [ ] Clamps gain to [-6.0, +20.0] range
- [ ] Returns 0.0 when gain magnitude < 1.0 dB (dead zone)
- [ ] Includes doc comment explaining the function's purpose
- [ ] Unit tests cover all 6 edge cases listed above
- [ ] All tests pass with `cargo test`

## Blocked by

None — can start immediately

## User stories addressed

- Gain calculation logic with safety guards (PRD Step 2)
- Unit test coverage for gain calculation (PRD Step 4)
