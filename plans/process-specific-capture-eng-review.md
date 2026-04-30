# Engineering Review: Process-Specific Audio Capture PRD (Issue #25)

**Branch:** master
**Date:** 2026-04-04
**PRD:** `plans/process-specific-capture-prd.md` / GitHub Issue #25
**Reviewer:** plan-eng-review

---

## Step 0: Scope Challenge

### What existing code already solves sub-problems?

| Sub-problem | Existing code | Reuse? |
|-------------|--------------|--------|
| CLI flag parsing (`--app`, `--app-window`) | `cli/src/cli.rs:168-169` — already parsed and stored in `RecordOptions` | ✅ Done |
| Error variants | `core/src/error.rs` — `ProcessNotFound`, `UnsupportedWindowsVersion` already exist | ✅ Done |
| Loopback recording pattern | `core/src/wasapi/loopback.rs` — full template for recording loop, silence injection, WAV writing | ✅ Reuse pattern |
| Shared capture utilities | `core/src/wasapi/capture_engine.rs` — format detection, sample conversion, write error tracking | ✅ Reuse directly |
| JSON event emission | `core/src/commands/record.rs:emit_event()` | ✅ Extend |
| Status output | `core/src/commands/status.rs:StatusOutput` | ✅ Add fields |
| Device enumeration | `core/src/wasapi/device_enumeration.rs` — `EnumWindows` pattern is similar | ✅ Pattern reference |

### Minimum change set

```
FILES TOUCHED
═══════════════════════════════════════════════════
NEW (2):
  core/src/wasapi/process_resolver.rs    — PID resolution + version check
  core/src/wasapi/process_loopback.rs    — Process-specific WASAPI recorder

MODIFIED (6):
  core/src/wasapi/mod.rs                 — Add module declarations
  core/src/wasapi/capture_engine.rs      — Add LoopbackCapture trait
  core/src/commands/record.rs            — Wire --app/--app-window into lifecycle
  core/src/commands/status.rs            — Add capture_mode, app fields
  core/src/status.rs                     — Add capture_mode to RecordingStatus
  core/Cargo.toml                        — Add windows crate features
  cli/src/cli.rs                         — Mutual exclusivity validation

TOTAL: 2 new + 7 modified = 9 files
```

**Complexity check:** 9 files, 2 new modules. Slightly over the 8-file threshold, but the two new files are tightly scoped (one is pure logic, one is a WASAPI recorder following an established pattern). The alternative of cramming both into one file would hurt readability. Acceptable.

### TODOS cross-reference

Nothing in `TODOS.md` blocks this feature. The `--legacy-capture` flag TODO is about event-driven capture (Phase 1), not process-specific capture. No conflicts, no bundling opportunities.

### Completeness check

The PRD is thorough. All edge cases (version fallback, process exit fallback, mutual exclusivity, `--output-device` warning) are specified. The mid-recording fallback is the most complex piece and is properly scoped.

### Distribution check

No new artifact type. Feature addition to existing binary. No CI/CD changes needed.

---

## Section 1: Architecture Review

### Architecture Diagram

