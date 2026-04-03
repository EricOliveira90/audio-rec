# Recording Quality Improvements — GitHub Issues

> Parent PRD: #6
> Generated from: `plans/recording-quality-prd.md`, `plans/recording-quality-eng-review.md`, `plans/recording-quality-plan.md`
> All 13 issues created: #7 through #19

## Issue Number Mapping

| # | GitHub | Title | Blocked by |
|---|--------|-------|------------|
| 1 | #7 | Capture Engine Refactor — Extract shared WASAPI logic | None |
| 2 | #8 | Post-Processing Pipeline Refactor | None |
| 3 | #9 | Event-Driven WASAPI Capture | #7 |
| 4 | #10 | Silent Loopback Fix (ClearBuffer) | #9 |
| 5 | #11 | Graceful Ctrl+C Shutdown | #9 |
| 6 | #12 | Loudness Normalization (EBU R128) | #8 |
| 7 | #13 | Soft Audio Limiter (Post-Processing) | #8 |
| 8 | #14 | RNNoise Real-Time Noise Suppression | #7 |
| 9 | #15 | Noise Gate (VAD-Driven) | #14 |
| 10 | #16 | Real-Time Limiter + Audio Processing Pipeline Integration | #14, #15 |
| 11 | #17 | Timestamp Sync + Device Reconnection | #9 |
| 12 | #18 | Process-Specific Audio Capture | #9 |
| 13 | #19 | Extended JSON Protocol + Started Event Features | #11, #8, #12-#18 |

## Dependency Graph

```
#7 (Capture Engine) ──► #9 (Event-Driven) ──► #10 (ClearBuffer)
                    │                      ├──► #11 (Ctrl+C)
                    │                      ├──► #17 (Timestamps + Reconnect)
                    │                      └──► #18 (Process Capture)
                    └──► #14 (RNNoise) ──► #15 (Gate) ──► #16 (RT Pipeline)

#8 (PostProc Refactor) ──► #12 (Normalization)
                        └──► #13 (Limiter)

#19 (JSON Protocol) ◄── all other issues
```

---

## Issue 1: Capture Engine Refactor — Extract shared WASAPI logic ✅ CREATED AS #7

Already created. See https://github.com/EricOliveira90/audio-rec/issues/7

---

## Issue 2: Post-Processing Pipeline Refactor

**Title:** Post-Processing Pipeline Refactor
**Labels:** enhancement
**Blocked by:** None — can start immediately

### Body

## Parent PRD

#6

## What to build

Refactor the 350+ line `merge_audio_streams_smart()` function in `recorder.rs` into a `PostProcessingPipeline` struct with discrete stages: merge, normalize, limit, trim, finalize. Each stage is a method. Progress monitoring is extracted to a shared helper.

The pipeline emits JSON `processing` events for each stage (`{"type": "processing", "stage": "merging"}`, etc.) so calling tools can show accurate progress.

The pipeline is configured at construction time with which stages are enabled (based on CLI flags). Stages that are disabled are simply skipped.

**Pipeline stages (in order):**
1. **Merge** — 4 scenarios: A (both audio), B (loopback only), C (mic only), D (both silent)
2. **Normalize** — placeholder, implemented in follow-up issue
3. **Limit** — placeholder, implemented in follow-up issue
4. **Trim** — existing silence trimming logic
5. **Finalize** — copy to output location

See PRD section: *Post-Processing Pipeline*
See Eng Review: Issue 1 (PostProcessingPipeline refactor)
See Plan: Phase 3

## Acceptance criteria

- [ ] `PostProcessingPipeline` struct with discrete stage methods: `merge()`, `normalize()`, `limit()`, `trim()`, `finalize()`
- [ ] Pipeline configured at construction with enabled/disabled stages
- [ ] Progress monitoring extracted to shared helper (reusable across stages)
- [ ] JSON `{"type": "processing", "stage": "..."}` events emitted for each active stage
- [ ] Existing merge scenarios (A: both audio, B: loopback only, C: mic only, D: both silent) still work correctly
- [ ] Existing silence trimming still works
- [ ] All existing tests pass (`cargo test`)

## Blocked by

None — can start immediately

## User stories addressed

- User story 15: processing JSON events for each post-processing stage

---

## Issue 3: Event-Driven WASAPI Capture

**Title:** Event-Driven WASAPI Capture
**Labels:** enhancement
**Blocked by:** #7

### Body

## Parent PRD

#6

## What to build

