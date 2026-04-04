# Engineering Review: Wire Dead CLI Flags

> 8 CLI flags are parsed but have zero effect on recording. This plan wires the first 4 features (leaving `--app`/`--app-window` behind).

**Branch:** master
**Date:** 2026-04-04
**Scope:** 5 files, 0 new abstractions, filling existing placeholders

---

## Features In Scope

| # | Feature | Flags | Status |
|---|---------|-------|--------|
| 1 | Post-processing loudness normalization | `--no-normalize`, `--target-loudness` | Placeholder in PostProcessingPipeline |
| 2 | Post-processing soft limiter | `--no-limiter` | Placeholder in PostProcessingPipeline |
| 3 | Real-time mic audio pipeline | `--no-denoise`, `--no-noise-gate`, `--no-limiter` (RT) | AudioPipeline exists, not wired to mic |
| 4 | Pass-through tuning knobs | `--target-loudness`, `--gate-threshold` | Parsed, never forwarded |

## NOT In Scope

- **`--app` / `--app-window`** — Process-specific WASAPI capture. Requires new Windows API integration (~200+ lines). Separate project.
- **`PostProcessingPipeline` refactor** — Switching `record_worker` to use `pipeline.run()` instead of direct calls. Separate PR.
- **RNNoise integration** — `NoiseSuppressor` remains a pass-through placeholder. `--no-denoise` toggles it, but actual denoising is a no-op until `nnnoiseless` crate is added.
- **Single-pass FFmpeg optimization** — Combining normalize+limiter+trim into one FFmpeg command. Future optimization.

---

## Architecture Decisions

### Decision 1: Pass full RecordOptions struct to record_worker

**Problem:** `record_worker()` at `commands/record.rs:170` takes 8 individual parameters, dropping 8 fields from `RecordOptions`. Adding 6 more params would make it 14 parameters.

**Solution:** Refactor `record_worker` to accept the full `RecordOptions` struct. One signature change, one call-site change. Future-proof for any new flags.

### Decision 2: Add FFmpeg functions to ffmpeg_encoder.rs, call directly

**Problem:** New `normalize_loudness()` and `apply_limiter()` functions need a home. The `PostProcessingPipeline` exists but isn't used by `record_worker`.

**Solution:** Add standalone functions to `ffmpeg_encoder.rs` and call them from `record_worker` directly, matching the existing pattern (`trim_silence()` is already called this way). Also fill in `PostProcessingPipeline` placeholders so they call the same functions.

### Decision 3: AudioPipeline lives on the recording thread

**Problem:** The mic recording thread has its own COM initialization. `AudioPipeline` has mutable internal state (not Send/Sync).