```
                          ┌─────────────────────────────────────┐
                          │           CLI (cli.rs)              │
                          │  --app Zoom.exe                     │
                          │  --app-window "Zoom Meeting"        │
                          └──────────────┬──────────────────────┘
                                         │ RecordOptions
                                         ▼
                          ┌─────────────────────────────────────┐
                          │      Record Command (record.rs)     │
                          │                                     │
                          │  1. Validate mutual exclusivity     │
                          │  2. Check Windows version           │
                          │  3. Resolve PID (if --app/window)   │
                          │  4. Create recorder (trait object)  │
                          │  5. Run recording loop              │
                          │  6. Handle mid-recording fallback   │
                          └──┬──────────────────────────────┬───┘
                             │                              │
                    ┌────────▼────────┐          ┌──────────▼──────────┐
                    │  Process        │          │  Microphone         │
                    │  Resolver       │          │  Recorder           │
                    │  (new module)   │          │  (unchanged)        │
                    │                 │          │                     │
                    │ resolve_by_name │          └─────────────────────┘
                    │ resolve_by_title│
                    │ check_version   │
                    └────────┬────────┘
                             │ PID
                             ▼
              ┌──────────────────────────────────┐
              │   Box<dyn LoopbackCapture>        │
              │                                   │
              │  ┌─────────────────────────────┐  │
              │  │ WasapiLoopbackRecorder       │  │
              │  │ (existing, system loopback)  │  │
              │  │ IMMDevice.Activate path      │  │
              │  └─────────────────────────────┘  │
              │           OR                      │
              │  ┌─────────────────────────────┐  │
              │  │ ProcessLoopbackRecorder      │  │
              │  │ (new, process loopback)      │  │
              │  │ ActivateAudioInterfaceAsync  │  │
              │  │ + process tree capture       │  │
              │  └─────────────────────────────┘  │
              └──────────────────────────────────┘
```

### Decision 1: LoopbackCapture Trait (accepted)

Define a `LoopbackCapture` trait in `capture_engine.rs` with methods matching the existing `WasapiLoopbackRecorder` public interface:

```rust
pub trait LoopbackCapture: Send {
    fn stop(&mut self) -> anyhow::Result<()>;
    fn get_frames_captured(&self) -> u64;
    fn has_audio_detected(&self) -> bool;
    fn get_sample_rate(&self) -> u32;
    fn get_start_time(&self) -> Option<Instant>;
}
```

Both `WasapiLoopbackRecorder` and `ProcessLoopbackRecorder` implement it. `record.rs` uses `Box<dyn LoopbackCapture>`. The vtable cost is zero since these methods are called once per second in the status check loop, not in the audio capture hot path.

**Confidence: 9/10** — This is the standard Rust pattern for runtime polymorphism with a small, fixed interface.

### Decision 2: Process Resolver as Separate Module (accepted)

The PRD proposes the process resolver and version check in one module, and the process loopback recorder in another. This is the right split:

- `process_resolver.rs` — Pure logic: `EnumWindows` → PID resolution, version check. Testable in isolation.
- `process_loopback.rs` — WASAPI COM code: `ActivateAudioInterfaceAsync`, recording loop. Follows `loopback.rs` pattern.

Keeping them separate means the resolver can be unit-tested with mock data without touching COM.

**Confidence: 8/10**

### Decision 3: Mid-Recording Fallback Architecture (critical)

[P1] (confidence: 8/10) `core/src/commands/record.rs` — The mid-recording fallback (process exits → switch to system loopback) is the hardest part of this feature.

The PRD says: "Detects mid-recording process exit via `GetBuffer` returning errors, and signals the fallback." This means the fallback signal originates from inside the process loopback recorder's recording thread, but the actual fallback (creating a new `WasapiLoopbackRecorder`) must happen in the record command's main loop.

**Recommended approach:**

```
FALLBACK STATE MACHINE
═══════════════════════════════════════════════════
                                    
  ┌──────────┐   process exits   ┌──────────────┐
  │ Process   │ ───────────────► │ Fallback      │
  │ Loopback  │  (AtomicBool     │ Pending       │
  │ Active    │   signaled by    │               │
  └──────────┘   recorder thread)└──────┬───────┘
                                        │ record.rs detects
                                        │ flag in status loop
                                        ▼
                                 ┌──────────────┐
                                 │ System        │
                                 │ Loopback      │
                                 │ Active        │
                                 └──────────────┘
```

The `ProcessLoopbackRecorder` exposes an `is_process_dead() -> bool` method (backed by an `AtomicBool`). The recording thread sets this flag when `GetBuffer` returns `AUDCLNT_E_DEVICE_INVALIDATED` or similar errors, or when a periodic `OpenProcess` + `WaitForSingleObject(0)` check detects the process has exited.

