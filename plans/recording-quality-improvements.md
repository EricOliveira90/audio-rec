# Recording Quality Improvements — Implementation Plan

> **Date:** 2026-04-03
> **Status:** Proposed
> **Research Sources:** OBS Studio (`obsproject/obs-studio`), ProcTap (`m96-chan/ProcTap`), ffmpeg-normalize (`peterforgacs/ffmpeg-normalize`), nnnoiseless/rnnoise-c (Rust RNNoise), OBS audio filters

## Overview

This document proposes 15 improvements to `audio-rec` for dramatically better meeting recording quality (system audio + mic on Zoom/Teams). Each improvement was identified by studying how OBS Studio and other open-source projects solve the same audio capture and processing challenges on Windows.

The improvements are organized into three tiers by quality impact, with detailed technical descriptions and implementation actions for each.

---

## Tier 1 — High Impact (Transformative)

### 1. Event-Driven WASAPI Capture

**Overall Description:**
Replace the current 10ms polling loop (`thread::sleep(10ms)` + `GetNextPacketSize()`) with event-driven capture using `AUDCLNT_STREAMFLAGS_EVENTCALLBACK`. Instead of constantly asking WASAPI "is there data yet?", the OS signals a Win32 event when audio data is ready.

**Benefits for Project Quality:**
- Eliminates potential audio gaps/glitches caused by polling timing mismatches
- Reduces CPU usage by ~50% (no busy-waiting)
- Lower capture latency — audio is processed the instant it's available
- More reliable under heavy system load (e.g., during screen sharing + video calls)

**Technical Description & Actions:**

1. **Modify `WasapiLoopbackRecorder::recording_thread()`** in `core/src/wasapi/loopback.rs`:
   - After `audio_client.Initialize()`, call `audio_client.SetEventHandle(event_handle)` with a Win32 event created via `CreateEventW`
   - Replace the `loop { sleep(10ms); GetNextPacketSize(); ... }` pattern with `WaitForSingleObject(event_handle, timeout)` followed by a drain loop that processes all available packets
   - Add a stop event and use `WaitForMultipleObjects` to wait on both the audio event and the stop signal

2. **Modify `WasapiMicrophoneRecorder::recording_thread()`** in `core/src/wasapi/microphone.rs`:
   - Same changes as loopback — note that microphone capture does NOT use `AUDCLNT_STREAMFLAGS_LOOPBACK`, so the event callback flag is added independently
   - For microphone, the initialization flags change from `0` to `AUDCLNT_STREAMFLAGS_EVENTCALLBACK`

3. **Update stop mechanism:**
   - Replace `is_recording.store(false)` with signaling a stop event
   - Use `WaitForMultipleObjects([audio_event, stop_event])` to handle both audio data and stop requests

**Reference:** OBS Studio `plugins/win-wasapi/win-wasapi.cpp` lines in `InitClient()`:
```cpp
DWORD flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
if (type != SourceType::Input)
    flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
client->Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_TIME_100NS, 0, pFormat, nullptr);
```

---

### 2. Process-Specific Audio Capture (Application Loopback)

**Overall Description:**
Instead of capturing ALL system audio (every app playing sound), capture audio only from a specific process tree — e.g., only Zoom or only Teams. This uses the Windows `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` API available on Windows 10 Build 20348+.

**Benefits for Project Quality:**
- **Dramatically cleaner recordings** — no notification dings, no Spotify music, no system sounds
- Only the meeting audio is captured, which is exactly what users want
- Eliminates the need for users to manually mute other apps before recording
- This is the single biggest quality improvement for meeting recording use cases

**Technical Description & Actions:**

1. **Add process detection module** (`core/src/wasapi/process_loopback.rs`):
   - Accept a process name (e.g., "Zoom.exe", "ms-teams.exe") or window title
   - Use `windows` crate APIs to find the process ID
   - Note: This must NOT use `tasklist` or process enumeration commands (per `claude.md` antivirus rules). Instead, use `FindWindowW` + `GetWindowThreadProcessId` to find the PID from a window title/class

