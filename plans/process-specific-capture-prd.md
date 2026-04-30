# PRD: Process-Specific Audio Capture (`--app` and `--app-window`)

## Problem Statement

When recording meetings (e.g., Zoom, Teams), the current system audio capture records **everything** playing on the system — including Slack notifications, Spotify music, Windows system sounds, and other unrelated audio. This pollutes meeting recordings with irrelevant noise, making them harder to review and less useful for transcription or AI-assisted summarization. Users need a way to isolate audio from a specific application so that only the meeting audio (plus their independent microphone) is captured.

## Solution

Add two new CLI flags — `--app <process_name>` and `--app-window <window_title>` — that use Windows 10's process-specific WASAPI loopback capture (`ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`) to record audio from only the specified application. The microphone channel remains independent and unaffected. The feature includes graceful fallback behavior: if the target process exits mid-recording, the recorder falls back to full system loopback with a JSON warning; if the Windows version doesn't support process loopback (Build < 20348), it falls back to full system loopback with a JSON warning at startup. If the process is not found at startup, the recorder fails hard with a `ProcessNotFound` error.

## User Stories

1. As a meeting participant, I want to record only Zoom's audio output, so that my recording doesn't include Slack notifications or Spotify music playing in the background.
2. As a meeting participant, I want to record only Teams' audio output, so that I get a clean meeting recording for later review.
3. As a user, I want to specify the target app by process name (e.g., `--app Zoom.exe`), so that I can easily target the right application using its executable name.
4. As a user, I want to specify the target app by window title (e.g., `--app-window "Zoom Meeting"`), so that I can target an application even if I don't know its process name.
5. As a user, I want `--app-window` to use case-insensitive substring matching, so that I don't need to know the exact full window title (which often changes dynamically).
6. As a user, I want `--app` to use exact process name matching, so that I can precisely target the right process without ambiguity.
7. As a user, I want the recorder to capture audio from the target process and all its child processes, so that I don't miss audio from sub-processes (e.g., Zoom's audio rendering child process).
8. As a user, I want my microphone to continue recording independently when using `--app`, so that my voice is always captured regardless of which app's system audio I'm targeting.
9. As a user, I want a clear `ProcessNotFound` error if the target app isn't running when I start recording, so that I know to launch the app first.
10. As a user, I want the recording to continue (falling back to full system audio) if the target app exits mid-recording, so that I don't lose my recording if the meeting app crashes.
11. As a user, I want a JSON warning event when the target process exits mid-recording, so that my calling tool (AI assistant, automation script) knows the capture mode changed.
12. As a user on an older Windows version (Build < 20348), I want the recording to fall back to full system loopback with a JSON warning, so that the recording still works even if process-specific capture isn't supported.
13. As a calling tool (AI assistant), I want the `started` event to include `app` or `app_window` fields, so that I know process-specific capture is active.
14. As a calling tool, I want the `status` command to show the target app name and capture mode, so that I can display this information to the user.
15. As a calling tool, I want a `capture_mode` field in `audio_state` events (values: `"process"` or `"system"`), so that I can detect when a fallback has occurred.
16. As a calling tool, I want a new `warning` JSON event type for non-fatal issues like fallbacks, so that I can distinguish warnings from errors.
17. As a user, I want `--app` and `--app-window` to be mutually exclusive, so that there's no ambiguity about which targeting method is used.
18. As a user, I want `--app` and `--app-window` to work alongside all existing flags (`--duration`, `--format`, `--mic-muted`, `--no-trim`, etc.), so that process-specific capture doesn't limit my other recording options.
19. As a user, I want `--app` and `--app-window` to work with device selection flags (`--input-device`, `--output-device`), so that I can choose my microphone while targeting a specific app's audio.
20. As a user, I want the `--output-device` flag to be ignored (or produce a warning) when using `--app`, since process-specific capture doesn't use a specific output device endpoint.

## Implementation Decisions

### Modules to Build/Modify