The record command's main loop (the 1-second status check loop) checks `is_process_dead()`. When true:
1. Stop the process loopback recorder
2. Create a new `WasapiLoopbackRecorder` (system loopback)
3. Emit a `warning` JSON event
4. Update `capture_mode` from `"process"` to `"system"`
5. Continue the recording loop with the new recorder

**Key concern:** There will be a brief gap in audio capture during the swap (a few hundred milliseconds). The silence injection logic in both recorders will fill this gap with silence, so the WAV file duration stays aligned with wall-clock time. This is acceptable for the use case (meeting app crashed).

**Confidence: 8/10**

### Decision 4: COM Activation Pattern

[P2] (confidence: 7/10) `core/src/wasapi/process_loopback.rs` — The `ActivateAudioInterfaceAsync` API requires implementing `IActivateAudioInterfaceCompletionHandler` as a COM callback. The `windows` crate's `#[implement]` macro handles this.

The PRD's approach (completion handler signals a Win32 event, main thread waits with `WaitForSingleObject`) is the standard pattern. The `windows` crate has examples of this. No concerns.

**New Cargo.toml features needed:**

```toml
"Win32_UI_WindowsAndMessaging"      # EnumWindows, GetWindowTextW
"Win32_System_Threading"            # OpenProcess, QueryFullProcessImageNameW
"Win32_System_SystemInformation"    # RtlGetVersion
"Win32_Media_Audio"                 # Already present
"Win32_System_Com"                  # Already present
```

Plus the `implement` feature for the `windows` crate to use `#[implement]` macro for the COM callback.

**Confidence: 7/10** — Verify that `windows 0.58` supports `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`. This API was added in Windows SDK 10.0.20348.0. The `windows` crate version 0.58 should include it, but confirm during implementation.

### Decision 5: Process Detection via EnumWindows (accepted)

