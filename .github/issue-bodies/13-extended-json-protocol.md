## Parent PRD

#6

## What to build

Final integration issue: ensure all new JSON events from previous issues are complete and consistent. Extend the `started` event to include which quality features are active.

**Extended `started` event:**
```json
{
  "type": "started",
  "session_id": "...",
  "features": {
    "denoise": true,
    "noise_gate": true,
    "normalize": true,
    "limiter": true,
    "process_capture": "Zoom.exe"
  }
}
```

**Verify all event schemas:**
- `{"type": "interrupted", "session_id": "...", "message": "Recording interrupted by signal"}`
- `{"type": "device_state", "session_id": "...", "device": "microphone", "state": "disconnected"}`
- `{"type": "device_state", "session_id": "...", "device": "microphone", "state": "reconnected"}`
- `{"type": "processing", "session_id": "...", "message": "Normalizing loudness", "stage": "normalizing"}`
- `{"type": "processing", "session_id": "...", "message": "Applying limiter", "stage": "limiting"}`

**Also:**
- Update CLI help text with all 8 new flags
- Update README with new features, flags, and JSON events
- Verify JSON protocol is backward-compatible (existing event types unchanged)

See PRD sections: *Extended started Event*, *New JSON Events*
See Plan: Phase 8

## Acceptance criteria

- [ ] `started` event includes `features` map with active quality features
- [ ] `features.process_capture` shows target app name when `--app` is used, `null` otherwise
- [ ] All JSON events include `session_id`
- [ ] `interrupted` event emitted on Ctrl+C (verified end-to-end)
- [ ] `device_state` events emitted on device disconnect/reconnect (verified end-to-end)
- [ ] `processing` events emitted for each post-processing stage (verified end-to-end)
- [ ] CLI help text updated with all 8 new flags
- [ ] README updated with new features, flags, and JSON events
- [ ] JSON protocol is backward-compatible — existing event types unchanged

## Blocked by

- Blocked by #11 (Graceful Ctrl+C Shutdown)
- Blocked by #8 (Post-Processing Pipeline Refactor)
- Blocked by #12 (Loudness Normalization)
- Blocked by #13 (Soft Audio Limiter)
- Blocked by #14 (RNNoise Noise Suppression)
- Blocked by #15 (Noise Gate)
- Blocked by #16 (RT Pipeline Integration)
- Blocked by #17 (Timestamp Sync + Device Reconnection)
- Blocked by #18 (Process-Specific Audio Capture)

## User stories addressed

- User story 14: JSON events for device state changes and interruption
- User story 15: processing JSON events for post-processing stages
- User story 19: started event includes active quality features
