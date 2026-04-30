## Parent PRD

#25

## What to build

Build the process resolver module — the pure-logic foundation for process-specific audio capture. This is a new `wasapi/process_resolver.rs` module that resolves a process name or window title to a PID, plus a Windows version check for process loopback support.

**Process resolution by name (`--app Zoom.exe`):**
- Enumerate top-level windows via `EnumWindows`
- Get each window's owning PID via `GetWindowThreadProcessId`
- Query the process image name via `OpenProcess` + `QueryFullProcessImageNameW`
- Return the PID of the first exact case-sensitive filename match

**Process resolution by window title (`--app-window "Zoom Meeting"`):**
- Enumerate top-level windows via `EnumWindows`
- Get window title via `GetWindowTextW`
- Perform case-insensitive substring matching
- Return the PID of the first match

**Windows version check:**
- Use `RtlGetVersion` to check if Build ≥ 20348 (required for process-specific loopback)
- If `RtlGetVersion` fails, treat as unsupported (safe default)

**CLI mutual exclusivity validation:**
- If both `--app` and `--app-window` are specified, emit an error and exit
- Validation goes in `handle_record_command()` in `cli.rs`, matching existing validation patterns

**Key design decisions (from eng review):**
- Uses `EnumWindows` approach (NOT `CreateToolhelp32Snapshot` or `tasklist` per `claude.md` antivirus rules)
- Only finds processes with visible top-level windows (acceptable — meeting apps always have windows)
- Extract matching logic into pure functions (`process_name_matches`, `window_title_matches`, `is_version_supported`) for unit testing
- Error messages are actionable: include process name, what went wrong, and suggested next step

**New `windows` crate features needed:** `Win32_UI_WindowsAndMessaging`, `Win32_System_Threading`, `Win32_System_SystemInformation`

**Files touched:**
- NEW: `core/src/wasapi/process_resolver.rs`
- MODIFIED: `core/src/wasapi/mod.rs` (module declaration)
- MODIFIED: `core/Cargo.toml` (new windows crate features)
- MODIFIED: `cli/src/cli.rs` (mutual exclusivity validation)

## Acceptance criteria

- [ ] New `wasapi/process_resolver.rs` module with `resolve_by_process_name(name) -> Result<u32>` and `resolve_by_window_title(title) -> Result<u32>`
- [ ] `resolve_by_process_name` uses `EnumWindows` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` — exact case-sensitive match on filename
- [ ] `resolve_by_window_title` uses `EnumWindows` + `GetWindowTextW` — case-insensitive substring match
- [ ] Both return `ProcessNotFound` with actionable error message when no match found (e.g., "Process 'Zoom.exe' not found. Ensure the application is running and has a visible window.")
- [ ] `check_windows_version() -> bool` returns true for Build ≥ 20348, false otherwise. `RtlGetVersion` failure → false
- [ ] CLI validation in `handle_record_command()`: both `--app` and `--app-window` specified → error + exit
- [ ] `Win32_UI_WindowsAndMessaging`, `Win32_System_Threading`, `Win32_System_SystemInformation` features added to `core/Cargo.toml`
- [ ] Module declared in `wasapi/mod.rs`
- [ ] Unit tests for pure matching functions: `process_name_matches` (exact match, no match, case sensitivity, full path extraction), `window_title_matches` (substring, case-insensitive, no match, empty query), `is_version_supported` (threshold, above, below, zero)
- [ ] CLI test: `--app` + `--app-window` mutual exclusivity validation
- [ ] All existing tests pass (`cargo test`)

## Blocked by

None — can start immediately

## User stories addressed

- User story 3: Specify target app by process name (`--app Zoom.exe`)
- User story 4: Specify target app by window title (`--app-window "Zoom Meeting"`)
- User story 5: Case-insensitive substring matching for `--app-window`
- User story 6: Exact process name matching for `--app`
- User story 9: Clear `ProcessNotFound` error if target app isn't running
- User story 12: Windows version check for fallback behavior
- User story 17: `--app` and `--app-window` mutually exclusive