The PRD correctly avoids `CreateToolhelp32Snapshot` and `tasklist` per `claude.md` antivirus rules. The `EnumWindows` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` approach is safe:

- `EnumWindows` iterates top-level windows (not processes)
- `GetWindowThreadProcessId` gets the owning PID of a window
- `QueryFullProcessImageNameW` gets the executable path from a PID (requires `OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION`)

For `--app Zoom.exe`: enumerate windows → get PID for each → query process image name → extract filename → exact match.
For `--app-window "Zoom Meeting"`: enumerate windows → get title → case-insensitive substring match → get PID.

**Edge case:** What if the target process has no visible windows? This is possible for background audio processes. The PRD's approach won't find them. This is acceptable because:
1. Meeting apps (the primary use case) always have windows
2. The `--app` flag documentation should note this limitation
3. Adding `CreateToolhelp32Snapshot` as a fallback would violate antivirus rules

**Confidence: 8/10**

### Decision 6: JSON Protocol — Warning Event Type

[P2] (confidence: 9/10) The PRD introduces a new `warning` event type. This is well-designed:

```json
{"type": "warning", "session_id": "...", "code": "process_exited", "message": "..."}
{"type": "warning", "session_id": "...", "code": "unsupported_version", "message": "..."}
{"type": "warning", "session_id": "...", "code": "output_device_ignored", "message": "..."}
```

The `code` field enables programmatic handling by calling tools. The `message` field provides human-readable context. This follows the existing pattern of `type` + structured fields.

**One addition:** The `warning` event should also include a `capture_mode` field when the warning is about a fallback, so the calling tool knows the new state without waiting for the next `audio_state` event.

### Production Failure Scenarios

| Codepath | Failure scenario | Handled? |
|----------|-----------------|----------|
| Process resolver | Target process has no windows (background service) | ⚠️ Returns `ProcessNotFound` — correct behavior, but could confuse users. Add a hint in the error message: "No windows found for process. Ensure the app is running and has a visible window." |
| COM activation | `ActivateAudioInterfaceAsync` fails (permission denied, invalid PID) | ✅ Returns error, falls back to system loopback |
| Mid-recording fallback | Process exits while `GetBuffer` is blocked | ✅ `GetBuffer` returns error, sets `is_process_dead` flag |
| Mid-recording fallback | Process exits between `GetBuffer` calls | ⚠️ Need periodic process liveness check (every 1s in the recording thread) |
| Version check | `RtlGetVersion` fails | ✅ Treat as unsupported version, fall back to system loopback |
| Mutual exclusivity | Both `--app` and `--app-window` specified | ✅ CLI validation error at startup |

---

## Section 2: Code Quality Review

### Issue 1: DRY — Recording Loop Duplication

[P2] (confidence: 9/10) The `ProcessLoopbackRecorder` recording thread will be ~90% identical to `WasapiLoopbackRecorder::recording_thread()`. Both need:
- COM initialization
- Audio client setup
- WAV writer creation
- Recording loop with format detection, sample conversion, silence injection
- Stop and cleanup

**Recommendation:** Extract the shared recording loop into a function in `capture_engine.rs`:

```rust
pub fn run_capture_loop(
    capture_client: &IAudioCaptureClient,
    sample_format: SampleFormat,
    channels: u16,
    sample_rate: u32,
    is_recording: &AtomicBool,
    frames_captured: &AtomicU64,
    has_audio: &AtomicBool,
    write_errors: &WriteErrorTracker,
    writer: &mut Option<WavWriter<BufWriter<File>>>,
) -> Result<()>
```

The process loopback recorder would call this same function. The only difference is how the `IAudioClient` is obtained (device activation vs. process activation) and the process liveness check.

For the process liveness check, add an optional callback parameter:

```rust
pub fn run_capture_loop(
    // ... same params ...
    on_no_packets: Option<&dyn Fn() -> bool>,  // returns true if should stop
) -> Result<()>
```

The process loopback recorder passes a closure that checks process liveness. The system loopback recorder passes `None`.

This eliminates ~150 lines of duplication. Aligns with your DRY preference.

**Effort:** human: ~2 hours / CC: ~10 min

### Issue 2: Mutual Exclusivity Validation Location

[P2] (confidence: 9/10) `cli/src/cli.rs` — The PRD says "CLI validation rejects both being specified simultaneously." Currently, `cli.rs` parses both flags independently with no cross-validation.

**Recommendation:** Add validation in `handle_record_command()` right after parsing both flags:

```rust
if app.is_some() && app_window.is_some() {
    output.error("Cannot specify both --app and --app-window. Use one or the other.");
    std::process::exit(1);
}
```

This is the right place because it matches the existing pattern (other validation errors like invalid duration also call `output.error()` + `process::exit(1)` in `cli.rs`).

### Issue 3: `--output-device` Warning When Used with `--app`

[P2] (confidence: 9/10) `core/src/commands/record.rs` — The PRD says to ignore `--output-device` and emit a warning when `--app` is used. This validation should happen in `record.rs` after device validation but before creating the recorder:

```rust
if (options.app.is_some() || options.app_window.is_some()) && options.output_device_index.is_some() {
    emit_event(&json!({
        "type": "warning",
        "session_id": session.id.as_str(),
        "code": "output_device_ignored",
        "message": "Output device selection is ignored when using --app (process-specific capture doesn't use a device endpoint)"
    }));
}
```

### Issue 4: Error Message Quality for ProcessNotFound

[P2] (confidence: 8/10) `core/src/error.rs:18` — The current `ProcessNotFound(String)` variant just wraps a string. The error message should be actionable:

For `--app`: "Process 'Zoom.exe' not found. Ensure the application is running and has a visible window. Run without --app to record all system audio."

For `--app-window`: "No window matching 'Zoom Meeting' found. Ensure the application is running. Run without --app-window to record all system audio."

This gives the user a clear next step.

---

## Section 3: Test Review

### Test Framework Detection

**Runtime:** Rust (`Cargo.toml` present)
**Test framework:** Built-in `#[cfg(test)]` + `#[test]` (standard Rust)
**Test command:** `cargo test`
**Existing test locations:** Inline `mod tests` blocks in each source file

### Code Path Coverage Diagram