2. **Implement `ActivateAudioInterfaceAsync` activation:**
   - Dynamically load `ActivateAudioInterfaceAsync` from `Mmdevapi.dll` (as OBS does)
   - Create `AUDIOCLIENT_ACTIVATION_PARAMS` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` and the target process ID
   - Set `ProcessLoopbackMode` to `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture the process and all its child processes
   - Implement `IActivateAudioInterfaceCompletionHandler` COM interface for the async activation callback

3. **Add CLI flags:**
   - `--app <name>` or `--process <name>` — capture audio from a specific application
   - `--app-window <title>` — capture audio from a window with a specific title
   - Fall back to full system loopback if the API is unavailable (older Windows) or if no app is specified

4. **Feature detection:**
   - Check Windows version (requires Build 20348+)
   - If unavailable, log a warning and fall back to standard loopback capture

**Reference:** OBS Studio `plugins/win-wasapi/win-wasapi.cpp` `InitClient()`:
```cpp
AUDIOCLIENT_ACTIVATION_PARAMS audioclientActivationParams;
audioclientActivationParams.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
audioclientActivationParams.ProcessLoopbackParams.TargetProcessId = process_id;
audioclientActivationParams.ProcessLoopbackParams.ProcessLoopbackMode =
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE;
```

Also see: `m96-chan/ProcTap` (Python library for process-specific capture), `TheEmeraldDerpLeader/WindowsLoopbackAudioMixerProofOfConcept`.

---

### 3. Silent Loopback Fix (ClearBuffer Technique)

**Overall Description:**
When system audio goes completely silent (common in meetings — e.g., someone presenting slides without talking), WASAPI loopback can stop delivering audio packets entirely. This causes the loopback recording to be shorter than the mic recording, leading to sync drift when merging. OBS solves this by playing a silent audio buffer to keep the loopback stream alive.

**Benefits for Project Quality:**
- Prevents audio sync drift between system audio and mic channels during silent periods
- Eliminates the root cause of the "audio merge duration mismatch" problem (your code already handles this with `apad`, but this prevents it entirely)
- More reliable packet delivery timing

**Technical Description & Actions:**

1. **Add `ClearBuffer` function** to `core/src/wasapi/loopback.rs`:
   - After initializing the loopback capture client, create a SECOND `IAudioClient` on the same device
   - Initialize this second client in normal (non-loopback) mode
   - Get an `IAudioRenderClient` from it
   - Write a buffer of zeros (silence) to keep the audio stream active
   - This secondary client runs alongside the capture client

2. **Implementation in recording thread:**
   ```
   // After capture client setup:
   let render_client = device.Activate::<IAudioClient>(CLSCTX_ALL, None)?;
   render_client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, BUFFER_TIME, 0, mix_format, None)?;
   let buffer_size = render_client.GetBufferSize()?;
   let renderer: IAudioRenderClient = render_client.GetService()?;
   let buffer = renderer.GetBuffer(buffer_size)?;
   // Zero-fill the buffer (silence)
   std::ptr::write_bytes(buffer, 0, buffer_size as usize * block_align);
   renderer.ReleaseBuffer(buffer_size, 0)?;
   ```

3. **Cleanup:** Stop and release the render client when recording stops.

**Reference:** OBS Studio `win-wasapi.cpp` `ClearBuffer()`:
```cpp
/* Silent loopback fix. Prevents audio stream from stopping and */
/* messing up timestamps and other weird glitches during silence */
/* by playing a silent sample all over again. */
```

---

### 4. Real-Time Noise Suppression (RNNoise)

**Overall Description:**
Apply neural network-based noise suppression to the microphone channel in real-time during recording. RNNoise is a lightweight ML model specifically designed for voice denoising, used by OBS Studio, Discord, and many other applications.

