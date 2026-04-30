//! Process resolver for process-specific audio capture
//!
//! Resolves a process name or window title to a PID for use with
//! Windows 10's process-specific loopback audio capture API.
//!
//! # Design
//!
//! Pure matching logic is extracted into testable functions.
//! Windows API calls (EnumWindows, etc.) are isolated in platform-specific functions.
//!
//! # Why process enumeration is used here
//!
//! This module uses EnumWindows + GetWindowThreadProcessId to find the target
//! process for audio capture. This is NOT surveillance or monitoring — it's
//! a one-time lookup to identify which process's audio to capture, similar to
//! how a user would select an audio source in OBS or similar tools.

/// Check if a full process image path matches a query process name.
///
/// Performs exact case-sensitive filename matching against the last
/// path component (the executable name).
///
/// # Examples
/// ```
/// use audio_rec_core::wasapi::process_resolver::process_name_matches;
/// assert!(process_name_matches(r"C:\Program Files\Zoom\Zoom.exe", "Zoom.exe"));
/// assert!(!process_name_matches(r"C:\Program Files\Zoom\Zoom.exe", "zoom.exe"));
/// ```
pub fn process_name_matches(full_path: &str, query: &str) -> bool {
    // Extract filename from full path
    std::path::Path::new(full_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == query)
        .unwrap_or(false)
}

/// Check if a window title matches a query string.
///
/// Performs case-insensitive substring matching.
///
/// # Examples
/// ```
/// use audio_rec_core::wasapi::process_resolver::window_title_matches;
/// assert!(window_title_matches("Zoom Meeting - My Room", "Zoom Meeting"));
/// assert!(window_title_matches("Zoom Meeting - My Room", "zoom meeting"));
/// ```
pub fn window_title_matches(title: &str, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    title.to_lowercase().contains(&query.to_lowercase())
}

/// Check if the Windows build number supports process-specific loopback.
///
/// Process loopback requires Windows 10 Build 20348 or later
/// (Windows Server 2022 / Windows 11).
pub fn is_version_supported(build_number: u32) -> bool {
    build_number >= 20348
}

/// Check if the current Windows version supports process-specific loopback.
///
/// Uses `RtlGetVersion` from `ntdll.dll` to get the actual OS version.
/// Unlike `GetVersionExW`, `RtlGetVersion` is not subject to application
/// compatibility shims on Windows 10, so it always returns the true build number.
/// Returns false if the version cannot be determined (safe default).
#[cfg(windows)]
pub fn check_windows_version() -> bool {
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

    // RtlGetVersion from ntdll.dll — returns the true OS version without shims
    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(lpVersionInformation: *mut OSVERSIONINFOW) -> i32;
    }

    unsafe {
        let mut version_info: OSVERSIONINFOW = std::mem::zeroed();
        version_info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;

        // RtlGetVersion returns NTSTATUS: 0 = STATUS_SUCCESS
        let status = RtlGetVersion(&mut version_info);
        if status != 0 {
            tracing::warn!("RtlGetVersion failed with NTSTATUS 0x{:08X}, assuming process loopback is unsupported", status);
            return false;
        }

        let build = version_info.dwBuildNumber;
        tracing::info!("Windows build number (via RtlGetVersion): {}", build);
        is_version_supported(build)
    }
}

#[cfg(not(windows))]
pub fn check_windows_version() -> bool {
    false
}

