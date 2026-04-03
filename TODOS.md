# TODOS

> Items deferred from active plans. Each entry has enough context for someone to pick it up in 3 months.

---

## From: Recording Quality PRD (CEO Review — Approach B Deferred)

### Dynamic Range Compression
- **What:** Add a dynamic range compressor to the post-processing pipeline (FFmpeg `acompressor` filter).
- **Why:** Evens out volume differences between loud and quiet speakers beyond what normalization handles. Normalization adjusts overall loudness; compression reduces the dynamic range within a recording.
- **Pros:** Better listening experience for meetings with mixed speaker volumes.
- **Cons:** Overlaps with loudness normalization (Phase 2). Subjective quality — some users prefer uncompressed audio.
- **Context:** Was item #9 in the original 15-item plan. Deferred as Tier 3 by CEO review. Can be added as an FFmpeg filter stage in the PostProcessingPipeline after normalization ships.
- **Depends on:** Phase 2 (PostProcessingPipeline refactor + normalization).

### 3-Band Voice EQ
- **What:** Add a 3-band equalizer optimized for voice (boost presence 2-4kHz, cut mud 200-400Hz, gentle high-shelf rolloff above 8kHz).
- **Why:** Improves speech clarity and intelligibility in recordings.
- **Pros:** Noticeable quality improvement for voice recordings.
- **Cons:** Subjective — "good EQ" varies by microphone and speaker. May need presets rather than fixed values.
- **Context:** Was item #10 in the original plan. Deferred as Tier 3. Implementable as FFmpeg `equalizer` filter chain.
- **Depends on:** Phase 2 (PostProcessingPipeline).

### RTWQ Low-Latency Capture
- **What:** Use Real-Time Work Queue (RTWQ) API for capture thread scheduling priority.
- **Why:** Guarantees audio capture thread gets CPU time even under heavy system load.
- **Pros:** Prevents audio glitches during CPU-intensive operations (screen sharing, video encoding).
- **Cons:** High complexity, requires MMCSS thread priority management. Marginal benefit over event-driven capture for most users.
- **Context:** Was item #12. Deferred as Tier 3. Event-driven capture (Phase 1) solves 90% of the problem. RTWQ is the remaining 10% for extreme scenarios.
- **Depends on:** Phase 1 (event-driven capture).

### Pre-Recording Buffer
- **What:** Maintain a rolling 5-10 second audio buffer that captures audio BEFORE the user presses record.
- **Why:** Catches the first few seconds of a meeting that users often miss.
- **Pros:** Never miss the start of a conversation.
- **Cons:** Requires always-on background capture, which has privacy and resource implications.
- **Context:** Was item #13. Deferred as Tier 3. Requires significant architecture change (always-on capture daemon).
- **Depends on:** Phase 1 (event-driven capture) + Phase 4 (device management).

### Sidechain Ducking
- **What:** Automatically reduce system audio volume when the user speaks (detected via microphone).
- **Why:** Makes the user's voice more prominent in recordings where system audio and mic overlap.
- **Pros:** Better balance between system audio and user speech.
- **Cons:** Requires real-time cross-channel processing (mic signal controls loopback gain). Complex and latency-sensitive.
- **Context:** Was item #15. Deferred as Tier 3. Requires both channels to be processed in the same thread or with shared state.
- **Depends on:** Phase 3 (audio processing pipeline) + Phase 1 (event-driven capture).

---

## From: Engineering Review (Issue 9)

### Refactor existing anyhow catch-alls to typed errors
- **What:** Replace `anyhow::bail!` and `anyhow::Context` usage in `ffmpeg_encoder.rs` and `recorder.rs` with typed `RecorderError` variants (e.g., `FfmpegNotFound`, `FfmpegFailed`, `ComInitFailed`).
- **Why:** Currently, all FFmpeg and COM errors fall through to `RecorderError::Other(anyhow::Error)`, which means callers can't match on specific failure modes. This makes error recovery and user-facing error messages less precise.
- **Pros:** Better error messages, ability to handle specific failures differently.
- **Cons:** ~30 minutes of work, touches multiple files.
- **Context:** Issue 9 of the eng review adds 5 new typed variants for new features. This TODO completes the error typing for existing code. Low priority since `Other(anyhow)` works fine.
- **Depends on:** Issue 9 (new error variants added as part of Phase 1-4 implementation).

---

## From: Engineering Review (Risk Mitigation)

### --legacy-capture flag during Phase 1 development
- **What:** Preserve the current polling-based capture loop behind a `--legacy-capture` flag during Phase 1 development. Remove the flag after event-driven capture is stable.
- **Why:** Phase 1 changes the fundamental capture loop. If event-driven capture introduces regressions on specific hardware, users need a fallback.
- **Pros:** Safety net during development and early adoption.
- **Cons:** Maintaining two code paths temporarily.
- **Context:** Development-time safety measure, NOT a permanent feature. Remove after event-driven capture passes integration testing on multiple hardware configurations (at least 3 different audio devices).
- **Depends on:** Phase 1 implementation start.