**Benefits for Project Quality:**
- Removes keyboard typing, fan noise, AC hum, and background chatter from mic recordings
- Massive quality improvement for meeting recordings in noisy home offices
- The `nnnoiseless` crate is a pure Rust port — no external C dependencies needed
- RNNoise operates at 48kHz with 480-sample frames, perfectly matching the project's professional quality preset

**Technical Description & Actions:**

1. **Add dependency:** `nnnoiseless = "0.5"` to `core/Cargo.toml`
   - Alternative: `rnnoise-c` crate for C bindings (slightly better performance, but requires C toolchain)

2. **Create noise suppression module** (`core/src/audio_processing/noise_suppress.rs`):
   - Initialize `DenoiseState` from nnnoiseless
   - RNNoise requires 48kHz mono float32 input in 480-sample frames
   - If mic is stereo, downmix to mono for processing, then upmix back
   - If mic sample rate ≠ 48kHz, resample before/after processing

3. **Integrate into microphone recording loop** in `core/src/wasapi/microphone.rs`:
   - After reading samples from WASAPI buffer and before writing to WAV
   - Accumulate samples into 480-frame chunks
   - Process each chunk through `DenoiseState::process_frame()`
   - Write denoised samples to WAV

4. **Add CLI flag:** `--denoise` or `--noise-suppress` to enable (off by default to avoid unexpected behavior changes)

5. **Optional: Configurable suppression level** — RNNoise outputs a VAD (voice activity detection) probability per frame. Use this to implement a configurable threshold.

**Reference:** OBS Studio `plugins/obs-filters/noise-suppress-filter.c`:
```c
#define RNNOISE_SAMPLE_RATE 48000
#define RNNOISE_FRAME_SIZE 480
```

Also: `nnnoiseless` crate (pure Rust), `qaziwaqar/kwite` (Rust RNNoise app).

---

## Tier 2 — Medium Impact (Significant improvements)

### 5. Audio Loudness Normalization (EBU R128)

**Overall Description:**
Apply EBU R128 loudness normalization as a post-processing step using FFmpeg's `loudnorm` filter. This ensures consistent volume levels across different speakers and between system audio and mic channels.

**Benefits for Project Quality:**
- Solves the common meeting problem where one person is too quiet and another too loud
- Makes recordings much more pleasant to listen to and easier to transcribe
- Industry-standard loudness targeting (EBU R128 / -16 LUFS for streaming, -23 LUFS for broadcast)
- Already listed in the project's README as a future plan

**Technical Description & Actions:**

1. **Add loudness normalization to `core/src/ffmpeg_encoder.rs`:**
   - Implement a two-pass normalization function:
     - **Pass 1 (analysis):** `ffmpeg -i input.wav -af loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json -f null -`
     - **Pass 2 (apply):** Use the measured values from pass 1 in a second `loudnorm` filter invocation with `linear=true` for high-quality linear normalization
   - Single-pass alternative (simpler, slightly lower quality): `ffmpeg -i input.wav -af loudnorm=I=-16:TP=-1.5:LRA=11 output.wav`

2. **Integrate into post-processing pipeline** in `core/src/commands/record.rs`:
   - Add normalization step after merge and before/after silence trimming
   - Order: Merge → Normalize → Trim → Finalize

3. **Add CLI flags:**
   - `--normalize` — enable loudness normalization (could be on by default)
   - `--target-loudness <LUFS>` — target loudness level (default: -16 LUFS)
   - `--no-normalize` — explicitly disable

4. **Progress reporting:** Reuse existing FFmpeg progress monitoring infrastructure.

**Reference:** `peterforgacs/ffmpeg-normalize`, `ffmpeg-loudnorm-helper`.

---

### 6. Automatic Device Reconnection

**Overall Description:**
When an audio device disconnects mid-recording (Bluetooth headset drops, USB device unplugged), automatically detect the disconnection and reconnect when the device becomes available again, instead of failing the entire recording.

