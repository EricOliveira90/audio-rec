# Audio Recorder Manager Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust application that records system audio + microphone via WASAPI, merges them with FFmpeg, and supports start/stop/mute commands.

**Architecture:** Cargo workspace with two crates — `audio-recorder-manager-core` (library) and `audio-recorder-manager-cli` (binary). The core library handles WASAPI capture, FFmpeg processing, file-based IPC, and JSON status reporting. The CLI is a thin dispatcher.

**Tech Stack:** Rust, WASAPI (via `windows` crate), FFmpeg (external process), `hound` (WAV I/O), `tokio` (async runtime), `anyhow`/`serde`/`chrono`/`tracing`

---

## Task 1: Workspace & Cargo.toml Setup

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `core/Cargo.toml`
- Create: `core/src/lib.rs` (placeholder)
- Create: `cli/Cargo.toml`
- Create: `cli/src/main.rs` (placeholder)

**Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
members = ["core", "cli"]
resolver = "2"
```

**Step 2: Create core crate Cargo.toml**

```toml
[package]
name = "audio-recorder-manager-core"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
dirs = "5"
hound = "3.5"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-log = "0.2"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_Media_Audio",
    "Win32_System_Com",
    "Win32_Foundation",
] }
```

> **Note:** The `windows` crate version should be the latest stable. Check with `cargo search windows` if 0.58 is not current. The feature list covers `IMMDeviceEnumerator`, `IAudioClient`, `IAudioCaptureClient`, `CoInitializeEx`, `CoCreateInstance`, `CoTaskMemFree`, `CoUninitialize`, and the audio endpoint constants (`eRender`, `eCapture`, `eConsole`, etc.).

**Step 3: Create core/src/lib.rs placeholder**

```rust
pub mod error;

pub use error::Result;
```

**Step 4: Create core/src/error.rs placeholder**

```rust
pub type Result<T> = std::result::Result<T, RecorderError>;

#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
```

**Step 5: Create CLI crate Cargo.toml**

```toml
[package]
name = "audio-recorder-manager-cli"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "audio-recorder-manager"
path = "src/main.rs"

[dependencies]
audio-recorder-manager-core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
```

**Step 6: Create cli/src/main.rs placeholder**

```rust
#[tokio::main]
async fn main() {
    println!("Audio Recorder Manager CLI");
}
```

**Step 7: Verify workspace compiles**

Run: `cargo build`
Expected: Compiles successfully with no errors.

**Step 8: Commit**

```bash
git init
git add Cargo.toml core/ cli/ CLAUDE.md reference/ docs/ mute-feature-plan.md
git commit -m "feat: scaffold workspace with core and cli crates"
```

---

## Task 2: Error Module

**Files:**
- Modify: `core/src/error.rs`

The error module is used throughout the codebase. We need `RecorderError` with variants matching what reference code uses, plus conversions from common error types.

**Step 1: Implement error module**

```rust
// core/src/error.rs
use thiserror::Error;

pub type Result<T> = std::result::Result<T, RecorderError>;

