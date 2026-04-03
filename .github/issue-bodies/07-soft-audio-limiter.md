## Parent PRD

#6

## What to build

Add a soft audio limiter as a post-processing stage in the `PostProcessingPipeline` via FFmpeg filter: `alimiter=limit=0.95:attack=0.001:release=50`.

This replaces the current hard clipping pattern (`sample.clamp(-1.0, 1.0)`) in capture threads with a proper soft limiter that prevents distortion from sudden loud sounds.

**Parameters:**
- Limit: 0.95 (soft ceiling)
- Attack: 1ms (fast, catches transients)
- Release: 50ms

**CLI flag:** `--no-limiter` — disable limiter (ON by default)

See PRD section: *Audio Limiter (Phase 2)*
See Plan: Phase 4

## Acceptance criteria

- [ ] Soft limiter applied via FFmpeg `alimiter` filter — loud transients limited without hard clipping
- [ ] `--no-limiter` flag disables limiter
- [ ] Limiter ON by default (zero-config best quality)
- [ ] Samples below threshold pass through unchanged
- [ ] Pipeline order: Merge → Normalize → **Limiter** → Trim → Finalize
- [ ] Unit tests: limiter threshold behavior (fixture WAVs), passthrough for quiet audio
- [ ] CLI tests: `--no-limiter` flag parsing

## Blocked by

- Blocked by #8 (Post-Processing Pipeline Refactor)

## User stories addressed

- User story 5: soft audio limiter for sudden loud sounds
- User story 8: `--no-limiter` disable flag
