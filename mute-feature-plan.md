# Mute/Unmute Feature — Implementation Plan

## Architecture Overview

The recording pipeline works as follows:

```
CLI (cli.rs)
  └─ commands::record::execute_with_output()
       └─ record_worker()
            ├─ WasapiLoopbackRecorder  → {session_id}_loopback.wav
            ├─ WasapiMicrophoneRecorder → {session_id}_mic.wav
            ├─ Status loop (1s tick) — polls .stop / .cancel signal files
            └─ Post-processing: merge_audio_streams_smart() via FFmpeg
```

**Mute strategy (Approach 1 — Zero-Fill):** An `Arc<AtomicBool>` flag is shared with the mic recorder. When `true`, the mic's audio callback writes zeros instead of real samples. The loopback stream is never affected. The resulting mic WAV is a continuous, valid file with silent segments where mute was active.

**Signal mechanism:** Follows the existing pattern — a `{session_id}.mute` file in the signals directory acts as a toggle. The recording loop already polls for `.stop` and `.cancel`; we add `.mute` to the same polling cycle.

---

## Files to Modify

| # | File | Change |
|---|------|--------|
| 1 | `wasapi_microphone/windows_microphone.rs` | Accept `Arc<AtomicBool>`, zero-fill in callback |
| 2 | `commands/record.rs` | Create the flag, pass it, poll `.mute` signal, update status |
| 3 | `status.rs` (RecordingStatus struct) | Add `is_mic_muted: Option<bool>` field |
| 4 | `commands/stop.rs` (or new `commands/mute.rs`) | Write the `.mute` signal file |
| 5 | `cli.rs` | Add `mute` CLI subcommand |

---

## Step-by-Step Implementation

### Step 1 — `WasapiMicrophoneRecorder`: Accept and Use the Mute Flag

**File:** `src/wasapi_microphone/windows_microphone.rs`

The mic recorder already has a constructor signature like:

```rust
pub fn new(output_path: PathBuf, target_sample_rate: u32) -> Result<Self>
```

**1a. Change the constructor to accept the mute flag:**

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct WasapiMicrophoneRecorder {
    // ... existing fields ...
    is_muted: Arc<AtomicBool>,  // NEW
}

impl WasapiMicrophoneRecorder {
    pub fn new(
        output_path: PathBuf,
        target_sample_rate: u32,
        is_muted: Arc<AtomicBool>,  // NEW parameter
    ) -> Result<Self> {
        // ... existing init code ...
        // Store the flag in self
        Ok(Self {
            // ... existing fields ...
            is_muted,
        })
    }
}
```

**1b. Zero-fill in the audio capture callback:**

Inside the WASAPI capture loop (wherever raw samples are read from the mic device and written to the WAV file), add the mute check. The exact location depends on how the capture thread is structured, but the pattern is:

```rust
// This is inside the capture thread/callback that processes mic audio buffers.
// The buffer variable holds the raw PCM samples from the mic device.

// --- INSERT THIS BLOCK right after obtaining the buffer, before writing ---
if self.is_muted.load(Ordering::Relaxed) {
    // Overwrite entire buffer with silence (zeros)
    // For f32 samples:
    for sample in buffer.iter_mut() {
        *sample = 0.0_f32;
    }
    // For i16 samples (if that's what the WASAPI capture uses):
    // for sample in buffer.iter_mut() {
    //     *sample = 0_i16;
    // }
}
// --- END INSERT ---

// existing code: write buffer to WAV writer
wav_writer.write_samples(&buffer)?;
```

> **Important:** The `is_muted` flag must be cloned into whatever closure or thread owns the capture loop. If the recorder spawns a `std::thread::spawn` for capture, clone the `Arc` into that thread. Example:
>
> ```rust
> let is_muted = self.is_muted.clone(); // clone for the capture thread
> std::thread::spawn(move || {
>     loop {
>         // ... read from WASAPI ...
>         if is_muted.load(Ordering::Relaxed) {
>             buffer.fill(0);
>         }
>         // ... write to WAV ...
>     }
> });
> ```

**1c. (Optional) Add a convenience getter:**

```rust
impl WasapiMicrophoneRecorder {
    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::Relaxed)
    }
}
```

---

### Step 2 — `record.rs`: Create, Pass, and Poll the Mute Flag

**File:** `src/commands/record.rs`

**2a. Create the shared flag and pass it to the mic recorder:**

Find the block where `WasapiMicrophoneRecorder::new` is called (around line ~110 of record.rs). Change it to:

```rust
use std::sync::atomic::{AtomicBool, Ordering};

