# Recording Quality Improvements — Engineering Review

> **Date:** 2026-04-03
> **Reviewer:** /plan-eng-review
> **Plan Under Review:** `plans/recording-quality-prd.md`
> **CEO Review:** `plans/recording-quality-review.md`
> **Branch:** master (`35e4122`)
> **Mode:** FULL_REVIEW
> **Status:** CLEAR — all 11 issues resolved

---

## Step 0: Scope Challenge

**Scope accepted as-is.** The Approach B scope (9 features, 4 domain-aligned phases) from the CEO review is well-calibrated. Each phase touches one subsystem, reducing context-switching and merge conflicts.

- **Complexity check:** ~10-12 files touched, 3-4 new modules. At the boundary of the 8-file smell but justified by domain alignment.
- **TODOS cross-reference:** No prior TODOS.md existed. Created one with 7 deferred items (5 from CEO review, 2 from this eng review).
- **Distribution check:** No new artifact type. Existing CLI binary unchanged.
- **Completeness check:** Plan does the complete version for each feature. Correct call with AI-assisted coding.

---

## What Already Exists

| Sub-problem | Existing Code | Status |
|-------------|--------------|--------|
| WASAPI loopback capture | `wasapi/loopback.rs` (polling loop, 4 format handlers) | Works, needs event-driven upgrade |
| WASAPI mic capture | `wasapi/microphone.rs` (polling loop, mute support) | Works, needs event-driven upgrade |
| Audio level detection | `audio_utils.rs` (RMS for i16, i32, f32) | Complete, reusable for noise gate |
| FFmpeg merge pipeline | `recorder.rs` (4 merge scenarios, progress monitoring) | Works, needs refactoring before extension |
| FFmpeg silence trimming | `ffmpeg_encoder.rs` (`trim_silence()`) | Complete, reusable pattern |
| FFmpeg duration detection | `ffmpeg_encoder.rs` (WAV header + ffprobe fallback) | Complete |
| Signal file IPC | `commands/record.rs` (stop, cancel, mute polling) | Works, extend for Ctrl+C |
| Device enumeration | `wasapi/device_enumeration.rs` | Complete, reuse for process capture |
| JSON event protocol | `commands/record.rs` (`emit_event()`) | Complete, extend for new event types |
| Error types | `error.rs` (4 variants: InvalidParameter, Io, Json, Other) | Minimal, needs expansion |

---

## NOT in Scope

| Item | Rationale |
|------|-----------|
| Dynamic Range Compression | Tier 3, overlaps with normalization. Deferred to TODOS.md. |
| 3-Band Voice EQ | Tier 3, subjective quality improvement. Deferred to TODOS.md. |
| RTWQ Low-Latency Capture | Tier 3, high complexity, marginal benefit over event-driven. Deferred to TODOS.md. |
| Pre-Recording Buffer | Tier 3, requires always-on background capture. Deferred to TODOS.md. |
| Sidechain Ducking | Tier 3, requires cross-channel real-time processing. Deferred to TODOS.md. |
| Cross-platform support | Out of scope for quality-focused plan. |
| Refactoring existing anyhow errors | Cleanup work, deferred to TODOS.md. |

---

## Architecture Review (6 issues, all resolved)

### Issue 1: PostProcessingPipeline refactor (ACCEPTED: 1A)

**Decision:** Refactor `merge_audio_streams_smart()` (350+ lines) into a `PostProcessingPipeline` struct with discrete stages (merge, normalize, limit, trim) BEFORE Phase 2. Each stage is a method. Progress monitoring extracted to shared helper.

**Rationale:** "Make the change easy, then make the easy change" (Beck). Adding normalization + limiter to the existing god function would push it to 500+ lines.

```
POST-PROCESSING PIPELINE (after refactor)
==========================================

  ┌─────────────┐    ┌─────────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐
  │   Merge     │───>│  Normalize  │───>│  Limiter  │───>│   Trim   │───>│ Finalize │
  │ (4 scenarios│    │ (2-pass WAV │    │ (alimiter │    │ (silence │    │ (copy to │
  │  A/B/C/D)   │    │  1-pass M4A)│    │  filter)  │    │  remove) │    │  output) │
  └─────────────┘    └─────────────┘    └──────────┘    └──────────┘    └──────────┘
        │                  │                 │               │               │
        ▼                  ▼                 ▼               ▼               ▼
   JSON: merging     JSON: normalizing  JSON: limiting  JSON: trimming  JSON: completed
   progress: 0-100%  progress: 0-100%
```

