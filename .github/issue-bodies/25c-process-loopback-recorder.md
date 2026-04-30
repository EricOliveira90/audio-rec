## Parent PRD

#25

## What to build

Build the `ProcessLoopbackRecorder` — a new WASAPI recorder that captures audio from a specific process (and its child processes) using Windows 10's process-specific loopback API. This recorder lives in a new `wasapi/process_loopback.rs` module and implements the `LoopbackCapture` trait.

**COM activation pattern:**
- Use `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` instead of the standard `IMMDevice.Activate` path
- Use `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to capture audio from the target process and all its child processes
- Implement `IActivateAudioInterfaceCompletionHandler` as a COM callback using the `windows` crate's `#[implement]` macro
- The callback signals a Win32 event on completion; the constructor waits with `WaitForSingleObject` to make activation synchronous

**Recording thread:**
- Follows the same pattern as `WasapiLoopbackRecorder::recording_thread()`: COM init, audio client setup, WAV writer, recording loop with format detection, sample conversion, silence injection, stop and cleanup
- Uses the shared utilities from `capture_engine.rs` (format detection, sample conversion, write error tracking)

**Key design decisions (from eng review):**
- The recording loop will be ~90% identical to the system loopback recorder. The eng review recommends extracting a shared `run_capture_loop()` function to eliminate ~150 lines of duplication — this is recommended but can be done as part of this issue or as a follow-up
- COM activation via `#[implement]` macro is the standard pattern for the `windows` crate (confidence: 7/10 — verify `windows 0.58` supports `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`)
- Process loopback doesn't use a device endpoint — it activates an audio client directly for a process ID
- The `implement` feature must be added to the `windows` crate in `Cargo.toml`

**Files touched:**
- NEW: `core/src/wasapi/process_loopback.rs`
- MODIFIED: `core/src/wasapi/mod.rs` (module declaration)
- MODIFIED: `core/Cargo.toml` (add `implement` feature to windows crate)

## Acceptance criteria

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

## Blocked by

- Blocked by #27 — LoopbackCapture Trait (needs the trait to implement)

## User stories addressed

- User story 1: Record only Zoom's audio output
- User story 2: Record only Teams' audio output
- User story 7: Capture audio from target process and all child processes
- User story 8: Microphone continues independently (process recorder only handles loopback channel)
