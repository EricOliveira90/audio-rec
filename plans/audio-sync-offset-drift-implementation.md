# Audio Sync Offset and Drift Implementation Handoff

## Fresh Session Prompt

Start the new session with:

```text
$diagnosing-bugs $tdd Implement plans/audio-sync-offset-drift-implementation.md.
Preserve the existing uncommitted fixes and unrelated worktree changes. Work test-first,
implement the offset and drift correction through the existing capture/merge seam, and
verify it with the synthetic FFmpeg harness before requesting another hardware soak test.
```

The implementation agent must read this document and
`plans/audio-desync-investigation.md` before editing.

## Objective

Produce one official recording whose computer-audio and microphone timelines:

1. Start at the correct relative wall-clock position.
2. Remain synchronized over long recordings despite independent device clocks.
3. Degrade safely when reliable timing metadata is unavailable.
4. Leave no user-visible `_merged` duplicate.

## Confirmed Diagnostic Result

The physical diagnostic ran for 90 minutes on the notebook's internal devices:

- Output: Speakers (Realtek(R) Audio)
- Input: Microphone Array (Intel Smart Sound Technology for Digital Microphones)
- Sample rate: 48 kHz
- Headphones disconnected
- Audio enhancements disabled
- Marker cadence: every 5 seconds for the first minute, then every 30 seconds

Primary report:

`sync-diagnostics/sync-20260722-195105/report.json`

Measured from the final merged WAV:

- Status: `FAIL`
- Paired markers: `188`
- Measurement span: `5366.14 seconds`
- Initial microphone-minus-loopback offset: `-460 ms`
- Final offset: approximately `-415 ms`
- End-to-end change: `45 ms`
- Fitted drift: `24.26 ms/hour`
- Clock delta: `6.74 ppm`
- P95 residual jitter: `11.55 ms`

The same drift was measured independently from the preserved raw tracks:

- 10 ms detector: approximately `20.4 ms/hour`
- 5 ms detector: approximately `24.5 ms/hour`

This establishes two separate defects:

1. A large constant startup offset.
2. A smaller but real linear device-clock drift.

The constant offset is probably the dominant user-visible defect. The measured drift is
real but only adds roughly 45 ms over 90 minutes.

## Diagnostic Artifacts

The run directory contains:

- `recording.wav`: official final output from the old installed binary
- `loopback-raw.wav`: raw loopback snapshot near the end of capture
- `mic-raw.wav`: raw microphone snapshot near the end of capture
- `report.json`: primary final-output analysis
- `report-raw-5ms.json` and `report-raw-10ms.json`: raw-track cross-checks
- `recorder.stdout.log` and `recorder.stderr.log`

The redundant `recording_merged.wav` was SHA-256 identical to `recording.wav` and was
removed. Temporary compact analysis WAVs were also removed.

`sync-diagnostics/` is ignored by Git. Keep the raw tracks until implementation and
verification are complete; they are useful evidence but total about 2 GB.

## Existing Uncommitted Fixes

Do not discard or recreate these changes.

### Physical Startup Delay

`core/src/recorder.rs` has already replaced ineffective FFmpeg input `-itsoffset` usage
with an `adelay` filter that inserts real silence before the stream that started later.

Current sign convention:

- Positive offset: loopback started first, so delay the microphone.
- Negative offset: microphone started first, so delay loopback.

The old globally installed `audio-rec.exe` predates this fix. The 90-minute physical
output therefore still contains the original `-460 ms` startup error. The worktree fix
has passed the synthetic FFmpeg offset fixture but has not yet been validated using a
newly built binary on hardware.

### Single Official Output

`core/src/commands/record.rs` now finalizes by copying the temporary merged output to the
official destination and strictly removing the temporary file. It no longer creates a
debug `_merged` output beside the official file.

A regression test named `finalize_output_keeps_only_official_file` was added.

### Existing Sync Fixture

`core/tests/audio_sync.rs` creates loopback and microphone pulse fixtures and verifies
that the current physical-delay filter aligns a common event.

### Diagnostic Tools

- `scripts/analyze_sync.py`
- `scripts/run_sync_diagnostic.ps1`
- `scripts/preserve_active_tracks.ps1`

The analyzer is red-capable and was synthetically validated against fixed-offset and
deliberate-drift fixtures. The 90-minute automatic pass initially failed because the
original full-rate file was too expensive to analyze directly. The final report was
created from a 4 kHz, 1 kHz-band-passed diagnostic stream.