1. **Process Resolver Module** (new) — Encapsulates all process/window resolution logic behind a simple interface: given a process name or window title, return a PID. Uses `EnumWindows` + `GetWindowThreadProcessId` for window enumeration (compliant with `claude.md` antivirus rules — no `tasklist`, no process enumeration). For `--app`, enumerates top-level windows, gets each window's owning PID, then queries the process image name to find an exact match. For `--app-window`, enumerates top-level windows and performs case-insensitive substring matching on window titles. Returns `ProcessNotFound` error if no match is found.

2. **Process Loopback Recorder** (new) — A new WASAPI recorder that uses `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` instead of the standard `IMMDevice.Activate` path. Implements `IActivateAudioInterfaceCompletionHandler` as a COM callback that signals a Win32 event on completion, then uses `WaitForSingleObject` to make the async activation synchronous. Uses `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture audio from the target process and all its child processes. Exposes the same public interface as `WasapiLoopbackRecorder` (start, stop, `get_frames_captured`, `has_audio_detected`, `get_sample_rate`, `get_start_time`). Detects mid-recording process exit via `GetBuffer` returning errors, and signals the fallback.

3. **Windows Version Check** (in process resolver module) — Checks whether the current Windows build supports process-specific loopback (Build ≥ 20348) using `RtlGetVersion` or the version helper APIs. Returns a boolean. Called by the record command to decide whether to attempt process-specific capture or fall back immediately.

4. **Record Command Integration** (modify existing) — Wires the `--app`/`--app-window` options into the recording lifecycle. At startup: checks Windows version → resolves PID → creates process loopback recorder (or falls back to standard loopback). Adds `app`, `app_window`, and `capture_mode` fields to JSON events (`started`, `audio_state`, `status`). Handles mid-recording fallback by switching from process loopback to standard loopback when the target process exits.

### Technical Decisions

- **Process detection method**: `EnumWindows` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` — NOT `tasklist` or `CreateToolhelp32Snapshot` (per `claude.md` antivirus rules)
- **`--app` matching**: Exact process name match (case-sensitive, e.g., `Zoom.exe` must match exactly)
- **`--app-window` matching**: Case-insensitive substring match on window titles (e.g., `"Zoom"` matches `"Zoom Meeting - John's Room"`)
- **Process tree capture**: `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture child processes
- **COM activation**: Blocking wrapper using `IActivateAudioInterfaceCompletionHandler` + Win32 event + `WaitForSingleObject`
- **Minimum Windows version**: Build 20348 (Windows 10 21H2 / Windows Server 2022)
- **`--app` and `--app-window` are mutually exclusive**: CLI validation rejects both being specified simultaneously
- **`--output-device` is ignored with `--app`**: Process loopback doesn't use a device endpoint; emit a JSON warning if both are specified
- **Microphone is independent**: `--app` only affects the loopback (system audio) channel; microphone capture is unchanged
- **New `windows` crate features needed**: `Win32_UI_WindowsAndMessaging` (for `EnumWindows`, `GetWindowTextW`, `FindWindowW`), `Win32_System_Threading` (for `QueryFullProcessImageNameW`, `OpenProcess`), `Win32_System_SystemInformation` (for `RtlGetVersion`)

### JSON Protocol Additions

- **`started` event**: Add optional `"app": "Zoom.exe"` and `"app_window": "Zoom Meeting"` fields, plus `"capture_mode": "process"` or `"capture_mode": "system"`
- **`audio_state` event**: Add `"capture_mode": "process"` or `"capture_mode": "system"` field
- **`status` command**: Add `app`, `app_window`, and `capture_mode` fields to status output
- **New `warning` event type**: `{"type": "warning", "session_id": "...", "code": "process_exited", "message": "Target process exited, falling back to full system audio"}` — used for non-fatal fallback scenarios

### Fallback Behavior

| Scenario | Behavior |
|----------|----------|
| Process not found at startup | Fail hard with `ProcessNotFound` error |
| Windows version < Build 20348 | Fall back to full system loopback + JSON `warning` event |
| Target process exits mid-recording | Fall back to full system loopback + JSON `warning` event, continue recording until explicit stop |
| `--app` + `--app-window` both specified | CLI validation error at startup |
| `--app` + `--output-device` both specified | Ignore `--output-device`, emit JSON `warning` event |

## Testing Decisions

Good tests verify external behavior (inputs → outputs) without coupling to implementation details. Tests should be structured similarly to the existing test suites in the codebase (e.g., `capture_engine` tests for format detection, `device_enumeration` tests for validation, `cli` tests for flag parsing).

### Modules to Test

1. **Process Resolver Module**
   - Test that `resolve_by_process_name` returns `ProcessNotFound` when no matching process exists
   - Test that `resolve_by_window_title` returns `ProcessNotFound` when no matching window exists
   - Test case-insensitive substring matching logic for window titles (unit-testable with mock data)
   - Test exact match logic for process names (unit-testable with mock data)
   - Test Windows version check returns correct boolean for known build numbers
   - Test mutual exclusivity validation (both `--app` and `--app-window` specified)

2. **COM Activation Wrapper (Process Loopback Recorder)**
   - Test that the completion handler correctly signals the Win32 event
   - Test that activation failure produces the correct error type
   - Test that the recorder exposes the same interface as `WasapiLoopbackRecorder`

3. **CLI Flag Parsing** (extend existing `cli` tests)
   - Test `--app` flag is parsed correctly (already exists)
   - Test `--app-window` flag is parsed correctly (already exists)
   - Test mutual exclusivity of `--app` and `--app-window`
   - Test `--app` combined with other flags (`--duration`, `--mic-muted`, etc.)

### Prior Art for Tests

- `core/src/wasapi/capture_engine.rs` — tests for format detection, sample conversion, write error tracking
- `core/src/wasapi/device_enumeration.rs` — tests for device validation, serialization
- `cli/src/cli.rs` — tests for flag parsing, `RecordOptions` construction

## Out of Scope

- **Microphone isolation**: `--app` only affects the loopback channel. Capturing only a specific app's microphone input is not included.
- **Multiple app targeting**: Capturing audio from multiple apps simultaneously (e.g., `--app Zoom.exe --app Slack.exe`) is not included.
- **App auto-detection**: Automatically detecting which app is a "meeting" app is not included.
- **App launch waiting**: Waiting for an app to launch if it's not running (e.g., `--app Zoom.exe --wait`) is not included.
- **Cross-platform support**: Process-specific capture is Windows-only (WASAPI). Linux/macOS equivalents are not included.
- **Real-time app switching**: Changing the target app mid-recording is not included.
- **Process name fuzzy matching**: Fuzzy or regex matching for `--app` is not included; only exact match.

## Further Notes

- The existing `--app` and `--app-window` flags are already parsed in the CLI (`cli/src/cli.rs`) and stored in `RecordOptions`, but are not wired to any functionality. The `ProcessNotFound` and `UnsupportedWindowsVersion` error variants already exist in `RecorderError`.
- The `ActivateAudioInterfaceAsync` API requires implementing a COM interface (`IActivateAudioInterfaceCompletionHandler`) which is more complex than the standard `IMMDevice.Activate` path used by the current loopback recorder. The `windows` crate supports implementing COM interfaces via the `#[implement]` macro.
- Process-specific loopback capture does NOT use a device endpoint — it activates an audio client directly for a process ID. This means `--output-device` is meaningless when `--app` is used.
- The silence injection logic in the current `WasapiLoopbackRecorder` (writing silence when no packets are available to keep WAV duration aligned with wall-clock time) should also be implemented in the process loopback recorder, since the target app may have periods of silence.
- The existing issue #12 in the repo covers this feature at a high level. This PRD supersedes that issue with detailed design decisions from the user interview.