/// Resolve a process name to a PID by enumerating top-level windows.
///
/// Uses EnumWindows to find visible windows, then checks each window's
/// owning process image name for an exact case-sensitive match.
///
/// # Why EnumWindows (not CreateToolhelp32Snapshot)
///
/// EnumWindows only finds processes with visible windows, which is acceptable
/// for meeting apps (Zoom, Teams, etc.) that always have windows. This approach
/// avoids CreateToolhelp32Snapshot which is more commonly flagged by antivirus.
#[cfg(windows)]
pub fn resolve_by_process_name(name: &str) -> crate::error::Result<u32> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::System::Threading::*;

    let result: Mutex<Option<u32>> = Mutex::new(None);
    let query = name.to_string();

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback_name),
            LPARAM(&(&result, &query) as *const (&Mutex<Option<u32>>, &String) as isize),
        );
    }

    result
        .into_inner()
        .unwrap_or(None)
        .ok_or_else(|| {
            crate::error::RecorderError::ProcessNotFound(format!(
                "Process '{}' not found. Ensure the application is running and has a visible window.",
                name
            ))
        })
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_callback_name(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::BOOL {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::System::Threading::*;
    use std::sync::Mutex;

    let data = &*(lparam.0 as *const (&Mutex<Option<u32>>, &String));
    let (result, query) = data;

    // Skip invisible windows
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1); // Continue enumeration
    }

    // Get the owning process ID
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));

    if pid == 0 {
        return BOOL(1);
    }

    // Open the process to query its image name
    if let Ok(process_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
        let mut buffer = [0u16; 260];
        let mut size = buffer.len() as u32;

        if QueryFullProcessImageNameW(process_handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buffer.as_mut_ptr()), &mut size).is_ok() {
            let path = String::from_utf16_lossy(&buffer[..size as usize]);
            if process_name_matches(&path, query) {
                if let Ok(mut r) = result.lock() {
                    *r = Some(pid);
                }
                let _ = windows::Win32::Foundation::CloseHandle(process_handle);
                return BOOL(0); // Stop enumeration
            }
        }

        let _ = windows::Win32::Foundation::CloseHandle(process_handle);
    }

    BOOL(1) // Continue enumeration
}

/// Resolve a window title to a PID by enumerating top-level windows.
///
/// Uses EnumWindows to find visible windows with titles that contain
/// the query string (case-insensitive substring match).
#[cfg(windows)]
pub fn resolve_by_window_title(title: &str) -> crate::error::Result<u32> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    let result: Mutex<Option<u32>> = Mutex::new(None);
    let query = title.to_string();

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback_title),
            LPARAM(&(&result, &query) as *const (&Mutex<Option<u32>>, &String) as isize),
        );
    }

    result
        .into_inner()
        .unwrap_or(None)
        .ok_or_else(|| {
            crate::error::RecorderError::ProcessNotFound(format!(
                "No window with title containing '{}' found. Ensure the application is running and has a visible window.",
                title
            ))
        })
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_callback_title(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::BOOL {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use std::sync::Mutex;

    let data = &*(lparam.0 as *const (&Mutex<Option<u32>>, &String));
    let (result, query) = data;

    // Skip invisible windows
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    // Get window title
    let mut title_buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, &mut title_buf);

    if len > 0 {
        let title = String::from_utf16_lossy(&title_buf[..len as usize]);

        if window_title_matches(&title, query) {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));

            if pid != 0 {
                if let Ok(mut r) = result.lock() {
                    *r = Some(pid);
                }
                return BOOL(0); // Stop enumeration
            }
        }
    }

    BOOL(1) // Continue enumeration
}

// Non-windows stubs
#[cfg(not(windows))]
pub fn resolve_by_process_name(name: &str) -> crate::error::Result<u32> {
    Err(crate::error::RecorderError::ProcessNotFound(format!(
        "Process resolution is only supported on Windows. Cannot find '{}'.",
        name
    )))
}