**Solution:** Create `AudioPipeline` inside `recording_thread()`. Pass config flags as plain values through the thread spawn closure (they don't change during recording).

### Decision 4: Post-processing order: normalize → limiter → trim

**Problem:** Order matters. If you limiter-then-normalize, the normalizer might push peaks back above the limit.

**Solution:** Enforce normalize → limiter → trim order in `record_worker`. This is the standard mastering chain.

### Decision 5: Single-pass loudnorm with linear=true

**Problem:** FFmpeg loudnorm has single-pass and two-pass modes.

**Solution:** Use `loudnorm=I={target}:TP=-1.5:LRA=11:linear=true`. Single-pass linear mode is predictable and handles true peak limiting + LRA adjustment.

---

## Data Flow — Current vs. Proposed

```
CURRENT (broken):
═══════════════════════════════════════════════════════════════

  cli.rs parses flags ──► RecordOptions (all 16 fields)
                                │
                          execute_with_output()
                                │
                          record_worker(session, config, output,     ◄── ONLY 8 PARAMS
                                        path, mic_muted, trim,          8 fields DROPPED
                                        input_idx, output_idx)
                                │
                    ┌───────────┴───────────┐
                    │                       │
              WasapiLoopback          WasapiMicrophone
              (raw samples              (raw samples
               → WAV file)              → WAV file)     ◄── NO AudioPipeline
                    │                       │
                    └───────────┬───────────┘
                                │
                    merge_audio_streams_smart()
                                │
                    trim_silence() (if enabled)
                                │
                          final file          ◄── NO normalize, NO limiter


PROPOSED (fixed):
═══════════════════════════════════════════════════════════════

  cli.rs parses flags ──► RecordOptions (all 16 fields)
                                │
                          execute_with_output()
                                │
                          record_worker(options: RecordOptions)     ◄── FULL STRUCT
                                │
                    ┌───────────┴───────────┐
                    │                       │
              WasapiLoopback          WasapiMicrophone
              (raw samples              (raw samples)
               → WAV file)                  │
                                      AudioPipeline.process()      ◄── NEW: RT processing
                                      (denoise → gate → limiter)
                                            │
                                      (processed → WAV file)
                    │                       │
                    └───────────┬───────────┘
                                │
                    merge_audio_streams_smart()
                                │
                    normalize_loudness()  (if enabled)             ◄── NEW: FFmpeg loudnorm
                                │
                    apply_limiter()       (if enabled)             ◄── NEW: FFmpeg alimiter
                                │
                    trim_silence()        (if enabled)
                                │
                          final file
```

---

## Implementation Plan

### Lane A: Post-processing FFmpeg functions (independent)

**File: `core/src/ffmpeg_encoder.rs`**

1. Add `pub async fn normalize_loudness(input_path: &PathBuf, target_lufs: f64) -> Result<()>`
   - FFmpeg command: `ffmpeg -i input -af loudnorm=I={target}:TP=-1.5:LRA=11:linear=true -y output`
   - Writes to a temp file, then replaces the original (same pattern as `trim_silence`)
   - On failure: log warning, return Ok (graceful degradation, don't crash the recording)

2. Add `pub async fn apply_limiter(input_path: &PathBuf) -> Result<()>`
   - FFmpeg command: `ffmpeg -i input -af alimiter=level_in=1:level_out=1:limit=0.95:attack=5:release=50 -y output`
   - Same temp-file-then-replace pattern
   - On failure: log warning, return Ok

**File: `core/src/post_processing/pipeline.rs`**

3. Fill in `normalize()` placeholder (line 155) to call `crate::ffmpeg_encoder::normalize_loudness()`
4. Fill in `limit()` placeholder (line 162) to call `crate::ffmpeg_encoder::apply_limiter()`

### Lane B: Real-time mic pipeline integration (independent)

**File: `core/src/wasapi/microphone.rs`**

1. Add `AudioPipelineConfig` and `AudioPipeline` imports
2. Add pipeline config parameters to `recording_thread()` signature: `denoise: bool, noise_gate: bool, rt_limiter: bool, gate_threshold: f64`
3. Create `AudioPipeline` at the start of `recording_thread()` with the config flags
4. In the `SampleFormat::Float32` branch (line 178): call `pipeline.process(&mut samples_vec)` before converting to i16
5. In the `SampleFormat::Int16` branch: convert i16→f32, process, convert f32→i16
6. Log which pipeline stages are active at recording start

### Lane C: Wiring + help text (depends on A + B)

**File: `core/src/commands/record.rs`**

1. Refactor `record_worker` signature to accept `RecordOptions` (+ session, config, output, final_output_path)
2. Update `execute_with_output()` call site
3. Inside `record_worker`, extract fields from `options` where currently using individual params
4. Pass pipeline flags to `WasapiMicrophoneRecorder::with_device_index()` (or a new constructor that accepts pipeline config)
5. After merge + before trim, add:
   ```rust
   if options.normalize {
       let _ = crate::ffmpeg_encoder::normalize_loudness(&merge_output_path, options.target_loudness).await;
   }
   if options.limiter {
       let _ = crate::ffmpeg_encoder::apply_limiter(&merge_output_path).await;
   }
   ```
6. Update `total_steps` calculation to account for normalize and limiter stages
7. Emit JSON processing events for normalize and limiter stages

**File: `cli/src/cli.rs`**

8. Add missing flags to `print_usage()`:
   ```
   --no-normalize      Disable loudness normalization (on by default)
   --no-limiter        Disable soft audio limiter (on by default)
   --no-denoise        Disable noise suppression (on by default)
   --no-noise-gate     Disable noise gate (on by default)
   --target-loudness   Target loudness in LUFS (default: -16)
   --gate-threshold    Noise gate threshold (default: 0.5)
   ```

---

## Tests to Add

### Unit tests in `ffmpeg_encoder.rs`:
1. `test_normalize_loudness_constructs_correct_command` — verify FFmpeg args
2. `test_apply_limiter_constructs_correct_command` — verify FFmpeg args

### Integration tests (require FFmpeg, `#[ignore]`):
3. `test_normalize_loudness_on_real_wav` — generate test WAV, normalize, verify LUFS close to target
4. `test_apply_limiter_on_real_wav` — generate test WAV with loud peaks, limit, verify peaks below threshold

### Existing tests that validate the pipeline:
- `audio_processing/pipeline.rs` — 8 tests covering all stage combinations ✓
- `audio_processing/limiter.rs` — 10 tests ✓
- `audio_processing/noise_gate.rs` — 12 tests ✓

---

## Failure Modes

| Codepath | Failure | Test? | Error handling? | User sees? |
|----------|---------|-------|-----------------|------------|
| `normalize_loudness()` | FFmpeg not found | ✓ (existing check at start) | ✓ | JSON error event |
| `normalize_loudness()` | FFmpeg fails on corrupt file | planned | graceful (log + continue) | Warning in logs, file unchanged |
| `apply_limiter()` | FFmpeg fails | planned | graceful (log + continue) | Warning in logs, file unchanged |
| `AudioPipeline` in mic thread | Pipeline panics | no | needs catch_unwind or just let it propagate | Recording thread error |
| `AudioPipeline` in mic thread | Processing too slow | no | N/A (limiter+gate are <1ms) | No impact |

**Critical design rule:** Post-processing failures must never lose the recording. If normalize or limiter fails, the un-processed merged file is still valid. Use `let _ =` pattern with logging.

---

## Parallelization Strategy

```
Lane A: ffmpeg_encoder.rs + post_processing/pipeline.rs  (independent)
Lane B: wasapi/microphone.rs                              (independent)
         ↓                    ↓
Lane C: commands/record.rs + cli.rs                       (depends on A + B)
```

Launch A + B in parallel. Merge both. Then C.

---

## What Already Exists (reuse, don't rebuild)

| Component | Location | Status |
|-----------|----------|--------|
| `AudioPipeline` (RT processing) | `audio_processing/pipeline.rs` | ✅ Fully implemented + tested |
| `Limiter` (RT) | `audio_processing/limiter.rs` | ✅ Fully implemented + 10 tests |
| `NoiseGate` (6-state machine) | `audio_processing/noise_gate.rs` | ✅ Fully implemented + 12 tests |
| `NoiseSuppressor` (RNNoise) | `audio_processing/noise_suppress.rs` | ⚠️ Placeholder pass-through |
| `PostProcessingPipeline` | `post_processing/pipeline.rs` | ⚠️ Structure exists, normalize/limit are placeholders |
| `measure_lufs()` | `ffmpeg_encoder.rs` | ✅ Working |
| `trim_silence()` | `ffmpeg_encoder.rs` | ✅ Working |
| `calculate_mic_gain_db()` | `ffmpeg_encoder.rs` | ✅ Working |
| All CLI flag parsing | `cli/src/cli.rs` | ✅ Working + tested |
| `RecordOptions` struct | `commands/record.rs` | ✅ All 16 fields defined |
