# Plan: Recording Quality Improvements

> Source PRD: `plans/recording-quality-prd.md`
> Engineering Review: `plans/recording-quality-eng-review.md`
> CEO Review: `plans/recording-quality-review.md`

## Architectural decisions

Durable decisions that apply across all phases:

- **CLI pattern**: All quality features ON by default. Disable with `--no-*` flags. Follows existing `--no-trim` pattern. New flags: `--no-normalize`, `--no-limiter`, `--no-denoise`, `--no-noise-gate`, `--target-loudness <LUFS>`, `--gate-threshold <dB>`, `--app <name>`, `--app-window <title>`.
- **Capture thread stop mechanism**: `WaitForMultipleObjects([audio_event, stop_event])` — pure Win32 pattern. Main thread calls `SetEvent(stop_event)` to stop. Extends to device change events in Phase 6.
- **Audio processing pipeline order (real-time mic)**: Resample to 48kHz → RNNoise (denoise + VAD) → Noise Gate (VAD-driven) → Limiter → Resample to native rate → WAV writer.
- **Post-processing pipeline order**: Merge → Normalize → Limiter → Trim → Finalize.
- **JSON protocol**: Additive — new event types (`interrupted`, `device_state`, `processing`) and new fields on `started` event. Existing events retain their schema.
- **Error types**: `RecorderError` expanded with `DeviceDisconnected`, `DeviceReconnected`, `ProcessNotFound`, `UnsupportedWindowsVersion`, `WriteError`.
- **New dependencies**: `nnnoiseless = "0.5"` (pure Rust RNNoise), `ctrlc = "3"` (or `tokio::signal::ctrl_c()`).
- **New `windows` crate features**: `Win32_System_Threading`, `Win32_UI_WindowsAndMessaging`, `Win32_System_SystemInformation`, `implement`.
- **Module structure**: New modules — `wasapi/capture_engine.rs`, `post_processing/pipeline.rs`, `audio_processing/noise_suppress.rs`, `audio_processing/noise_gate.rs`, `audio_processing/limiter.rs`, `audio_processing/pipeline.rs`.
- **Resampling**: Linear interpolation for non-48kHz mic → 48kHz (and back). No new dependency. Good enough for speech.
- **WAV output**: Always 16-bit PCM for compatibility (existing pattern preserved).

---

## Phase 1: Capture Engine Refactor

**User stories**: #7 (DRY extraction from eng review), #8 (write error handling from eng review)

### What to build

Extract the ~70% shared WASAPI capture logic from `loopback.rs` and `microphone.rs` into a new `capture_engine.rs` module. Both recorders become thin wrappers that configure the shared engine differently (loopback flag, mute support, etc.).

Store `JoinHandle<Result<()>>` from `std::thread::spawn` in both recorder structs. The `stop()` method signals the stop flag and then calls `join()` on the handle with a 5-second timeout, replacing the current `thread::sleep(1000ms)` hack. Thread errors propagate to the caller.

Replace all silent error swallowing on sample writes (`let _ = writer.write_sample(...)`) with tracked error handling: an `AtomicBool` error flag logs the first failure and continues recording (best effort). On stop, emit a warning if any writes failed.

### Acceptance criteria

- [ ] New `wasapi/capture_engine.rs` module contains shared COM init, device enumeration, audio client init, capture loop, buffer processing, format detection, sample conversion, and WAV writer management
- [ ] `loopback.rs` is a thin wrapper that configures `CaptureEngine` with `AUDCLNT_STREAMFLAGS_LOOPBACK`
- [ ] `microphone.rs` is a thin wrapper that configures `CaptureEngine` with mute support and (later) audio processing pipeline injection point
- [ ] `stop()` uses `JoinHandle::join()` with a 5-second timeout instead of `thread::sleep(1000ms)`
- [ ] Sample write errors are tracked with `AtomicBool` — first failure logged, recording continues, warning emitted on stop
- [ ] All existing tests pass (`cargo test`)
- [ ] Recording produces identical output to pre-refactor (manual verification)
- [ ] Unit tests for sample conversion: f32→i16, i32→i16, unsupported format fallback (write silence)
- [ ] Unit tests for format detection: WAVEFORMATEXTENSIBLE SubFormat GUID, legacy format tag

---

## Phase 2: Event-Driven Capture + Silent Loopback Fix + Graceful Shutdown

**User stories**: #1 (event-driven capture), #2 (silent loopback fix), #3 (graceful Ctrl+C shutdown), #18 (lower CPU usage)

