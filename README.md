# Audio Rec

A Rust-based command-line tool for recording system audio and microphone on Windows. Designed to be **spawned by other tools** (e.g., AI assistants, automation scripts) via process spawning, with structured JSON output for easy integration.

## Features

- **Dual-channel recording** — Captures system audio (WASAPI loopback) and microphone simultaneously
- **Real-time noise suppression** — RNNoise-based neural network noise removal on microphone audio (keyboard typing, fan noise)
- **Noise gate** — VAD-driven 6-state noise gate with smooth attack/release ramps (no clicks)
- **Real-time limiter** — Soft limiter prevents clipping from sudden loud sounds
- **Loudness normalization** — EBU R128 loudness normalization to -16 LUFS (configurable)
- **Post-processing pipeline** — Discrete stages: Merge → Normalize → Limit → Trim → Finalize
- **Device selection** — Choose specific audio input/output devices by index
- **Device enumeration** — List all available audio devices with `audio-rec devices`
- **Process-specific capture** — Capture audio from a specific application only (e.g., `--app Zoom.exe`)
- **Enhanced status** — View active recording session info (elapsed time, devices, audio detection, etc.)
- **Structured JSON protocol** — All stdout output is JSON, one object per line, for easy parsing
- **Silence trimming** — Automatically trims leading/trailing silence and caps internal silence gaps at 2 seconds
- **Microphone mute** — Start muted or toggle mute during recording via IPC
- **Custom output path** — Save recordings wherever you want
- **Multiple formats** — WAV (lossless) and M4A (AAC compressed)
- **File-based IPC** — Stop, cancel, and mute via signal files (no sockets needed)
- **Zero-config quality** — All quality features ON by default, disable with `--no-*` flags

## Requirements

- **Windows 10/11** (uses WASAPI for audio capture)
- **FFmpeg** in PATH (required for audio merging, encoding, and silence trimming)
- **Rust toolchain** (for building from source)

## Building

```bash
cargo build --release
```

The binary will be at `target/release/audio-rec.exe`.

### Running from the Project Folder

```bash
cargo run --release -- record
cargo run --release -- stop
cargo run --release -- mute
```

### Install Globally

To make `audio-rec` available as a command from any folder:

```bash
cargo install --path cli
```

This installs `audio-rec.exe` to `~/.cargo/bin/` which is already in your PATH if you have Rust installed.

### Update Global Install

After pulling new changes or making local modifications, re-run the install command to update the global binary:

```bash
cargo install --path cli
```

This will rebuild and replace the existing `audio-rec.exe` in `~/.cargo/bin/`.

## Commands

### `record` — Start Recording

```bash
audio-rec record [options]
```

**Options:**

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--output <path>` | `-o` | Output file path (directory + filename) | `cwd/<auto-name>.wav` |
| `--duration <secs>` | `-d` | Recording duration in seconds | Indefinite (until `stop`) |
| `--format <wav\|m4a>` | `-f` | Audio format | `wav` (or inferred from `--output` extension) |
| `--quality <level>` | `-q` | Quality preset: `quick`, `standard`, `professional`, `high` | `professional` |
| `--input-device <idx>` | `-id` | Input device index (microphone). Use `devices` to list. | System default |
| `--output-device <idx>` | `-od` | Output device index (loopback). Use `devices` to list. | System default |
| `--mic-muted` | | Start with microphone muted | Off |
| `--no-trim` | | Disable automatic silence trimming | Trimming ON |
| `--no-normalize` | | Disable loudness normalization | Normalization ON |
| `--no-limiter` | | Disable soft audio limiter | Limiter ON |
| `--no-denoise` | | Disable real-time noise suppression | Denoise ON |
| `--no-noise-gate` | | Disable noise gate | Gate ON |
| `--target-loudness <LUFS>` | | Target loudness level (e.g., `-23` for broadcast) | `-16` (streaming) |
| `--gate-threshold <val>` | | Noise gate threshold (VAD probability 0.0-1.0) | `0.5` |
| `--app <name>` | | Capture audio from specific process (e.g., `Zoom.exe`) | All system audio |
| `--app-window <title>` | | Capture audio from specific window title | All system audio |

**Examples:**

```bash
# Record indefinitely with all quality features (default)
audio-rec record

