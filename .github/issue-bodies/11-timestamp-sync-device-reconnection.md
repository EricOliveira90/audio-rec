## Parent PRD

#6

## What to build

**Timestamp synchronization:** Capture `pu64DevicePosition` and `pu64QPCPosition` from `GetBuffer()` (currently passed as `None`). Track QPC (QueryPerformanceCounter) timestamps for each buffer in both loopback and mic streams. Calculate the offset between stream start times. Apply offset during merge via FFmpeg's `-itsoffset` parameter. Handle `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` flag — fall back to frame-count-based timing.

**Device reconnection:** Implement `IMMNotificationClient` COM interface for device change notifications. Register via `IMMDeviceEnumerator::RegisterEndpointNotificationCallback`. Handle `OnDefaultDeviceChanged` and `OnDeviceStateChanged` events. When `GetBuffer` returns `AUDCLNT_E_DEVICE_INVALIDATED`, enter reconnection mode: write silence to WAV to maintain timing/sync, retry device initialization every 2 seconds. Resume capture when device becomes available. Emit JSON `device_state` events.

**New error variants:** `DeviceDisconnected`, `DeviceReconnected` added to `RecorderError`.

**New `windows` crate features:** `implement` (for COM callback), `Win32_System_SystemInformation`.

See PRD sections: *Timestamp Synchronization*, *Device Reconnection*
See Eng Review: Failure Modes (device never comes back)
See Plan: Phase 6

## Acceptance criteria

- [ ] `GetBuffer()` captures QPC timestamps for both loopback and mic streams
- [ ] Stream start time offset calculated and applied via FFmpeg `-itsoffset` during merge
- [ ] `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` handled — falls back to frame-count timing
- [ ] `IMMNotificationClient` COM interface implemented for device change notifications
- [ ] Device disconnection detected via `AUDCLNT_E_DEVICE_INVALIDATED` from `GetBuffer`
- [ ] Silence written to WAV during device disconnection (maintains timing/sync)
- [ ] Device reconnection retried every 2 seconds without excessive resource consumption
- [ ] Recording resumes when device becomes available again
- [ ] `{"type": "device_state", "state": "disconnected"}` JSON event emitted on disconnect
- [ ] `{"type": "device_state", "state": "reconnected"}` JSON event emitted on reconnect
- [ ] `DeviceDisconnected` and `DeviceReconnected` error variants added to `RecorderError`
- [ ] `implement` and `Win32_System_SystemInformation` features added to `windows` crate
- [ ] Unit tests: offset calculation, TIMESTAMP_ERROR handling
- [ ] Unit tests: silence writing during disconnect, retry interval, JSON device_state events

## Blocked by

- Blocked by #9 (Event-Driven WASAPI Capture)

## User stories addressed

- User story 9: WASAPI device timestamp tracking for sync accuracy
- User story 10: automatic device reconnection
- User story 11: silence during device disconnection
- User story 14: JSON device_state events for calling tools
- User story 23: reasonable retry intervals
