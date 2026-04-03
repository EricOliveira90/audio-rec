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