```
CODE PATH COVERAGE
═══════════════════════════════════════════════════

[+] core/src/wasapi/process_resolver.rs (NEW)
    │
    ├── resolve_by_process_name(name: &str) -> Result<u32>
    │   ├── [GAP] Happy path — finds process with matching name
    │   ├── [GAP] No windows found for process → ProcessNotFound
    │   ├── [GAP] Multiple windows for same process → returns first PID
    │   └── [GAP] Process name case sensitivity (exact match)
    │
    ├── resolve_by_window_title(title: &str) -> Result<u32>
    │   ├── [GAP] Happy path — finds window with matching substring
    │   ├── [GAP] No matching window → ProcessNotFound
    │   ├── [GAP] Case-insensitive matching ("zoom" matches "Zoom Meeting")
    │   └── [GAP] Multiple matching windows → returns first match
    │
    ├── check_windows_version() -> bool
    │   ├── [GAP] Build >= 20348 → true
    │   ├── [GAP] Build < 20348 → false
    │   └── [GAP] RtlGetVersion failure → false (safe default)
    │
    └── (internal) window_title_matches(title: &str, query: &str) -> bool
        ├── [GAP] Exact substring match
        ├── [GAP] Case-insensitive match
        ├── [GAP] Empty query → matches everything (edge case)
        └── [GAP] Empty title → no match

[+] core/src/wasapi/process_loopback.rs (NEW)
    │
    ├── ProcessLoopbackRecorder::new(filepath, pid) -> Result<Self>
    │   ├── [GAP] Happy path — COM activation succeeds
    │   ├── [GAP] ActivateAudioInterfaceAsync fails → error
    │   └── [GAP] Invalid PID → error
    │
    ├── ProcessLoopbackRecorder::is_process_dead() -> bool
    │   ├── [GAP] Process still running → false
    │   └── [GAP] Process exited → true
    │
    └── LoopbackCapture trait implementation
        ├── [GAP] stop() — clean shutdown
        ├── [GAP] get_frames_captured() — returns count
        ├── [GAP] has_audio_detected() — returns flag
        ├── [GAP] get_sample_rate() — returns rate
        └── [GAP] get_start_time() — returns time

[+] core/src/wasapi/capture_engine.rs (MODIFIED — add trait)
    │
    └── LoopbackCapture trait
        └── [GAP] Both WasapiLoopbackRecorder and ProcessLoopbackRecorder implement it

[+] core/src/commands/record.rs (MODIFIED — fallback logic)
    │
    ├── Startup: --app specified
    │   ├── [GAP] Version check passes → resolve PID → create process recorder
    │   ├── [GAP] Version check fails → fallback to system loopback + warning
    │   ├── [GAP] PID resolution fails → ProcessNotFound error
    │   └── [GAP] --output-device + --app → warning emitted, device ignored
    │
    ├── Mid-recording fallback
    │   ├── [GAP] Process exits → detect via is_process_dead()
    │   ├── [GAP] Swap to system loopback → warning event emitted
    │   └── [GAP] capture_mode changes from "process" to "system"
    │
    └── JSON events
        ├── [GAP] started event includes app, app_window, capture_mode
        ├── [GAP] audio_state event includes capture_mode
        └── [GAP] warning event emitted on fallback

[+] cli/src/cli.rs (MODIFIED — validation)
    │
    ├── [★★★ TESTED] --app flag parsed — cli.rs:test_app_flag_parsed
    ├── [★★★ TESTED] --app-window flag parsed — cli.rs:test_app_window_flag_parsed
    ├── [★★★ TESTED] app defaults to None — cli.rs:test_record_options_app_defaults_to_none
    └── [GAP] --app + --app-window mutual exclusivity validation

[+] core/src/commands/status.rs (MODIFIED — new fields)
    │
    ├── [GAP] StatusOutput includes app, app_window, capture_mode fields
    └── [GAP] parse_status_file handles new fields

─────────────────────────────────────────────────────
COVERAGE: 3/30+ paths tested (10%)
  Code paths: 3/25+ (12%) — only CLI flag parsing
  User flows: 0/5+ (0%)
QUALITY:  ★★★: 3  ★★: 0  ★: 0
GAPS: 27+ paths need tests
─────────────────────────────────────────────────────
```