### Issue 2: Event-driven capture stop mechanism (ACCEPTED: 2A)

**Decision:** `WaitForMultipleObjects([audio_event, stop_event])` in capture threads. Main thread calls `SetEvent(stop_event)` to stop. Pure Win32 pattern.

**Rationale:** Keeps capture threads in Win32 domain, no async runtime bridging. Naturally extends to Phase 4 (add third event for device change notifications). `SetEvent()` is safe to call from any thread.

```
EVENT-DRIVEN CAPTURE THREAD
============================

  Main Thread (tokio)              Capture Thread (std::thread)
  ─────────────────                ────────────────────────────
       │                                    │
       │                           ┌────────┴────────┐
       │                           │ WaitForMultiple  │
       │                           │ Objects([        │
       │                           │   audio_event,   │◄── WASAPI signals when audio ready
       │                           │   stop_event,    │◄── Main thread signals to stop
       │                           │   device_event   │◄── Phase 4: device change
       │                           │ ])               │
       │                           └────────┬────────┘
       │                                    │
       │                           WAIT_OBJECT_0 → GetBuffer → Process → Write → ReleaseBuffer
       │                           WAIT_OBJECT_0+1 → Stop, finalize WAV, exit thread
       │                           WAIT_OBJECT_0+2 → Device change (Phase 4)
       │                                    │
       │──── SetEvent(stop_event) ─────────>│
       │                                    │
       │◄─── JoinHandle.join() ────────────│ (thread exits cleanly)
       │                                    
       ▼ Post-processing starts
```

### Issue 3: COM async activation for process-specific capture (ACCEPTED: 3A)

**Decision:** Blocking COM wrapper with `IActivateAudioInterfaceCompletionHandler` that signals Win32 event on completion, then `WaitForSingleObject` to make it synchronous. Process detection via `FindWindowW` + `GetWindowThreadProcessId` (per `claude.md` antivirus rules).

**Rationale:** Well-established pattern in Windows audio programming. The `windows` crate's `#[implement]` macro handles COM vtable generation. Highest-risk item but design is sound.

### Issue 4: Audio processing pipeline order (ACCEPTED: 4C — CHANGED FROM PRD)

**Decision:** Changed pipeline order from PRD's `gate → RNNoise → limiter` to `RNNoise (with VAD) → Gate (VAD-driven) → Limiter`.

**Rationale:** RNNoise's VAD output is specifically trained for speech detection. Using it to drive the noise gate gives better speech/silence discrimination than raw RMS thresholding on noisy audio. The `nnnoiseless` crate exposes VAD probability per frame.

```
REAL-TIME AUDIO PROCESSING PIPELINE (revised)
===============================================

  Mic samples (native rate)
       │
       ▼
  ┌──────────────┐
  │  Resample    │  (linear interpolation to 48kHz, if needed)
  │  to 48kHz    │
  └──────┬───────┘
         │
         ▼
  ┌──────────────┐
  │   RNNoise    │──── VAD probability (0.0 - 1.0) ────┐
  │  (denoise)   │                                       │
  └──────┬───────┘                                       │
         │ denoised samples                              │
         ▼                                               ▼
  ┌──────────────┐                              ┌──────────────┐
  │  Noise Gate  │◄──── gate open/close ────────│  VAD-driven  │
  │  (6-state    │      based on VAD prob       │  threshold   │
  │   machine)   │                              └──────────────┘
  └──────┬───────┘
         │
         ▼
  ┌──────────────┐
  │   Limiter    │  (soft limit at 0.95, 1ms attack)
  └──────┬───────┘
         │
         ▼
  ┌──────────────┐
  │  Resample    │  (back to native rate, if needed)
  │  to native   │
  └──────┬───────┘
         │
         ▼
    WAV writer
```

**Impact on CLI:** `--gate-threshold` accepts VAD probability (0.0-1.0) when RNNoise is active, falls back to dB threshold when `--no-denoise` is used.

