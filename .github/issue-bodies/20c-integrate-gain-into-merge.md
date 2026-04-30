## Parent PRD

#20

## What to build

Wire `measure_lufs()` and `calculate_mic_gain_db()` into the Scenario A branch of `merge_audio_streams_smart()` in `core/src/recorder.rs`, so that when both loopback and microphone have audio, the mic stream is gain-compensated to match the system audio loudness before merging.

### Changes to `merge_audio_streams_smart()` — Scenario A only

In the `if loopback_has_audio && mic_has_audio` branch:

1. **Measure LUFS in parallel** — Run `measure_lufs()` on both WAV files using `tokio::join!` (same pattern already used for duration detection in this function):

```rust
let (loopback_lufs, mic_lufs) = tokio::join!(
    crate::ffmpeg_encoder::measure_lufs(loopback_wav),
    crate::ffmpeg_encoder::measure_lufs(mic_wav)
);
```

2. **Calculate gain** — Call `calculate_mic_gain_db()` with the measured values.

3. **Apply gain in FFmpeg filter** — If gain != 0.0, insert `volume={gain}dB` into the mic input's filter chain:

```
// Before (current):
[1:a]aformat=channel_layouts=mono,apad=whole_dur=...

// After (with gain compensation):
[1:a]volume=12.0dB,aformat=channel_layouts=mono,apad=whole_dur=...
```

4. **Log measurements** — Add `tracing::info!` calls to log:
   - Loopback LUFS measurement
   - Microphone LUFS measurement
   - Calculated gain (or "skipped" if 0.0)

### What NOT to change

- Scenarios B (loopback only), C (mic only), and D (both silent) remain unchanged
- No new CLI flags — this is automatic, zero-config behavior
- No changes to function signature — all new behavior is internal

Reference: PRD Step 3 from #20.

## Acceptance criteria

- [ ] Scenario A merge measures LUFS on both files in parallel using `tokio::join!`
- [ ] Scenario A merge calls `calculate_mic_gain_db()` to compute gain offset
- [ ] When gain != 0.0, `volume={gain}dB` is inserted into the mic filter chain before `aformat`
- [ ] When gain == 0.0, the filter chain is unchanged (no unnecessary `volume=0dB`)
- [ ] LUFS measurements and applied gain are logged via `tracing::info!`
- [ ] If LUFS measurement fails for either file, merge proceeds without gain compensation (graceful fallback)
- [ ] Scenarios B, C, D are completely unchanged
- [ ] LUFS measurements run in parallel for minimal latency (~100ms each, not sequential)

## Blocked by

- Blocked by #22 (`calculate_mic_gain_db`)
- Blocked by #23 (`measure_lufs`)

## User stories addressed

- Pre-merge volume balancing so both streams are at comparable loudness (PRD Step 3)
- Automatic, zero-config behavior — no new CLI flags needed