**Benefits for Project Quality:**
- Prevents lost recordings from Bluetooth headset reconnections (very common in meetings)
- Handles Windows default device changes gracefully
- More robust long-running recordings

**Technical Description & Actions:**

1. **Implement `IMMNotificationClient`** in `core/src/wasapi/device_notification.rs`:
   - Register for device change notifications via `IMMDeviceEnumerator::RegisterEndpointNotificationCallback`
   - Handle `OnDefaultDeviceChanged` — when the default device changes, restart capture on the new device
   - Handle `OnDeviceStateChanged` — detect device removal and addition

2. **Add reconnection logic to recording threads:**
   - When `GetBuffer` or `GetNextPacketSize` returns `AUDCLNT_E_DEVICE_INVALIDATED`, enter reconnection mode
   - Write silence to the WAV file during disconnection to maintain timing
   - Retry device initialization at intervals (e.g., every 2 seconds)
   - Resume capture when device is available again

3. **Emit JSON events** for device state changes:
   ```json
   {"type": "device_state", "session_id": "...", "device": "microphone", "state": "disconnected"}
   {"type": "device_state", "session_id": "...", "device": "microphone", "state": "reconnected"}
   ```

**Reference:** OBS Studio `win-wasapi.cpp` — `ReconnectThread`, `SetDefaultDevice()`, `wasapi-notify.hpp`.

---

### 7. Noise Gate for Microphone

**Overall Description:**
Apply a noise gate that silences the mic channel when audio falls below a configurable threshold. Unlike noise suppression (which removes noise from speech), a noise gate completely mutes the channel during non-speech periods.

**Benefits for Project Quality:**
- Eliminates low-level ambient noise between speech segments
- Much lighter than RNNoise — pure math, no ML model
- Can be combined with RNNoise for best results (gate handles silence, RNNoise handles noise during speech)

**Technical Description & Actions:**

1. **Create noise gate module** (`core/src/audio_processing/noise_gate.rs`):
   - Parameters: `open_threshold` (dB), `close_threshold` (dB), `attack_time` (ms), `hold_time` (ms), `release_time` (ms)
   - State machine: CLOSED → OPENING → OPEN → HOLDING → CLOSING → CLOSED
   - Calculate per-sample attenuation based on state and envelope

2. **Algorithm (from OBS):**
   ```
   level = calculate_rms(samples)
   if level > open_threshold → transition to OPEN (with attack ramp)
   if level < close_threshold → start HOLD timer
   if hold_time expires → transition to CLOSING (with release ramp)
   Apply attenuation to each sample
   ```

3. **Integrate into mic recording loop** in `core/src/wasapi/microphone.rs`

4. **Add CLI flags:** `--noise-gate`, `--gate-threshold <dB>` (default: -32 dB)

**Reference:** OBS Studio `plugins/obs-filters/noise-gate-filter.c`.

---

### 8. Proper Audio Synchronization with Device Timestamps

**Overall Description:**
Track WASAPI device timestamps and system clock timestamps to ensure perfect sync between loopback and mic channels, especially in long recordings.

**Benefits for Project Quality:**
- Prevents lip-sync drift in 1+ hour meeting recordings
- Detects and handles bad timestamps (`AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR`)
- More accurate merge alignment

**Technical Description & Actions:**

1. **Capture timestamps in recording loops:**
   - Use the `pu64DevicePosition` and `pu64QPCPosition` parameters of `GetBuffer()` (currently passed as `None`)
   - Track the QPC (QueryPerformanceCounter) timestamp of each buffer

2. **Implement timestamp-based alignment:**
   - Record the start timestamp of both loopback and mic streams
   - Calculate the offset between them
   - During merge, apply the offset as a delay to the stream that started later

3. **Handle timestamp errors:**
   - Check for `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` flag
   - Fall back to frame-count-based timing when timestamps are unreliable

