# PRD: Recording Quality Improvements (Approach B — Domain-Aligned Minimal)

> **Date:** 2026-04-03
> **Status:** Approved
> **Scope:** 9 features across 4 phases (Approach B from CEO review)
> **Review Source:** `plans/recording-quality-review.md`

---

## Problem Statement

Users of `audio-rec` who record Zoom/Teams meetings experience several concrete quality problems that degrade the usefulness of their recordings:

1. **Audio gaps during silence** — When system audio goes completely silent (e.g., someone presenting slides without talking), WASAPI loopback stops delivering packets, causing the loopback recording to be shorter than the mic recording and creating sync drift when merging.

2. **Background noise pollution** — Keyboard typing, fan noise, AC hum, and background chatter from the microphone channel make recordings unpleasant to listen to and harder to transcribe.

3. **Notification dings in recordings** — All system audio is captured, including Slack notifications, Spotify music, and Windows system sounds, polluting the meeting recording.

4. **Inconsistent volume levels** — Different meeting participants speak at different volumes, and the system audio vs. microphone channels have wildly different levels, making recordings hard to follow.

5. **Corrupted files on Ctrl+C** — Pressing Ctrl+C during recording leaves corrupted temporary WAV files and skips post-processing entirely, resulting in data loss.

6. **Potential sync drift in long recordings** — No timestamp tracking between loopback and mic channels means 1+ hour recordings can develop lip-sync drift.

7. **Lost recordings from device disconnection** — Bluetooth headset drops or USB device unplugs cause the entire recording to fail instead of recovering gracefully.

8. **CPU waste from polling** — The current 10ms polling loop (`thread::sleep(10ms)` + `GetNextPacketSize()`) wastes CPU cycles and can miss audio data under heavy system load.

The user outcome we're solving for: **"I press record before a Zoom/Teams meeting and get a clean, well-balanced audio file I can transcribe or review — automatically, with no manual tweaking."**

---

## Solution

Implement 9 targeted improvements across 4 domain-aligned phases that transform `audio-rec` from a basic capture tool into a professional-quality meeting recorder. All quality features are **enabled by default** — users get the best possible recording with zero configuration. Power users can disable individual features with `--no-*` flags.

**Phase 1 (Capture Reliability):** Replace the polling-based WASAPI capture with event-driven capture, fix the silent loopback problem, and add graceful Ctrl+C shutdown. This fixes the foundation before building on it.

**Phase 2 (Post-Processing Pipeline):** Add EBU R128 loudness normalization and a soft audio limiter as FFmpeg post-processing steps. Quick wins that immediately improve output quality.

**Phase 3 (Real-Time Audio Processing):** Add RNNoise neural network noise suppression and a noise gate to the microphone capture pipeline. The biggest perceived quality jump for mic recordings.

**Phase 4 (Advanced Capture):** Add device timestamp tracking for sync accuracy, automatic device reconnection for resilience, and process-specific audio capture (the flagship feature — capture only Zoom/Teams audio, not all system sounds).

---

## User Stories

1. As a meeting recorder, I want the tool to use event-driven WASAPI capture instead of polling, so that audio is captured the instant it's available with no gaps or glitches, even under heavy CPU load during screen sharing.

2. As a meeting recorder, I want the loopback stream to stay alive during silent periods, so that system audio and microphone recordings stay perfectly in sync throughout the entire meeting.

3. As a user who accidentally presses Ctrl+C, I want the tool to gracefully stop recording, finalize WAV files, and run post-processing, so that I never lose a recording due to accidental termination.

4. As a meeting recorder, I want my recordings automatically loudness-normalized to EBU R128 standards, so that all speakers are at consistent volume levels without me having to manually adjust anything.

5. As a meeting recorder, I want a soft audio limiter applied automatically, so that sudden loud sounds (laughing, coughing, desk slamming) don't cause distortion in my recordings.

6. As a meeting recorder, I want real-time noise suppression on my microphone channel, so that keyboard typing, fan noise, and background chatter are removed from my recordings automatically.

7. As a meeting recorder, I want a noise gate on my microphone, so that low-level ambient noise between my speech segments is completely silenced.

8. As a power user, I want to disable any quality feature with `--no-normalize`, `--no-limiter`, `--no-denoise`, or `--no-noise-gate` flags, so that I can get raw unprocessed audio when I need it.

