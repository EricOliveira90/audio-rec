## Parent PRD

#25

## What to build

Define a `LoopbackCapture` trait that abstracts the public interface shared by both loopback recorders (system and process-specific), and implement it on the existing `WasapiLoopbackRecorder`. This is a prerequisite for the process loopback recorder and the record command integration — both need to work with recorders through a common trait.

**LoopbackCapture trait** (in `capture_engine.rs`):

```rust
pub trait LoopbackCapture: Send {
    fn stop(&mut self) -> anyhow::Result<()>;
    fn get_frames_captured(&self) -> u64;
    fn has_audio_detected(&self) -> bool;
    fn get_sample_rate(&self) -> u32;
    fn get_start_time(&self) -> Option<Instant>;
}
```

These methods are called once per second in the status check loop, not in the audio hot path — zero performance concern from vtable dispatch.

**Key design decisions (from eng review):**
- Standard Rust pattern for runtime polymorphism with a small, fixed interface (confidence: 9/10)
- `record.rs` will use `Box<dyn LoopbackCapture>` instead of the concrete `WasapiLoopbackRecorder` type
- The trait is `Send` because the recorder is created on the main task and used in the async recording loop
- Implementing the trait on `WasapiLoopbackRecorder` is a non-breaking change — all existing methods already exist

**Files touched:**
- MODIFIED: `core/src/wasapi/capture_engine.rs` (add trait definition)
- MODIFIED: `core/src/wasapi/loopback.rs` (implement trait on `WasapiLoopbackRecorder`)

## Acceptance criteria

- [ ] `LoopbackCapture` trait defined in `capture_engine.rs` with methods: `stop()`, `get_frames_captured()`, `has_audio_detected()`, `get_sample_rate()`, `get_start_time()`
- [ ] Trait requires `Send` bound
- [ ] `WasapiLoopbackRecorder` implements `LoopbackCapture` (delegates to existing methods)
- [ ] All existing tests pass (`cargo test`) — this is a purely additive change
- [ ] No changes to `record.rs` yet (that happens in the integration issue)

## Blocked by

None — can start immediately (parallel with Process Resolver issue)

## User stories addressed

- User story 1: Record only Zoom's audio output (enables the polymorphic recorder architecture)
- User story 2: Record only Teams' audio output (same architecture)
- User story 8: Microphone continues independently (trait only covers loopback channel)