### Issue 5: Graceful shutdown thread join (ACCEPTED: 5A)

**Decision:** Store `JoinHandle<Result<()>>` in both recorder structs. `stop()` signals Win32 event + joins thread. Thread errors propagate to caller. The 1-second `thread::sleep` hack is removed.

**Rationale:** The current `stop()` with `thread::sleep(1000ms)` is a race condition. On slow disks or large buffers, the thread may not finish in 1 second, leading to corrupted WAV files.

### Issue 6: Missing Windows crate features (ACCEPTED: 6A)

**Decision:** Add required `windows` crate features to `core/Cargo.toml`:
- `Win32_System_Threading` — for `CreateEventW`, `SetEvent`, `WaitForMultipleObjects`
- `Win32_UI_WindowsAndMessaging` — for `FindWindowW`, `GetWindowThreadProcessId`
- `Win32_System_SystemInformation` — for Windows version check
- `implement` feature — for COM callback implementation (`IMMNotificationClient`, `IActivateAudioInterfaceCompletionHandler`)

---

## Code Quality Review (3 issues, all resolved)

### Issue 7: DRY violation between loopback.rs and microphone.rs (ACCEPTED: 7A)

**Decision:** Extract shared WASAPI capture logic into `wasapi/capture_engine.rs` with a `CaptureConfig` struct. Loopback and mic become thin wrappers that configure the engine differently.

**Shared logic (~70% of both files):**
- COM initialization/cleanup
- Device enumeration and selection
- Audio client initialization
- Event-driven capture loop (WaitForMultipleObjects)
- Buffer processing (GetBuffer/ReleaseBuffer)
- Sample format detection and conversion (f32, i32, i16)
- WAV writer management
- Stop/cleanup

**Loopback-specific:**
- `AUDCLNT_STREAMFLAGS_LOOPBACK` flag
- ClearBuffer silence fix (secondary render client)

**Microphone-specific:**
- Mute support (`is_muted` AtomicBool)
- Audio processing pipeline injection (Phase 3)
- No loopback flag

### Issue 8: Silent error swallowing on sample writes (ACCEPTED: 8A)

**Decision:** Track write errors with an `AtomicBool` error flag. On first failure, log a warning. Continue recording (best effort). On stop, emit `{"type": "warning", "message": "Some audio samples could not be written"}`.

**Current pattern (12+ occurrences):**
```rust
let _ = writer.write_sample(sample_i16);  // silently swallows errors
```

**New pattern:**
```rust
if let Err(e) = writer.write_sample(sample_i16) {
    if !write_error_logged.swap(true, Ordering::Relaxed) {
        tracing::warn!("Sample write error: {}. Recording may be incomplete.", e);
    }
}
```

### Issue 9: Error type expansion (ACCEPTED: 9A)

**Decision:** Add 5 new `RecorderError` variants:
- `DeviceDisconnected` — audio device was disconnected mid-recording
- `DeviceReconnected` — audio device was reconnected after disconnection
- `ProcessNotFound` — target process for `--app` was not found
- `UnsupportedWindowsVersion` — Windows version doesn't support process-specific capture
- `WriteError` — sample write failure during recording

---

## Test Review

### Coverage Diagram