# Record for 30 seconds
audio-rec record -d 30

# Record to a specific file
audio-rec record -o C:\recordings\meeting.wav

# Record as M4A with mic muted
audio-rec record -o standup.m4a --mic-muted

# Record without any audio processing (raw capture)
audio-rec record --no-normalize --no-limiter --no-denoise --no-noise-gate --no-trim

# Record with broadcast loudness standard
audio-rec record --target-loudness -23

# Record only Zoom audio (no Slack notifications, no Spotify)
audio-rec record --app Zoom.exe

# Record with sensitive noise gate (catches quieter speech)
audio-rec record --gate-threshold 0.3

# Record using specific input and output devices
audio-rec record -id 0 -od 1
```

### `stop` — Stop Recording

```bash
audio-rec stop [session_id]
```

Stops the active recording and triggers post-processing (merge, normalize, limit, trim). If no `session_id` is provided, stops the most recent active session.

### `mute` — Toggle Microphone Mute

```bash
audio-rec mute [session_id]
```

Toggles the microphone mute state on an active recording. When muted, the mic channel writes silence (zeros) instead of real audio, producing a continuous valid audio stream.

### `devices` — List Audio Devices

```bash
audio-rec devices
```

Lists all available audio input (microphone) and output (speakers/headphones) devices as JSON. Use the device indices with `--input-device` / `--output-device` flags on the `record` command.

**Example output:**

```json
{
  "type": "devices",
  "input_devices": [
    {"index": 0, "name": "Microphone (Blue Yeti)", "is_default": true, "sample_rate": 48000, "channels": 2}
  ],
  "output_devices": [
    {"index": 0, "name": "Speakers (Realtek)", "is_default": true, "sample_rate": 48000, "channels": 2},
    {"index": 1, "name": "HDMI Audio (NVIDIA)", "is_default": false, "sample_rate": 48000, "channels": 8}
  ]
}
```

> **Note:** Device indices may change between runs if devices are plugged/unplugged. Run `audio-rec devices` before each recording session to get current indices.

### `status` — Active Recording Info

```bash
audio-rec status
```

Shows information about the active recording session, including elapsed time, devices in use, audio detection status, and more. When no recording is active, reports that no session is found.

**Example output (active recording):**

```json
{
  "type": "status",
  "active": true,
  "session_id": "rec-20260329_120000",
  "filename": "recording.wav",
  "elapsed": "2m 34s",
  "duration": "indefinite",
  "format": "wav",
  "quality": "Professional (48kHz Stereo)",
  "is_mic_muted": false,
  "device": "Dual-Channel (System + Microphone)",
  "sample_rate": 48000,
  "loopback_has_audio": true,
  "mic_has_audio": true,
  "loopback_frames": 1500,
  "mic_frames": 1200
}
```

## Audio Quality Pipeline

All quality features are **ON by default** for zero-config best quality. Disable any feature with `--no-*` flags.

### Real-Time Processing (Microphone)

Applied in real-time during recording, before samples are written to WAV:

```
Mic samples → Resample to 48kHz → RNNoise (denoise + VAD) → Noise Gate (VAD-driven) → Limiter → Resample to native → WAV
```

| Stage | What it does | Disable flag |
|-------|-------------|--------------|
| **RNNoise** | Neural network noise suppression — removes keyboard typing, fan noise, background hum | `--no-denoise` |
| **Noise Gate** | 6-state machine (Closed→Opening→Open→Holding→Closing→Closed) with smooth ramps. VAD-driven when RNNoise active, dB threshold when disabled | `--no-noise-gate` |
| **Limiter** | Soft limiter at 0.95 threshold with 1ms attack — prevents clipping from sudden loud sounds | `--no-limiter` |

### Post-Processing Pipeline

Applied after recording stops, before the final file is saved:

```
Merge → Normalize → Limiter → Trim → Finalize
```

| Stage | What it does | Disable flag |
|-------|-------------|--------------|
| **Merge** | Combines loopback + mic into stereo (4 scenarios based on which channels have audio) | Always active |
| **Normalize** | EBU R128 loudness normalization to -16 LUFS (two-pass for WAV, single-pass for M4A) | `--no-normalize` |
| **Limiter** | FFmpeg `alimiter` filter for post-processing limiting | `--no-limiter` |
| **Trim** | Removes leading/trailing silence, caps internal gaps at 2s | `--no-trim` |
| **Finalize** | Copies to final output path | Always active |

Each stage emits a JSON `processing` event so calling tools can show accurate progress.

## JSON Protocol (stdout)

All stdout output follows a structured JSON protocol — one JSON object per line. This makes it easy for calling tools to parse output by reading lines and deserializing JSON.

### Event Types

#### `started` — Recording has begun

```json
{
  "type": "started",
  "session_id": "rec-20260320_114500",
  "file_path": "C:\\recordings\\meeting.wav",
  "duration": -1,
  "format": "wav",
  "quality": "Professional (48kHz Stereo)",
  "mic_muted": false,
  "trim_silence": true,
  "input_device": "Microphone (Blue Yeti)",
  "input_device_index": 0,
  "output_device": "Speakers (Realtek)",
  "output_device_index": 0
}
```

- `duration`: `-1` means indefinite, positive number means fixed seconds
- `input_device` / `output_device`: Only present when device selection flags are used

#### `audio_state` — Audio state changed

Emitted only when audio detection or mute state changes (not periodically). This keeps stdout quiet during normal recording.

```json
{
  "type": "audio_state",
  "session_id": "rec-20260320_114500",
  "loopback_has_audio": true,
  "mic_has_audio": false,
  "is_mic_muted": false
}
```

#### `muted` — Microphone mute state changed

```json
{
  "type": "muted",
  "session_id": "rec-20260320_114500",
  "is_muted": true
}
```

#### `processing` — Post-processing stage update

```json
{
  "type": "processing",
  "session_id": "rec-20260320_114500",
  "message": "Normalizing loudness",
  "stage": "normalizing"
}
```

#### `completed` — Recording finished successfully

```json
{
  "type": "completed",
  "session_id": "rec-20260320_114500",
  "file_path": "C:\\recordings\\meeting.wav",
  "filename": "meeting.wav",
  "file_size_mb": "2.34",
  "format": "wav",
  "duration_secs": -1,
  "message": "Recording completed successfully"
}
```

#### `cancelled` — Recording was cancelled

```json
{
  "type": "cancelled",
  "session_id": "rec-20260320_114500",
  "message": "Recording cancelled by user"
}
```

#### `error` — An error occurred

```json
{
  "type": "error",
  "message": "FFmpeg is not installed or not found in PATH..."
}
```

### Stop/Mute Command Output

```json
{
  "status": "success",
  "data": {
    "session_id": "rec-20260320_114500",
    "message": "Stop signal sent successfully"
  }
}
```

## Integration Example

Here's how another tool (e.g., a Python script) would use this:

```python
import subprocess
import json