#[derive(Debug, Error)]
pub enum RecorderError {
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
```

**Step 2: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 3: Commit**

```bash
git add core/src/error.rs
git commit -m "feat: implement error types for core crate"
```

---

## Task 3: Audio Utils

**Files:**
- Create: `core/src/audio_utils.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/audio_utils.rs`

Port the RMS calculation functions directly from reference. These are used by both WASAPI recorders to detect whether audio is present.

**Step 1: Create audio_utils.rs**

Port directly from `reference/audio_utils.rs`. The file contains three functions:
- `calculate_rms_i16(samples: &[i16]) -> f32`
- `calculate_rms_i32(samples: &[i32]) -> f32`
- `calculate_rms_f32(samples: &[f32]) -> f32`

All follow the same pattern: sum of squares divided by count, then sqrt.

**Step 2: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod error;

pub use error::Result;
```

**Step 3: Write tests**

Add to `core/src/audio_utils.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_f32_silence() {
        assert_eq!(calculate_rms_f32(&[0.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn test_rms_f32_signal() {
        let rms = calculate_rms_f32(&[1.0, -1.0, 1.0, -1.0]);
        assert!((rms - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_rms_i16_silence() {
        assert_eq!(calculate_rms_i16(&[0, 0, 0]), 0.0);
    }

    #[test]
    fn test_rms_empty() {
        assert_eq!(calculate_rms_f32(&[]), 0.0);
        assert_eq!(calculate_rms_i16(&[]), 0.0);
        assert_eq!(calculate_rms_i32(&[]), 0.0);
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p audio-recorder-manager-core`
Expected: All tests pass.

**Step 5: Commit**

```bash
git add core/src/audio_utils.rs core/src/lib.rs
git commit -m "feat: add audio RMS utility functions"
```

---

## Task 4: Domain Types

**Files:**
- Create: `core/src/domain.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/domain.rs`

This module defines `AudioFormat`, `RecordingDuration`, `SessionId`, and `RecordingSession`. It depends on `error` (already done) and `recorder::RecordingQuality` (not yet done). To break the circular dependency, we'll forward-declare the dependency on `RecordingQuality` and implement `RecordingSession` after Task 10 (recorder). For now, implement everything except `RecordingSession`.

**Step 1: Create domain.rs with AudioFormat, RecordingDuration, SessionId**

Port from `reference/domain.rs`. Include:
- `AudioFormat` enum (Wav, M4a) with Display, FromStr, Serialize, Deserialize, extension(), codec()
- `RecordingDuration` enum (Fixed, Manual) with from_secs(), effective_duration(), to_api_value()
- `SessionId` newtype with new(), as_str(), Display, From<String>, From<&str>, Default

Leave `RecordingSession` with a `TODO` comment — it requires `RecordingQuality` which is defined in `recorder.rs` (Task 10).

**Step 2: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod domain;
pub mod error;

pub use error::Result;
```

**Step 3: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_from_str() {
        assert_eq!(AudioFormat::from_str("wav").unwrap(), AudioFormat::Wav);
        assert_eq!(AudioFormat::from_str("M4A").unwrap(), AudioFormat::M4a);
        assert!(AudioFormat::from_str("mp3").is_err());
    }

    #[test]
    fn test_audio_format_extension() {
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert_eq!(AudioFormat::M4a.extension(), "m4a");
    }

    #[test]
    fn test_recording_duration() {
        let fixed = RecordingDuration::from_secs(30, 7200).unwrap();
        assert_eq!(fixed.effective_duration(), 30);
        assert_eq!(fixed.to_api_value(), 30);

        let manual = RecordingDuration::from_secs(-1, 7200).unwrap();
        assert_eq!(manual.effective_duration(), 7200);
        assert_eq!(manual.to_api_value(), -1);

        assert!(RecordingDuration::from_secs(0, 7200).is_err());
        assert!(RecordingDuration::from_secs(-2, 7200).is_err());
    }

    #[test]
    fn test_session_id_format() {
        let id = SessionId::new();
        assert!(id.as_str().starts_with("rec-"));
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p audio-recorder-manager-core`
Expected: All tests pass.

**Step 5: Commit**

```bash
git add core/src/domain.rs core/src/lib.rs
git commit -m "feat: add domain types (AudioFormat, RecordingDuration, SessionId)"
```

---

## Task 5: Config Module

**Files:**
- Create: `core/src/config.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/config.rs`

Port `RecorderConfig` with its storage directory resolution, JSON config loading/saving, and directory creation. This module uses `anyhow`, `serde`, `dirs`, and `std::fs`.

**Step 1: Create config.rs**

Port directly from `reference/config.rs`. Key struct:
- `RecorderConfig` with fields: storage_dir, recordings_dir, transcriptions_dir, status_dir, signals_dir, max_manual_duration_secs, status_update_interval, file_write_delay_ms
- `Default` implementation that finds workspace root
- `from_storage_dir()`, `new()`, `ensure_directories()`, `load()`, `save()`

**Step 2: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod config;
pub mod domain;
pub mod error;

pub use error::Result;
```

**Step 3: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add core/src/config.rs core/src/lib.rs
git commit -m "feat: add RecorderConfig with storage directory management"
```

---

## Task 6: Output Module

**Files:**
- Create: `core/src/output.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/output.rs`

Port `UserOutput` with its quiet/color modes and message formatting methods. This module handles user-facing terminal output separately from tracing/logging.

**Step 1: Create output.rs**

Port directly from `reference/output.rs`. Key struct:
- `UserOutput` with quiet and use_color fields
- Methods: new(), quiet(), with_color(), info(), success(), warning(), error(), progress(), prefixed(), status(), clear_status()
- Color support detection (NO_COLOR, TERM, WT_SESSION, ConEmuANSI env vars)

**Step 2: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod config;
pub mod domain;
pub mod error;
pub mod output;

pub use error::Result;
```

**Step 3: Verify compiles and run existing tests**

Run: `cargo test -p audio-recorder-manager-core`
Expected: All tests pass.

**Step 4: Commit**

```bash
git add core/src/output.rs core/src/lib.rs
git commit -m "feat: add UserOutput for terminal messaging"
```

---

## Task 7: Logging Module

**Files:**
- Create: `core/src/logging.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/logging.rs`

Port the dual-output logging system with daily rolling file appender. This is used by the CLI binary's main.rs to initialize tracing.

**Step 1: Create logging.rs**

Port directly from `reference/logging.rs`. Key components:
- `get_log_dir()` — returns `%APPDATA%\audio-recorder-manager\logs\`
- `cleanup_old_logs()` — removes logs older than retention period
- `parse_log_level()` — string to `tracing::Level`
- `CustomDailyRollingWriter` + `DailyRollingAppender` — custom file appender with daily rotation
- `init_cli_logging()` — sets up file + terminal layers with `EnvFilter`
- `init_test_logging()` — test-only logging setup

**Step 2: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod config;
pub mod domain;
pub mod error;
pub mod logging;
pub mod output;

pub use error::Result;
```

**Step 3: Run tests (includes logging tests from reference)**

Run: `cargo test -p audio-recorder-manager-core`
Expected: All tests pass.

**Step 4: Commit**

```bash
git add core/src/logging.rs core/src/lib.rs
git commit -m "feat: add dual-output logging with daily rotation"
```

---

## Task 8: Status Types

**Files:**
- Create: `core/src/status.rs`
- Modify: `core/src/lib.rs`

This module defines the status reporting types used by the recording loop and FFmpeg processing. These types are NOT in the reference files, so we must infer them from usage in `record.rs`, `recorder.rs`, and `ffmpeg_encoder.rs`.

**Step 1: Create status.rs with RecordingStatus, RecordingResult, StatusObserver, JsonFileObserver**

From usage in reference code, the types are:

```rust
// core/src/status.rs
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Recording progress data, written to JSON status file every second
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
    pub session_id: String,
    pub filename: String,
    pub duration: u64,
    pub elapsed: u64,
    pub progress: u8,
    pub quality: String,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames_captured: u64,
    pub has_audio: bool,
    pub status: String,
    // Per-channel data
    pub loopback_frames: Option<u64>,
    pub loopback_has_audio: Option<bool>,
    pub mic_frames: Option<u64>,
    pub mic_has_audio: Option<bool>,
    // Mute state (new for mute feature)
    pub is_mic_muted: Option<bool>,
    // FFmpeg progress
    pub ffmpeg_progress: Option<u8>,
    pub processing_speed: Option<String>,
}

/// Final recording result, written when recording completes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingResult {
    pub session_id: String,
    pub filename: String,
    pub file_path: Option<String>,
    pub duration: i64,
    pub file_size_mb: String,
    pub format: String,
    pub codec: String,
    pub status: String,
    pub message: String,
}

/// Trait for observing recording status changes
pub trait StatusObserver: Send + Sync {
    fn on_progress(&self, status: RecordingStatus) -> crate::error::Result<()>;
    fn on_complete(&self, result: RecordingResult) -> crate::error::Result<()>;
}

/// Writes status JSON files to the status directory
pub struct JsonFileObserver {
    status_dir: PathBuf,
}

impl JsonFileObserver {
    pub fn new(status_dir: PathBuf) -> Self {
        Self { status_dir }
    }

    /// Write processing status with stage information (used during FFmpeg post-processing)
    pub fn write_processing_status_v2(
        &self,
        session_id: &str,
        message: &str,
        current_stage: Option<u8>,
        total_stages: Option<u8>,
        stage_name: Option<&str>,
        ffmpeg_progress: Option<u8>,
        processing_speed: Option<String>,
    ) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "processing",
            "message": message,
            "current_stage": current_stage,
            "total_stages": total_stages,
            "stage_name": stage_name,
            "ffmpeg_progress": ffmpeg_progress,
            "processing_speed": processing_speed,
        });
        self.write_status_file(session_id, &status)
    }

    /// Update FFmpeg merge/encode progress
    pub fn update_ffmpeg_progress(
        &self,
        session_id: &str,
        progress_pct: u8,
        speed: Option<String>,
        total_duration_ms: Option<u64>,
        current_time_ms: Option<u64>,
        estimated_remaining_secs: Option<u64>,
    ) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "processing",
            "ffmpeg_progress": progress_pct,
            "processing_speed": speed,
            "total_duration_ms": total_duration_ms,
            "current_time_ms": current_time_ms,
            "estimated_remaining_secs": estimated_remaining_secs,
        });
        self.write_status_file(session_id, &status)
    }

    /// Write compression progress (used by ffmpeg_encoder compress function)
    pub fn write_compression_progress(
        &self,
        session_id: &str,
        progress_pct: u8,
        speed: Option<String>,
        total_duration_ms: Option<u64>,
        current_time_ms: Option<u64>,
        estimated_remaining_secs: Option<u64>,
    ) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "compressing",
            "compression_progress": progress_pct,
            "processing_speed": speed,
            "total_duration_ms": total_duration_ms,
            "current_time_ms": current_time_ms,
            "estimated_remaining_secs": estimated_remaining_secs,
        });
        self.write_status_file(session_id, &status)
    }

    /// Mark FFmpeg processing as complete
    pub fn mark_ffmpeg_complete(&self, session_id: &str) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "processing",
            "ffmpeg_complete": true,
            "ffmpeg_progress": 100,
        });
        self.write_status_file(session_id, &status)
    }

    /// Internal: write a JSON value to the session's status file
    fn write_status_file(
        &self,
        session_id: &str,
        value: &serde_json::Value,
    ) -> crate::error::Result<()> {
        let path = self.status_dir.join(format!("{}.json", session_id));
        let content = serde_json::to_string_pretty(value)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}

impl StatusObserver for JsonFileObserver {
    fn on_progress(&self, status: RecordingStatus) -> crate::error::Result<()> {
        let value = serde_json::to_value(&status)?;
        self.write_status_file(&status.session_id, &value)
    }

    fn on_complete(&self, result: RecordingResult) -> crate::error::Result<()> {
        let value = serde_json::to_value(&result)?;
        self.write_status_file(&result.session_id, &value)
    }
}
```

**Step 2: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod config;
pub mod domain;
pub mod error;
pub mod logging;
pub mod output;
pub mod status;

pub use error::Result;
```

**Step 3: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add core/src/status.rs core/src/lib.rs
git commit -m "feat: add status types (RecordingStatus, JsonFileObserver)"
```

---

## Task 9: FFmpeg Encoder

**Files:**
- Create: `core/src/ffmpeg_encoder.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/ffmpeg_encoder.rs`

Port the FFmpeg integration module. This handles audio duration detection, WAV-to-M4A conversion, and compression. It uses `tokio::process::Command` with `CREATE_NO_WINDOW` on Windows.

**Step 1: Create ffmpeg_encoder.rs**

Port directly from `reference/ffmpeg_encoder.rs`. Key functions:
- `get_audio_duration_ms()` — multi-fallback duration detection (WAV header → FFprobe format → FFprobe stream → file size estimation)
- `try_wav_header_duration()` — direct RIFF/WAVE binary parsing
- `try_ffprobe_format_duration()` / `try_ffprobe_stream_duration()` — FFprobe subprocess calls
- `estimate_duration_from_filesize()` — last resort estimation
- `convert_wav_to_m4a_optimized()` / `convert_wav_to_m4a_with_progress()` — WAV→M4A encoding with progress monitoring
- `compress_audio_for_transcription()` — compression with quality levels
- `estimate_compression()` — predict compression results
- `CompressionQuality` enum, `CompressionEstimate` struct, `FfmpegProgress` struct

**Important:** All `Command::new("ffmpeg")` and `Command::new("ffprobe")` calls must use:
```rust
#[cfg(windows)]
cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW - prevents console flashing
```

Import note: Replace `crate::status::JsonFileObserver` references appropriately since status.rs is now in the same crate.

**Step 2: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod config;
pub mod domain;
pub mod error;
pub mod ffmpeg_encoder;
pub mod logging;
pub mod output;
pub mod status;

pub use error::Result;
```

**Step 3: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add core/src/ffmpeg_encoder.rs core/src/lib.rs
git commit -m "feat: add FFmpeg encoder with duration detection and M4A conversion"
```

---

## Task 10: Recorder (Merge Logic + Quality Presets)

**Files:**
- Create: `core/src/recorder.rs`
- Modify: `core/src/lib.rs`
- Modify: `core/src/domain.rs` (add RecordingSession now that RecordingQuality exists)

**Reference:** `reference/recorder.rs`

Port `RecordingQuality` and `merge_audio_streams_smart()`. Then complete `domain.rs` by adding `RecordingSession`.

**Step 1: Create recorder.rs**

Port from `reference/recorder.rs`. Key components:
- `RecordingQuality` struct with factory methods: professional(), quick(), standard(), high()
- `convert_wav_to_m4a()` — thin wrapper to `ffmpeg_encoder::convert_wav_to_m4a_optimized()`
- `merge_audio_streams_smart()` — the 4-scenario merge function (~460 lines)

The merge function handles:
- Scenario A: Both loopback + mic have audio → dual-mono stereo via amerge+pan
- Scenario B: Loopback only → duplicate to stereo via -ac 2
- Scenario C: Mic only → duplicate via asplit+amerge
- Scenario D: Both silent → silent stereo via aformat

All scenarios include:
- `OnceLock` FFmpeg availability check
- Parallel duration detection via `tokio::join!`
- Real-time progress monitoring via `-progress pipe:2`
- `CREATE_NO_WINDOW` on Windows
- Stage transitions via observer

**Step 2: Complete domain.rs — add RecordingSession**

Now that `recorder::RecordingQuality` exists, add `RecordingSession` to `domain.rs`:

```rust
#[derive(Debug, Clone)]
pub struct RecordingSession {
    pub id: SessionId,
    pub format: AudioFormat,
    pub quality: crate::recorder::RecordingQuality,
    pub duration: RecordingDuration,
    pub started_at: DateTime<Local>,
}

impl RecordingSession {
    pub fn new(
        format: AudioFormat,
        quality: crate::recorder::RecordingQuality,
        duration: RecordingDuration,
    ) -> Self {
        Self {
            id: SessionId::new(),
            format,
            quality,
            duration,
            started_at: Local::now(),
        }
    }

    pub fn filename(&self) -> String {
        let timestamp = self.started_at.format("%Y-%m-%d-%H_%M");
        format!("{}_recording.{}", timestamp, self.format.extension())
    }

    pub fn temp_filename(&self) -> String {
        let timestamp = self.started_at.format("%Y-%m-%d-%H_%M");
        format!("{}_recording.wav", timestamp)
    }
}
```

**Step 3: Add module to lib.rs**

```rust
pub mod audio_utils;
pub mod config;
pub mod domain;
pub mod error;
pub mod ffmpeg_encoder;
pub mod logging;
pub mod output;
pub mod recorder;
pub mod status;

pub use error::Result;
```

**Step 4: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 5: Commit**

```bash
git add core/src/recorder.rs core/src/domain.rs core/src/lib.rs
git commit -m "feat: add recording quality presets and smart audio merge"
```

---

## Task 11: WASAPI Loopback Recorder

**Files:**
- Create: `core/src/wasapi/mod.rs`
- Create: `core/src/wasapi/loopback.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/wasapi_loopback.rs`

Port the WASAPI loopback recorder. This captures system audio (speakers/output) via the Windows Audio Session API. It runs a recording thread that writes 16-bit PCM WAV via `hound`.

**Step 1: Create wasapi/mod.rs**

```rust
#[cfg(windows)]
pub mod loopback;

#[cfg(windows)]
pub mod microphone;
```

**Step 2: Create wasapi/loopback.rs**

Port from `reference/wasapi_loopback.rs` but **remove the outer `pub mod windows_loopback` wrapper** — the module system handles this via the file path. Keep the contents of the inner module as the top-level file content.

Key struct: `WasapiLoopbackRecorder`
- `new(filepath: PathBuf) -> Result<Self>` — initializes COM, gets default render endpoint's sample rate, spawns recording thread
- `recording_thread()` — the main capture loop: COM init → get device → get mix format → initialize audio client in loopback mode → capture loop → write 16-bit WAV
- `get_frames_captured()`, `has_audio_detected()`, `get_sample_rate()`, `stop()`

**Important adaptations from reference:**
- Remove `pub mod windows_loopback { ... }` wrapper — use flat module structure
- Change `use crate::audio_utils::*` to match new module path
- Keep `#[cfg(windows)]` at the module level in `wasapi/mod.rs`, not inside the file
- All `unsafe` blocks follow the safety documentation in the reference comments

**Step 3: Add wasapi module to lib.rs**

```rust
pub mod audio_utils;
pub mod config;
pub mod domain;
pub mod error;
pub mod ffmpeg_encoder;
pub mod logging;
pub mod output;
pub mod recorder;
pub mod status;
pub mod wasapi;

pub use error::Result;
```

**Step 4: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully on Windows. On non-Windows, the wasapi module is conditionally compiled out.

**Step 5: Commit**

```bash
git add core/src/wasapi/
git commit -m "feat: add WASAPI loopback recorder for system audio capture"
```

---

## Task 12: WASAPI Microphone Recorder (with Mute Support)

**Files:**
- Create: `core/src/wasapi/microphone.rs`

**Reference:** `reference/wasapi_microphone.rs` + `mute-feature-plan.md`

Port the WASAPI microphone recorder AND integrate the mute feature from the plan. The mic recorder captures from the default input device and writes 16-bit PCM WAV. When muted, it writes zeros instead of real samples.

**Step 1: Create wasapi/microphone.rs**

Port from `reference/wasapi_microphone.rs` with these modifications per `mute-feature-plan.md`:

1. **Add `is_muted: Arc<AtomicBool>` field** to `WasapiMicrophoneRecorder`
2. **Change constructor** to accept `is_muted: Arc<AtomicBool>` parameter
3. **Clone the flag** into the recording thread
4. **Add mute check** in the capture loop — when `is_muted.load(Ordering::Relaxed)` is true, write zeros instead of real samples

Key changes from reference:

```rust
pub struct WasapiMicrophoneRecorder {
    is_recording: Arc<AtomicBool>,
    frames_captured: Arc<AtomicU64>,
    has_audio: Arc<AtomicBool>,
    is_muted: Arc<AtomicBool>,  // NEW
}

impl WasapiMicrophoneRecorder {
    pub fn new(
        filepath: PathBuf,
        target_sample_rate: u32,
        is_muted: Arc<AtomicBool>,  // NEW parameter
    ) -> Result<Self> {
        // ... existing init ...
        let is_muted_clone = Arc::clone(&is_muted);  // NEW

        std::thread::spawn(move || {
            if let Err(e) = Self::recording_thread(
                filepath,
                target_sample_rate,
                is_recording_clone,
                frames_captured_clone,
                has_audio_clone,
                is_muted_clone,  // NEW
            ) { /* ... */ }
        });

        Ok(Self {
            is_recording,
            frames_captured,
            has_audio,
            is_muted,  // NEW
        })
    }
}
```

In the recording thread, add the mute check inside the capture loop, BEFORE writing samples:

```rust
// Inside the recording loop, after GetBuffer and before writing:
if is_muted.load(Ordering::Relaxed) {
    // Write silence (zeros) for the number of frames
    if let Some(writer) = writer.as_mut() {
        for _ in 0..(num_frames * channels as u32) {
            let _ = writer.write_sample(0i16);
        }
    }
} else {
    // existing sample processing code (float/int16/etc.)
}
```

Also add a convenience getter:
```rust
pub fn is_muted(&self) -> bool {
    self.is_muted.load(Ordering::Relaxed)
}
```

**Step 2: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 3: Commit**

```bash
git add core/src/wasapi/microphone.rs
git commit -m "feat: add WASAPI microphone recorder with mute support"
```

---

## Task 13: Commands — Status

**Files:**
- Create: `core/src/commands/mod.rs`
- Create: `core/src/commands/status.rs`
- Modify: `core/src/lib.rs`

**Reference:** `reference/status.rs` (the command, not the types)

Port the simple status command that reports platform capabilities.

**Step 1: Create commands/mod.rs**

```rust
pub mod status;
```

**Step 2: Create commands/status.rs**

Port directly from `reference/status.rs`. The function outputs JSON:
- On Windows: `{"status": "success", "data": {"message": "Using WASAPI...", "platform": "Windows"}}`
- On other platforms: `{"status": "error", "error": "Recording is currently only supported on Windows..."}`

**Step 3: Add commands module to lib.rs**

```rust
pub mod audio_utils;
pub mod commands;
pub mod config;
pub mod domain;
pub mod error;
pub mod ffmpeg_encoder;
pub mod logging;
pub mod output;
pub mod recorder;
pub mod status;
pub mod wasapi;

pub use error::Result;
```

**Step 4: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 5: Commit**

```bash
git add core/src/commands/
git commit -m "feat: add status command"
```

---

## Task 14: Commands — Stop

**Files:**
- Create: `core/src/commands/stop.rs`
- Modify: `core/src/commands/mod.rs`

**Reference:** `reference/stop.rs`

Port the stop command. It finds the active recording session and writes a `.stop` signal file.

**Step 1: Create commands/stop.rs**

Port directly from `reference/stop.rs`. Key functions:
- `execute(session_id: Option<String>, config: RecorderConfig) -> Result<()>` — writes `{session_id}.stop` to signals_dir
- `find_active_session(status_dir: &PathBuf) -> Result<String>` — scans status JSON files for `status == "recording"`, returns latest session_id

**Step 2: Update commands/mod.rs**

```rust
pub mod status;
pub mod stop;
```

**Step 3: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add core/src/commands/stop.rs core/src/commands/mod.rs
git commit -m "feat: add stop command with signal file IPC"
```

---

## Task 15: Commands — Mute (New)

**Files:**
- Create: `core/src/commands/mute.rs`
- Modify: `core/src/commands/mod.rs`

**Reference:** `mute-feature-plan.md` Step 4

Create the mute toggle command. It follows the same pattern as stop — find active session, write signal file.

**Step 1: Create commands/mute.rs**

```rust
use anyhow::Context;
use serde_json::json;
use std::path::PathBuf;

use crate::config::RecorderConfig;
use crate::error::Result;
use crate::status::RecordingStatus;

/// Toggle mute for a recording session's microphone.
/// Writes a signal file that the recording loop picks up.
pub async fn execute(session_id: Option<String>, config: RecorderConfig) -> Result<()> {
    config.ensure_directories()?;

    // Find target session
    let target_session = if let Some(id) = session_id {
        id
    } else {
        find_active_session(&config.status_dir)?
    };

    // Write the mute toggle signal file
    let signal_path = config
        .signals_dir
        .join(format!("{}.mute", target_session));
    std::fs::write(&signal_path, "")
        .with_context(|| format!("Failed to create mute signal file: {:?}", signal_path))?;

    // Return success
    let result = json!({
        "status": "success",
        "data": {
            "session_id": target_session,
            "message": "Mute toggle signal sent successfully"
        }
    });

    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn find_active_session(status_dir: &PathBuf) -> Result<String> {
    use std::fs;

    let entries = fs::read_dir(status_dir).context("Failed to read status directory")?;

    let mut active_sessions = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(status) = serde_json::from_str::<RecordingStatus>(&content) {
                if status.status == "recording" {
                    active_sessions.push((status.session_id, path));
                }
            }
        }
    }

