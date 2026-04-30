## Parent PRD

#20

## What to build

Add an async function `measure_lufs(audio_path: &PathBuf) -> Result<f64>` to `core/src/ffmpeg_encoder.rs` that measures the integrated loudness (LUFS) of an audio file using FFmpeg's `loudnorm` filter in measurement mode.

### Function signature

```rust
/// Measure the integrated loudness (LUFS) of an audio file using FFmpeg's loudnorm filter.
/// Returns the integrated loudness in LUFS, or an error if measurement fails.
pub async fn measure_lufs(audio_path: &PathBuf) -> Result<f64>
```

### Implementation

Runs FFmpeg in measurement-only mode:

```
ffmpeg -i <file> -af loudnorm=print_format=json -f null -
```

Parses the JSON output from stderr for the `input_i` (integrated loudness) value. The JSON block is embedded in FFmpeg's stderr output and looks like:

```json
{
    "input_i" : "-12.04",
    "input_tp" : "-0.50",
    "input_lra" : "7.30",
    ...
}
```

The function should:
1. Spawn FFmpeg with the loudnorm filter in measure mode
2. Capture stderr output
3. Find and parse the JSON block from the output
4. Extract the `input_i` field as `f64`
5. Use `CREATE_NO_WINDOW` flag on Windows (consistent with all other FFmpeg calls in the codebase)

Reference: PRD Step 1 from #20.

### Integration test

A `#[ignore]`-gated integration test (requires FFmpeg installed):
- Create or use a known WAV file with audio content
- Call `measure_lufs()` on it
- Assert the returned value is a reasonable LUFS measurement (e.g., between -70.0 and 0.0)

Reference: PRD Step 4 (integration test) from #20.

## Acceptance criteria

- [ ] `measure_lufs()` is a `pub async` function in `core/src/ffmpeg_encoder.rs`
- [ ] Returns accurate LUFS measurement for WAV files by parsing FFmpeg loudnorm JSON output
- [ ] Correctly extracts `input_i` from FFmpeg's stderr JSON block
- [ ] Uses `CREATE_NO_WINDOW` flag on Windows (consistent with existing FFmpeg calls)
- [ ] Returns an error with descriptive message if FFmpeg fails or JSON parsing fails
- [ ] Includes doc comment explaining the function's purpose
- [ ] Integration test gated behind `#[ignore]` validates a reasonable LUFS value
- [ ] Measurement completes in ~100ms for typical WAV files (as noted in PRD architecture)

## Blocked by

None — can start immediately (independent of `calculate_mic_gain_db`)

## User stories addressed

- LUFS measurement capability (PRD Step 1)
- Integration test coverage for LUFS measurement (PRD Step 4)