// Create the shared mute flag (initially unmuted)
let is_mic_muted = Arc::new(AtomicBool::new(false));

// Initialize microphone recorder with the mute flag
let mic_recorder = match WasapiMicrophoneRecorder::new(
    mic_temp.clone(),
    target_sample_rate,
    is_mic_muted.clone(),  // pass the flag
) {
    Ok(recorder) => {
        tracing::info!("Microphone recorder initialized successfully");
        Some(recorder)
    }
    Err(e) => {
        tracing::warn!("Failed to initialize microphone: {}", e);
        output.warning("Microphone unavailable - recording system audio only");
        None
    }
};
```

**2b. Poll for the `.mute` signal file in the recording loop:**

Inside the existing `loop { ... }` that checks for `.stop` and `.cancel` signals, add this block immediately after the stop signal check:

```rust
// --- EXISTING: Check for stop signal ---
let stop_signal = config
    .signals_dir
    .join(format!("{}.stop", session.id.as_str()));
if stop_signal.exists() {
    tracing::info!("Stop signal received for session {}", session.id);
    let _ = std::fs::remove_file(stop_signal);
    break;
}

// --- NEW: Check for mute toggle signal ---
let mute_signal = config
    .signals_dir
    .join(format!("{}.mute", session.id.as_str()));
if mute_signal.exists() {
    let _ = std::fs::remove_file(&mute_signal);

    // Toggle: flip the boolean
    let was_muted = is_mic_muted.fetch_xor(true, Ordering::Relaxed);
    let now_muted = !was_muted;

    if now_muted {
        tracing::info!("🔇 Microphone muted for session {}", session.id);
        output.prefixed("Mute", "Microphone muted");
    } else {
        tracing::info!("🔊 Microphone unmuted for session {}", session.id);
        output.prefixed("Mute", "Microphone unmuted");
    }
}
```

**2c. Include mute state in status updates:**

In the `RecordingStatus` struct construction inside the loop (the `observer.on_progress(...)` call), add the new field:

```rust
observer.on_progress(RecordingStatus {
    session_id: session.id.as_str().to_string(),
    // ... all existing fields ...
    mic_has_audio: mic_recorder.as_ref().map(|m| m.has_audio_detected()),

    // NEW: mute state
    is_mic_muted: Some(is_mic_muted.load(Ordering::Relaxed)),

    // ... remaining fields ...
    ffmpeg_progress: None,
    processing_speed: None,
})?;
```

Do this for **both** status writes — the initial one (before the loop) and the one inside the loop. For the initial write, the value is `Some(false)`.

**2d. Update the progress display to show mute state:**

In the progress output line inside the loop, modify the mic status display:

```rust
if let Some(ref mic) = mic_recorder {
    let mute_indicator = if is_mic_muted.load(Ordering::Relaxed) {
        "🔇 MUTED"
    } else if mic.has_audio_detected() {
        "Audio"
    } else {
        "Silent"
    };

    output.prefixed(
        "Recording",
        &format!(
            "Progress: {}% | Elapsed: {}s / {}s | Loopback: {} frames ({}) | Mic: {} frames ({})",
            progress,
            elapsed,
            effective_duration,
            loopback_recorder.get_frames_captured(),
            if loopback_recorder.has_audio_detected() { "Audio" } else { "Silent" },
            mic.get_frames_captured(),
            mute_indicator,
        )
    );
}
```

---

### Step 3 — `status.rs`: Add Mute State to `RecordingStatus`

**File:** `src/status.rs`

Add the new field to the `RecordingStatus` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
    // ... all existing fields ...
    pub mic_has_audio: Option<bool>,

    // NEW
    /// Whether the microphone is currently muted (None if no mic available)
    pub is_mic_muted: Option<bool>,

    // ... remaining fields ...
    pub ffmpeg_progress: Option<u8>,
    pub processing_speed: Option<String>,
}
```

