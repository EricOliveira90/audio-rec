## Pre-Merge Microphone Gain Compensation

### Problem

When recording meetings, the system audio (loopback) and microphone audio often have very different volume levels. A typical scenario:

- System audio (remote participants): **-12 LUFS** (loud)
- Microphone (local user): **-30 LUFS** (quiet)

Currently, these two streams are merged **as-is** with no volume balancing. The merge in `recorder.rs::merge_audio_streams_smart()` combines them at their raw captured volumes. This means:

1. The microphone is hard to hear in the merged recording
2. Transcription services may miss what the local user is saying
3. The planned post-merge normalization (Issue #12) won't fix this because it normalizes the **combined** signal — the loud system audio dominates the target, keeping the mic relatively quiet

### Solution

Add pre-merge LUFS measurement and mic gain compensation so both streams are at comparable loudness levels before merging.

### Architecture

```
┌──────────────┐     ┌──────────────┐
│  Loopback    │     │  Microphone   │
│  (raw WAV)   │     │  (raw WAV)    │
└──────┬───────┘     └──────┬────────┘
       │                    │
       ▼                    ▼
  ┌─────────────────────────────────┐
  │  measure_lufs() × 2            │  ← NEW (parallel, ~100ms each)
  │  FFmpeg loudnorm measure mode   │
  └──────────────┬──────────────────┘
                 │
                 │  gain_db = loopback_lufs - mic_lufs
                 │  (capped: -6dB to +20dB)
                 │
                 ▼
  ┌─────────────────────────────────┐
  │  merge_audio_streams_smart()    │
  │  FFmpeg filter (modified):      │
  │  [1:a]volume={gain_db}dB,...    │  ← MODIFIED
  │  amerge + pan                   │
  └──────────────┬──────────────────┘
                 │
                 ▼
            Merged output
```

### Implementation Steps

#### Step 1: Add `measure_lufs()` to `core/src/ffmpeg_encoder.rs`

```rust
/// Measure the integrated loudness (LUFS) of an audio file using FFmpeg's loudnorm filter.
/// Returns the integrated loudness in LUFS, or an error if measurement fails.
pub async fn measure_lufs(audio_path: &PathBuf) -> Result<f64>
```

Uses FFmpeg: `ffmpeg -i <file> -af loudnorm=print_format=json -f null -` and parses the JSON output for `input_i` (integrated loudness).

#### Step 2: Add `calculate_mic_gain_db()` to `core/src/ffmpeg_encoder.rs`

```rust
/// Calculate the gain adjustment (in dB) to apply to the microphone stream
/// so it matches the system audio loudness level.
pub fn calculate_mic_gain_db(loopback_lufs: f64, mic_lufs: f64) -> f64
```

Logic:
- If `mic_lufs < -50.0` → return 0.0 (silent mic guard — avoid amplifying noise floor)
- If `loopback_lufs < -50.0` → return 0.0 (silent system audio — nothing to match)
- `gain = loopback_lufs - mic_lufs`
- Clamp to range `[-6.0, 20.0]`
- If `gain.abs() < 1.0` → return 0.0 (already close enough)

#### Step 3: Modify `merge_audio_streams_smart()` in `core/src/recorder.rs`

In Scenario A (both streams have audio):

1. Run `measure_lufs()` on both files in parallel using `tokio::join!` (same pattern as existing duration detection)
2. Call `calculate_mic_gain_db()` to get the gain offset
3. If gain != 0.0, insert `volume={gain}dB` into the mic input filter chain:

```
// Before (current):
[1:a]aformat=channel_layouts=mono,apad=whole_dur=...

// After (with gain compensation):
[1:a]volume=12.0dB,aformat=channel_layouts=mono,apad=whole_dur=...
```

4. Log the LUFS measurements and applied gain via `tracing::info!`

#### Step 4: Add tests

**Unit tests** (pure function, no FFmpeg needed) for `calculate_mic_gain_db`:
- Mic quieter than system → positive gain
- Mic louder than system → negative gain (capped at -6 dB)
- Similar levels (within 1 dB) → 0.0
- Silent mic (< -50 LUFS) → 0.0
- Silent system (< -50 LUFS) → 0.0
- Extreme difference → capped at +20 dB

**Integration test** (`#[ignore]` — requires FFmpeg) for `measure_lufs`:
- Valid WAV with audio → returns reasonable LUFS value

### Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Gain strategy | Match mic to system level | Simplest correct answer; both streams at equal perceived loudness |
| Safety caps | -6 dB to +20 dB | Allows modest attenuation and generous boost for quiet laptop mics |
| Silent mic guard | Skip if mic < -50 LUFS | Prevents amplifying noise floor on effectively silent mic |
| When to apply | Scenario A only (both have audio) | Other scenarios don't have a cross-stream balance problem |
| Dead zone | Skip if gain < 1 dB | Avoid unnecessary processing when streams are already balanced |

### Files Modified

| File | Change |
|------|--------|
| `core/src/ffmpeg_encoder.rs` | Add `measure_lufs()` and `calculate_mic_gain_db()` + unit tests |
| `core/src/recorder.rs` | Modify Scenario A in `merge_audio_streams_smart()` to measure LUFS and apply gain |

### Acceptance Criteria

- [ ] `measure_lufs()` returns accurate LUFS measurement for WAV files
- [ ] `calculate_mic_gain_db()` correctly computes gain with all safety caps
- [ ] Scenario A merge applies volume gain to mic input when streams differ by > 1 dB
- [ ] Scenario A merge skips gain when mic is silent (< -50 LUFS)
- [ ] Scenarios B, C, D are unchanged (no gain compensation)
- [ ] LUFS measurements and applied gain are logged via `tracing::info!`
- [ ] LUFS measurements run in parallel (tokio::join!) for minimal latency
- [ ] Unit tests for `calculate_mic_gain_db()` cover all edge cases
- [ ] Integration test for `measure_lufs()` (gated behind `#[ignore]`)

### Relationship to Other Issues

- **Complements Issue #12 (Loudness Normalization):** Issue #12 normalizes the merged output to a target LUFS. This issue ensures the *relative balance* between streams is correct before merging. They solve different problems.
- **Independent of Sidechain Ducking (TODOS.md):** Ducking is real-time cross-channel processing. This is a static gain offset at merge time.
- **No new CLI flags needed:** This is automatic, zero-config behavior.