9. As a meeting recorder, I want the tool to track WASAPI device timestamps, so that loopback and microphone channels are perfectly aligned even in 1+ hour recordings.

10. As a user with Bluetooth headphones, I want the tool to automatically detect device disconnection and reconnect when the device becomes available again, so that a brief Bluetooth dropout doesn't ruin my entire recording.

11. As a user with Bluetooth headphones, I want silence written to the recording during device disconnection, so that audio sync is preserved even when my device temporarily drops.

12. As a meeting recorder, I want to capture audio only from a specific application (e.g., Zoom, Teams) using `--app <name>`, so that notification dings, Spotify music, and other system sounds don't pollute my meeting recording.

13. As a user on an older Windows version (pre-Build 20348), I want the tool to gracefully fall back to full system loopback when process-specific capture is unavailable, so that recording still works even without the advanced feature.

14. As a calling tool (e.g., AI assistant), I want to receive JSON events for device state changes (`device_state: disconnected/reconnected`) and interruption (`interrupted`), so that I can inform the user about recording issues in real-time.

15. As a meeting recorder, I want the tool to emit a `processing` JSON event for each post-processing stage (normalizing, limiting), so that the calling tool can show accurate progress to the user.

16. As a power user, I want to configure the noise gate threshold with `--gate-threshold <dB>`, so that I can tune the gate sensitivity for my specific environment.

17. As a power user, I want to configure the target loudness with `--target-loudness <LUFS>`, so that I can target different loudness standards (e.g., -16 LUFS for streaming, -23 LUFS for broadcast).

18. As a meeting recorder, I want the event-driven capture to use less CPU than the current polling approach, so that my system has more resources available for the video call itself.

19. As a developer integrating with audio-rec, I want the `started` JSON event to include which quality features are active (denoise, normalize, limiter, gate), so that I can display the recording configuration to the user.

20. As a meeting recorder using `--app`, I want to specify the target application by process name (e.g., `--app Zoom.exe`) or window title (e.g., `--app-window "Zoom Meeting"`), so that I have flexible ways to target the right application.

21. As a user, I want the tool to check Windows version compatibility for process-specific capture at startup and emit a clear warning if the feature is unavailable, so that I understand why `--app` might not work.

22. As a meeting recorder, I want all quality features to work together seamlessly (denoise → gate → limiter in real-time, then normalize + limiter in post-processing), so that I get the best possible output without feature conflicts.

23. As a user who records long meetings (2+ hours), I want device reconnection to retry at reasonable intervals (every 2 seconds) without consuming excessive resources, so that reconnection doesn't impact recording quality.

24. As a meeting recorder, I want the noise suppression to work regardless of my microphone's native sample rate and channel count, so that I don't need a specific microphone configuration.

---

## Implementation Decisions

### Architecture: Domain-Aligned Phases

The 9 features are organized into 4 phases by domain (not by priority), so each phase touches one subsystem at a time, reducing context-switching and merge conflicts:

- **Phase 1 (Capture Reliability):** WASAPI subsystem only — event-driven capture, ClearBuffer, graceful shutdown
- **Phase 2 (Post-Processing):** FFmpeg subsystem only — normalization, limiter
- **Phase 3 (Real-Time DSP):** New audio processing module — RNNoise, noise gate
- **Phase 4 (Advanced Capture):** WASAPI + sync — timestamps, reconnection, process-specific capture

### Event-Driven Capture (Phase 1)

- Replace `thread::sleep(10ms)` + `GetNextPacketSize()` polling with `AUDCLNT_STREAMFLAGS_EVENTCALLBACK` + `WaitForMultipleObjects`
- Stop mechanism: Win32 events via `WaitForMultipleObjects([audio_event, stop_event])` — keeps capture threads in the Win32 domain, no async runtime bridging needed
- The stop event is signaled from the main thread via `SetEvent()` when a stop signal is received
- This pattern naturally extends to Phase 4 (add a third event for device change notifications)
- Both loopback and microphone capture threads get the same upgrade

### Silent Loopback Fix (Phase 1)