Replace the `thread::sleep(10ms)` + `GetNextPacketSize()` polling loop in the capture engine with `AUDCLNT_STREAMFLAGS_EVENTCALLBACK` + `WaitForMultipleObjects([audio_event, stop_event])`. WASAPI signals the audio event when a buffer is ready; the main thread signals the stop event via `SetEvent()`. This eliminates CPU waste from polling and ensures audio is captured the instant it's available.

**Key implementation details:**
- Initialize audio client with `AUDCLNT_STREAMFLAGS_EVENTCALLBACK` flag
- Create Win32 events via `CreateEventW` for audio readiness and stop signaling
- Set the event handle on the audio client via `SetEventHandle()`
- Capture loop uses `WaitForMultipleObjects([audio_event, stop_event])` instead of sleep+poll
- `WAIT_OBJECT_0` → audio ready: `GetBuffer` → process → `ReleaseBuffer`
- `WAIT_OBJECT_0+1` → stop signal: finalize WAV, exit thread
- Both loopback and microphone capture threads get the same upgrade
- Add `Win32_System_Threading` feature to `windows` crate in `core/Cargo.toml`

This pattern naturally extends to Phase 4 (add a third event for device change notifications).

See PRD section: *Event-Driven Capture (Phase 1)*
See Eng Review: Issue 2 (event-driven capture stop mechanism)
See Plan: Phase 2

## Acceptance criteria

- [ ] Capture threads use `WaitForMultipleObjects` instead of `thread::sleep` + `GetNextPacketSize` polling
- [ ] `stop()` signals Win32 stop event via `SetEvent()`, threads exit cleanly
- [ ] CPU usage during recording is measurably lower than polling approach
- [ ] `Win32_System_Threading` feature added to `core/Cargo.toml`
- [ ] Both loopback and microphone capture use event-driven pattern
- [ ] All existing tests pass (`cargo test`)
- [ ] Recording produces identical output to pre-upgrade (manual verification)

## Blocked by

- Blocked by #7 (Capture Engine Refactor)

## User stories addressed

- User story 1: event-driven WASAPI capture instead of polling
- User story 18: lower CPU usage than polling

---

## Issue 4: Silent Loopback Fix (ClearBuffer)

**Title:** Silent Loopback Fix (ClearBuffer)
**Labels:** enhancement
**Blocked by:** Event-Driven WASAPI Capture issue

### Body

## Parent PRD

#6

## What to build

Fix the silent loopback problem where WASAPI loopback stops delivering packets when system audio goes completely silent, causing the loopback recording to be shorter than the mic recording and creating sync drift when merging.

**Solution:** Create a secondary `IAudioClient` on the same output device in normal (non-loopback) render mode. Write a buffer of zeros (silence) to keep the WASAPI audio stream active during silent periods. The render client runs alongside the capture client.

**Key implementation details:**
- Secondary render client initialized alongside the loopback capture client
- Writes silence buffers to keep the audio engine active
- Cleanup order: stop capture client first, then stop and release the render client
- **Critical failure mode:** If the secondary render client fails to initialize, fall back to no-ClearBuffer mode (log warning, accept potential sync drift)

See PRD section: *Silent Loopback Fix (Phase 1)*
See Eng Review: Failure Modes — ClearBuffer render client init failure
See Plan: Phase 2

## Acceptance criteria

- [ ] Loopback recording stays in sync with mic during silent periods (ClearBuffer silence fix active)
- [ ] Secondary render client writes silence to keep WASAPI stream alive
- [ ] ClearBuffer render client init failure falls back gracefully with a logged warning
- [ ] Cleanup order: capture client stopped first, then render client
- [ ] No audible artifacts from the silence injection
- [ ] Manual test: record a meeting with silent presentation slides, verify loopback and mic stay in sync

## Blocked by

- Blocked by Event-Driven WASAPI Capture issue

## User stories addressed

- User story 2: loopback stream stays alive during silent periods

---

## Issue 5: Graceful Ctrl+C Shutdown

**Title:** Graceful Ctrl+C Shutdown
**Labels:** enhancement
**Blocked by:** Event-Driven WASAPI Capture issue

### Body

## Parent PRD

#6

## What to build

Add graceful Ctrl+C/SIGTERM handling so that pressing Ctrl+C during recording doesn't corrupt files or skip post-processing.

