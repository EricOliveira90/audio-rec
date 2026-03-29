# Plan: Audio Device Selection & Enhanced Status Command

> Source PRD: [GitHub Issue #1 — PRD: Audio Device Selection & Enhanced Status Command](https://github.com/EricOliveira90/audio-rec/issues/1)

## Architectural decisions

Durable decisions that apply across all phases:

- **CLI commands**: `devices` (new), `record` (modified with new flags), `status` (rewritten)
- **Device identification**: Numeric index only, corresponding to the order returned by WASAPI `EnumAudioEndpoints`. Indices are not stable across device plug/unplug events.
- **Device data model**: `AudioDevice` struct with fields: `index: u32`, `name: String`, `is_default: bool`, `sample_rate: u32`, `channels: u16`. Separate lists for input (capture) and output (render) devices.
- **Device enumeration module**: Deep module in the WASAPI layer. Two main interfaces: `list_devices(direction) -> Vec<AudioDevice>` and `get_device_by_index(direction, index) -> Result<IMMDevice>`. Self-contained COM initialization/cleanup.
- **RecordOptions extension**: Two new optional fields: `input_device_index: Option<u32>` and `output_device_index: Option<u32>`.
- **CLI flag conventions**: `--input-device <index>` / `-i <index>` for microphone selection. `--output-device <index>` / `-O <index>` (capital O) for loopback device selection. Capital O avoids conflict with existing `-o` (output file path).
- **JSON protocol**: All stdout output remains JSON (one object per line), consistent with existing `started`, `completed`, `error`, `audio_state` event types.
- **Status files**: JSON files in `storage/status/` directory, one per session. The `status` command reads these to find active sessions.
- **Platform scope**: Windows-only via WASAPI. Non-Windows platforms return a JSON error message, consistent with existing `record` command behavior.

---

## Phase 1: Device Enumeration & `devices` Command

**User stories**: 1, 2, 3, 4, 5, 10, 22

### What to build

A complete vertical slice from WASAPI device enumeration through to CLI output. Create a device enumeration module in the WASAPI layer that initializes COM, calls `EnumAudioEndpoints` for both `eCapture` and `eRender` directions, queries each device for its friendly name, default status, sample rate, and channel count, then returns clean `AudioDevice` structs. Build a `devices` command in the commands layer that calls the enumeration module and outputs JSON to stdout. Wire the `devices` command into the CLI dispatcher with help text. On non-Windows platforms, return an appropriate JSON error.

The JSON output format:
```json
{
  "type": "devices",
  "input_devices": [
    {"index": 0, "name": "Microphone (Blue Yeti)", "is_default": true, "sample_rate": 48000, "channels": 2}
  ],
  "output_devices": [
    {"index": 0, "name": "Speakers (Realtek)", "is_default": true, "sample_rate": 48000, "channels": 2}
  ]
}
```

### Acceptance criteria

- [ ] `audio-rec devices` outputs a JSON object with `input_devices` and `output_devices` arrays
- [ ] Each device entry includes `index`, `name`, `is_default`, `sample_rate`, and `channels`
- [ ] Default devices are marked with `is_default: true`
- [ ] Device indices start at 0 and increment sequentially per direction
- [ ] The command works without any active recording session
- [ ] `audio-rec help` and `audio-rec --help` show the new `devices` command in the usage text
- [ ] On non-Windows platforms, returns a JSON error message
- [ ] Unit tests cover `AudioDevice` struct serialization and index validation

---

## Phase 2: Device Selection on `record` Command

**User stories**: 6, 7, 8, 9, 23, 24

### What to build

A complete vertical slice from CLI flag parsing through to WASAPI device usage during recording. Add `--input-device`/`-i` and `--output-device`/`-O` flag parsing in the CLI dispatcher. Extend `RecordOptions` with the two new optional device index fields. In the record command, resolve device indices to WASAPI device handles using the enumeration module from Phase 1 — validate indices early and emit a JSON error event if invalid. Modify the loopback recorder to accept an optional device (use specific device or fall back to `GetDefaultAudioEndpoint(eRender, eConsole)`). Modify the microphone recorder similarly for capture devices. Include the selected device names in the `started` JSON event. When no device flags are provided, behavior is identical to current (system defaults).

### Acceptance criteria

- [ ] `audio-rec record --input-device 0` records using the microphone at index 0
- [ ] `audio-rec record --output-device 1` captures loopback audio from the output device at index 1
- [ ] `audio-rec record -i 0 -O 1` works with short flags
- [ ] `audio-rec record` (no device flags) uses system default devices, identical to current behavior
- [ ] Invalid device index (e.g., `--input-device 99`) produces a clear JSON error with available device count
- [ ] Non-numeric device index (e.g., `--input-device abc`) produces a clear error message
- [ ] The `started` JSON event includes the names of the selected input and output devices
- [ ] Help text documents the new `--input-device` and `--output-device` flags with examples
- [ ] Unit tests cover RecordOptions parsing with device index flags

---

## Phase 3: Enhanced `status` Command

**User stories**: 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21

### What to build

A complete vertical slice that rewrites the status command to read real recording session data. The status command reads JSON status files from the configured status directory, finds the most recent active session (by file modification time, filtering for `status: "recording"` or `status: "processing"`), and outputs a JSON object with comprehensive recording information. Fields include: session ID, output filename, elapsed time (human-readable format like "2m 34s"), recording duration setting (fixed seconds or "indefinite"), audio format and quality level, microphone muted status, audio devices in use (device name string from the status file), sample rate, audio detection status per channel (loopback and mic), frames captured per channel, and processing progress (current stage, FFmpeg progress percentage, processing speed) when in post-processing. When no active recording session exists, output a JSON message indicating no active session. Wire into the CLI dispatcher, replacing the current static message implementation.

### Acceptance criteria

- [ ] `audio-rec status` during an active recording shows session ID, filename, elapsed time, duration, format, quality, mic muted state, devices, sample rate, audio detection, and frames captured
- [ ] Elapsed time is displayed in human-readable format (e.g., "2m 34s")
- [ ] During post-processing, shows processing stage, FFmpeg progress, and speed
- [ ] When no active recording exists, outputs `{"type": "status", "active": false, "message": "No active recording session found"}`
- [ ] Completed and cancelled sessions are not shown as active
- [ ] Output is valid JSON consistent with the application's protocol
- [ ] Unit tests cover status file parsing, elapsed time formatting, and the no-active-session case