### What to build

Replace the `thread::sleep(10ms)` + `GetNextPacketSize()` polling loop in the capture engine with `AUDCLNT_STREAMFLAGS_EVENTCALLBACK` + `WaitForMultipleObjects([audio_event, stop_event])`. WASAPI signals the audio event when a buffer is ready; the main thread signals the stop event via `SetEvent()`. This eliminates CPU waste from polling and ensures audio is captured the instant it's available.

Fix the silent loopback problem by creating a secondary `IAudioClient` on the same output device in normal render mode. Write a buffer of zeros (silence) to keep the WASAPI audio stream active during silent periods. The render client runs alongside the capture client and is cleaned up when recording stops (capture first, then render).

Add graceful Ctrl+C/SIGTERM handling: intercept the signal, set stop events on both capture threads, wait for threads to finish via `JoinHandle::join()`, then run the full post-processing pipeline. Emit `{"type": "interrupted"}` JSON event before starting shutdown. Ensure temp files are cleaned up even on abnormal exit.

Add `Win32_System_Threading` feature to the `windows` crate dependency for `CreateEventW`, `SetEvent`, `WaitForMultipleObjects`.

Handle the critical failure mode: if the ClearBuffer secondary render client fails to initialize, fall back to no-ClearBuffer mode (log warning, accept potential sync drift).

### Acceptance criteria

- [ ] Capture threads use `WaitForMultipleObjects` instead of `thread::sleep` + `GetNextPacketSize` polling
- [ ] `stop()` signals Win32 stop event via `SetEvent()`, threads exit cleanly
- [ ] Loopback recording stays in sync with mic during silent periods (ClearBuffer silence fix active)
- [ ] ClearBuffer render client init failure falls back gracefully with a logged warning
- [ ] Ctrl+C during recording: WAV files finalized, post-processing runs, output file produced
- [ ] `{"type": "interrupted"}` JSON event emitted on Ctrl+C before shutdown begins
- [ ] CPU usage during recording is measurably lower than polling approach
- [ ] `Win32_System_Threading` feature added to `core/Cargo.toml`
- [ ] Temp files cleaned up on both normal and abnormal exit
- [ ] `--legacy-capture` flag preserved as development-time fallback (remove after stabilization)

---

## Phase 3: Post-Processing Pipeline Refactor

**User stories**: #15 (processing stage events)

### What to build

Refactor the 350+ line `merge_audio_streams_smart()` function in `recorder.rs` into a `PostProcessingPipeline` struct with discrete stages: merge, normalize, limit, trim, finalize. Each stage is a method. Progress monitoring is extracted to a shared helper.

The pipeline emits JSON `processing` events for each stage (`{"type": "processing", "stage": "merging"}`, etc.) so calling tools can show accurate progress.

The pipeline is configured at construction time with which stages are enabled (based on CLI flags). Stages that are disabled are simply skipped.

### Acceptance criteria

- [ ] `PostProcessingPipeline` struct with discrete stage methods: `merge()`, `normalize()`, `limit()`, `trim()`, `finalize()`
- [ ] Pipeline configured at construction with enabled/disabled stages
- [ ] Progress monitoring extracted to shared helper (reusable across stages)
- [ ] JSON `{"type": "processing", "stage": "..."}` events emitted for each active stage
- [ ] Existing merge scenarios (A: both audio, B: loopback only, C: mic only, D: both silent) still work correctly
- [ ] Existing silence trimming still works
- [ ] All existing tests pass

---

## Phase 4: Loudness Normalization + Limiter

**User stories**: #4 (loudness normalization), #5 (soft limiter), #8 (disable flags), #17 (target loudness config), #22 (pipeline order)

### What to build

Add EBU R128 loudness normalization as a post-processing stage in the `PostProcessingPipeline`. For WAV output, use two-pass normalization: pass 1 analyzes with FFmpeg's `loudnorm` filter and outputs JSON measurements, pass 2 applies correction with `linear=true` for transparent normalization. For M4A output, use single-pass (quality difference negligible since M4A re-encodes anyway).

Default target: -16 LUFS (streaming standard), true peak limit -1.5 dBTP, loudness range 11 LU. Configurable via `--target-loudness <LUFS>`.

Add a soft audio limiter as a post-processing stage via FFmpeg filter: `alimiter=limit=0.95:attack=0.001:release=50`. This replaces the current hard clipping pattern (`sample.clamp(-1.0, 1.0)`) in capture threads.