## Worktree State

Expected modified or untracked paths include:

```text
M  .gitignore
M  core/src/commands/record.rs
M  core/src/recorder.rs
M  core/src/wasapi/loopback.rs
M  core/src/wasapi/microphone.rs
?? core/tests/
?? scripts/
?? .agents/
?? skills-lock.json
```

Some paths may be unrelated repository setup. Do not revert changes that were not made
as part of this bug fix.

## Why Current Logic Still Drifts

Both WASAPI capture loops currently call `IAudioCaptureClient::GetBuffer` with:

```rust
None,
None,
```

for device position and QPC position. The timing evidence needed to compare the two
hardware clocks is discarded.

Both loops also insert wall-clock-derived silence only when no packet is available.
That helps loopback silence gaps, but it cannot correct drift while both devices deliver
continuous packets. Each device continues producing samples according to its own clock.

Do not estimate drift from final WAV durations. The raw durations also contain different
start, stop, copy, and silence-fill behavior and differed by several seconds despite the
measured content drift being only tens of milliseconds.

## Historical Attempts Not to Repeat

See `plans/audio-desync-investigation.md` and Git history:

- `8deed86`: synchronized start signal, later reverted
- `2cee723`: drained stale loopback buffers, later reverted
- `a11f4cd`: wall-clock silence fill and `-itsoffset`
- `c8c5661`: microphone wall-clock silence fill

Synchronized starts alone did not solve missing loopback silence and cannot solve
independent clock rates. Stale-buffer draining did not address the failure.

## Selected Design

Use post-capture affine alignment through the existing capture/merge seam. Do not
rewrite the entire recording lifecycle or generalize to arbitrary N-stream routing in
this change.

### Capture Timing

Add crate-local types in `core/src/wasapi/capture_engine.rs`, approximately:

```rust
struct TimingAnchor {
    wav_frame: u64,
    device_position: u64,
    qpc_100ns: u64,
}

struct CaptureTiming {
    sample_rate: u32,
    start_time: Option<Instant>,
    anchors: Vec<TimingAnchor>,
    timestamp_error_count: u64,
}
```

Exact names and storage are implementation choices. Required behavior:

- Request device and QPC positions from every successful `GetBuffer`.
- Record the WAV frame position before writing the packet.
- Exclude anchors with `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR`.
- Keep audio even when its timestamp is invalid.
- Retain sparse periodic anchors rather than every packet.
- Include first and last valid anchors and anchors around discontinuities.
- Support system loopback, microphone, and process loopback.
- Snapshot timing only after the recorder has stopped and the WAV writer is finalized.

### Clock Model

Fit an affine mapping between each WAV timeline and the common QPC clock:

```text
qpc_time = origin + seconds_per_wav_frame * wav_frame
```

From the two fitted models, derive one private `AudioAlignment` value containing:

- Which stream needs leading silence.
- Sample-accurate leading-silence length.
- Relative rate correction.
- Confidence or fallback reason.

Keep fields private and provide constructors such as:

```rust
AudioAlignment::from_captures(loopback_timing, mic_timing)
AudioAlignment::fixed_offset(...)
AudioAlignment::none(...)
```

Use multiple anchors and a robust fit rather than trusting only two points. Validate:

- Monotonic QPC, device, and WAV positions.
- Sufficient time span for drift estimation.
- Finite rates.
- Residual error below a defined threshold.
- Relative correction within a conservative bound, initially 1000 ppm.

Use offset-only correction for short recordings or insufficient drift evidence.

### Merge

Replace the `start_offset_ms` parameter of `merge_audio_streams_smart` with
`AudioAlignment`.

Treat loopback as the reference timeline. The merge module, not `record.rs`, owns:

- Sign handling.
- Rate-ratio handling.
- FFmpeg filter construction.
- Padding and trimming.
- Fallback logging.

Prototype and lock down the FFmpeg affine transform with a synthetic fixture before
integrating it. Preferred direction:

1. Correct timestamps/rate using `asetpts` plus `aresample=async`.
2. Materialize both streams at the requested output sample rate.
3. Insert sample-accurate leading silence.
4. Apply microphone gain and channel layout.
5. Pad both streams to the common output interval.
6. Merge.

If `asetpts` plus asynchronous resampling is not deterministic in the installed FFmpeg,
evaluate `atempo` as the measured relative-rate correction. Do not choose based on a
plausible command alone; the early/late marker fixture decides.