4. **Pass timing metadata to FFmpeg merge:**
   - Use `-itsoffset` parameter to apply the calculated offset during merge

**Reference:** OBS Studio `win-wasapi.cpp` — `useDeviceTiming`, `sawBadTimestamp`, timestamp calculation.

---

## Tier 3 — Lower Impact (Nice-to-have)

### 9. Dynamic Range Compression

**Overall Description:**
Apply audio compression to reduce the dynamic range — making quiet speech louder and preventing loud sounds from overwhelming.

**Benefits:** More consistent volume levels, especially with multiple speakers at different volumes.

**Technical Actions:**
1. Create `core/src/audio_processing/compressor.rs` with envelope follower + gain reduction
2. Parameters: ratio (4:1 default), threshold (-20dB), attack (5ms), release (100ms), output gain
3. Apply as post-processing FFmpeg filter: `-af acompressor=threshold=-20dB:ratio=4:attack=5:release=100`
4. CLI flag: `--compress`

**Reference:** OBS Studio `plugins/obs-filters/compressor-filter.c`.

---

### 10. 3-Band Voice Equalizer

**Overall Description:**
Apply a 3-band EQ optimized for voice clarity: cut lows (reduce rumble), boost mids (voice presence), gently cut highs (reduce sibilance).

**Benefits:** Clearer, more intelligible voice recordings on lower-quality microphones.