    if active_sessions.is_empty() {
        return Err(anyhow::anyhow!("No active recording sessions found").into());
    }

    active_sessions.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(active_sessions[0].0.clone())
}
```

**Step 2: Update commands/mod.rs**

```rust
pub mod mute;
pub mod status;
pub mod stop;
```

**Step 3: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add core/src/commands/mute.rs core/src/commands/mod.rs
git commit -m "feat: add mute toggle command"
```

---

## Task 16: Commands — Record

**Files:**
- Create: `core/src/commands/record.rs`
- Modify: `core/src/commands/mod.rs`

**Reference:** `reference/record.rs` + `mute-feature-plan.md` Steps 2a-2d

This is the largest and most complex module. Port the recording lifecycle with mute support integrated.

**Step 1: Create commands/record.rs**

Port from `reference/record.rs` with these modifications for mute support:

**1a. Import changes:**
```rust
use std::sync::atomic::{AtomicBool, Ordering};
// ... existing imports ...
#[cfg(windows)]
use crate::wasapi::microphone::WasapiMicrophoneRecorder;
#[cfg(windows)]
use crate::wasapi::loopback::WasapiLoopbackRecorder;
```

Note: The WASAPI import paths change from `crate::wasapi_loopback::windows_loopback::WasapiLoopbackRecorder` to `crate::wasapi::loopback::WasapiLoopbackRecorder` (and similarly for microphone).