#[cfg(not(windows))]
pub fn resolve_by_window_title(title: &str) -> crate::error::Result<u32> {
    Err(crate::error::RecorderError::ProcessNotFound(format!(
        "Window title resolution is only supported on Windows. Cannot find '{}'.",
        title
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── process_name_matches: exact case-sensitive filename match ──

    #[test]
    fn process_name_matches_exact_match() {
        assert!(process_name_matches(
            r"C:\Program Files\Zoom\bin\Zoom.exe",
            "Zoom.exe"
        ));
    }

    #[test]
    fn process_name_matches_no_match() {
        assert!(!process_name_matches(
            r"C:\Program Files\Zoom\bin\Zoom.exe",
            "Teams.exe"
        ));
    }

    #[test]
    fn process_name_matches_case_sensitive() {
        // Must be exact case — "zoom.exe" != "Zoom.exe"
        assert!(!process_name_matches(
            r"C:\Program Files\Zoom\bin\Zoom.exe",
            "zoom.exe"
        ));
    }

    #[test]
    fn process_name_matches_extracts_filename_from_full_path() {
        assert!(process_name_matches(
            r"C:\Users\user\AppData\Local\Programs\Microsoft VS Code\Code.exe",
            "Code.exe"
        ));
    }

    #[test]
    fn process_name_matches_bare_filename() {
        // Works even without a directory path
        assert!(process_name_matches("Zoom.exe", "Zoom.exe"));
    }

    #[test]
    fn process_name_matches_empty_query_returns_false() {
        assert!(!process_name_matches(r"C:\Zoom.exe", ""));
    }

    #[test]
    fn process_name_matches_empty_path_returns_false() {
        assert!(!process_name_matches("", "Zoom.exe"));
    }

    #[test]
    fn process_name_matches_partial_name_no_match() {
        // "Zoom" should NOT match "Zoom.exe" — must be exact
        assert!(!process_name_matches(
            r"C:\Program Files\Zoom\Zoom.exe",
            "Zoom"
        ));
    }

    // ── window_title_matches: case-insensitive substring match ──

    #[test]
    fn window_title_matches_substring() {
        assert!(window_title_matches(
            "Zoom Meeting - My Room",
            "Zoom Meeting"
        ));
    }

    #[test]
    fn window_title_matches_case_insensitive() {
        assert!(window_title_matches(
            "Zoom Meeting - My Room",
            "zoom meeting"
        ));
    }

    #[test]
    fn window_title_matches_no_match() {
        assert!(!window_title_matches(
            "Zoom Meeting - My Room",
            "Microsoft Teams"
        ));
    }

    #[test]
    fn window_title_matches_empty_query_returns_false() {
        assert!(!window_title_matches("Zoom Meeting", ""));
    }

    #[test]
    fn window_title_matches_exact_match() {
        assert!(window_title_matches("Zoom Meeting", "Zoom Meeting"));
    }

    #[test]
    fn window_title_matches_partial_word() {
        assert!(window_title_matches("Microsoft Teams Meeting", "Teams"));
    }

    #[test]
    fn window_title_matches_empty_title_returns_false() {
        assert!(!window_title_matches("", "Zoom"));
    }

    // ── is_version_supported: build number threshold ──

    #[test]
    fn version_supported_at_threshold() {
        assert!(is_version_supported(20348));
    }

    #[test]
    fn version_supported_above_threshold() {
        assert!(is_version_supported(22621)); // Windows 11 22H2
    }

    #[test]
    fn version_unsupported_below_threshold() {
        assert!(!is_version_supported(19045)); // Windows 10 22H2
    }

    #[test]
    fn version_unsupported_zero() {
        assert!(!is_version_supported(0));
    }

    #[test]
    fn version_supported_well_above() {
        assert!(is_version_supported(30000)); // Future version
    }

    #[test]
    fn version_unsupported_just_below() {
        assert!(!is_version_supported(20347));
    }

    // ── check_windows_version: integration test ──

    #[cfg(windows)]
    #[test]
    fn check_windows_version_returns_true_on_modern_windows() {
        // On Windows 11 (Build 22000+), this should return true.
        // This test verifies the function actually calls the OS and gets a real build number.
        // If the function is broken (e.g., GetVersionExW returns shimmed values),
        // this test would fail on Windows 10 Build 20348+ where shims report lower builds.
        let result = check_windows_version();
        // We're running on Windows 11, so this must be true
        assert!(
            result,
            "check_windows_version() should return true on Windows 11 (Build 22000+)"
        );
    }
}