### Required Tests (by module)

#### 1. Process Resolver — Unit Tests (`process_resolver.rs`)

The key insight: `EnumWindows` and `QueryFullProcessImageNameW` are Windows API calls that can't be easily mocked. But the **matching logic** can be extracted into pure functions and tested:

```rust
// Pure function: testable without Windows APIs
fn process_name_matches(image_path: &str, target_name: &str) -> bool {
    // Extract filename from full path, exact match
}

// Pure function: testable without Windows APIs
fn window_title_matches(title: &str, query: &str) -> bool {
    // Case-insensitive substring match
}

// Pure function: testable without Windows APIs
fn is_version_supported(build_number: u32) -> bool {
    build_number >= 20348
}
```

**Tests to add:**

| Test | What it verifies |
|------|-----------------|
| `process_name_matches_exact` | `"C:\\...\\Zoom.exe"` matches `"Zoom.exe"` |
| `process_name_no_match` | `"C:\\...\\Teams.exe"` does NOT match `"Zoom.exe"` |
| `process_name_case_sensitive` | `"zoom.exe"` does NOT match `"Zoom.exe"` |
| `process_name_extracts_filename` | Full path `"C:\\Program Files\\Zoom\\Zoom.exe"` → `"Zoom.exe"` |
| `window_title_substring_match` | `"Zoom Meeting - John's Room"` matches query `"Zoom"` |
| `window_title_case_insensitive` | `"ZOOM MEETING"` matches query `"zoom"` |
| `window_title_no_match` | `"Microsoft Teams"` does NOT match `"Zoom"` |
| `window_title_empty_query` | Empty query matches everything (or returns error — decide) |
| `version_supported_exact_threshold` | Build 20348 → true |
| `version_supported_above` | Build 22000 → true |
| `version_unsupported_below` | Build 19041 → false |
| `version_unsupported_zero` | Build 0 → false |

#### 2. CLI Validation — Extend Existing Tests (`cli.rs`)

| Test | What it verifies |
|------|-----------------|
| `test_app_and_app_window_mutual_exclusivity` | Both flags present → error |
| `test_app_with_duration` | `--app Zoom.exe -d 60` → both parsed correctly |
| `test_app_with_mic_muted` | `--app Zoom.exe --mic-muted` → both parsed correctly |
| `test_app_window_with_spaces` | `--app-window "Zoom Meeting"` → parsed with spaces |

#### 3. Status Output — Extend Existing Tests (`status.rs`)

| Test | What it verifies |
|------|-----------------|
| `test_parse_status_with_capture_mode` | Status JSON with `capture_mode: "process"` parsed correctly |
| `test_parse_status_with_app_field` | Status JSON with `app: "Zoom.exe"` parsed correctly |
| `test_status_output_serializes_capture_mode` | `StatusOutput` with capture_mode serializes to JSON |

#### 4. Integration-Level Tests (record command logic)

These are harder to unit test because they involve WASAPI. The recommended approach is to test the **decision logic** separately from the WASAPI calls:

| Test | What it verifies |
|------|-----------------|
| `test_should_use_process_loopback` | Given app=Some, version_ok=true → returns ProcessLoopback |
| `test_should_fallback_on_old_version` | Given app=Some, version_ok=false → returns SystemLoopback + warning |
| `test_should_use_system_loopback_no_app` | Given app=None → returns SystemLoopback |

### Test Plan Summary

**Total new tests:** ~20
**Test files modified:** 3 (process_resolver.rs, cli.rs, status.rs)
**Test approach:** Pure function extraction for unit testing, integration tests deferred to manual QA

---

## Section 4: Performance Review

### Issue 1: EnumWindows Performance

[P3] (confidence: 6/10) — Medium confidence, verify this is actually an issue.