```
CODE PATH COVERAGE
=====================================================
[+] wasapi/capture_engine.rs (NEW — Phase 1)
    │
    ├── sample_conversion()
    │   ├── [GAP] f32 → i16 conversion — UNIT TESTABLE
    │   ├── [GAP] i32 → i16 conversion (shift right 16) — UNIT TESTABLE
    │   └── [GAP] Unsupported format fallback (write silence) — UNIT TESTABLE
    │
    └── format_detection()
        ├── [GAP] WAVEFORMATEXTENSIBLE SubFormat GUID — UNIT TESTABLE
        └── [GAP] Legacy format tag — UNIT TESTABLE

[+] post_processing/pipeline.rs (NEW — Phase 2)
    │
    ├── normalize_loudness()
    │   ├── [GAP] Two-pass WAV normalization — NEEDS FIXTURE WAV
    │   ├── [GAP] Single-pass M4A normalization — NEEDS FIXTURE WAV
    │   ├── [GAP] Output within ±1 LU of target — NEEDS FIXTURE WAV
    │   ├── [GAP] Silent file edge case — NEEDS FIXTURE WAV
    │   └── [GAP] Custom --target-loudness — UNIT TESTABLE
    │
    └── apply_limiter()
        ├── [GAP] Samples above threshold limited — NEEDS FIXTURE WAV
        └── [GAP] Samples below threshold passthrough — NEEDS FIXTURE WAV

[+] audio_processing/noise_suppress.rs (NEW — Phase 3)
    │
    ├── [GAP] 48kHz mono → denoised output — UNIT TESTABLE
    ├── [GAP] Stereo downmix/upmix — UNIT TESTABLE
    ├── [GAP] Non-48kHz resampling — UNIT TESTABLE
    ├── [GAP] VAD probability output — UNIT TESTABLE
    ├── [GAP] Noise-only → lower RMS — UNIT TESTABLE
    └── [GAP] Frame boundary buffering — UNIT TESTABLE

[+] audio_processing/noise_gate.rs (NEW — Phase 3)
    │
    ├── [GAP] 6 state transitions — UNIT TESTABLE
    ├── [GAP] Hold time prevents chattering — UNIT TESTABLE
    ├── [GAP] Attack/release ramps (no clicks) — UNIT TESTABLE
    └── [GAP] VAD-driven mode — UNIT TESTABLE

[+] audio_processing/limiter.rs (NEW — Phase 3)
    │
    ├── [GAP] Above threshold → limited — UNIT TESTABLE
    ├── [GAP] Below threshold → passthrough — UNIT TESTABLE
    └── [GAP] Fast attack envelope — UNIT TESTABLE

[+] audio_processing/pipeline.rs (NEW — Phase 3)
    │
    ├── [GAP] Full chain: RNNoise → Gate → Limiter — UNIT TESTABLE
    └── [GAP] Each stage independently disabled — UNIT TESTABLE

[+] wasapi/timestamp_sync.rs (NEW — Phase 4)
    │
    ├── [GAP] Offset calculation — UNIT TESTABLE
    └── [GAP] TIMESTAMP_ERROR handling — UNIT TESTABLE

[+] wasapi/device_reconnection.rs (NEW — Phase 4)
    │
    ├── [GAP] Silence writing during disconnect — UNIT TESTABLE
    ├── [GAP] 2-second retry interval — UNIT TESTABLE
    └── [GAP] JSON device_state events — UNIT TESTABLE

[+] wasapi/process_capture.rs (NEW — Phase 4)
    │
    ├── [GAP] Windows version check — UNIT TESTABLE
    └── [GAP] Fallback to full loopback — UNIT TESTABLE

[+] cli/src/cli.rs (MODIFIED — all phases)
    │
    ├── [★★★ TESTED] Existing flags — 8 tests
    └── [GAP] 8 new flags (--no-normalize, --no-limiter, etc.) — UNIT TESTABLE

─────────────────────────────────────────────────────
COVERAGE: 17 existing tests, 36 new unit tests needed
HARDWARE-DEPENDENT (manual only): ~15 paths
QUALITY: ★★★: 1 set (existing)
─────────────────────────────────────────────────────
```

### Test Plan (10A — Full)

| Phase | Test File | Tests | What |
|-------|-----------|-------|------|
| 1 | `capture_engine_test.rs` | 5 | Format detection, sample conversion (f32→i16, i32→i16), unsupported format |
| 1 | `cli.rs` (additions) | 8 | New flag parsing for all 8 new CLI flags |
| 2 | `post_processing_test.rs` | 7 | Normalization (2-pass, 1-pass, silent, already-normalized, custom target), limiter (above/below threshold) |
| 3 | `noise_gate_test.rs` | 6 | All 6 state transitions, hold time, attack/release ramps, VAD-driven mode |
| 3 | `noise_suppress_test.rs` | 5 | Stereo downmix, resampling, noise reduction, VAD output, frame buffering |
| 3 | `limiter_test.rs` | 3 | Threshold behavior, passthrough, attack envelope |
| 3 | `pipeline_test.rs` | 3 | Full chain, independent disable, config from flags |
| 3 | `noise_suppress_bench.rs` | 1 | Performance: 1000 frames < 2ms average |
| 4 | `timestamp_sync_test.rs` | 2 | Offset calculation, TIMESTAMP_ERROR handling |
| 4 | `device_reconnection_test.rs` | 3 | Silence writing, retry interval, JSON events |
| **Total** | **10 files** | **~43** | |