This field will be serialized into the JSON status file, making it available to any frontend (Tauri, web UI) that reads the status.

---

### Step 4 — Mute Command: Write the Signal File

**Option A: New file `src/commands/mute.rs` (recommended — cleanest separation)**

Create a new command module:

```rust
// src/commands/mute.rs

use crate::config::RecorderConfig;
use crate::error::Result;

/// Toggle mute for a recording session's microphone.
/// Works by writing a signal file that the recording loop picks up.
pub async fn execute(
    session_id: Option<String>,
    config: RecorderConfig,
) -> Result<()> {
    config.ensure_directories()?;

    // If no session_id provided, find the active session
    // (same pattern as commands::stop)
    let target_session_id = if let Some(id) = session_id {
        id
    } else {
        find_active_session(&config)?
    };

    // Write the mute toggle signal file
    let mute_signal_path = config
        .signals_dir
        .join(format!("{}.mute", target_session_id));

    std::fs::write(&mute_signal_path, "")?;

    tracing::info!(
        "Mute toggle signal written for session: {}",
        target_session_id
    );

    Ok(())
}

/// Find the currently active recording session by scanning status files.
/// This mirrors the logic in commands::stop.
fn find_active_session(config: &RecorderConfig) -> Result<String> {
    // Read status directory, find the file with status = "recording"
    // Parse JSON, extract session_id
    // This is the same logic that commands::stop already uses.
    // If commands::stop has a helper function for this, reuse it.
    // Otherwise, extract the shared logic into a common utility.

    let status_dir = &config.status_dir;
    for entry in std::fs::read_dir(status_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(status) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if status.get("status").and_then(|s| s.as_str()) == Some("recording") {
                        if let Some(id) = status.get("session_id").and_then(|s| s.as_str()) {
                            return Ok(id.to_string());
                        }
                    }
                }
            }
        }
    }

    Err(crate::error::RecorderError::Other(anyhow::anyhow!(
        "No active recording session found"
    )))
}
```

Then register it in `src/commands/mod.rs`:

```rust
pub mod mute;    // NEW
pub mod record;
pub mod recover;
pub mod status;
pub mod stop;
```

**Option B: Extend `commands/stop.rs`**

If you prefer not to create a new file, add a `mute` function to `stop.rs` that simply writes the `.mute` file. The logic is almost identical — just change the file extension from `.stop` to `.mute`. However, a separate module is cleaner since mute is semantically different from stop.

---

### Step 5 — `cli.rs`: Add the `mute` Subcommand

