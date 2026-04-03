## Parent PRD

#6

## What to build

Add EBU R128 loudness normalization as a post-processing stage in the `PostProcessingPipeline`.

**Two-pass for WAV output:** Pass 1 analyzes with FFmpeg's `loudnorm` filter and outputs JSON measurements. Pass 2 applies correction with `linear=true` for transparent normalization.

**Single-pass for M4A output:** Since M4A already re-encodes, the quality difference from single-pass is negligible. Simpler and faster.

**Parameters:**
- Default target: -16 LUFS (streaming standard)
- True peak limit: -1.5 dBTP
- Loudness range: 11 LU
- Configurable via `--target-loudness <LUFS>`

**Pipeline order:** Merge → **Normalize** → Limiter → Trim → Finalize

**CLI flags:**
- `--no-normalize` — disable normalization (ON by default)
- `--target-loudness <LUFS>` — configure target loudness level

See PRD section: *Loudness Normalization (Phase 2)*
See Plan: Phase 4

## Acceptance criteria

- [ ] Two-pass loudness normalization for WAV output produces output within ±1 LU of -16 LUFS target
- [ ] Single-pass normalization for M4A output produces output within ±2 LU of target
- [ ] `--target-loudness <LUFS>` flag configures target (e.g., `-23` for broadcast)
- [ ] `--no-normalize` flag disables normalization
- [ ] Normalization ON by default (zero-config best quality)
- [ ] Edge cases handled: silent file (skip normalization), already-normalized file (minimal change)
- [ ] Pipeline order is Merge → Normalize → Limiter → Trim → Finalize
- [ ] Unit tests: normalization output within target range (fixture WAVs generated with `hound`)
- [ ] CLI tests: `--no-normalize`, `--target-loudness` flag parsing

## Blocked by

- Blocked by #8 (Post-Processing Pipeline Refactor)

## User stories addressed

- User story 4: automatic loudness normalization to EBU R128
- User story 8: `--no-normalize` disable flag
- User story 17: configurable target loudness