Pipeline order: Merge → Normalize → Limiter → Trim → Finalize.

Both features are ON by default. Add `--no-normalize` and `--no-limiter` CLI flags to disable.

### Acceptance criteria

- [ ] Two-pass loudness normalization for WAV output produces output within ±1 LU of -16 LUFS target
- [ ] Single-pass normalization for M4A output produces output within ±2 LU of target
- [ ] `--target-loudness <LUFS>` flag configures target (e.g., `-23` for broadcast)
- [ ] Soft limiter applied via FFmpeg `alimiter` filter — loud transients limited without hard clipping
- [ ] `--no-normalize` flag disables normalization
- [ ] `--no-limiter` flag disables limiter
- [ ] Both features ON by default (zero-config best quality)
- [ ] Edge cases handled: silent file (skip normalization), already-normalized file (minimal change)
- [ ] Pipeline order is Merge → Normalize → Limiter → Trim → Finalize
- [ ] Unit tests: normalization output within target range (fixture WAVs), limiter threshold behavior, custom target loudness
- [ ] CLI tests: 4 new flag parsing tests (`--no-normalize`, `--no-limiter`, `--target-loudness`)

---

## Phase 5: Real-Time Noise Suppression + Noise Gate + Real-Time Limiter

**User stories**: #6 (noise suppression), #7 (noise gate), #8 (disable flags), #16 (gate threshold config), #22 (pipeline order), #24 (sample rate independence)

### What to build

Add a real-time audio processing pipeline injected into the microphone capture thread. The pipeline processes samples before they're written to WAV.

**RNNoise noise suppression** using the `nnnoiseless` crate (pure Rust, no C dependencies). RNNoise requires 48kHz mono float32 in 480-sample frames. The module handles stereo downmix/upmix and resampling (linear interpolation) internally. RNNoise also outputs a VAD (voice activity detection) probability per frame.

Wrap `denoise_frame()` in `std::panic::catch_unwind` — if RNNoise panics on malformed input, disable it for the rest of the recording and log a warning (critical failure mode from eng review).

Add a benchmark test: process 1000 frames, assert average < 2ms. Runtime CPU budget check: if first 100 frames average > 5ms, log performance warning.

**Noise gate** with 6-state machine (CLOSED → OPENING → OPEN → HOLDING → CLOSING → CLOSED). When RNNoise is active, the gate is driven by VAD probability instead of raw RMS thresholding. When `--no-denoise` is used, falls back to dB threshold. Configurable via `--gate-threshold`.

**Real-time limiter** (soft limit at 0.95, 1ms attack) as the final stage before WAV writing.

Pipeline order: Resample to 48kHz → RNNoise → Noise Gate (VAD-driven) → Limiter → Resample to native → WAV writer.

Each stage independently toggleable via `--no-denoise`, `--no-noise-gate`, `--no-limiter`.

### Acceptance criteria

- [ ] `nnnoiseless` dependency added to `core/Cargo.toml`
- [ ] RNNoise processes mic audio in real-time — keyboard typing and fan noise removed from recordings
- [ ] RNNoise handles stereo downmix/upmix and non-48kHz resampling internally
- [ ] RNNoise panic caught via `catch_unwind` — disables denoise for rest of recording with warning
- [ ] Noise gate opens/closes based on VAD probability when RNNoise active, dB threshold when `--no-denoise`
- [ ] Noise gate 6-state machine with smooth attack/release ramps (no clicks)
- [ ] Hold time prevents rapid gate cycling (chattering)
- [ ] Real-time limiter limits samples above 0.95 threshold
- [ ] `--no-denoise`, `--no-noise-gate` flags disable respective features
- [ ] `--gate-threshold` configures gate sensitivity (VAD probability 0.0-1.0 or dB)
- [ ] All features ON by default
- [ ] Benchmark: 1000 RNNoise frames < 2ms average
- [ ] Runtime: if first 100 frames > 5ms average, log performance warning
- [ ] Unit tests: noise gate state transitions, hold time, attack/release ramps, VAD-driven mode
- [ ] Unit tests: RNNoise stereo downmix, resampling, noise reduction (output RMS lower for noise-only), VAD output, frame buffering
- [ ] Unit tests: limiter threshold behavior, passthrough, attack envelope
- [ ] Unit tests: full pipeline chain, each stage independently disabled
- [ ] CLI tests: `--no-denoise`, `--no-noise-gate`, `--gate-threshold` flag parsing

---

## Phase 6: Timestamp Sync + Device Reconnection