`EnumWindows` iterates all top-level windows on the system. On a typical Windows desktop, there are 50-200 top-level windows. For each window, we call `GetWindowThreadProcessId` (fast) and potentially `OpenProcess` + `QueryFullProcessImageNameW` (slightly slower due to process handle creation).

This happens once at startup, not in a hot loop. Even with 200 windows, this should complete in <50ms. Not a concern.

### Issue 2: Process Liveness Check in Recording Thread

[P2] (confidence: 8/10) `core/src/wasapi/process_loopback.rs` — The recording thread needs to periodically check if the target process is still alive. Two approaches:

**A) Check on every `GetBuffer` error** — Only detects exit when WASAPI reports it. May have a delay.

**B) Periodic `OpenProcess` + `WaitForSingleObject(0)` check** — Explicit check every N iterations of the recording loop.

**Recommendation:** Use both. Check `GetBuffer` errors for immediate detection, and add a periodic liveness check every ~100 loop iterations (~1 second) as a safety net. The `WaitForSingleObject(handle, 0)` call is non-blocking and takes <1μs.

### Issue 3: Mid-Recording Fallback Audio Gap

[P3] (confidence: 7/10) — When the process loopback recorder stops and the system loopback recorder starts, there's a gap of ~100-500ms where no audio is captured. The silence injection logic in both recorders will fill this gap with silence in the WAV file, maintaining duration alignment.

For meeting recordings, a 500ms silence gap when the meeting app crashes is completely acceptable. No action needed.

### Issue 4: Memory — No Concerns

The new module adds one thread (same as existing loopback recorder), one WAV writer, and a few atomic variables. Memory impact is negligible.

---

## What Already Exists

| Component | Status | Notes |
|-----------|--------|-------|
| `--app` CLI flag parsing | ✅ Complete | `cli/src/cli.rs:168` |
| `--app-window` CLI flag parsing | ✅ Complete | `cli/src/cli.rs:169` |
| `RecordOptions.app` field | ✅ Complete | `core/src/commands/record.rs:38` |
| `RecordOptions.app_window` field | ✅ Complete | `core/src/commands/record.rs:39` |
| `ProcessNotFound` error variant | ✅ Complete | `core/src/error.rs:18` |
| `UnsupportedWindowsVersion` error variant | ✅ Complete | `core/src/error.rs:21` |
| Loopback recording pattern | ✅ Template | `core/src/wasapi/loopback.rs` |
| Capture engine utilities | ✅ Reusable | `core/src/wasapi/capture_engine.rs` |
| JSON event emission | ✅ Extend | `core/src/commands/record.rs:emit_event()` |
| CLI tests for --app flags | ✅ Complete | `cli/src/cli.rs:test_app_flag_parsed` |

## NOT in Scope

| Item | Rationale |
|------|-----------|
| Extracting shared recording loop (DRY refactor) | Recommended but can be done as a prep PR before the main feature. Not blocking. |
| Background process detection (no windows) | Would require `CreateToolhelp32Snapshot` which violates `claude.md` antivirus rules |
| Multiple app targeting | PRD explicitly defers this |
| App auto-detection | PRD explicitly defers this |
| Cross-platform support | Windows-only WASAPI feature |
| Real-time app switching | PRD explicitly defers this |
| Event-driven capture integration | Separate feature (Phase 1 in TODOS.md) |

## Failure Modes

| Codepath | Failure | Test? | Error handling? | User sees? | Critical? |
|----------|---------|-------|-----------------|------------|-----------|
| Process resolver: no windows for process | `ProcessNotFound` | ✅ (planned) | ✅ | Clear error message | No |
| COM activation: `ActivateAudioInterfaceAsync` fails | Activation error | ✅ (planned) | ✅ | Error event | No |
| COM activation: invalid PID | Activation error | ✅ (planned) | ✅ | Error event | No |
| Mid-recording: process exits | Fallback to system loopback | ✅ (planned) | ✅ | Warning event | No |
| Mid-recording: fallback recorder creation fails | **Recording stops** | ❌ No test | ⚠️ Partial | Error event, recording lost | **YES — CRITICAL GAP** |
| Version check: RtlGetVersion fails | Treat as unsupported | ✅ (planned) | ✅ | Warning + fallback | No |
| `--app` + `--output-device`: device ignored | Warning emitted | ✅ (planned) | ✅ | Warning event | No |

