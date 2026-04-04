# Audio Desync Investigation: System Audio Starts Earlier Than Microphone

## Issue Description

When recording with both system audio (loopback) and microphone enabled, the system audio in the final merged output appears to start **earlier than expected** relative to the microphone audio. The two streams are not synchronized — the loopback audio leads the microphone audio by a noticeable amount.

**Reproduction:** Record with default options (no special flags), with both system sound playing and speaking into the microphone. In the output file, the system audio is audibly ahead of the microphone audio.

**What works correctly:**
- Microphone audio is now correctly included in the output (fixed in commit `4e13163`)
- Both streams are captured and written to separate WAV files
- The FFmpeg merge correctly combines both streams
- Audio detection (`has_audio`) works properly during recording

## Architecture Overview

The recording pipeline works as follows:

1. **Two separate WASAPI capture threads** run in parallel:
   - `WasapiLoopbackRecorder` — captures system audio via `AUDCLNT_STREAMFLAGS_LOOPBACK` on the render (output) device
   - `WasapiMicrophoneRecorder` — captures microphone audio via standard capture on the input device

2. **Each thread writes to its own WAV file** (`{session_id}_loopback.wav` and `{session_id}_mic.wav`)

3. **After recording stops**, FFmpeg merges the two WAV files into a single stereo output using a `filter_complex` that creates dual-mono stereo (system audio + mic mixed into both channels)

4. The merge uses `apad=whole_dur` to pad the shorter stream and `amerge` to combine them, starting both from time 0

## Root Cause Analysis

There were **two independent causes** of the desync, both of which needed to be fixed:

### Cause 1: Sequential Recorder Start (~1.7s offset)

The loopback and microphone recorders start sequentially, not simultaneously. The loopback recorder's thread spawns and calls `audio_client.Start()` almost immediately. But the microphone recorder's thread must complete a lengthy initialization sequence before it can start:

```
16:02:23.59  Loopback audio_client.Start()
16:02:23.56  Mic thread begins COM init, device enum, format negotiation...
16:02:24.52  Mic audio_client.Initialize() completes (1 second!)
16:02:25.32  Mic audio_client.Start() (another 0.8 seconds)
```

**Result:** The loopback starts recording ~1.7 seconds before the microphone. Both WAV files start at their respective t=0, but the loopback's t=0 corresponds to a wall-clock time 1.7s earlier than the mic's t=0.

### Cause 2: Loopback WAV Missing Silence Periods (the main issue)

This was the bigger, more fundamental problem. **WASAPI loopback only delivers audio packets when system audio is actually playing.** Unlike the microphone (which always delivers samples, including silence), the loopback capture client's `GetNextPacketSize()` returns 0 when no application is producing sound.

The old recording loop did nothing when `packet_length == 0`:

```rust
if packet_length > 0 {
    // write audio to WAV
} 
// else: nothing happens — no silence written!
```

This meant the loopback WAV file only contained the time periods when system audio was active. In a 26-second recording where system audio started at second 11:

- **Loopback WAV:** 8.88 seconds (only the active audio, no leading silence)
- **Mic WAV:** 23.55 seconds (continuous recording including silence)

When FFmpeg merged these from t=0, the system audio that should appear at ~11 seconds appeared at the very beginning.

## Fix Attempts (Failed)

### Attempt #1: Synchronized Start Signal (FAILED)

**Hypothesis:** Synchronizing `audio_client.Start()` calls would fix the desync.

**Implementation:** Added a shared `Arc<AtomicBool>` start signal. Both threads initialized fully, then waited for the signal before calling `Start()`.

**Result:** Did not fix the desync. The 1.7s start offset was eliminated, but the loopback WAV still only contained active audio periods (Cause 2 was the dominant issue).

**Commit:** `8deed86` (reverted in `4337177`)

### Attempt #2: Drain Stale Loopback Buffer (FAILED)

**Hypothesis:** Pre-existing audio in the loopback buffer was causing the offset.

**Implementation:** After `Start()`, drained all available packets without writing them.

**Result:** Did not fix the desync. The buffer was empty at start time.

**Commit:** `2cee723` (reverted in `ea14461`)

## Solution Implemented

The fix has two parts that work together:

### Part A: Wall-Clock Silence Fill in Loopback Recorder

**File:** `core/src/wasapi/loopback.rs`