### Caller

Change `core/src/commands/record.rs` to:

1. Stop and finalize both recorders.
2. Read both `CaptureTiming` summaries.
3. Construct `AudioAlignment::from_captures`.
4. Pass the alignment into the merge module.

Remove caller-side offset arithmetic after the replacement is verified.

The separate caller in `core/src/post_processing/pipeline.rs` currently passes a
hard-coded zero offset. It must accept captured alignment metadata, route through the
same merge interface, or be proven unused and removed. Do not leave a second incorrect
merge path.

## Implementation Sequence

### Phase 1: Red Tests and FFmpeg Prototype

Before production edits:

- Extend `core/tests/audio_sync.rs` with early and late markers.
- Reproduce a positive and negative startup offset.
- Reproduce positive and negative drift.
- Include the measured `460 ms` and `6.7 ppm` model.
- Use exaggerated drift in a short fixture to keep the loop fast.
- Prove the chosen FFmpeg filter aligns both early and late markers.

The fast red-capable command should exercise the real FFmpeg merge path.

### Phase 2: Timing Telemetry Without Output Changes

- Add timing anchors to all WASAPI capture adapters.
- Add pure clock-model tests.
- Log calculated startup displacement, ppm, fit span, residual, and fallback reason.
- Keep the existing `adelay` output behavior initially.
- Confirm the notebook reports values close to the known `466 ms` and `6.7 ppm`.

### Phase 3: Enable Affine Correction

- Replace `start_offset_ms` with `AudioAlignment`.
- Enable rate correction for trustworthy timing.
- Keep offset-only and uncorrected fallbacks.
- Ensure padding preserves the tail after delay and rate correction.

### Phase 4: Regression Coverage

Cover:

- Loopback starts first.
- Microphone starts first.
- Positive and negative drift.
- Short recording with offset-only correction.
- Missing or timestamp-error timing.
- Non-monotonic anchors or clock reset.
- Excessive ppm rejection.
- Silent loopback.
- Silent or muted microphone.
- Single-stream output.
- Process-specific capture and fallback.
- WAV and M4A.
- Silence trimming.
- Exactly one official output.

### Phase 5: Hardware Verification

Build and install the new binary, then run:

1. A 45-second marker check for startup alignment.
2. A 90-minute soak test for drift.

Do not use the current global `audio-rec.exe` for acceptance until it has been rebuilt
from the worktree.

## Acceptance Criteria

Synthetic:

- Pure clock model endpoint error: at most one target-rate frame.
- FFmpeg fixture early and late marker separation: at most two output frames.
- Both offset directions and drift directions pass.

Physical:

- Absolute fitted initial offset: at most 20 ms.
- Absolute drift: target at most 5 ms/hour; hard limit 10 ms/hour.
- P95 residual jitter: at most 20 ms.
- Both loopback and microphone markers detected.

Output:

- One official recording.
- No user-visible `_merged` copy.
- No leaked temporary raw tracks after successful production recording.
- Raw tracks retained on post-processing failure when needed for recovery.

## Environment Limitation

Rust tests previously failed at link time because Windows SDK libraries such as
`kernel32.lib` were unavailable in the current shell environment. This is an environment
failure, not a test assertion failure.

The next agent should:

1. Try the repository's normal Rust test command from a Visual Studio developer shell.
2. If the SDK is still unavailable, run the deterministic FFmpeg harness and all
   non-linking checks.
3. Report the exact linker limitation instead of treating tests as passed.

Do not weaken or remove tests to accommodate the missing SDK.

## Completion Checklist

- [ ] Fast deterministic red test exists and fails before drift correction.
- [ ] WASAPI QPC/device timing is captured for all relevant adapters.
- [ ] Affine clock model has pure tests.
- [ ] FFmpeg transform passes early and late marker fixtures.
- [ ] Caller no longer computes raw offset signs.
- [ ] Hard-coded zero-offset merge path is removed or supplied with timing.
- [ ] Offset-only and invalid-timing fallbacks are observable.
- [ ] Existing offset and single-output fixes remain intact.
- [ ] WAV, M4A, mute, silence, trimming, and process capture are covered.
- [ ] No debug instrumentation remains.
- [ ] 45-second physical validation passes.
- [ ] 90-minute physical validation passes.
- [ ] Exactly one official output remains.