**Technical Actions:**
1. Apply via FFmpeg post-processing: `-af equalizer=f=200:t=h:w=200:g=-3,equalizer=f=2500:t=h:w=1000:g=2,equalizer=f=8000:t=h:w=2000:g=-2`
2. Or implement biquad filters in Rust for real-time processing (from OBS's `eq-filter.c` — crossover at 800Hz and 5000Hz)
3. CLI flag: `--voice-enhance`

**Reference:** OBS Studio `plugins/obs-filters/eq-filter.c`.

---

### 11. Audio Limiter (Soft Clipping Prevention)

**Overall Description:**
Replace hard clipping (`sample.clamp(-1.0, 1.0)`) with a soft limiter that smoothly reduces gain when audio approaches the threshold.

**Benefits:** Prevents distortion from sudden loud sounds (laughing, coughing, desk slamming).

**Technical Actions:**
1. Create `core/src/audio_processing/limiter.rs` with fast attack (1ms) and configurable release
2. Apply in the recording loop before sample conversion to i16
3. Or apply via FFmpeg: `-af alimiter=limit=0.95:attack=0.001:release=50`
4. CLI flag: `--limiter`

**Reference:** OBS Studio `plugins/obs-filters/limiter-filter.c`.

---

### 12. RTWQ Low-Latency Capture

**Overall Description:**
Use Windows Real-Time Work Queue (`RTWorkQ.dll`) for OS-level priority scheduling of audio capture callbacks.

**Benefits:** More reliable capture under heavy CPU load during video calls.

**Technical Actions:**
1. Dynamically load `RTWorkQ.dll` (available on Windows 10 1703+)
2. Use `RtwqLockSharedWorkQueue("Capture", ...)` for priority scheduling
3. Replace thread-based capture with async callbacks via `RtwqPutWaitingWorkItem`
4. Fall back to standard threads if RTWQ is unavailable

**Reference:** OBS Studio `win-wasapi.cpp` — RTWQ initialization and callback infrastructure.

---

### 13. Pre-Recording Buffer

**Overall Description:**
Maintain a rolling circular buffer (5-10 seconds) that continuously captures audio, so when recording starts, the buffer contents are prepended.

**Benefits:** Never miss the first few seconds of a meeting.

**Technical Actions:**
1. Create a `RingBuffer<f32>` that holds N seconds of audio
2. Run WASAPI capture continuously in the background
3. When `record` command is received, flush the ring buffer to the WAV file first, then continue with live capture
4. CLI flag: `--pre-buffer <seconds>` (default: 5)

---

### 14. Graceful Shutdown / Signal Handling

**Overall Description:**
Handle SIGTERM/Ctrl+C to properly stop WASAPI, finalize WAV files, and run post-processing before exiting.

**Benefits:** Prevents data loss from accidental termination. Already listed in README as a future plan.

**Technical Actions:**
1. Add `ctrlc` crate or use `tokio::signal::ctrl_c()`
2. On signal: set `is_recording` to false, wait for threads to finish, run post-processing
3. Emit `{"type": "interrupted", ...}` JSON event
4. Ensure temp files are cleaned up even on abnormal exit

---

### 15. Sidechain Ducking

**Overall Description:**
When the mic detects speech, automatically reduce the system audio level in the recording. This mimics what Zoom/Teams do internally.

**Benefits:** Better voice clarity when both system audio and mic are active simultaneously.

**Technical Actions:**
1. Detect mic activity using RMS level (already implemented in `audio_utils.rs`)
2. When mic RMS exceeds threshold, apply gain reduction to loopback samples
3. Use smooth attack/release envelope to avoid pumping artifacts
4. CLI flag: `--auto-duck`, `--duck-amount <dB>` (default: -6dB)

**Reference:** OBS Studio `plugins/obs-filters/compressor-filter.c` — sidechain source support.

---

## Recommended Implementation Order

For maximum quality improvement with reasonable effort:

| Phase | Items | Rationale |
|-------|-------|-----------|
| **Phase 1** | #1 (Event-Driven Capture), #3 (Silent Loopback Fix), #14 (Graceful Shutdown) | Foundation fixes — make capture more reliable before adding features |
| **Phase 2** | #5 (Loudness Normalization), #11 (Limiter) | Quick wins via FFmpeg — low effort, immediate quality improvement |
| **Phase 3** | #4 (RNNoise Noise Suppression), #7 (Noise Gate) | Real-time audio processing — biggest perceived quality jump |
| **Phase 4** | #8 (Timestamp Sync), #6 (Device Reconnection) | Reliability for long recordings |
| **Phase 5** | #2 (Process-Specific Capture) | Transformative feature but highest complexity |
| **Phase 6** | #9 (Compressor), #10 (Voice EQ), #15 (Sidechain Ducking) | Polish and professional-grade features |
| **Phase 7** | #12 (RTWQ), #13 (Pre-Recording Buffer) | Advanced optimizations |

---

## Impact Summary

| Rank | Improvement | Impact | Complexity | Phase |
|------|------------|--------|------------|-------|
| 1 | Event-Driven WASAPI Capture | 🔴 High | Medium | 1 |
| 2 | Process-Specific Audio Capture | 🔴 High | High | 5 |
| 3 | Silent Loopback Fix (ClearBuffer) | 🔴 High | Medium | 1 |
| 4 | Real-Time Noise Suppression (RNNoise) | 🔴 High | Medium | 3 |
| 5 | Audio Loudness Normalization | 🟡 Medium | Low | 2 |
| 6 | Automatic Device Reconnection | 🟡 Medium | Medium-High | 4 |
| 7 | Noise Gate for Microphone | 🟡 Medium | Low-Medium | 3 |
| 8 | Proper Audio Sync with Timestamps | 🟡 Medium | Medium | 4 |
| 9 | Dynamic Range Compression | 🟢 Lower | Medium | 6 |
| 10 | 3-Band Voice EQ | 🟢 Lower | Low-Medium | 6 |
| 11 | Audio Limiter (Soft Clipping) | 🟢 Lower | Low | 2 |
| 12 | RTWQ Low-Latency Capture | 🟢 Lower | High | 7 |
| 13 | Pre-Recording Buffer | 🟢 Lower | Medium | 7 |
| 14 | Graceful Shutdown | 🟢 Lower | Low | 1 |
| 15 | Sidechain Ducking | 🟢 Lower | Medium-High | 6 |