- Create a secondary `IAudioClient` on the same output device in normal (non-loopback) render mode
- Write a buffer of zeros (silence) to keep the audio stream active
- The render client runs alongside the capture client and is cleaned up when recording stops
- Cleanup order: stop capture client first, then stop and release the render client

### Graceful Shutdown (Phase 1)

- Use `ctrlc` crate or `tokio::signal::ctrl_c()` to intercept Ctrl+C/SIGTERM
- On signal: set stop events on both capture threads, wait for threads to finish, run full post-processing pipeline
- Emit `{"type": "interrupted", ...}` JSON event before starting shutdown
- Ensure temp files are cleaned up even on abnormal exit (register cleanup handler)

### Loudness Normalization (Phase 2)

- **Two-pass for WAV output:** Pass 1 analyzes with `loudnorm` filter and outputs JSON measurements. Pass 2 applies correction with `linear=true` for transparent normalization.
- **Single-pass for M4A output:** Since M4A already re-encodes, the quality difference from single-pass is negligible. Simpler and faster.
- Default target: -16 LUFS (streaming standard), configurable via `--target-loudness`
- True peak limit: -1.5 dBTP
- Loudness range: 11 LU
- Integrated into post-processing pipeline after merge, before silence trimming
- Pipeline order: Merge → Normalize → Trim → Finalize

### Audio Limiter (Phase 2)

- Applied via FFmpeg filter: `-af alimiter=limit=0.95:attack=0.001:release=50`
- Replaces the current hard clipping pattern (`sample.clamp(-1.0, 1.0)`) in capture threads
- Also available as real-time processing in the audio processing pipeline (Phase 3)
- Fast attack (1ms) to catch transients, configurable release

### RNNoise Noise Suppression (Phase 3)

- Primary: Real-time processing in the microphone capture thread, before writing to WAV
- Secondary: FFmpeg-based post-processing fallback for reprocessing existing recordings
- Uses `nnnoiseless` crate (pure Rust, no C dependencies)
- RNNoise requires 48kHz mono float32 in 480-sample frames
- The noise suppression module handles stereo downmix/upmix internally — the capture thread passes raw samples in, gets clean samples out
- If mic sample rate ≠ 48kHz, the module handles resampling internally
- VAD (voice activity detection) probability output can be used by the noise gate for smarter gating

### Noise Gate (Phase 3)

- State machine with 6 states: CLOSED → OPENING → OPEN → HOLDING → CLOSING → CLOSED
- Parameters: open threshold (-32 dB default), close threshold, attack time, hold time, release time
- Processes samples using RMS level calculation (reuses existing `audio_utils.rs` functions)
- Applied before RNNoise in the pipeline (gate handles silence, RNNoise handles noise during speech)
- Configurable threshold via `--gate-threshold <dB>`

### Audio Processing Pipeline (Phase 3)

- Chain: samples → noise gate → RNNoise → limiter → output
- Each stage is independently toggleable via CLI flags
- The pipeline is injected into the microphone capture thread
- Pipeline configuration is determined at startup from CLI flags and passed to the capture thread

### Timestamp Synchronization (Phase 4)

- Capture `pu64DevicePosition` and `pu64QPCPosition` from `GetBuffer()` (currently passed as `None`)
- Track QPC (QueryPerformanceCounter) timestamps for each buffer in both streams
- Calculate the offset between loopback and mic stream start times
- Apply offset during merge via FFmpeg's `-itsoffset` parameter
- Handle `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` flag — fall back to frame-count-based timing

### Device Reconnection (Phase 4)

- Implement `IMMNotificationClient` COM interface for device change notifications
- Register via `IMMDeviceEnumerator::RegisterEndpointNotificationCallback`
- Handle `OnDefaultDeviceChanged` and `OnDeviceStateChanged` events
- When `GetBuffer` returns `AUDCLNT_E_DEVICE_INVALIDATED`, enter reconnection mode
- During disconnection: write silence to WAV to maintain timing/sync
- Retry device initialization every 2 seconds
- Resume capture when device becomes available again
- Emit JSON events for device state changes

### Process-Specific Capture (Phase 4)

