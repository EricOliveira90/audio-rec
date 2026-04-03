# Recording Quality Improvements — CEO/Founder Review

> **Date:** 2026-04-03
> **Mode:** SELECTIVE EXPANSION + Approach B (Domain-Aligned Minimal)
> **Plan Under Review:** `plans/recording-quality-improvements.md`
> **Status:** Step 0 Complete — Awaiting mode confirmation to proceed with Sections 1-10

---

## PRE-REVIEW SYSTEM AUDIT

### Current System State
- **Repo**: `audio-rec` — single branch (`master`), 19 commits, clean working tree
- **No stashes**, no open PRs, no other branches in flight
- **Zero TODO/FIXME/HACK markers** in the codebase — unusually clean
- **Recently touched files** (last 30 days): Nearly every source file — the project is young and actively evolving. 23 files changed across all 4 recent commits.

### Git History (4 commits, all recent):
```
35e4122 refactor: change device flag aliases from -i/-O to -id/-od
5f90490 feat: device selection, enumeration, enhanced status command
a7e05f0 Replace per-second progress events with state-change-only events
a492d25 feat: rename to audio-rec, add output/duration/mute flags, JSON protocol, silence trimming, FFmpeg check
```

### Retrospective Check
The flag alias refactor (`-i/-O` → `-id/-od`) in the latest commit suggests the team hit a UX issue with flag naming. No prior review cycles or reverted changes detected. The codebase is in its first generation — no architectural debt from prior rewrites.

### Existing Known Pain Points (from README "Future Plans"):
- No graceful shutdown (Ctrl+C leaves corrupted temp files)
- No audio normalization
- No cross-platform support
- No real-time audio level monitoring
- No configurable silence parameters
- No pause/resume
- No pre-recording buffer
- No automatic gain control

### Design Doc Check
Found 3 design documents:
- `docs/plans/2026-03-01-audio-recorder-design.md` — original architecture
- `plans/device-selection-and-status.md` — recently completed (Phase 1-3 all done)
- `plans/recording-quality-improvements.md` — **THE PLAN UNDER REVIEW**

### Frontend/UI Scope Detection
**No UI scope detected.** This is a CLI tool with JSON stdout protocol. Section 11 (Design & UX Review) will be SKIPPED.

### Taste Calibration

**Well-designed patterns (style references):**
1. `core/src/commands/record.rs` — The 4-scenario merge strategy (A/B/C/D based on audio detection) is elegant and explicit. Each scenario is clearly documented and handled independently.
2. `core/src/wasapi/loopback.rs` — Safety documentation at the module level is excellent. Every unsafe invariant is named and explained.

**Anti-patterns to avoid:**
1. `recorder.rs` `merge_audio_streams_smart()` — 350+ lines with deeply nested async closures, progress monitoring interleaved with business logic, and inline FFmpeg command construction. This is the "god function" anti-pattern — any new feature added here will make it worse.
2. The `let _ = writer.write_sample(...)` pattern in both WASAPI modules silently swallows write errors. Every sample write could fail and nobody would know.

---

## STEP 0: NUCLEAR SCOPE CHALLENGE

### 0A. Premise Challenge

**1. Is this the right problem to solve?**

Yes, but the framing needs sharpening. The plan is titled "Recording Quality Improvements" but it's actually **three different problem domains** bundled together:

- **Capture reliability** (#1 Event-Driven, #3 Silent Fix, #8 Timestamps, #6 Reconnection, #12 RTWQ, #14 Graceful Shutdown)
- **Audio processing** (#4 RNNoise, #5 Normalization, #7 Noise Gate, #9 Compressor, #10 EQ, #11 Limiter, #15 Ducking)
- **Feature expansion** (#2 Process-Specific Capture, #13 Pre-Recording Buffer)

This matters because the implementation order in the plan **interleaves these domains**, which means each phase touches different subsystems and requires different expertise. A domain-aligned ordering would be more efficient.

**2. What is the actual user/business outcome?**

The user outcome is: **"I press record before a Zoom/Teams meeting and get a clean, well-balanced audio file I can transcribe or review."** The plan is the most direct path to that outcome — these are exactly the right improvements for meeting recording.

**3. What would happen if we did nothing?**

Real pain. The current system has concrete problems:
- Audio gaps during silence (no ClearBuffer fix)
- Notification dings polluting recordings (no process-specific capture)
- Background noise in mic channel (no noise suppression)
- Volume inconsistency between speakers (no normalization)
- Corrupted files on Ctrl+C (no graceful shutdown)
- Potential sync drift in long meetings (no timestamp tracking)

### 0B. Existing Code Leverage

| Sub-problem | Existing Code | Reuse? |
|-------------|--------------|--------|
| Audio capture loop | `wasapi/loopback.rs`, `wasapi/microphone.rs` | Modify in-place (event-driven) |
| Audio level detection | `audio_utils.rs` (RMS functions) | Reuse for noise gate, ducking |
| FFmpeg post-processing | `ffmpeg_encoder.rs` | Extend for normalization, compressor, EQ, limiter |
| Sample format conversion | Inline in loopback.rs/microphone.rs | Extract to shared module for audio processing pipeline |
| Progress monitoring | `recorder.rs` merge function | Reuse pattern for new FFmpeg stages |
| Signal file IPC | `commands/record.rs` signal loop | Extend for graceful shutdown |
| Device enumeration | `wasapi/device_enumeration.rs` | Reuse for process-specific capture device selection |

**Key observation:** The plan does NOT propose rebuilding anything. All 15 items are additive. This is correct — the existing architecture supports extension.

### 0C. Dream State Mapping

```
CURRENT STATE                    THIS PLAN                      12-MONTH IDEAL
─────────────────────────────    ─────────────────────────────   ─────────────────────────────
Polling-based WASAPI capture     Event-driven capture            Event-driven + RTWQ priority
  with 10ms sleep                  with OS-level events            scheduling

Captures ALL system audio        Process-specific capture        Auto-detect meeting app,
                                   via --app flag                  capture automatically

No audio processing pipeline     RNNoise + Noise Gate +          Full real-time DSP chain
                                   Limiter (real-time)             with configurable presets

No post-processing beyond        Loudness normalization +        Loudness norm + compressor +
  silence trimming                 compressor + EQ                 EQ + voice enhancement AI

No sync tracking                 Device timestamps +             Sub-sample accurate sync
                                   offset alignment                with drift compensation

Ctrl+C = corrupted files         Graceful shutdown               Graceful shutdown + auto-
                                                                   recovery of partial files

No device resilience             Auto-reconnection               Auto-reconnect + hot-swap
                                                                   + device preference memory
```

**This plan moves strongly toward the 12-month ideal.** The only gap is that it doesn't address cross-platform (Linux/macOS), but that's correctly out of scope for a quality-focused plan.

### 0C-bis. Implementation Alternatives

```
APPROACH A: "Phase-by-Phase as Written"
  Summary: Implement all 15 items in the 7-phase order proposed in the plan.
  Effort:  XL (15 features across 7 phases)
  Risk:    Medium — interleaved domains mean context-switching between phases
  Pros:    - Comprehensive coverage
           - Each phase delivers incremental value
           - Matches the plan's rationale (foundation → features → polish)
  Cons:    - 15 items is a LOT — risk of plan fatigue and abandonment
           - Phases mix capture reliability with audio processing
           - No clear "done" milestone short of completing all 7 phases
  Reuses:  All existing code as extension points

APPROACH B: "Domain-Aligned Minimal" ✅ RECOMMENDED
  Summary: Reorder into 3 domain tracks, cut Tier 3 items to TODOS.md,
           ship 8-9 high/medium impact items in 4 focused phases.
  Effort:  L (8-9 features across 4 phases)
  Risk:    Low — each phase is cohesive, touching one subsystem
  Pros:    - Each phase is self-contained (one subsystem at a time)
           - Cuts 6 lower-impact items without losing core value
           - Clear "done" after Phase 4 — everything else is polish
           - Faster time-to-value for the user
  Cons:    - Defers RTWQ, pre-buffer, compressor, EQ, ducking
           - Requires re-ordering the plan
  Reuses:  Same as A

APPROACH C: "Killer Feature First"
  Summary: Ship Process-Specific Capture (#2) first as the flagship feature,
           then backfill reliability and processing.
  Effort:  L-XL
  Risk:    High — #2 is the highest complexity item and depends on Windows
           Build 20348+, which limits user base
  Pros:    - Biggest single quality improvement ships first
           - Marketing/demo value is immediate ("only records Zoom audio")
  Cons:    - Building on unreliable foundation (no event-driven capture yet)
           - Process-specific capture has the most unknowns (COM async activation)
           - Requires Windows version gating that may frustrate users
  Reuses:  Minimal — #2 is mostly new code
```

**RECOMMENDATION:** Approach **B** — aligns with the engineering preference for "focus as subtraction" — fewer things done better. Each phase touches one subsystem, reducing context-switching and merge conflicts. The deferred items (RTWQ, pre-buffer, compressor, EQ, ducking) are all Tier 3 "nice-to-have" items that can ship independently later.

### Approach B — Proposed Phase Reordering

| Phase | Domain | Items | Rationale |
|-------|--------|-------|-----------|
| **Phase 1: Capture Reliability** | WASAPI | #1 (Event-Driven), #3 (Silent Fix), #14 (Graceful Shutdown) | Fix the foundation before building on it |
| **Phase 2: Post-Processing Pipeline** | FFmpeg | #5 (Loudness Normalization), #11 (Limiter) | Quick wins via FFmpeg — low effort, immediate quality improvement |
| **Phase 3: Real-Time Audio Processing** | DSP | #4 (RNNoise), #7 (Noise Gate) | Biggest perceived quality jump for mic recordings |
| **Phase 4: Advanced Capture** | WASAPI + Sync | #8 (Timestamp Sync), #6 (Device Reconnection), #2 (Process-Specific Capture) | Reliability for long recordings + flagship feature |

**Deferred to TODOS.md:**
- #9 Dynamic Range Compression
- #10 3-Band Voice EQ
- #12 RTWQ Low-Latency Capture
- #13 Pre-Recording Buffer
- #15 Sidechain Ducking

### 0D. Mode-Specific Analysis (SELECTIVE EXPANSION)

**Complexity check:** The full plan (Approach A) touches 15+ files and introduces 6+ new modules. This is a smell. Approach B reduces to ~10 files and 3-4 new modules — more manageable.

**Minimum set for core goal:** Items #1, #3, #5, #14 achieve the stated goal of "dramatically better meeting recording quality" with the least effort. Items #4 and #2 are the transformative additions.

### 0E. Temporal Interrogation

```
HOUR 1 (foundations — Phase 1):
  Decision needed NOW: Event-driven capture (#1) changes the fundamental recording loop.
  Should the stop mechanism use WaitForMultipleObjects (plan says yes) or tokio channels?
  → Recommend: WaitForMultipleObjects — keeps it in the WASAPI/Win32 domain, no async runtime in capture threads.

  Decision needed NOW: ClearBuffer (#3) creates a second IAudioClient. When does it get cleaned up?
  → The plan says "when recording stops" but doesn't specify the cleanup order relative to the capture client.

HOUR 2-3 (core logic — Phase 2-3):
  Ambiguity: Loudness normalization (#5) — should it be two-pass (higher quality) or single-pass (simpler)?
  → The plan describes both. Recommend two-pass for WAV, single-pass for M4A (since M4A already re-encodes).

  Ambiguity: RNNoise (#4) requires 48kHz mono float32 in 480-sample frames. The mic may be stereo.
  → The plan mentions downmix/upmix but doesn't specify WHERE in the pipeline this happens.

HOUR 4-5 (integration — Phase 4):
  Surprise: Process-specific capture (#2) uses ActivateAudioInterfaceAsync which is COM async.
  The current codebase uses synchronous COM. This is a significant architectural mismatch.
  → Need to decide: implement IActivateAudioInterfaceCompletionHandler in Rust, or use a blocking wrapper?

  Surprise: Device reconnection (#6) requires IMMNotificationClient COM callback.
  This means the recording thread needs to handle device change events mid-recording.
  → The plan doesn't address what happens to in-flight audio buffers during reconnection.

HOUR 6+ (polish/tests):
  Wish: A test harness for WASAPI capture is nearly impossible without real audio devices.
  → Need to decide: mock the WASAPI interfaces, or rely on integration tests only?
```

*(With AI coding tools, these 6 human-hours compress to ~30-60 minutes of implementation time. The decisions are identical.)*

---

## MODE SELECTION

**Selected Mode: SELECTIVE EXPANSION**
**Selected Approach: B (Domain-Aligned Minimal)**

In SELECTIVE EXPANSION mode:
- The plan's scope (as reordered in Approach B) is the baseline
- Every expansion opportunity will be presented individually for cherry-picking
- Neutral recommendation posture — effort, risk, and the user decides

---

## "What Already Exists" Section

| Sub-problem | Existing Code | Status |
|-------------|--------------|--------|
| WASAPI loopback capture | `wasapi/loopback.rs` (polling loop, 4 format handlers) | Works, needs event-driven upgrade |
| WASAPI mic capture | `wasapi/microphone.rs` (polling loop, mute support) | Works, needs event-driven upgrade |
| Audio level detection | `audio_utils.rs` (RMS for i16, i32, f32) | Complete, reusable |
| FFmpeg merge pipeline | `recorder.rs` (4 merge scenarios, progress monitoring) | Works, needs refactoring before extension |
| FFmpeg silence trimming | `ffmpeg_encoder.rs` (`trim_silence()`) | Complete, reusable pattern |
| FFmpeg duration detection | `ffmpeg_encoder.rs` (WAV header + ffprobe fallback) | Complete |
| Signal file IPC | `commands/record.rs` (stop, cancel, mute polling) | Works, extend for Ctrl+C |
| Device enumeration | `wasapi/device_enumeration.rs` | Complete, reuse for process capture |
| JSON event protocol | `commands/record.rs` (`emit_event()`) | Complete, extend for new event types |
| Error types | `error.rs` (RecorderError enum) | Minimal — needs expansion for new failure modes |

---

## "NOT in Scope" Section (Deferred)

| Item | Rationale |
|------|-----------|
| #9 Dynamic Range Compression | Tier 3, overlaps with #5 normalization. Defer to TODOS.md. |
| #10 3-Band Voice EQ | Tier 3, subjective quality improvement. Defer to TODOS.md. |
| #12 RTWQ Low-Latency Capture | Tier 3, high complexity, marginal benefit over event-driven. Defer to TODOS.md. |
| #13 Pre-Recording Buffer | Tier 3, requires always-on background capture. Defer to TODOS.md. |
| #15 Sidechain Ducking | Tier 3, requires cross-channel real-time processing. Defer to TODOS.md. |
| Cross-platform support | Out of scope for quality-focused plan. |

---

## "Dream State Delta" Section

After completing Approach B (Phases 1-4), the system will be:

| Dimension | Current → After Plan | Gap to 12-Month Ideal |
|-----------|---------------------|----------------------|
| Capture reliability | Polling → Event-driven + ClearBuffer + graceful shutdown | RTWQ scheduling (deferred) |
| Audio cleanliness | Raw capture → RNNoise + Noise Gate on mic | Full DSP chain with presets |
| Post-processing | Silence trim only → + Normalization + Limiter | + Compressor + EQ |
| Sync accuracy | No tracking → Device timestamps + offset alignment | Sub-sample drift compensation |
| Device resilience | None → Auto-reconnection | + hot-swap + preference memory |
| App-specific capture | All system audio → Process-specific via --app | Auto-detect meeting app |

**Coverage: ~75% of the 12-month ideal with ~50% of the total effort.** The remaining 25% is polish (Tier 3 items) that can ship incrementally.

---

## NEXT STEPS

Proceed with Sections 1-10 of the review against the Approach B scope:
- **Phase 1:** #1 Event-Driven Capture, #3 Silent Loopback Fix, #14 Graceful Shutdown
- **Phase 2:** #5 Loudness Normalization, #11 Limiter
- **Phase 3:** #4 RNNoise Noise Suppression, #7 Noise Gate
- **Phase 4:** #8 Timestamp Sync, #6 Device Reconnection, #2 Process-Specific Capture

Each section will surface issues one at a time with concrete recommendations.