Fixture WAVs generated programmatically using `hound` crate (already a dependency).

---

## Performance Review (2 issues, all resolved)

### Issue 11: RNNoise CPU budget (ACCEPTED: 11A)

**Decision:** Add benchmark test + runtime CPU budget check. Benchmark: process 1000 frames, assert average < 2ms. Runtime: if first 100 frames average > 5ms, log performance warning.

### Issue 12: Resampling algorithm (ACCEPTED: 12A)

**Decision:** Linear interpolation for non-48kHz mic resampling. Fast (~0.01ms per frame), no new dependency. Good enough for speech (the stated use case). Quality difference vs. sinc resampling only matters for music.

---

## Failure Modes

| Codepath | Failure Mode | Test? | Error Handling? | User Sees? |
|----------|-------------|-------|-----------------|------------|
| Event-driven capture | `WaitForMultipleObjects` returns `WAIT_FAILED` | No (hardware) | Yes (log + fallback) | Silent fallback |
| ClearBuffer silence fix | Secondary render client fails to initialize | No (hardware) | Should add | **CRITICAL GAP** — silent failure, loopback sync drift |
| Graceful shutdown | Thread join times out (deadlock) | No | Should add timeout | **CRITICAL GAP** — process hangs |
| Normalization | FFmpeg `loudnorm` filter fails | Yes (fixture) | Yes (skip stage) | JSON warning |
| RNNoise | `nnnoiseless` panics on malformed input | Yes (unit) | Should add catch_unwind | **CRITICAL GAP** — capture thread dies |
| Noise gate | Gate stuck in CLOSED state | Yes (unit) | N/A (logic bug) | Silent mic |
| Device reconnection | Device never comes back | No (hardware) | Yes (silence + events) | JSON events |
| Process-specific capture | Target process exits mid-recording | No (hardware) | Should add | **CRITICAL GAP** — undefined behavior |
| Process-specific capture | Windows version too old | Yes (unit) | Yes (fallback + warning) | JSON warning |

**Critical gaps (4):**
1. ClearBuffer render client init failure — add error handling, fall back to no-ClearBuffer mode
2. Thread join timeout — add 5-second timeout on `join()`, force-kill if exceeded
3. RNNoise panic — wrap `denoise_frame()` in `std::panic::catch_unwind`, disable RNNoise on panic
4. Target process exits mid-recording — detect via `GetBuffer` returning error, fall back to full loopback

---

## Worktree Parallelization Strategy

| Step | Modules Touched | Depends On |
|------|----------------|------------|
| Phase 1: Capture Engine Refactor | `wasapi/` | — |
| Phase 1: Event-Driven + ClearBuffer + Shutdown | `wasapi/`, `commands/record.rs` | Capture Engine Refactor |
| Phase 2: PostProcessingPipeline Refactor | `recorder.rs`, `ffmpeg_encoder.rs` | — |
| Phase 2: Normalization + Limiter | `post_processing/` | Pipeline Refactor |
| Phase 3: Audio Processing Modules | `audio_processing/` (NEW) | — |
| Phase 3: Pipeline Integration | `wasapi/microphone.rs`, `audio_processing/` | Phase 1 + Audio Processing Modules |
| Phase 4: Timestamps + Reconnection | `wasapi/` | Phase 1 |
| Phase 4: Process-Specific Capture | `wasapi/` | Phase 1 |

**Parallel Lanes:**

```
Lane A (WASAPI):     Capture Engine Refactor → Event-Driven + ClearBuffer + Shutdown
                                                        ↓
                                              Phase 4: Timestamps + Reconnection + Process Capture

Lane B (FFmpeg):     PostProcessingPipeline Refactor → Normalization + Limiter
                     (independent of Lane A)

Lane C (DSP):        Audio Processing Modules (noise_gate, noise_suppress, limiter, pipeline)
                     (independent of Lane A and B)
                                    ↓
                     Pipeline Integration (needs Lane A complete)
```