**File:** `src/cli.rs` (the binary crate's CLI)

**5a. Add the match arm:**

```rust
match command.as_str() {
    "record" => handle_record_command(&args, &output).await,
    "status" => commands::status::execute().await,
    "stop"   => handle_stop_command(&args, &output).await,
    "mute"   => handle_mute_command(&args, &output).await,  // NEW
    "recover" => handle_recover_command(&args, &output).await,
    _ => { /* existing error handling */ }
}
```

**5b. Add the handler function:**

```rust
async fn handle_mute_command(args: &[String], _output: &UserOutput) -> Result<()> {
    let config = RecorderConfig::new();

    // Optional session_id argument
    let session_id = if args.len() > 2 {
        Some(args[2].clone())
    } else {
        None
    };

    commands::mute::execute(session_id, config).await
}
```

**5c. Update `print_usage()`:**

```rust
fn print_usage() {
    // ... existing lines ...
    println!("  audio-recorder-manager mute [session_id]");
    // ...
    println!("  audio-recorder-manager mute                    # Toggle mic mute on active recording");
    println!("  audio-recorder-manager mute rec-20250109_120000 # Toggle mute on specific session");
    // ...
}
```

---

## Signal Flow Summary

```
User runs:  audio-recorder-manager mute
                │
                ▼
commands/mute.rs: writes "{session_id}.mute" file to signals_dir
                │
                ▼
commands/record.rs: recording loop detects .mute file (next 1s tick)
                │
                ├─ Deletes the signal file
                ├─ Flips is_mic_muted AtomicBool
                ├─ Logs mute/unmute event
                └─ Updates RecordingStatus JSON with is_mic_muted = true/false
                │
                ▼
wasapi_microphone: capture callback sees is_muted == true
                │
                ├─ Overwrites audio buffer with zeros
                └─ Writes zeros to mic WAV file (continuous, valid stream)
                │
                ▼
Post-processing: merge_audio_streams_smart() receives a normal WAV
                │
                └─ No changes needed — silent mic segments merge fine
```

---

## What Does NOT Need to Change

These files and systems are **unaffected** by this implementation:

| Component | Why it's unaffected |
|-----------|-------------------|
| `recorder.rs` / `merge_audio_streams_smart()` | Receives a valid WAV with silent segments; all 4 merge scenarios already handle silent mic input |
| `ffmpeg_encoder.rs` | Encodes whatever WAV it receives; silence is valid PCM data |
| `commands/recover.rs` | Recovery reads WAV files as-is; muted segments are just silence in the file |
| `WasapiLoopbackRecorder` | System audio is never muted — completely independent |
| `RecordingDuration` / timing logic | Muted time still counts as recording time; no duration adjustment needed |
| FFmpeg progress monitoring | `effective_duration_ms` is based on the WAV file length which includes muted (silent) segments, so progress % stays accurate |

---

## Edge Cases to Handle

1. **No mic available:** If `mic_recorder` is `None`, the mute signal should be silently ignored (the mic was never recording). Add a log:
   ```rust
   if mute_signal.exists() {
       let _ = std::fs::remove_file(&mute_signal);
       if mic_recorder.is_some() {
           // toggle logic from Step 2b
       } else {
           tracing::warn!("Mute signal ignored — no microphone is active");
       }
   }
   ```

2. **Mute signal arrives after recording ends:** The signal file would be an orphan. Clean it up alongside the temp WAV files at the end of `record_worker`:
   ```rust
   // Cleanup section (after post-processing)
   let _ = std::fs::remove_file(&loopback_temp);
   let _ = std::fs::remove_file(&mic_temp);
   // NEW: clean up any stale mute signal
   let mute_signal = config.signals_dir.join(format!("{}.mute", session.id.as_str()));
   let _ = std::fs::remove_file(&mute_signal);
   ```

3. **Rapid toggle (multiple .mute files before the 1s poll):** Since we delete the file on read and `fetch_xor` is atomic, even if two signals arrive within one tick, the second write will overwrite the first (empty file), and the loop will only see one file → one toggle. This is correct behavior. If you need guaranteed even/odd semantics, write `"mute"` or `"unmute"` as the file content instead of toggling:
   ```rust
   // In commands/mute.rs — write explicit state
   pub async fn execute(session_id: Option<String>, config: RecorderConfig, mute: bool) -> Result<()> {
       let content = if mute { "mute" } else { "unmute" };
       std::fs::write(&mute_signal_path, content)?;
       Ok(())
   }

   // In record.rs — read explicit state instead of toggling
   if mute_signal.exists() {
       let content = std::fs::read_to_string(&mute_signal).unwrap_or_default();
       let _ = std::fs::remove_file(&mute_signal);
       let should_mute = content.trim() != "unmute";
       is_mic_muted.store(should_mute, Ordering::Relaxed);
   }
   ```
   This is more robust for frontends (Tauri UI) that might send explicit mute/unmute rather than toggle.

4. **`has_audio_detected()` during mute:** The mic recorder's `has_audio_detected()` flag might still report `true` from audio captured *before* muting. This is fine — the merge function uses it to pick a strategy, and Scenario A (both have audio) handles a mostly-silent mic correctly. No change needed.

---

## Testing Checklist

- [ ] Record with mic + mute mid-recording → verify WAV has audible silence in the muted segment
- [ ] Record with mic + mute/unmute multiple times → verify transitions are clean (no clicks/pops)
- [ ] Record with no mic available + send mute signal → verify signal is ignored gracefully
- [ ] Record in manual mode (-1) + mute → verify status JSON has `is_mic_muted: true`
- [ ] Stop recording while muted → verify post-processing completes normally
- [ ] Cancel recording while muted → verify cleanup removes all temp files + signal files
- [ ] Mute command with no active session → verify clear error message
- [ ] Mute command with explicit session_id → verify correct session is targeted
- [ ] Check output file in Audacity: loopback channel should have continuous audio; mic channel should have silence in muted region