**1b. Create mute flag and pass to mic recorder:**
```rust
// Create the shared mute flag (initially unmuted)
let is_mic_muted = Arc::new(AtomicBool::new(false));

// Initialize microphone recorder with the mute flag
let mic_recorder = match WasapiMicrophoneRecorder::new(
    mic_temp.clone(),
    target_sample_rate,
    is_mic_muted.clone(),  // NEW parameter
) {
    Ok(recorder) => { /* ... */ Some(recorder) }
    Err(e) => { /* ... */ None }
};
```

**1c. Add `is_mic_muted` to RecordingStatus writes:**

In both the initial status write and the loop status write, add:
```rust
is_mic_muted: Some(is_mic_muted.load(Ordering::Relaxed)),
```

**1d. Add mute signal polling in recording loop:**

After the stop signal check, add:
```rust
// Check for mute toggle signal
let mute_signal = config
    .signals_dir
    .join(format!("{}.mute", session.id.as_str()));
if mute_signal.exists() {
    let _ = std::fs::remove_file(&mute_signal);
    if mic_recorder.is_some() {
        let was_muted = is_mic_muted.fetch_xor(true, Ordering::Relaxed);
        let now_muted = !was_muted;
        if now_muted {
            tracing::info!("Microphone muted for session {}", session.id);
            output.prefixed("Mute", "Microphone muted");
        } else {
            tracing::info!("Microphone unmuted for session {}", session.id);
            output.prefixed("Mute", "Microphone unmuted");
        }
    } else {
        tracing::warn!("Mute signal ignored - no microphone is active");
    }
}
```

