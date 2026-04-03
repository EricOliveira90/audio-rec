## Parent PRD

#6

## What to build

Refactor the 350+ line `merge_audio_streams_smart()` function in `recorder.rs` into a `PostProcessingPipeline` struct with discrete stages: merge, normalize, limit, trim, finalize. Each stage is a method. Progress monitoring is extracted to a shared helper.

The pipeline emits JSON `processing` events for each stage (`{"type": "processing", "stage": "merging"}`, etc.) so calling tools can show accurate progress.

The pipeline is configured at construction time with which stages are enabled (based on CLI flags). Stages that are disabled are simply skipped.

**Pipeline stages (in order):**
1. **Merge** — 4 scenarios: A (both audio), B (loopback only), C (mic only), D (both silent)
2. **Normalize** — placeholder, implemented in follow-up issue
3. **Limit** — placeholder, implemented in follow-up issue
4. **Trim** — existing silence trimming logic
5. **Finalize** — copy to output location

See PRD section: *Post-Processing Pipeline*
See Eng Review: Issue 1 (PostProcessingPipeline refactor)
See Plan: Phase 3

## Acceptance criteria

- [ ] `PostProcessingPipeline` struct with discrete stage methods: `merge()`, `normalize()`, `limit()`, `trim()`, `finalize()`
- [ ] Pipeline configured at construction with enabled/disabled stages
- [ ] Progress monitoring extracted to shared helper (reusable across stages)
- [ ] JSON `{"type": "processing", "stage": "..."}` events emitted for each active stage
- [ ] Existing merge scenarios (A: both audio, B: loopback only, C: mic only, D: both silent) still work correctly
- [ ] Existing silence trimming still works
- [ ] All existing tests pass (`cargo test`)

## Blocked by

None — can start immediately

## User stories addressed

- User story 15: processing JSON events for each post-processing stage