**Critical gap:** If the mid-recording fallback fails (e.g., system loopback recorder creation throws an error), the recording is lost. The plan should handle this by:
1. Catching the error from `WasapiLoopbackRecorder::new()`
2. If it fails, continue without any loopback (mic-only recording)
3. Emit an error event explaining the situation

This is a realistic scenario: if the audio device is disconnected at the same time the target process exits, the fallback recorder creation would fail.

## Worktree Parallelization Strategy

| Step | Modules touched | Depends on |
|------|----------------|------------|
| Process Resolver | `wasapi/process_resolver.rs` | — |
| Process Loopback Recorder | `wasapi/process_loopback.rs`, `wasapi/capture_engine.rs` | — |
| Record Command Integration | `commands/record.rs`, `commands/status.rs`, `status.rs` | Process Resolver + Process Loopback Recorder |
| CLI Validation | `cli/src/cli.rs` | — |
| Cargo.toml Features | `core/Cargo.toml` | — |

**Parallel lanes:**

```
Lane A: Process Resolver (independent)
Lane B: Process Loopback Recorder + LoopbackCapture trait (independent)
Lane C: CLI Validation + Cargo.toml (independent)
  ─── merge A + B + C ───
Lane D: Record Command Integration (depends on A + B)
```

Launch A + B + C in parallel. Merge all three. Then D.

**Conflict flags:** Lanes A and B both touch `wasapi/` but different files. No conflict. Lane D touches `record.rs` which no other lane touches. Clean.

---

## Completion Summary

- **Step 0: Scope Challenge** — Scope accepted as-is (9 files, 2 new modules, within complexity threshold)
- **Architecture Review:** 6 decisions made, 0 unresolved
  - LoopbackCapture trait (accepted)
  - Separate process_resolver.rs and process_loopback.rs (accepted)
  - Mid-recording fallback via AtomicBool + status loop detection (accepted)
  - COM activation via #[implement] macro (accepted)
  - EnumWindows-based process detection (accepted)
  - Warning event type with code field (accepted)
- **Code Quality Review:** 4 issues found, all resolved
  - DRY: Extract shared recording loop (recommended, not blocking)
  - Mutual exclusivity validation in cli.rs (accepted)
  - --output-device warning in record.rs (accepted)
  - Actionable error messages for ProcessNotFound (accepted)
- **Test Review:** Diagram produced, 27+ gaps identified, ~20 new tests planned
- **Performance Review:** 0 blocking issues (EnumWindows is fast, liveness check is cheap, fallback gap is acceptable)
- **NOT in scope:** Written (7 items)
- **What already exists:** Written (10 items)
- **TODOS.md updates:** 1 item proposed (see below)
- **Failure modes:** 1 critical gap flagged (fallback recorder creation failure)
- **Parallelization:** 4 lanes, 3 parallel / 1 sequential
- **Lake Score:** 6/6 recommendations chose complete option

## TODOS.md Update

### Fallback Recorder Creation Failure Handling
- **What:** Handle the case where the mid-recording fallback to system loopback fails (e.g., audio device disconnected simultaneously).
- **Why:** If the target process exits AND the system loopback recorder can't be created, the recording is lost. Currently unhandled.
- **Pros:** Prevents total recording loss in a rare but realistic scenario.
- **Cons:** Adds complexity to the fallback path. The scenario is rare (process exit + device disconnect simultaneously).
- **Context:** During mid-recording fallback, if `WasapiLoopbackRecorder::new()` fails, the recorder should continue with mic-only recording and emit an error event. This is a defensive measure, not a common path.
- **Depends on:** Process-specific capture implementation (this PRD).