**1e. Update progress display to show mute state:**

```rust
if let Some(ref mic) = mic_recorder {
    let mute_indicator = if is_mic_muted.load(Ordering::Relaxed) {
        "MUTED"
    } else if mic.has_audio_detected() {
        "Audio"
    } else {
        "Silent"
    };
    // ... format with mute_indicator instead of plain audio detection
}
```

**1f. Clean up stale mute signal in cleanup section:**

```rust
// After post-processing, alongside temp file cleanup:
let _ = std::fs::remove_file(&loopback_temp);
let _ = std::fs::remove_file(&mic_temp);
// Clean up any stale mute signal
let mute_signal = config.signals_dir.join(format!("{}.mute", session.id.as_str()));
let _ = std::fs::remove_file(&mute_signal);
```

**Step 2: Update commands/mod.rs**

```rust
pub mod mute;
pub mod record;
pub mod status;
pub mod stop;
```

**Step 3: Verify compiles**

Run: `cargo build -p audio-recorder-manager-core`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add core/src/commands/record.rs core/src/commands/mod.rs
git commit -m "feat: add record command with mute signal polling"
```

---

## Task 17: CLI Binary

**Files:**
- Modify: `cli/src/main.rs`
- Create: `cli/src/cli.rs`

**Reference:** `reference/main.rs` + `reference/cli.rs`

Port the CLI entry point and command dispatcher, adding the mute command.

**Step 1: Create cli/src/cli.rs**

Port from `reference/cli.rs` with these modifications:
- Remove the `recover` command (out of scope)
- Add `"mute"` match arm
- Update import paths (no change needed since we use the same crate name)

```rust
use std::str::FromStr;