**Execution order:**
1. Launch Lane A + Lane B + Lane C in parallel
2. Lane C's Pipeline Integration waits for Lane A to complete
3. Phase 4 waits for Lane A to complete
4. All lanes merge at the end

**Conflict flags:** Lane A and Lane C both touch `wasapi/microphone.rs` for pipeline integration. Coordinate this merge point.

---

## Diagrams for Implementation

### Noise Gate State Machine

```
                    ┌─────────┐
                    │ CLOSED  │◄──────────────────────────────┐
                    └────┬────┘                               │
                         │ VAD prob > open_threshold          │
                         ▼                                    │
                    ┌─────────┐                               │
                    │ OPENING │  (attack ramp: 0.0 → 1.0)    │
                    └────┬────┘                               │
                         │ gain reaches 1.0                   │
                         ▼                                    │
                    ┌─────────┐                               │
              ┌────>│  OPEN   │                               │
              │     └────┬────┘                               │
              │          │ VAD prob < close_threshold         │
              │          ▼                                    │
              │     ┌─────────┐                               │
              │     │ HOLDING │  (hold_time countdown)        │
              │     └────┬────┘                               │
              │          │         │                           │
              │          │ timeout │ VAD prob > open_threshold │
              │          ▼         └──────────────────────────>│
              │     ┌─────────┐                               │
              │     │ CLOSING │  (release ramp: 1.0 → 0.0)   │
              │     └────┬────┘                               │
              │          │ gain reaches 0.0                   │
              │          └────────────────────────────────────>│
              │                                               │
              └───── VAD prob > open_threshold (re-trigger) ──┘
```

### Device Reconnection Flow (Phase 4)

```
  Normal Recording
       │
       ▼
  GetBuffer() returns AUDCLNT_E_DEVICE_INVALIDATED
       │
       ▼
  ┌──────────────────┐
  │ DISCONNECTED     │──── emit JSON: {"type":"device_state","state":"disconnected"}
  │                  │
  │ Write silence    │──── maintains WAV timing/sync
  │ to WAV           │
  │                  │
  │ Retry every 2s:  │
  │  - Re-enumerate  │
  │  - Re-initialize │
  │  - Re-start      │
  └────────┬─────────┘
           │ Device available
           ▼
  ┌──────────────────┐
  │ RECONNECTED      │──── emit JSON: {"type":"device_state","state":"reconnected"}
  │                  │
  │ Resume capture   │
  └──────────────────┘
```

---

## Completion Summary

- **Step 0: Scope Challenge** — scope accepted as-is (Approach B, 9 features, 4 phases)
- **Architecture Review:** 6 issues found, all resolved
- **Code Quality Review:** 3 issues found, all resolved
- **Test Review:** diagram produced, 36 unit-testable gaps identified, full test plan accepted (10A)
- **Performance Review:** 2 issues found, all resolved
- **NOT in scope:** written (7 items)
- **What already exists:** written (10 items)
- **TODOS.md updates:** 2 items proposed, both accepted
- **Failure modes:** 4 critical gaps flagged
- **Outside voice:** skipped
- **Parallelization:** 3 lanes, 2 parallel / 1 dependent
- **Lake Score:** 11/11 recommendations chose complete option

### Key Changes to PRD

1. **Pipeline order changed:** `RNNoise → Gate (VAD-driven) → Limiter` instead of `Gate → RNNoise → Limiter`
2. **Pre-requisite refactors added:** `capture_engine.rs` extraction (Issue 7) and `PostProcessingPipeline` refactor (Issue 1) before feature work
3. **Thread join fix:** Store `JoinHandle`, proper join on stop (Issue 5)
4. **Write error handling:** Track and report sample write failures (Issue 8)
5. **Windows crate features:** Document required feature additions (Issue 6)
6. **Resampling specified:** Linear interpolation for non-48kHz mics (Issue 12)
7. **CPU budget check:** Benchmark + runtime warning for RNNoise (Issue 11)
8. **4 critical failure modes** identified that need error handling

### Unresolved Decisions

None. All 11 issues were resolved during the review.
