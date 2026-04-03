## Parent PRD

#6

## What to build

Fix the silent loopback problem where WASAPI loopback stops delivering packets when system audio goes completely silent, causing the loopback recording to be shorter than the mic recording and creating sync drift when merging.

**Solution:** Create a secondary `IAudioClient` on the same output device in normal (non-loopback) render mode. Write a buffer of zeros (silence) to keep the WASAPI audio stream active during silent periods. The render client runs alongside the capture client.

**Key implementation details:**
- Secondary render client initialized alongside the loopback capture client
- Writes silence buffers to keep the audio engine active
- Cleanup order: stop capture client first, then stop and release the render client
- **Critical failure mode:** If the secondary render client fails to initialize, fall back to no-ClearBuffer mode (log warning, accept potential sync drift)

See PRD section: *Silent Loopback Fix (Phase 1)*
See Eng Review: Failure Modes — ClearBuffer render client init failure
See Plan: Phase 2

## Acceptance criteria

- [ ] Loopback recording stays in sync with mic during silent periods (ClearBuffer silence fix active)
- [ ] Secondary render client writes silence to keep WASAPI stream alive
- [ ] ClearBuffer render client init failure falls back gracefully with a logged warning
- [ ] Cleanup order: capture client stopped first, then render client
- [ ] No audible artifacts from the silence injection
- [ ] Manual test: record a meeting with silent presentation slides, verify loopback and mic stay in sync

## Blocked by

- Blocked by #9 (Event-Driven WASAPI Capture)

## User stories addressed

- User story 2: loopback stream stays alive during silent periods