use audio_recorder_manager_core::commands;
use audio_recorder_manager_core::config::RecorderConfig;
use audio_recorder_manager_core::domain::{AudioFormat, RecordingDuration};
use audio_recorder_manager_core::error::Result;
use audio_recorder_manager_core::output::UserOutput;
use audio_recorder_manager_core::recorder::RecordingQuality;

pub async fn run(args: Vec<String>) -> Result<()> {
    let output = UserOutput::new();

    if args.len() <= 1 {
        print_usage();
        return Ok(());
    }

    let command = &args[1];

    match command.as_str() {
        "record" => handle_record_command(&args, &output).await,
        "status" => commands::status::execute().await,
        "stop" => handle_stop_command(&args, &output).await,
        "mute" => handle_mute_command(&args, &output).await,  // NEW
        _ => {
            output.error(&format!("Unknown command: {}", command));
            print_usage();
            std::process::exit(1);
        }
    }
}

// ... handle_record_command and handle_stop_command ported from reference ...

async fn handle_mute_command(args: &[String], _output: &UserOutput) -> Result<()> {
    let config = RecorderConfig::new();
    let session_id = if args.len() > 2 {
        Some(args[2].clone())
    } else {
        None
    };

    commands::mute::execute(session_id, config).await
}

fn print_usage() {
    println!("============================================================");
    println!("Audio Recorder Manager - Rust Edition");
    println!("============================================================");
    println!();
    println!("Usage:");
    println!("  audio-recorder-manager record <duration> [format] [quality]");
    println!("  audio-recorder-manager stop [session_id]");
    println!("  audio-recorder-manager mute [session_id]");
    println!("  audio-recorder-manager status");
    println!();
    println!("Examples:");
    println!("  audio-recorder-manager record 30 wav           # Record for 30 seconds");
    println!("  audio-recorder-manager record -1 m4a standard  # Manual mode");
    println!("  audio-recorder-manager stop                    # Stop latest recording");
    println!("  audio-recorder-manager mute                    # Toggle mic mute on active recording");
    println!("  audio-recorder-manager status                  # Show system audio status");
    println!();
}
```

**Step 2: Update cli/src/main.rs**

Port from `reference/main.rs`:

```rust
mod cli;

