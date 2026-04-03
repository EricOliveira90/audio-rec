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

- Blocked by #9 (Event-Driven WASAPI Capture)

## User stories addressed

- User story 12: capture audio only from specific application
- User story 13: graceful fallback on older Windows versions
- User story 20: process name and window title targeting
- User story 21: Windows version compatibility check
