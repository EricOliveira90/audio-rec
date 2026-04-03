## Parent PRD

#6

## What to build

Add graceful Ctrl+C/SIGTERM handling so that pressing Ctrl+C during recording doesn't corrupt files or skip post-processing.

**Key implementation details:**
- Use `ctrlc` crate or `tokio::signal::ctrl_c()` to intercept Ctrl+C/SIGTERM
- On signal: set stop events on both capture threads, wait for threads to finish via `JoinHandle::join()`, then run the full post-processing pipeline
- Emit `{"type": "interrupted", "session_id": "...", "message": "Recording interrupted by signal"}` JSON event before starting shutdown
- Ensure temp files are cleaned up even on abnormal exit (register cleanup handler)
- Add `ctrlc` dependency to `core/Cargo.toml` (or use `tokio::signal::ctrl_c()`)

See PRD section: *Graceful Shutdown (Phase 1)*
See Plan: Phase 2

## Acceptance criteria

- [ ] Ctrl+C during recording: WAV files finalized, post-processing runs, output file produced
- [ ] `{"type": "interrupted"}` JSON event emitted on Ctrl+C before shutdown begins
- [ ] Temp files cleaned up on both normal and abnormal exit
- [ ] No corrupted WAV files left behind after Ctrl+C
- [ ] Post-processing pipeline runs to completion after interrupt
- [ ] Manual test: start recording, press Ctrl+C, verify output file is valid and complete

## Blocked by

- Blocked by #9 (Event-Driven WASAPI Capture)

## User stories addressed

- User story 3: graceful Ctrl+C shutdown with finalized files
- User story 14: JSON `interrupted` event for calling tools