- Use `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`
- Blocking COM wrapper: implement `IActivateAudioInterfaceCompletionHandler` that signals a Win32 event on completion, then `WaitForSingleObject` to make it synchronous
- Process detection: use `FindWindowW` + `GetWindowThreadProcessId` (NOT `tasklist` — per `claude.md` antivirus rules)
- Set `ProcessLoopbackMode` to `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture child processes
- Requires Windows 10 Build 20348+ — check version at startup, fall back to full loopback with warning if unavailable
- CLI: `--app <name>` for process name, `--app-window <title>` for window title

### Default Behavior

- **All quality features ON by default** — `audio-rec record` gives maximum quality with zero configuration
- Disable flags: `--no-normalize`, `--no-limiter`, `--no-denoise`, `--no-noise-gate`
- Process-specific capture (`--app`) is always opt-in since it requires a target
- This follows the existing pattern where silence trimming is ON by default with `--no-trim` to disable

### New CLI Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--no-normalize` | Disable loudness normalization | Normalization ON |
| `--no-limiter` | Disable soft audio limiter | Limiter ON |
| `--no-denoise` | Disable RNNoise noise suppression | Denoise ON |
| `--no-noise-gate` | Disable noise gate | Gate ON |
| `--target-loudness <LUFS>` | Target loudness level | -16 LUFS |
| `--gate-threshold <dB>` | Noise gate open threshold | -32 dB |
| `--app <name>` | Capture audio from specific process | Off (full loopback) |
| `--app-window <title>` | Capture audio from specific window | Off (full loopback) |

### New JSON Events

```json
{"type": "interrupted", "session_id": "...", "message": "Recording interrupted by signal"}
{"type": "device_state", "session_id": "...", "device": "microphone", "state": "disconnected"}
{"type": "device_state", "session_id": "...", "device": "microphone", "state": "reconnected"}
{"type": "processing", "session_id": "...", "message": "Normalizing loudness", "stage": "normalizing"}
{"type": "processing", "session_id": "...", "message": "Applying limiter", "stage": "limiting"}
```

### Extended `started` Event

The `started` JSON event will include active quality features:
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

### Error Type Expansion

New `RecorderError` variants:
- `DeviceDisconnected` — audio device was disconnected mid-recording
- `DeviceReconnected` — audio device was reconnected after disconnection
- `ProcessNotFound` — target process for `--app` was not found
- `UnsupportedWindowsVersion` — Windows version doesn't support process-specific capture

### New Dependencies

- `nnnoiseless = "0.5"` — Pure Rust RNNoise port for noise suppression
- `ctrlc = "3"` — Cross-platform Ctrl+C handler (or use `tokio::signal::ctrl_c()`)

---

## Testing Decisions

### Testing Philosophy

Tests should verify **external behavior** (inputs → outputs), not implementation details. A good test for the audio processing module feeds in a known audio signal and asserts the output has the expected characteristics (e.g., silence where noise was, consistent loudness levels). Tests should NOT assert on internal state machine transitions or intermediate buffer contents.

### Modules to Test

1. **Audio Processing Pipeline** (`audio_processing/`)
   - `noise_gate.rs`: Feed synthetic audio (alternating speech-level and silence-level samples), verify gate opens/closes at correct thresholds, verify attack/release ramps are smooth
   - `limiter.rs`: Feed samples above threshold, verify output is limited without hard clipping, verify samples below threshold pass through unchanged
   - `noise_suppress.rs`: Feed noisy synthetic audio, verify output RMS is lower than input RMS for noise-only segments, verify speech segments are preserved
   - `pipeline.rs`: Feed audio through full chain, verify each stage can be independently disabled

2. **Normalization Pipeline** (`ffmpeg_encoder.rs` extensions)
   - Create fixture WAV files with known loudness levels
   - Verify two-pass normalization produces output within ±1 LU of target
   - Verify single-pass normalization produces output within ±2 LU of target
   - Verify normalization handles edge cases (silent file, already-normalized file)

3. **Noise Gate State Machine** (`noise_gate.rs`)
   - Verify all 6 state transitions with deterministic inputs
   - Verify hold time prevents rapid gate cycling (chattering)
   - Verify attack/release ramps produce smooth gain changes (no clicks)

### Prior Art

Existing tests in the codebase follow the same pattern:
- `audio_utils.rs` tests: Feed known samples, assert RMS output matches expected values
- `cli.rs` tests: Construct argument vectors, assert parsed values match expectations

