# Plan: Process-Specific Capture Bugfixes

> Source: Code review of GitHub issues #25-#30 (process-specific audio capture via `--app` flag)
> 
> **Problem:** `--app vlc` captures ALL system audio (VLC + Firefox + everything else) instead of only VLC's audio output. The root cause is a malformed PROPVARIANT struct passed to `ActivateAudioInterfaceAsync`, plus several secondary issues in version detection, status reporting, and mid-recording fallback.

## Architectural decisions

Durable decisions that apply across all phases:

- **PROPVARIANT layout**: Use `#[repr(C)]` with correct field ordering to match the 24-byte Windows PROPVARIANT. No manual padding fields — Rust's `repr(C)` alignment handles it.
- **Version detection**: Use `RtlGetVersion` from `ntdll.dll` (not `GetVersionExW`) to avoid compatibility shims on Windows 10.
- **COM lifetime**: Let the `windows` crate's `#[implement]` macro handle COM reference counting. Do not manually `Box::from_raw` COM objects that may still have outstanding references.
- **Recording loop**: The `record_worker` loop owns a `Box<dyn LoopbackCapture>` and is responsible for checking `is_process_dead()` and performing fallback. The recorder itself only sets the flag.

---

## Phase 1: Fix PROPVARIANT_BLOB layout (P0)

**Review findings**: P0 — PROPVARIANT_BLOB struct is 28 bytes instead of the 24 bytes Windows expects, causing process loopback activation parameters to be silently ignored.

### What to build

Fix the `PROPVARIANT_BLOB` struct in `process_loopback.rs` so that `ActivateAudioInterfaceAsync` receives correctly-formatted activation parameters. The current struct has an extra `_pad: u32` field after `pBlobData` that makes the struct 28 bytes. Rust's `#[repr(C)]` already inserts the correct 4-byte padding between `cbSize` (u32 at offset 8) and `pBlobData` (pointer at offset 16), so the `_pad` field is both unnecessary and harmful.

After the fix, `--app vlc.exe` should capture ONLY VLC's audio output, with no bleed from other applications.

### Acceptance criteria

- [ ] `PROPVARIANT_BLOB` struct is exactly 24 bytes on 64-bit Windows (remove `_pad` field entirely)
- [ ] `#[repr(C)]` is present on the struct to guarantee C-compatible layout
- [ ] Add a compile-time assertion: `const _: () = assert!(std::mem::size_of::<PROPVARIANT_BLOB>() == 24);`
- [ ] Manual test: `audio-rec record --app vlc.exe -d 10` with VLC playing audio and Firefox playing a YouTube video — output WAV contains ONLY VLC audio
- [ ] Manual test: `audio-rec record --app-window "VLC media player" -d 10` — same result
- [ ] All existing tests pass (`cargo test`)

---

## Phase 2: Fix version check + status event fields

**Review findings**: P1 — `GetVersionExW` subject to compatibility shims; INFO — status events hardcode `capture_mode: "system"`.

### What to build

Two independent fixes that together ensure the `--app` feature reports accurate information through the JSON protocol:

**Version check fix:** Replace `GetVersionExW` in `check_windows_version()` with `RtlGetVersion` from `ntdll.dll`. `GetVersionExW` is subject to application compatibility shims on Windows 10 and may report a lower build number than reality, causing false negatives for users on Windows 10 Build 20348+. `RtlGetVersion` always returns the true OS version. Also fix the doc comment which already says "Uses RtlGetVersion" but the code uses `GetVersionExW`.

**Status event fix:** The `record_worker` function computes `capture_mode` correctly as `"process"` or `"system"`, but both `RecordingStatus` writes (initial status at recording start, and state-change updates during recording) hardcode `capture_mode: Some("system".to_string())` with `app: None` and `app_window: None`. Wire the actual `capture_mode` variable and the `app`/`app_window` option values into these status structs.

### Acceptance criteria

- [ ] `check_windows_version()` calls `RtlGetVersion` via `#[link(name = "ntdll")]` extern, not `GetVersionExW`
- [ ] Doc comment on `check_windows_version()` matches the implementation (both say `RtlGetVersion`)
- [ ] `is_version_supported()` unit tests still pass (pure logic, no API change)
- [ ] Both `RecordingStatus` writes in `record_worker` use the computed `capture_mode` variable instead of hardcoded `"system"`
- [ ] Both `RecordingStatus` writes pass `app` and `app_window` from the record options
- [ ] `started` JSON event includes `capture_mode`, `app`, and `app_window` fields when `--app` is used
- [ ] `audio_state` JSON events include `capture_mode` field
- [ ] `audio-rec status` shows correct capture mode and app name during a `--app` recording
- [ ] All existing tests pass (`cargo test`)

---

## Phase 3: Mid-recording fallback + COM lifetime cleanup

**Review findings**: INFO — mid-recording fallback (issue #25e) not implemented; P1 — CompletionHandler COM lifetime issue with `Box::from_raw`.

### What to build

**Mid-recording fallback:** Add process liveness checking to the `record_worker` loop. Every iteration (1 second), check `loopback_recorder.is_process_dead()`. If the target process has exited:

1. Emit a `warning` JSON event with code `"process_exited"` and a human-readable message
2. Stop the current `ProcessLoopbackRecorder`
3. Create a new `WasapiLoopbackRecorder` (system-wide) writing to the same temp file path
4. Replace the `Box<dyn LoopbackCapture>` with the new recorder
5. Update `capture_mode` to `"system"` in subsequent status events
6. Continue recording seamlessly

If the fallback recorder creation fails (rare: audio device disconnected simultaneously), continue with mic-only recording and emit an error event. This matches the defensive measure described in TODOS.md.

**COM lifetime fix:** In `activate_process_loopback()`, the `CompletionHandler` is created via `Box::into_raw` and later reclaimed via `Box::from_raw`. But the `#[implement]` macro creates a reference-counted COM object. The correct pattern is to let COM release the handler naturally. Remove the manual `Box::from_raw` cleanup calls and instead let the COM reference counting handle the lifetime. The handler will be released when the last COM reference is dropped.

### Acceptance criteria

- [ ] Recording loop checks `is_process_dead()` every iteration
- [ ] When target process exits: `warning` event emitted with `code: "process_exited"`
- [ ] When target process exits: recording continues with system-wide loopback (seamless fallback)
- [ ] When target process exits: subsequent status events show `capture_mode: "system"`
- [ ] If fallback recorder creation fails: recording continues with mic-only, error event emitted
- [ ] `CompletionHandler` lifetime managed by COM reference counting, not manual `Box::from_raw`
- [ ] No memory leaks or use-after-free in the COM activation path (verify with ASAN or manual audit)
- [ ] Manual test: start `--app vlc.exe` recording, close VLC mid-recording — recording continues with system audio, warning event emitted
- [ ] All existing tests pass (`cargo test`)