# Start recording with all quality features (default)
proc = subprocess.Popen(
    ["audio-rec", "record", "-o", "meeting.wav"],
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True
)

# Read events from stdout (only emitted on state changes, not every second)
for line in proc.stdout:
    event = json.loads(line.strip())
    
    if event["type"] == "started":
        print(f"Recording started: {event['session_id']}")
    
    elif event["type"] == "audio_state":
        print(f"Audio: System={event['loopback_has_audio']}, Mic={event['mic_has_audio']}")
    
    elif event["type"] == "processing":
        print(f"Processing: {event['message']}")
    
    elif event["type"] == "completed":
        print(f"Done! File: {event['file_path']} ({event['file_size_mb']} MB)")
        break

# To stop early (from another process/thread):
# subprocess.run(["audio-rec", "stop"])

# To toggle mute:
# subprocess.run(["audio-rec", "mute"])
```

## Architecture

```
audio-rec/
├── Cargo.toml                    # Workspace root
├── core/                         # Library crate
│   └── src/
│       ├── lib.rs
│       ├── audio_utils.rs        # RMS audio level detection
│       ├── config.rs             # Storage/config management
│       ├── domain.rs             # AudioFormat, RecordingDuration, SessionId
│       ├── error.rs              # Error types (incl. DeviceDisconnected, ProcessNotFound, etc.)
│       ├── ffmpeg_encoder.rs     # FFmpeg integration (merge, encode, trim, duration)
│       ├── logging.rs            # Dual-output logging (file + terminal)
│       ├── output.rs             # User-facing terminal output
│       ├── recorder.rs           # Quality presets, smart audio merge (4 scenarios)
│       ├── status.rs             # JSON status file observer
│       ├── audio_processing/     # Real-time audio processing pipeline
│       │   ├── noise_suppress.rs # RNNoise noise suppression + VAD + resampling
│       │   ├── noise_gate.rs     # 6-state noise gate (VAD-driven or dB threshold)
│       │   ├── limiter.rs        # Real-time soft limiter (0.95 threshold, 1ms attack)
│       │   └── pipeline.rs       # Chains: RNNoise → Gate → Limiter
│       ├── post_processing/      # Post-recording processing pipeline
│       │   └── pipeline.rs       # Discrete stages: Merge → Normalize → Limit → Trim → Finalize
│       ├── commands/
│       │   ├── devices.rs        # Devices command (enumerate audio devices)
│       │   ├── record.rs         # Record command (main recording lifecycle)
│       │   ├── stop.rs           # Stop command (signal file IPC)
│       │   ├── mute.rs           # Mute toggle command (signal file IPC)
│       │   └── status.rs         # Status command (active session info)
│       └── wasapi/
│           ├── capture_engine.rs # Shared WASAPI capture logic (format detection, sample conversion)
│           ├── device_enumeration.rs  # WASAPI device enumeration & validation
│           ├── loopback.rs       # System audio capture (thin wrapper over capture engine)
│           └── microphone.rs     # Microphone capture (thin wrapper with mute + audio processing)
├── cli/                          # Binary crate
│   └── src/
│       ├── main.rs               # Entry point, logging init
│       └── cli.rs                # Argument parsing, command dispatch
└── storage/                      # Runtime data (auto-created)
    ├── recordings/               # Temporary WAV files during recording
    ├── status/                   # JSON status files
    └── signals/                  # IPC signal files (.stop, .mute, .cancel)
