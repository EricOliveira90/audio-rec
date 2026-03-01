# Audio Recorder Manager -- Design Document

## Overview

A Rust application that records system audio (loopback) and microphone audio simultaneously on Windows via WASAPI, merges them with FFmpeg, and supports start/stop/mute commands via file-based IPC.

## Scope

Core commands only: **start**, **stop**, **mute**. No recover command or transcription compression features.

Output formats: WAV and M4A.

## Architecture

Cargo workspace with two crates:

- **`audio-recorder-manager-core`** (library) -- domain types, WASAPI capture, FFmpeg processing, commands, config, logging, output, status
- **`audio-recorder-manager-cli`** (binary) -- thin CLI layer for argument parsing and command dispatch

### Project Structure

```
audio-recorder-manager-efo/
├── Cargo.toml                          # Workspace root
├── core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── audio_utils.rs
│       ├── config.rs
│       ├── domain.rs
│       ├── logging.rs
│       ├── output.rs
│       ├── recorder.rs
│       ├── ffmpeg_encoder.rs
│       ├── status.rs
│       ├── commands/
│       │   ├── mod.rs
│       │   ├── record.rs
│       │   ├── stop.rs
│       │   ├── mute.rs
│       │   └── status.rs
│       └── wasapi/
│           ├── mod.rs
│           ├── loopback.rs
│           └── microphone.rs
├── cli/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       └── cli.rs
```

## Recording Lifecycle

1. CLI parses args (duration, format, quality) and calls `record::execute_with_output()`
2. `record_worker()` creates temp WAV paths, initializes WASAPI recorders (loopback required, mic optional with graceful degradation), creates `Arc<AtomicBool>` mute flag
3. Recording loop polls every 1 second: checks elapsed time, checks signal files (.cancel, .stop, .mute), writes RecordingStatus JSON
4. On stop: stops recorders, waits for temp files to stabilize, runs FFmpeg merge
5. `merge_audio_streams_smart()` selects one of four merge scenarios based on which streams have audio, optionally encodes to M4A in same pass
6. Cleans up temp files, writes final RecordingResult

## Mute Implementation

Per mute-feature-plan.md:

- **Signal**: `mute` CLI command writes `{session_id}.mute` to signals directory
- **Detection**: Recording loop polls for `.mute` files, deletes on detection, flips `Arc<AtomicBool>` via `fetch_xor`
- **Effect**: Microphone WASAPI capture callback checks the flag; when muted, writes zeros instead of real PCM samples
- **Result**: Continuous valid WAV with silent segments. FFmpeg merge is completely unaffected.

### Edge Cases

- No mic available: mute signal silently ignored
- Mute after recording ends: orphan file cleaned up
- Rapid toggles: atomic operation, one toggle per poll cycle

## Merge Scenarios (FFmpeg)

| Scenario | Loopback | Mic | Strategy |
|----------|----------|-----|----------|
| A | Has audio | Has audio | Dual-mono stereo: L=system, R=mic via amerge+pan |
| B | Has audio | Silent | Duplicate loopback to stereo via -ac 2 |
| C | Silent | Has audio | Duplicate mic to stereo via asplit+amerge |
| D | Silent | Silent | Create silent stereo from loopback via aformat |

For M4A output, encoding happens in the same FFmpeg pass (AAC, 192kbps, faststart).

## IPC Protocol

File-based signal files in `storage/signals/`:
- `.stop` -- stop recording and proceed to post-processing
- `.cancel` -- stop recording and discard
- `.mute` -- toggle microphone mute

JSON status files in `storage/status/{session_id}.json` updated every 1 second:
```json
{
  "session_id": "rec-20260301_143000",
  "status": "recording",
  "progress_percent": 45.0,
  "elapsed_seconds": 27,
  "is_mic_muted": false,
  "has_microphone": true
}
```

All CLI commands output JSON to stdout for frontend consumption.

## Dependencies

| Crate | Purpose |
|-------|---------|
| windows | WASAPI COM APIs |
| hound | WAV file I/O |
| tokio | Async runtime |
| anyhow | Error handling |
| serde / serde_json | JSON serialization |
| chrono | Timestamps |
| tracing / tracing-subscriber | Logging |
| tracing-log | Log bridge |
| dirs | Platform config directory |

## Error Handling

- WASAPI init failures: `anyhow::Result` with `.context()`. Mic failure degrades to loopback-only.
- FFmpeg failures: detected via exit code and pipe I/O errors (no process monitoring per CLAUDE.md)
- Signal file failures: writing fails returns error to CLI; polling failures log and continue
- Platform gating: `#[cfg(windows)]` with compile-time errors on other platforms

## Changes from Reference

| Change | File | Description |
|--------|------|-------------|
| Mute flag | wasapi/microphone.rs | Accept Arc<AtomicBool>, zero-fill when muted |
| Mute polling | commands/record.rs | Check .mute file in loop, toggle flag |
| Mute status | status.rs | Add is_mic_muted field to RecordingStatus |
| Mute command | commands/mute.rs | New: find active session, write .mute signal |
| Mute dispatch | cli.rs | Add "mute" match arm |

Everything else is a direct port of the reference code.
