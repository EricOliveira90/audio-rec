## Parent PRD

#25

## What to build

Wire the `--app`/`--app-window` options into the record command's startup flow and add JSON protocol extensions for process-specific capture. This is the integration issue that connects the process resolver, LoopbackCapture trait, and process loopback recorder into the existing recording lifecycle.

**Startup flow in `record.rs`:**
1. Validate mutual exclusivity (already done in CLI from Process Resolver issue)
2. If `--app` or `--app-window` specified, check Windows version via `check_windows_version()`
3. If version unsupported → fall back to system loopback + emit `warning` event with code `"unsupported_version"`
4. If version OK → resolve PID via process resolver
5. If PID not found → `ProcessNotFound` error (hard fail)
6. If PID found → create `ProcessLoopbackRecorder`
7. If `--output-device` also specified → emit `warning` event with code `"output_device_ignored"` (device ignored)

**Record worker refactor:**
- Change `record_worker` to use `Box<dyn LoopbackCapture>` instead of the concrete `WasapiLoopbackRecorder` type
- The rest of the recording loop (status checks, signal handling, post-processing) works identically regardless of which recorder is behind the trait

**JSON protocol additions:**
- `started` event: add `app`, `app_window`, and `capture_mode` fields
- `audio_state` event: add `capture_mode` field (`"process"` or `"system"`)
- New `warning` event type: `{"type": "warning", "session_id": "...", "code": "<code>", "message": "<human-readable>"}`
  - Codes: `"unsupported_version"`, `"output_device_ignored"`, `"process_exited"` (last one used by fallback issue)

**Status extensions:**
- `RecordingStatus` struct: add `capture_mode`, `app`, `app_window` fields
- `StatusOutput` struct: add `capture_mode`, `app`, `app_window` fields
- `parse_status_file` handles new fields

**Key design decisions (from eng review):**
- `--app` only affects the loopback channel; microphone capture is completely unchanged
- `--app` works alongside all existing flags (`--duration`, `--format`, `--mic-muted`, `--no-trim`, etc.)
- The `warning` event includes a `code` field for programmatic handling and a `message` field for human-readable context

**Files touched:**
- MODIFIED: `core/src/commands/record.rs` (startup flow, Box<dyn LoopbackCapture>, JSON events)
- MODIFIED: `core/src/commands/status.rs` (StatusOutput new fields, parse_status_file)
- MODIFIED: `core/src/status.rs` (RecordingStatus new fields)

## Acceptance criteria

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

## Blocked by

- Blocked by #26 — Process Resolver + CLI Validation (needs PID resolution + version check)
- Blocked by #27 — LoopbackCapture Trait (needs the trait for `Box<dyn LoopbackCapture>`)
- Blocked by #28 — Process Loopback Recorder (needs `ProcessLoopbackRecorder` to create)

## User stories addressed

- User story 10: Recording continues (falling back to system audio) if target app exits mid-recording (warning event infrastructure)
- User story 11: JSON warning event when target process exits
- User story 13: `started` event includes `app`/`app_window` fields
- User story 14: `status` command shows target app name and capture mode
- User story 15: `capture_mode` field in `audio_state` events
- User story 16: New `warning` JSON event type
- User story 18: `--app` works alongside all existing flags
- User story 19: `--app` works with device selection flags
- User story 20: `--output-device` ignored with warning when using `--app`