**Key implementation details:**
- Use `ctrlc` crate or `tokio::signal::ctrl_c()` to intercept Ctrl+C/SIGTERM
- On signal: set stop events on both capture threads, wait for threads to finish via `JoinHandle::join()`, then run the full post-processing pipeline
- Emit `{"type": "interrupted", "session_id": "...", "message": "Recording interrupted by signal"}` JSON event before starting shutdown
- Ensure temp files are cleaned up even on abnormal exit (register cleanup handler)
- Add `ctrlc` dependency to `core/Cargo.toml` (or use `tokio::signal::ctrl_c()`)

See PRD section: *Graceful Shutdown (Phase 1)*
See Plan: Phase 2

## Acceptance criteria

- [ ] Ctrl+C during recording: WAV files finalized, post-processing runs, output file produced
- [ ] `{"type": "interrupted"}` JSON event emitted on Ctrl+C before shutdown begins
- [ ] Temp files cleaned up on both normal and abnormal exit
- [ ] No corrupted WAV files left behind after Ctrl+C
- [ ] Post-processing pipeline runs to completion after interrupt
- [ ] Manual test: start recording, press Ctrl+C, verify output file is valid and complete

## Blocked by

- Blocked by Event-Driven WASAPI Capture issue

## User stories addressed

- User story 3: graceful Ctrl+C shutdown with finalized files
- User story 14: JSON `interrupted` event for calling tools

---

## Issue 6: Loudness Normalization (EBU R128)

**Title:** Loudness Normalization (EBU R128)
**Labels:** enhancement
**Blocked by:** Post-Processing Pipeline Refactor issue

### Body

## Parent PRD

#6

## What to build

Add EBU R128 loudness normalization as a post-processing stage in the `PostProcessingPipeline`.

**Two-pass for WAV output:** Pass 1 analyzes with FFmpeg's `loudnorm` filter and outputs JSON measurements. Pass 2 applies correction with `linear=true` for transparent normalization.

**Single-pass for M4A output:** Since M4A already re-encodes, the quality difference from single-pass is negligible. Simpler and faster.

**Parameters:**
- Default target: -16 LUFS (streaming standard)
- True peak limit: -1.5 dBTP
- Loudness range: 11 LU
- Configurable via `--target-loudness <LUFS>`

**Pipeline order:** Merge → **Normalize** → Limiter → Trim → Finalize

**CLI flags:**
- `--no-normalize` — disable normalization (ON by default)
- `--target-loudness <LUFS>` — configure target loudness level

See PRD section: *Loudness Normalization (Phase 2)*
See Plan: Phase 4

## Acceptance criteria

- [ ] Two-pass loudness normalization for WAV output produces output within ±1 LU of -16 LUFS target
- [ ] Single-pass normalization for M4A output produces output within ±2 LU of target
- [ ] `--target-loudness <LUFS>` flag configures target (e.g., `-23` for broadcast)
- [ ] `--no-normalize` flag disables normalization
- [ ] Normalization ON by default (zero-config best quality)
- [ ] Edge cases handled: silent file (skip normalization), already-normalized file (minimal change)
- [ ] Pipeline order is Merge → Normalize → Limiter → Trim → Finalize
- [ ] Unit tests: normalization output within target range (fixture WAVs generated with `hound`)
- [ ] CLI tests: `--no-normalize`, `--target-loudness` flag parsing

## Blocked by

- Blocked by Post-Processing Pipeline Refactor issue

## User stories addressed

- User story 4: automatic loudness normalization to EBU R128
- User story 8: `--no-normalize` disable flag
- User story 17: configurable target loudness

---

## Issue 7: Soft Audio Limiter (Post-Processing)

**Title:** Soft Audio Limiter (Post-Processing)
**Labels:** enhancement
**Blocked by:** Post-Processing Pipeline Refactor issue

### Body

## Parent PRD

#6

## What to build

Add a soft audio limiter as a post-processing stage in the `PostProcessingPipeline` via FFmpeg filter: `alimiter=limit=0.95:attack=0.001:release=50`.

This replaces the current hard clipping pattern (`sample.clamp(-1.0, 1.0)`) in capture threads with a proper soft limiter that prevents distortion from sudden loud sounds.

**Parameters:**
- Limit: 0.95 (soft ceiling)
- Attack: 1ms (fast, catches transients)
- Release: 50ms

**CLI flag:** `--no-limiter` — disable limiter (ON by default)

See PRD section: *Audio Limiter (Phase 2)*
See Plan: Phase 4

## Acceptance criteria