New tests will follow this same pattern — synthetic inputs, deterministic expected outputs.

### What We Don't Test

- WASAPI event-driven capture (requires real audio hardware)
- Process-specific capture (requires running target process on Windows 20348+)
- Device reconnection (requires physical device changes)
- FFmpeg availability (environment-dependent)

These are verified through manual integration testing during development.

---

## Out of Scope

| Item | Rationale |
|------|-----------|
| Dynamic Range Compression (#9) | Tier 3, overlaps with normalization. Deferred to TODOS.md. |
| 3-Band Voice EQ (#10) | Tier 3, subjective quality improvement. Deferred to TODOS.md. |
| RTWQ Low-Latency Capture (#12) | Tier 3, high complexity, marginal benefit over event-driven. Deferred to TODOS.md. |
| Pre-Recording Buffer (#13) | Tier 3, requires always-on background capture. Deferred to TODOS.md. |
| Sidechain Ducking (#15) | Tier 3, requires cross-channel real-time processing. Deferred to TODOS.md. |
| Cross-platform support (Linux/macOS) | Out of scope for a quality-focused plan. |
| Real-time audio level monitoring (VU meters) | Listed in README future plans but not part of quality improvements. |
| Configurable silence parameters | Listed in README future plans but orthogonal to this work. |
| Pause/resume | Listed in README future plans but orthogonal to this work. |

---

## Further Notes

### Risk Mitigation

- **Phase 1 is the riskiest** — it changes the fundamental capture loop. If event-driven capture introduces regressions, the polling fallback should be preserved behind a `--legacy-capture` flag during development.
- **Process-specific capture (Phase 4)** has the most unknowns — COM async activation in Rust is not well-documented. Budget extra time for this item.
- **RNNoise CPU impact** — The `nnnoiseless` crate processes 480 samples at a time. At 48kHz, that's 100 frames/second. Benchmark on low-end hardware to ensure it doesn't cause capture thread starvation.

### Migration Path

- Existing users who run `audio-rec record` will automatically get all quality improvements — no flag changes needed.
- Users who want the old behavior can use: `audio-rec record --no-denoise --no-normalize --no-limiter --no-noise-gate`
- The JSON protocol is backward-compatible — new event types are additive, existing events retain their schema.

### Performance Expectations

- Event-driven capture should reduce CPU usage by ~50% compared to polling
- RNNoise adds ~2-5% CPU per microphone channel (negligible on modern hardware)
- Two-pass normalization adds ~2x processing time for WAV files (acceptable for post-processing)
- Process-specific capture has no performance difference from full loopback once initialized

### Completion Criteria

After all 4 phases are complete, the system achieves **~75% of the 12-month ideal** with **~50% of the total effort**. The remaining 25% (deferred Tier 3 items) can ship incrementally as independent improvements.

---

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 1 | CLEAR | Approach B selected, 5 items deferred |
| Codex Review | `/codex review` | Independent 2nd opinion | 0 | — | — |
| Eng Review | `/plan-eng-review` | Architecture & tests (required) | 1 | CLEAR (PLAN) | 11 issues, 4 critical gaps, 0 unresolved |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | — | N/A (CLI tool) |

- **UNRESOLVED:** 0 decisions across all reviews
- **VERDICT:** CEO + ENG CLEARED — ready to implement

### Eng Review Key Changes to PRD
1. Pipeline order changed: `RNNoise → Gate (VAD-driven) → Limiter` (was `Gate → RNNoise → Limiter`)
2. Pre-requisite refactors: `capture_engine.rs` extraction + `PostProcessingPipeline` struct
3. Thread join fix: Store `JoinHandle`, proper join on stop (removes 1s sleep hack)
4. Write error handling: Track and report sample write failures
5. Windows crate features: `Win32_System_Threading`, `Win32_UI_WindowsAndMessaging`, `Win32_System_SystemInformation`, `implement`
6. Resampling: Linear interpolation for non-48kHz mics
7. CPU budget: Benchmark + runtime warning for RNNoise
8. 4 critical failure modes identified (ClearBuffer init, thread join timeout, RNNoise panic, target process exit)
9. Full test plan: ~43 tests across 10 files
10. TODOS.md created with 7 deferred items

**Full review:** `plans/recording-quality-eng-review.md`