**User stories**: #9 (timestamp tracking), #10 (device reconnection), #11 (silence during disconnection), #14 (device state JSON events), #23 (retry interval)

### What to build

**Timestamp synchronization**: Capture `pu64DevicePosition` and `pu64QPCPosition` from `GetBuffer()` (currently passed as `None`). Track QPC (QueryPerformanceCounter) timestamps for each buffer in both loopback and mic streams. Calculate the offset between stream start times. Apply offset during merge via FFmpeg's `-itsoffset` parameter. Handle `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` flag — fall back to frame-count-based timing.

**Device reconnection**: Implement `IMMNotificationClient` COM interface for device change notifications. Register via `IMMDeviceEnumerator::RegisterEndpointNotificationCallback`. Handle `OnDefaultDeviceChanged` and `OnDeviceStateChanged` events. When `GetBuffer` returns `AUDCLNT_E_DEVICE_INVALIDATED`, enter reconnection mode: write silence to WAV to maintain timing/sync, retry device initialization every 2 seconds. Resume capture when device becomes available. Emit JSON `device_state` events for disconnection and reconnection.

Handle the critical failure mode: target device never comes back — continue writing silence and emitting events indefinitely until recording is stopped.

Add `implement` feature to `windows` crate for COM callback implementation.

### Acceptance criteria

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
- [ ] 1+ hour recording with both channels → channels aligned within acceptable tolerance
- [ ] Unit tests: offset calculation, TIMESTAMP_ERROR handling
- [ ] Unit tests: silence writing during disconnect, retry interval, JSON device_state events

---

## Phase 7: Process-Specific Audio Capture

**User stories**: #12 (app-specific capture), #13 (graceful fallback), #20 (process name and window title), #21 (version check)

### What to build

Use `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` to capture audio from a specific application only (e.g., Zoom, Teams). This is the flagship feature — it eliminates Slack notifications, Spotify music, and Windows system sounds from meeting recordings.

Implement a blocking COM wrapper: `IActivateAudioInterfaceCompletionHandler` that signals a Win32 event on completion, then `WaitForSingleObject` to make it synchronous.

Process detection via `FindWindowW` + `GetWindowThreadProcessId` (per `claude.md` antivirus rules — NOT `tasklist`). Set `ProcessLoopbackMode` to `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture child processes.

Requires Windows 10 Build 20348+. Check version at startup using `Win32_System_SystemInformation`. If unavailable, fall back to full system loopback with a clear JSON warning.

Handle the critical failure mode: target process exits mid-recording — detect via `GetBuffer` returning error, fall back to full system loopback with a JSON warning event.

Add `--app <name>` (process name) and `--app-window <title>` (window title) CLI flags. Always opt-in (not default).

Add `Win32_UI_WindowsAndMessaging` feature to `windows` crate for `FindWindowW`, `GetWindowThreadProcessId`.

### Acceptance criteria

- [ ] `--app <name>` captures audio only from the specified process (e.g., `--app Zoom.exe`)
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

---

## Phase 8: Extended JSON Protocol + Started Event

**User stories**: #14 (device state events for calling tools), #15 (processing stage events), #19 (started event features)

### What to build

Ensure all new JSON events from previous phases are complete and consistent. Extend the `started` event to include which quality features are active:

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

Verify all event schemas:
- `{"type": "interrupted", "session_id": "...", "message": "Recording interrupted by signal"}`
- `{"type": "device_state", "session_id": "...", "device": "microphone", "state": "disconnected"}`
- `{"type": "device_state", "session_id": "...", "device": "microphone", "state": "reconnected"}`
- `{"type": "processing", "session_id": "...", "message": "Normalizing loudness", "stage": "normalizing"}`
- `{"type": "processing", "session_id": "...", "message": "Applying limiter", "stage": "limiting"}`

Update the CLI help text and README to document all new flags and events.

### Acceptance criteria

- [ ] `started` event includes `features` map with active quality features
- [ ] `features.process_capture` shows target app name when `--app` is used, `null` otherwise
- [ ] All JSON events include `session_id`
- [ ] `interrupted` event emitted on Ctrl+C (verified end-to-end)
- [ ] `device_state` events emitted on device disconnect/reconnect (verified end-to-end)
- [ ] `processing` events emitted for each post-processing stage (verified end-to-end)
- [ ] CLI help text updated with all 8 new flags
- [ ] README updated with new features, flags, and JSON events
- [ ] JSON protocol is backward-compatible — existing event types unchanged