- [ ] Soft limiter applied via FFmpeg `alimiter` filter — loud transients limited without hard clipping
- [ ] `--no-limiter` flag disables limiter
- [ ] Limiter ON by default (zero-config best quality)
- [ ] Samples below threshold pass through unchanged
- [ ] Pipeline order: Merge → Normalize → **Limiter** → Trim → Finalize
- [ ] Unit tests: limiter threshold behavior (fixture WAVs), passthrough for quiet audio
- [ ] CLI tests: `--no-limiter` flag parsing

## Blocked by

- Blocked by Post-Processing Pipeline Refactor issue

## User stories addressed

- User story 5: soft audio limiter for sudden loud sounds
- User story 8: `--no-limiter` disable flag

---

## Issue 8: RNNoise Real-Time Noise Suppression

**Title:** RNNoise Real-Time Noise Suppression
**Labels:** enhancement
**Blocked by:** #7

### Body

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

---

## Issue 9: Noise Gate (VAD-Driven)

**Title:** Noise Gate (VAD-Driven)
**Labels:** enhancement
**Blocked by:** RNNoise Real-Time Noise Suppression issue

### Body

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

- Blocked by RNNoise Real-Time Noise Suppression issue

## User stories addressed

- User story 7: noise gate on microphone
- User story 8: `--no-noise-gate` disable flag
- User story 16: configurable gate threshold
- User story 22: quality features work together seamlessly

---

## Issue 10: Real-Time Limiter + Audio Processing Pipeline Integration

**Title:** Real-Time Limiter + Audio Processing Pipeline Integration
**Labels:** enhancement
**Blocked by:** RNNoise issue + Noise Gate issue

### Body

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

- Blocked by RNNoise Real-Time Noise Suppression issue
- Blocked by Noise Gate (VAD-Driven) issue

## User stories addressed

- User story 22: all quality features work together seamlessly

---

## Issue 11: Timestamp Sync + Device Reconnection

**Title:** Timestamp Sync + Device Reconnection
**Labels:** enhancement
**Blocked by:** Event-Driven WASAPI Capture issue

### Body

## Parent PRD

#6

## What to build

**Timestamp synchronization:** Capture `pu64DevicePosition` and `pu64QPCPosition` from `GetBuffer()` (currently passed as `None`). Track QPC (QueryPerformanceCounter) timestamps for each buffer in both loopback and mic streams. Calculate the offset between stream start times. Apply offset during merge via FFmpeg's `-itsoffset` parameter. Handle `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` flag — fall back to frame-count-based timing.

**Device reconnection:** Implement `IMMNotificationClient` COM interface for device change notifications. Register via `IMMDeviceEnumerator::RegisterEndpointNotificationCallback`. Handle `OnDefaultDeviceChanged` and `OnDeviceStateChanged` events. When `GetBuffer` returns `AUDCLNT_E_DEVICE_INVALIDATED`, enter reconnection mode: write silence to WAV to maintain timing/sync, retry device initialization every 2 seconds. Resume capture when device becomes available. Emit JSON `device_state` events.

**New error variants:** `DeviceDisconnected`, `DeviceReconnected` added to `RecorderError`.

**New `windows` crate features:** `implement` (for COM callback), `Win32_System_SystemInformation`.

See PRD sections: *Timestamp Synchronization*, *Device Reconnection*
See Eng Review: Failure Modes (device never comes back)
See Plan: Phase 6

## Acceptance criteria

- [ ] `GetBuffer()` captures QPC timestamps for both loopback and mic streams
- [ ] Stream start time offset calculated and applied via FFmpeg `-itsoffset` during merge
- [ ] `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` handled — falls back to frame-count timing
- [ ] `IMMNotificationClient` COM interface implemented for device change notifications
- [ ] Device disconnection detected via `AUDCLNT_E_DEVICE_INVALIDATED` from `GetBuffer`
- [ ] Silence written to WAV during device disconnection (maintains timing/sync)
- [ ] Device reconnection retried every 2 seconds without excessive resource consumption
- [ ] Recording resumes when device becomes available again
- [ ] `{"type": "device_state", "state": "disconnected"}` JSON event emitted on disconnect
- [ ] `{"type": "device_state", "state": "reconnected"}` JSON event emitted on reconnect
- [ ] `DeviceDisconnected` and `DeviceReconnected` error variants added to `RecorderError`
- [ ] `implement` and `Win32_System_SystemInformation` features added to `windows` crate
- [ ] Unit tests: offset calculation, TIMESTAMP_ERROR handling
- [ ] Unit tests: silence writing during disconnect, retry interval, JSON device_state events

