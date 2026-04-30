# Plan: Process-Specific Audio Capture

> Source PRD: `plans/process-specific-capture-prd.md` / GitHub Issue #25
> Engineering Review: `plans/process-specific-capture-eng-review.md`

## Architectural decisions

Durable decisions that apply across all phases:

- **CLI flags**: `--app <process_name>` (exact match, case-sensitive) and `--app-window <window_title>` (case-insensitive substring match). Mutually exclusive — CLI validation rejects both being specified simultaneously. Both are opt-in (not default). Compatible with all existing flags (`--duration`, `--format`, `--mic-muted`, `--no-trim`, etc.).
- **Process detection method**: `EnumWindows` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` — NOT `tasklist` or `CreateToolhelp32Snapshot` (per `claude.md` antivirus rules). Only finds processes with visible top-level windows.
- **Process tree capture**: `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture audio from the target process and all its child processes.
- **COM activation pattern**: `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`. Blocking wrapper via `IActivateAudioInterfaceCompletionHandler` COM callback + Win32 event + `WaitForSingleObject`.
- **Minimum Windows version**: Build 20348 (Windows 10 21H2 / Windows Server 2022). Checked via `RtlGetVersion`.
- **LoopbackCapture trait**: Runtime polymorphism via `Box<dyn LoopbackCapture>` in `record.rs`. Both `WasapiLoopbackRecorder` and `ProcessLoopbackRecorder` implement it. Trait methods (`stop`, `get_frames_captured`, `has_audio_detected`, `get_sample_rate`, `get_start_time`) are called once per second in the status loop, not in the audio hot path — zero performance concern.
- **Module structure**: Two new modules — `wasapi/process_resolver.rs` (pure logic: PID resolution + version check) and `wasapi/process_loopback.rs` (WASAPI COM code: process-specific recorder).
- **Microphone independence**: `--app` only affects the loopback (system audio) channel. Microphone capture is completely unchanged.
- **`--output-device` behavior**: Ignored when `--app` is used (process loopback doesn't use a device endpoint). JSON `warning` event emitted if both are specified.
- **JSON protocol additions**: New `warning` event type with `code` field for programmatic handling. New `capture_mode` field (`"process"` or `"system"`) on `started` and `audio_state` events. New `app`/`app_window` fields on `started` event and status output.
- **Fallback behavior**: Process not found at startup → hard fail (`ProcessNotFound`). Windows version too old → fall back to system loopback + JSON warning. Process exits mid-recording → fall back to system loopback + JSON warning. Fallback recorder creation fails → continue mic-only + JSON error.
- **Error messages**: Actionable — include the process name, what went wrong, and a suggested next step (e.g., "Run without --app to record all system audio").
- **New `windows` crate features**: `Win32_UI_WindowsAndMessaging`, `Win32_System_Threading`, `Win32_System_SystemInformation`, plus `implement` feature for COM callback.

---

## Phase 1: Process Resolver + CLI Validation + Version Check

**User stories**: #3, #4, #5, #6, #9, #12, #17

### What to build

Build the process resolver module — the pure-logic foundation that everything else depends on. Given a process name (e.g., `"Zoom.exe"`), enumerate top-level windows via `EnumWindows`, get each window's owning PID via `GetWindowThreadProcessId`, query the process image name via `OpenProcess` + `QueryFullProcessImageNameW`, and return the PID of the first exact match. Given a window title (e.g., `"Zoom Meeting"`), enumerate windows and perform case-insensitive substring matching on titles via `GetWindowTextW`.

Add a Windows version check function that uses `RtlGetVersion` to determine if the current build supports process-specific loopback (Build ≥ 20348). If `RtlGetVersion` fails, treat as unsupported (safe default).

Add CLI mutual exclusivity validation: if both `--app` and `--app-window` are specified, emit an error and exit. This validation goes in `handle_record_command()` in `cli.rs`, matching the existing pattern for other validation errors.

Extract the matching logic into pure functions (`process_name_matches`, `window_title_matches`, `is_version_supported`) that are unit-testable without Windows APIs.

Add the new `windows` crate features needed for process resolution: `Win32_UI_WindowsAndMessaging`, `Win32_System_Threading`, `Win32_System_SystemInformation`.

### Acceptance criteria

- [ ] New `wasapi/process_resolver.rs` module with `resolve_by_process_name(name) -> Result<u32>` and `resolve_by_window_title(title) -> Result<u32>`
- [ ] `resolve_by_process_name` uses `EnumWindows` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` — exact case-sensitive match on filename
- [ ] `resolve_by_window_title` uses `EnumWindows` + `GetWindowTextW` — case-insensitive substring match
- [ ] Both return `ProcessNotFound` with actionable error message when no match found (e.g., "Process 'Zoom.exe' not found. Ensure the application is running and has a visible window.")
- [ ] `check_windows_version() -> bool` returns true for Build ≥ 20348, false otherwise. `RtlGetVersion` failure → false.
- [ ] CLI validation in `handle_record_command()`: both `--app` and `--app-window` specified → error + exit
- [ ] `Win32_UI_WindowsAndMessaging`, `Win32_System_Threading`, `Win32_System_SystemInformation` features added to `core/Cargo.toml`
- [ ] Module declared in `wasapi/mod.rs`
- [ ] Unit tests for pure matching functions: `process_name_matches` (exact match, no match, case sensitivity, full path extraction), `window_title_matches` (substring, case-insensitive, no match, empty query), `is_version_supported` (threshold, above, below, zero)
- [ ] CLI test: `--app` + `--app-window` mutual exclusivity validation
- [ ] All existing tests pass (`cargo test`)

---

## Phase 2: LoopbackCapture Trait + Process Loopback Recorder

**User stories**: #1, #2, #7, #8

### What to build

Define a `LoopbackCapture` trait in `capture_engine.rs` that abstracts the public interface shared by both loopback recorders: `stop()`, `get_frames_captured()`, `has_audio_detected()`, `get_sample_rate()`, `get_start_time()`. Implement this trait on the existing `WasapiLoopbackRecorder`.

Build the new `ProcessLoopbackRecorder` in `wasapi/process_loopback.rs`. This recorder uses `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` and `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` instead of the standard `IMMDevice.Activate` path. Implement `IActivateAudioInterfaceCompletionHandler` as a COM callback using the `windows` crate's `#[implement]` macro — the callback signals a Win32 event on completion, and the constructor waits with `WaitForSingleObject` to make activation synchronous.

The recording thread follows the same pattern as `WasapiLoopbackRecorder::recording_thread()`: COM init, audio client setup, WAV writer, recording loop with format detection, sample conversion, silence injection, stop and cleanup. The process recorder also implements `LoopbackCapture`.

Add the `implement` feature to the `windows` crate for the COM callback macro.

### Acceptance criteria

- [ ] `LoopbackCapture` trait defined in `capture_engine.rs` with methods: `stop()`, `get_frames_captured()`, `has_audio_detected()`, `get_sample_rate()`, `get_start_time()`
- [ ] `WasapiLoopbackRecorder` implements `LoopbackCapture`
- [ ] New `wasapi/process_loopback.rs` module with `ProcessLoopbackRecorder`
- [ ] `ProcessLoopbackRecorder::new(filepath, pid) -> Result<Self>` uses `ActivateAudioInterfaceAsync` with process loopback activation type
- [ ] COM callback implemented via `#[implement]` macro — signals Win32 event on completion
- [ ] `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` used to capture child processes
- [ ] Recording thread captures audio, writes WAV with silence injection (same pattern as system loopback)
- [ ] `ProcessLoopbackRecorder` implements `LoopbackCapture` trait
- [ ] `implement` feature added to `windows` crate in `core/Cargo.toml`
- [ ] Module declared in `wasapi/mod.rs`
- [ ] Activation failure (invalid PID, permission denied) returns a clear error
- [ ] All existing tests pass (`cargo test`)

---

## Phase 3: Record Command Integration + JSON Protocol

**User stories**: #10, #11, #13, #14, #15, #16, #18, #19, #20

### What to build

Wire the `--app`/`--app-window` options into the record command's startup flow in `record.rs`. The flow is: (1) validate mutual exclusivity (already done in CLI), (2) if `--app` or `--app-window` specified, check Windows version, (3) if version unsupported, fall back to system loopback + emit `warning` event, (4) if version OK, resolve PID via process resolver, (5) if PID found, create `ProcessLoopbackRecorder`, (6) if `--output-device` also specified, emit `warning` event (device ignored).

Change `record_worker` to use `Box<dyn LoopbackCapture>` instead of the concrete `WasapiLoopbackRecorder` type. The rest of the recording loop (status checks, signal handling, post-processing) works identically regardless of which recorder is behind the trait.

Add `capture_mode` field to JSON events: `"process"` when using process-specific capture, `"system"` when using system loopback. Add `app` and `app_window` fields to the `started` event. Add `capture_mode`, `app`, and `app_window` fields to `RecordingStatus` and `StatusOutput`.

Introduce the `warning` JSON event type: `{"type": "warning", "session_id": "...", "code": "<code>", "message": "<human-readable>"}`. Codes: `"unsupported_version"`, `"output_device_ignored"`, `"process_exited"`.

### Acceptance criteria

- [ ] `record_worker` uses `Box<dyn LoopbackCapture>` — works with both `WasapiLoopbackRecorder` and `ProcessLoopbackRecorder`
- [ ] Startup flow: `--app` specified + version OK → resolve PID → create `ProcessLoopbackRecorder`
- [ ] Startup flow: `--app` specified + version too old → fall back to `WasapiLoopbackRecorder` + emit `warning` event with code `"unsupported_version"`
- [ ] Startup flow: `--app` specified + process not found → `ProcessNotFound` error
- [ ] `--app` + `--output-device` → `warning` event with code `"output_device_ignored"`, device selection ignored
- [ ] `started` event includes `app`, `app_window`, and `capture_mode` fields
- [ ] `audio_state` event includes `capture_mode` field
- [ ] `RecordingStatus` struct has `capture_mode`, `app`, `app_window` fields
- [ ] `StatusOutput` struct has `capture_mode`, `app`, `app_window` fields
- [ ] `parse_status_file` handles new fields
- [ ] `warning` event type emitted as JSON with `code` and `message` fields
- [ ] `--app` works alongside all existing flags (`--duration`, `--format`, `--mic-muted`, `--no-trim`, etc.)
- [ ] Status tests: parse status with `capture_mode`, `app` fields; serialize `StatusOutput` with new fields
- [ ] All existing tests pass (`cargo test`)
- [ ] Manual verification: `--app Zoom.exe` records only Zoom audio (mic unaffected)

---

## Phase 4: Mid-Recording Fallback

**User stories**: #10, #11, #15, #16

### What to build

Implement mid-recording process exit detection and graceful fallback to system loopback. This is the most complex piece of the feature.

Add `is_process_dead() -> bool` to `ProcessLoopbackRecorder`, backed by an `AtomicBool`. The recording thread sets this flag in two ways: (1) when `GetBuffer` returns `AUDCLNT_E_DEVICE_INVALIDATED` or similar errors, and (2) via a periodic liveness check every ~100 loop iterations (~1 second) using `OpenProcess` + `WaitForSingleObject(handle, 0)`.

In the record command's 1-second status check loop, check `is_process_dead()`. When true: (1) stop the process loopback recorder, (2) create a new `WasapiLoopbackRecorder` (system loopback), (3) emit a `warning` event with code `"process_exited"`, (4) update `capture_mode` from `"process"` to `"system"`, (5) continue the recording loop with the new recorder.

Handle the critical failure mode: if the fallback `WasapiLoopbackRecorder::new()` fails (e.g., audio device disconnected simultaneously), continue with mic-only recording and emit an error event. This prevents total recording loss in a rare but realistic scenario.

The brief audio gap during the swap (~100-500ms) is filled with silence by both recorders' silence injection logic, maintaining WAV duration alignment with wall-clock time.

### Acceptance criteria

- [ ] `ProcessLoopbackRecorder::is_process_dead() -> bool` exposed (backed by `AtomicBool`)
- [ ] Recording thread sets `is_process_dead` flag on `GetBuffer` errors indicating process exit
- [ ] Recording thread performs periodic liveness check (~1s interval) via `OpenProcess` + `WaitForSingleObject(0)`
- [ ] Record command's status loop detects `is_process_dead()` and initiates fallback
- [ ] Fallback: stop process recorder → create system loopback recorder → swap `Box<dyn LoopbackCapture>`
- [ ] `warning` event emitted with code `"process_exited"` and `capture_mode: "system"` on fallback
- [ ] `capture_mode` in subsequent `audio_state` events changes from `"process"` to `"system"`
- [ ] Critical failure handled: if fallback recorder creation fails, continue mic-only + emit error event
- [ ] Audio gap during swap filled with silence (WAV duration stays aligned with wall-clock time)
- [ ] Manual verification: start recording with `--app`, close the target app → recording continues with system audio, warning event emitted
- [ ] All existing tests pass (`cargo test`)