When the loopback capture client returns no packets (`packet_length == 0`), we now write silence to the WAV file to keep its duration aligned with wall-clock time. The implementation uses wall-clock tracking rather than a fixed amount per iteration:

```rust
// Track total frames written and recording start time
let mut total_frames_written: u64 = 0;
let recording_start = Instant::now();

// In the recording loop, when packet_length == 0:
let elapsed_secs = recording_start.elapsed().as_secs_f64();
let expected_frames = (elapsed_secs * sample_rate as f64) as u64;

if expected_frames > total_frames_written {
    let gap_frames = (expected_frames - total_frames_written) as usize;
    // Write gap_frames * channels silence samples
    total_frames_written += gap_frames as u64;
}
```

**Why wall-clock tracking instead of fixed per-tick?** `thread::sleep(10ms)` on Windows actually sleeps ~15.6ms due to the default timer resolution. Over a 30-minute recording, writing a fixed 10ms of silence per 15ms tick would cause the WAV to be ~20 minutes instead of 30. The wall-clock approach self-corrects on every iteration.

**Effect:** The loopback WAV now has the same duration as the recording session, with silence filling the gaps where no system audio was playing. System audio appears at the correct position in the WAV file.

### Part B: Start Time Offset Compensation via `-itsoffset`

**Files:** `core/src/wasapi/loopback.rs`, `core/src/wasapi/microphone.rs`, `core/src/recorder.rs`, `core/src/commands/record.rs`

Both recorders now record an `Instant::now()` timestamp immediately after `audio_client.Start()` succeeds. After recording stops, the offset is computed:

```rust
let start_offset_ms = match (loopback_start, mic_start) {
    (Some(lb), Some(mic)) => {
        if lb <= mic {
            mic.duration_since(lb).as_millis() as i64  // positive: loopback first
        } else {
            -(lb.duration_since(mic).as_millis() as i64)  // negative: mic first
        }
    }
    _ => 0,
};
```

This offset is passed to `merge_audio_streams_smart()`, which applies FFmpeg's `-itsoffset` flag before the appropriate input:

```
ffmpeg -itsoffset 1.730 -i loopback.wav -i mic.wav -filter_complex "..." output.wav
```

This shifts the loopback stream forward by 1.73 seconds, aligning it with the mic stream's timeline.

### Part C: Silence Trimming Extension Bug Fix

**File:** `core/src/ffmpeg_encoder.rs`

Fixed a bug where `trim_silence()` used `.with_extension("trimmed.tmp")` which produced `merged.trimmed.tmp`. FFmpeg couldn't detect the output format from `.tmp`. Changed to `.with_extension(format!("trimmed.{}", extension))` which produces `merged.trimmed.wav` or `merged.trimmed.m4a`.

## Files Changed

| File | Change |
|------|--------|
| `core/src/wasapi/loopback.rs` | Wall-clock silence fill + `start_time` tracking |
| `core/src/wasapi/microphone.rs` | `start_time` tracking |
| `core/src/recorder.rs` | Added `start_offset_ms` parameter to `merge_audio_streams_smart()`, applies `-itsoffset` |
| `core/src/commands/record.rs` | Computes offset from start times, passes to merge |
| `core/src/post_processing/pipeline.rs` | Updated caller with `start_offset_ms: 0` |
| `core/src/ffmpeg_encoder.rs` | Fixed `.tmp` extension in `trim_silence()` |

## How the Two Fixes Work Together

```
Timeline (wall-clock):
  0s        1.7s       3s        8s        11s       26s
  |---------|----------|---------|---------|---------|
  LB Start  Mic Start  User     System    System    Stop
            ↓          talks    audio     audio
            ↓                   starts    ends
            ↓
  
Loopback WAV (with silence fill):
  [silence...................][system audio][silence]
  0s                          11s          19.9s    ~24.3s
  
Mic WAV:
  [silence][user talking............................]
  0s       ~1.3s                                    23.5s

After -itsoffset 1.730 on loopback:
  Loopback shifted right by 1.73s → system audio at ~12.7s in output
  Mic starts at 0s → user talking at ~1.3s in output
  
  Both streams now reference the same wall-clock origin (mic start time)
```

## Test Results

All 160 tests pass (22 CLI + 138 core). Zero new warnings. Manual testing confirmed system audio appears at the correct position in the merged output.