## Blocked by

- Blocked by Event-Driven WASAPI Capture issue

## User stories addressed

- User story 9: WASAPI device timestamp tracking for sync accuracy
- User story 10: automatic device reconnection
- User story 11: silence during device disconnection
- User story 14: JSON device_state events for calling tools
- User story 23: reasonable retry intervals

---

## Issue 12: Process-Specific Audio Capture

**Title:** Process-Specific Audio Capture
**Labels:** enhancement
**Blocked by:** Event-Driven WASAPI Capture issue

### Body

## Parent PRD

#6

## What to build

Use `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` to capture audio from a specific application only (e.g., Zoom, Teams). This is the flagship feature — it eliminates Slack notifications, Spotify music, and Windows system sounds from meeting recordings.

**Key implementation details:**
- Blocking COM wrapper: implement `IActivateAudioInterfaceCompletionHandler` that signals a Win32 event on completion, then `WaitForSingleObject` to make it synchronous
- Process detection: `FindWindowW` + `GetWindowThreadProcessId` (per `claude.md` antivirus rules — NOT `tasklist`)
- Set `ProcessLoopbackMode` to `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture child processes
- Requires Windows 10 Build 20348+ — check version at startup using `Win32_System_SystemInformation`, fall back to full system loopback with clear JSON warning if unavailable
- Handle target process exit mid-recording — detect via `GetBuffer` returning error, fall back to full system loopback with JSON warning

**New error variants:** `ProcessNotFound`, `UnsupportedWindowsVersion` added to `RecorderError`.

**New `windows` crate feature:** `Win32_UI_WindowsAndMessaging` for `FindWindowW`, `GetWindowThreadProcessId`.

**CLI flags:**
- `--app <name>` — capture audio from specific process (e.g., `--app Zoom.exe`)
- `--app-window <title>` — capture audio from specific window title (e.g., `--app-window "Zoom Meeting"`)
- Always opt-in (not default)

See PRD section: *Process-Specific Capture (Phase 4)*
See Eng Review: Issue 3 (COM async activation), Failure Modes (target process exit)
See Plan: Phase 7

## Acceptance criteria

- [ ] `--app <name>` captures audio only from the specified process
- [ ] `--app-window <title>` captures audio from the window with the specified title
- [ ] Process detection uses `FindWindowW` + `GetWindowThreadProcessId` (not `tasklist`)
- [ ] `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` captures child processes
- [ ] Windows version check at startup — clear JSON warning if Build < 20348
- [ ] Graceful fallback to full system loopback when process-specific capture unavailable
- [ ] Target process exit mid-recording detected — falls back to full loopback with JSON warning
- [ ] `ProcessNotFound` and `UnsupportedWindowsVersion` error variants added to `RecorderError`
- [ ] `Win32_UI_WindowsAndMessaging` feature added to `windows` crate
- [ ] COM async activation wrapper implemented with `IActivateAudioInterfaceCompletionHandler`
- [ ] Unit tests: Windows version check, fallback to full loopback
- [ ] CLI tests: `--app`, `--app-window` flag parsing

## Blocked by

- Blocked by Event-Driven WASAPI Capture issue

## User stories addressed

- User story 12: capture audio only from specific application
- User story 13: graceful fallback on older Windows versions
- User story 20: process name and window title targeting
- User story 21: Windows version compatibility check

---

## Issue 13: Extended JSON Protocol + Started Event Features

**Title:** Extended JSON Protocol + Started Event Features
**Labels:** enhancement
**Blocked by:** All other issues (final integration)

### Body

## Parent PRD

#6

## What to build

Final integration issue: ensure all new JSON events from previous issues are complete and consistent. Extend the `started` event to include which quality features are active.

**Extended `started` event:**
```json
{
  "type": "started",
  "session_id": "...",
  "features": {
    "denoise": true,
    "noise_gate": true,
    "normalize": true,
    "limiter": true,
    "process_capture": "Zoom.exe"
  }
}
```

**Verify all event schemas:**
- `{"type": "interrupted", "session_id": "...", "message": "Recording interrupted by signal"}`
- `{"type": "device_state", "session_id": "...", "device": "microphone", "state": "disconnected"}`
- `{"type": "device_state", "session_id": "...", "device": "microphone", "state": "reconnected"}`