use audio_recorder_manager_core::logging;
use audio_recorder_manager_core::Result;
use std::env;
use tracing::Level;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let log_level = parse_log_level_from_args(&args);
    let (log_to_file, log_to_terminal) = parse_log_output_from_args(&args);

    if let Err(e) = logging::init_cli_logging(log_level, log_to_file, log_to_terminal) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }

    tracing::info!("Audio Recorder Manager CLI started");

    let result = cli::run(args).await;

    if result.is_ok() {
        tracing::info!("CLI completed successfully");
    } else {
        tracing::error!("CLI completed with error");
    }

    result
}

fn parse_log_level_from_args(args: &[String]) -> Option<Level> {
    for (i, arg) in args.iter().enumerate() {
        if (arg == "--log-level" || arg == "-l") && i + 1 < args.len() {
            if let Some(level) = logging::parse_log_level(&args[i + 1]) {
                return Some(level);
            }
        }
    }
    None
}

fn parse_log_output_from_args(args: &[String]) -> (bool, bool) {
    let mut log_to_file = true;
    let mut log_to_terminal = true;

    for arg in args {
        match arg.as_str() {
            "--no-log-file" => log_to_file = false,
            "--no-log-terminal" => log_to_terminal = false,
            _ => {}
        }
    }

    (log_to_file, log_to_terminal)
}
```

**Step 3: Verify full workspace compiles**

Run: `cargo build`
Expected: Both crates compile successfully.

**Step 4: Test CLI runs**

Run: `cargo run --bin audio-recorder-manager`
Expected: Prints usage information.

Run: `cargo run --bin audio-recorder-manager -- status`
Expected: Prints JSON with WASAPI platform info.

**Step 5: Commit**

```bash
git add cli/src/main.rs cli/src/cli.rs
git commit -m "feat: add CLI binary with start/stop/mute/status commands"
```

---

## Task 18: End-to-End Smoke Test

**Files:** None (manual testing only)

Run through the complete recording lifecycle to verify everything works together.

**Step 1: Start a 10-second recording**

Run: `cargo run --bin audio-recorder-manager -- record 10 wav professional --no-log-terminal`
Expected: JSON output with session_id, recording runs for 10 seconds, creates WAV file in `storage/recordings/`

**Step 2: Verify status files are written**

Check: `ls storage/status/`
Expected: A `.json` file with the session ID, containing `"status": "recording"` during recording and `"status": "completed"` after.

**Step 3: Test manual mode with stop**

Terminal 1: `cargo run --bin audio-recorder-manager -- record -1 wav professional --no-log-terminal`
Terminal 2 (after a few seconds): `cargo run --bin audio-recorder-manager -- stop`
Expected: Recording stops, post-processing runs, final WAV created.

**Step 4: Test mute toggle**

Terminal 1: `cargo run --bin audio-recorder-manager -- record -1 wav professional --no-log-terminal`
Terminal 2 (after a few seconds): `cargo run --bin audio-recorder-manager -- mute`
Expected: JSON success response. Terminal 1 shows "Microphone muted". Status JSON shows `is_mic_muted: true`.

Terminal 2 (again): `cargo run --bin audio-recorder-manager -- mute`
Expected: Terminal 1 shows "Microphone unmuted". Status JSON shows `is_mic_muted: false`.

Terminal 2: `cargo run --bin audio-recorder-manager -- stop`
Expected: Recording stops and processes normally.

**Step 5: Verify output file**

Check the output WAV/M4A file exists and has non-zero size. If FFmpeg is installed, also test with M4A format:
Run: `cargo run --bin audio-recorder-manager -- record 5 m4a professional --no-log-terminal`

**Step 6: Final commit**

```bash
git add -A
git commit -m "chore: verify end-to-end recording lifecycle"
```

---

## Dependency Order Summary

```
Task 1: Workspace setup ──┐
Task 2: Error module ─────┤
Task 3: Audio utils ──────┤
Task 4: Domain types ─────┼──▶ Task 10: Recorder (completes RecordingSession)
Task 5: Config ───────────┤
Task 6: Output ───────────┤
Task 7: Logging ──────────┤
Task 8: Status types ─────┤
Task 9: FFmpeg encoder ───┘
Task 10: Recorder ────────┐
Task 11: WASAPI loopback ─┤
Task 12: WASAPI mic+mute ─┼──▶ Task 16: Record command
Task 13: Cmd: status ─────┤
Task 14: Cmd: stop ────────┤
Task 15: Cmd: mute ────────┘
Task 16: Record command ──▶ Task 17: CLI binary ──▶ Task 18: Smoke test
```

Tasks 2-9 can be done in any order (they only depend on Task 1).
Tasks 11-15 can be done in any order (they depend on Tasks 1-10).
Task 16 depends on Tasks 10-15.
Task 17 depends on Task 16.
Task 18 depends on Task 17.