```

### Recording Lifecycle

1. **Startup**: CLI parses flags (including quality features), checks FFmpeg availability, creates session
2. **Recording**: WASAPI captures system audio + mic in parallel threads, writes temp WAV files. Mic audio passes through the real-time processing pipeline (denoise → gate → limiter)
3. **Signal check loop**: Every 1s — checks signal files (.stop, .mute, .cancel) and detects audio state changes; emits JSON events only when state changes
4. **Stop**: Signals capture threads via Win32 events, joins with 5s timeout (no more sleep hacks)
5. **Post-processing pipeline**: Merge → Normalize → Limit → Trim → Finalize (each stage emits JSON events)
6. **Finalize**: Copies to final output path, emits `completed` JSON, cleans up temp files

### Audio Merge Scenarios

| Scenario | System Audio | Microphone | Strategy |
|----------|-------------|------------|----------|
| A | ✓ Has audio | ✓ Has audio | Dual-mono stereo (L=system, R=mic) |
| B | ✓ Has audio | ✗ Silent | Duplicate system to stereo |
| C | ✗ Silent | ✓ Has audio | Duplicate mic to stereo |
| D | ✗ Silent | ✗ Silent | Silent stereo file |

### IPC (Inter-Process Communication)

File-based signal files in `storage/signals/`:
- `{session_id}.stop` — Stop recording, proceed to post-processing
- `{session_id}.cancel` — Stop recording, discard everything
- `{session_id}.mute` — Toggle microphone mute

This approach is simple, cross-process compatible, and doesn't require sockets or shared memory.

## Silence Trimming

Enabled by default (disable with `--no-trim`). Uses FFmpeg's `silenceremove` filter:

- **Leading silence**: Removed completely
- **Trailing silence**: Removed completely
- **Internal silence**: Capped at 2 seconds (silence beyond 2s is removed, up to 2s is kept)
- **Threshold**: -50 dB (configurable in code)

If trimming fails (e.g., FFmpeg error), the original file is kept — trimming failure is non-fatal.

If the entire recording is silence, the original file is kept (empty file prevention).

## Logging

Logs go to two places:
- **File**: `%APPDATA%\audio-rec\logs\cli-YYYY-MM-DD.log` (daily rotation, 7-day retention)
- **Terminal**: stderr (can be disabled with `--no-log-terminal`)

Log level can be set with `--log-level <trace|debug|info|warn|error>` (default: `info`).

## Testing

The project has comprehensive test coverage with 160 tests:

```bash
cargo test
```

| Module | Tests | What's covered |
|--------|-------|----------------|
| `capture_engine` | 22 | Sample conversion (f32→i16, i32→i16), format detection, write error tracking |
| `noise_suppress` | 14 | Stereo downmix/upmix, resampling, VAD output, enable/disable |
| `noise_gate` | 12 | All 6 state transitions, hold time, attack/release ramps, VAD-driven mode |
| `limiter` | 10 | Threshold behavior, passthrough, gain recovery, buffer processing |
| `audio_pipeline` | 8 | Full chain, each stage independently disabled, config from flags |
| `post_processing` | 13 | Pipeline config, stage counting, event emitter, M4A vs WAV stages |
| `cli` | 22 | All 16 CLI flags parsed correctly, RecordOptions defaults |
| Other modules | 59 | Domain types, audio utils, device enumeration, status, logging |

## Future Implementation Plans

- [ ] **Graceful Ctrl+C shutdown** — Handle SIGTERM/Ctrl+C to properly finalize recordings
- [ ] **Event-driven WASAPI capture** — Replace polling with `WaitForMultipleObjects` for lower CPU usage
- [ ] **Silent loopback fix** — ClearBuffer silence injection to keep WASAPI stream alive during silent periods
- [ ] **Device reconnection** — Automatic reconnection when audio devices are disconnected/reconnected
- [ ] **Timestamp synchronization** — QPC timestamps for precise loopback/mic alignment
- [ ] **Cross-platform support** — Linux (PulseAudio/PipeWire) and macOS (CoreAudio) recording backends
- [x] **Device selection** — Choose specific audio input/output devices by index
- [x] **Device enumeration** — List available audio devices with `audio-rec devices`
- [x] **Enhanced status** — Show active recording session info
- [x] **Real-time noise suppression** — RNNoise-based neural network noise removal
- [x] **Noise gate** — VAD-driven 6-state noise gate with smooth ramps
- [x] **Audio limiter** — Real-time soft limiter + post-processing limiter
- [x] **Loudness normalization** — EBU R128 normalization pipeline stage
- [x] **Post-processing pipeline** — Discrete stages with JSON progress events
- [x] **Process-specific capture** — `--app` and `--app-window` flags for app-specific audio
- [ ] **Real-time audio level monitoring** — Emit audio level (RMS/peak) in progress events
- [ ] **Configurable silence parameters** — Expose silence threshold and max gap duration as CLI flags
- [ ] **Pause/resume** — Pause recording without stopping
- [ ] **Multiple output formats** — Support FLAC, OGG, MP3 output via FFmpeg
- [ ] **Pre-recording buffer** — Keep a rolling buffer so you never miss the start of audio
