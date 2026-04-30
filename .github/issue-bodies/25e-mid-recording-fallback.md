## Parent PRD

#25

## What to build

Implement mid-recording process exit detection and graceful fallback to system loopback. This is the most complex piece of the process-specific capture feature — when the target process exits during recording, the recorder seamlessly switches to full system audio capture instead of stopping.

**Process exit detection in `ProcessLoopbackRecorder`:**
- Add `is_process_dead() -> bool` method backed by an `AtomicBool`
- The recording thread sets this flag in two ways:
  1. When `GetBuffer` returns `AUDCLNT_E_DEVICE_INVALIDATED` or similar errors (immediate detection)
  2. Via a periodic liveness check every ~100 loop iterations (~1 second) using `OpenProcess` + `WaitForSingleObject(handle, 0)` (safety net — non-blocking, takes <1μs)

**Fallback state machine in `record.rs`:**

```
  ┌──────────┐   process exits   ┌──────────────┐
  │ Process   │ ───────────────► │ Fallback      │
  │ Loopback  │  (AtomicBool     │ Pending       │
  │ Active    │   signaled by    │               │
  └──────────┘   recorder thread)└──────┬───────┘
                                        │ record.rs detects
                                        │ flag in status loop
                                        ▼
                                 ┌──────────────┐
                                 │ System        │
                                 │ Loopback      │
                                 │ Active        │
                                 └──────────────┘
```

In the record command's 1-second status check loop, check `is_process_dead()`. When true:
1. Stop the process loopback recorder
2. Create a new `WasapiLoopbackRecorder` (system loopback)
3. Emit a `warning` event with code `"process_exited"` and `capture_mode: "system"`
4. Update `capture_mode` from `"process"` to `"system"`
5. Swap the `Box<dyn LoopbackCapture>` and continue the recording loop

**Critical failure handling (from eng review — flagged as critical gap):**
- If the fallback `WasapiLoopbackRecorder::new()` fails (e.g., audio device disconnected simultaneously), continue with mic-only recording and emit an error event
- This prevents total recording loss in a rare but realistic scenario

**Audio gap during swap:**
- There will be a brief gap (~100-500ms) during the recorder swap
- Both recorders' silence injection logic fills this gap with silence, maintaining WAV duration alignment with wall-clock time
- For meeting recordings, a 500ms silence gap when the meeting app crashes is completely acceptable

**Files touched:**
- MODIFIED: `core/src/wasapi/process_loopback.rs` (add `is_process_dead()`, liveness check in recording thread)
- MODIFIED: `core/src/commands/record.rs` (fallback detection in status loop, recorder swap logic)

## Acceptance criteria

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

## Blocked by

- Blocked by #29 — Record Command Integration + JSON Protocol (needs `Box<dyn LoopbackCapture>` infrastructure and warning event type)

## User stories addressed

- User story 10: Recording continues (falling back to full system audio) if target app exits mid-recording
- User story 11: JSON warning event when target process exits mid-recording
- User story 15: `capture_mode` field changes from `"process"` to `"system"` on fallback
- User story 16: `warning` JSON event type used for fallback notification
