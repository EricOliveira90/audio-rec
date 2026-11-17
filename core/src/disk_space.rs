//! Free disk space checks for recording and post-processing.
//!
//! Why this module exists: on 2026-11-12, a 33-minute recording failed in the final
//! merge because drive C: had 0 bytes free. FFmpeg died partway through writing the
//! merged WAV with exit code -28 (`AVERROR(ENOSPC)`, "No space left on device"). The
//! error message did not say "disk full", so the cause was not obvious.
//!
//! This module lets the recorder:
//! 1. Warn at recording start when free space is low.
//! 2. Refuse to start a merge that cannot fit, with a clear message, before
//!    FFmpeg spends minutes writing a file that will be truncated.
//! 3. Recognise a disk-full FFmpeg failure and report it in plain words.

use std::path::Path;

/// Extra room kept free on top of the estimate (status files, logs, rounding).
pub const SAFETY_MARGIN_BYTES: u64 = 128 * 1024 * 1024;

/// AAC bitrate used by the M4A merge (see `add_encoding_params` in recorder.rs).
const M4A_BITS_PER_SECOND: u64 = 192_000;

/// Raw capture rates written during recording: loopback is 16-bit stereo at the
/// session rate. The mic is 16-bit mono at its device rate (at most 48 kHz).
const BYTES_PER_SAMPLE: u64 = 2;

/// Duration assumed when estimating the raw capture size of an indefinite recording.
pub const INDEFINITE_ESTIMATE_SECS: u64 = 60 * 60;

/// FFmpeg exits with `AVERROR(ENOSPC)` = -28 when a write fails because the disk is full.
const FFMPEG_ENOSPC_EXIT_CODE: i32 = -28;

/// Returns the bytes free for the current user on the volume that holds `path`.
/// Returns `None` if the value cannot be read (the caller then skips the check).
///
/// `path` may be a file or directory that does not exist yet; the nearest existing
/// ancestor is queried.
pub fn available_bytes(path: &Path) -> Option<u64> {
    let mut dir = path;
    while !dir.exists() {
        dir = dir.parent()?;
    }
    if dir.is_file() {
        dir = dir.parent()?;
    }
    query_available_bytes(dir)
}

#[cfg(windows)]
fn query_available_bytes(dir: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let dir_name = HSTRING::from(dir);
    let mut free_bytes_for_caller: u64 = 0;
    // SAFETY: `dir_name` is a valid, null-terminated wide string that outlives the call,
    // and the out pointer refers to a live local u64. This is the standard Win32 call for
    // reading free disk space and only reads volume metadata.
    unsafe { GetDiskFreeSpaceExW(&dir_name, Some(&mut free_bytes_for_caller), None, None) }
        .ok()
        .map(|_| free_bytes_for_caller)
}

#[cfg(not(windows))]
fn query_available_bytes(_dir: &Path) -> Option<u64> {
    None
}

/// Estimated size of the merged output file.
/// WAV: 16-bit PCM (FFmpeg's default WAV encoder) at `sample_rate` with `channels`.
/// M4A: AAC at 192 kbps, plus 5% for container overhead.
pub fn estimate_merged_output_bytes(
    duration_secs: f64,
    sample_rate: u32,
    channels: u16,
    is_m4a: bool,
) -> u64 {
    let secs = duration_secs.max(0.0);
    if is_m4a {
        (secs * (M4A_BITS_PER_SECOND / 8) as f64 * 1.05).ceil() as u64
    } else {
        (secs * f64::from(sample_rate) * f64::from(channels) * BYTES_PER_SAMPLE as f64).ceil()
            as u64
            + 44
    }
}

/// Peak extra bytes that post-processing writes to the recordings directory.
///
/// The merge writes one merged file. Silence trimming writes a second, trimmed copy
/// next to it before it deletes the original, so with trimming the peak is two files.
/// The raw tracks are already on disk and are kept until the end, so they are not
/// counted here.
pub fn required_post_processing_bytes(merged_output_bytes: u64, trim_silence: bool) -> u64 {
    let copies = if trim_silence { 2 } else { 1 };
    merged_output_bytes * copies + SAFETY_MARGIN_BYTES
}

/// Estimated total bytes a recording uses at its peak: raw tracks + post-processing.
pub fn estimate_recording_peak_bytes(
    duration_secs: u64,
    sample_rate: u32,
    channels: u16,
    is_m4a: bool,
    trim_silence: bool,
) -> u64 {
    let secs = duration_secs as f64;
    let loopback_raw = secs * f64::from(sample_rate) * f64::from(channels) * BYTES_PER_SAMPLE as f64;
    let mic_raw = secs * f64::from(sample_rate) * BYTES_PER_SAMPLE as f64;
    let merged = estimate_merged_output_bytes(secs, sample_rate, channels, is_m4a);
    (loopback_raw + mic_raw).ceil() as u64 + required_post_processing_bytes(merged, trim_silence)
}

/// Returns true when an FFmpeg failure was caused by a full disk.
pub fn is_disk_full_failure(exit_code: Option<i32>, stderr: &str) -> bool {
    if exit_code == Some(FFMPEG_ENOSPC_EXIT_CODE) {
        return true;
    }
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("no space left on device") || stderr.contains("there is not enough space")
}

/// Formats a byte count as megabytes for user messages.
pub fn format_mb(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_estimate_matches_failed_recording() {
        // The failed 2026-11-12 recording: 2020.417 s, 48 kHz stereo WAV.
        // The recovered file is 387,920,142 bytes.
        let bytes = estimate_merged_output_bytes(2020.417, 48_000, 2, false);
        assert!((387_000_000..389_000_000).contains(&bytes), "{bytes}");
    }

    #[test]
    fn m4a_estimate_is_about_192_kbps() {
        let bytes = estimate_merged_output_bytes(3600.0, 48_000, 2, true);
        assert_eq!(bytes, (3600.0 * 24_000.0 * 1.05_f64).ceil() as u64);
    }

    #[test]
    fn trimming_doubles_the_post_processing_need() {
        let merged = 400 * 1024 * 1024;
        assert_eq!(required_post_processing_bytes(merged, false), merged + SAFETY_MARGIN_BYTES);
        assert_eq!(required_post_processing_bytes(merged, true), 2 * merged + SAFETY_MARGIN_BYTES);
    }

    #[test]
    fn recording_peak_includes_raw_tracks() {
        let peak = estimate_recording_peak_bytes(3600, 48_000, 2, false, false);
        let merged = estimate_merged_output_bytes(3600.0, 48_000, 2, false);
        assert!(peak > merged * 2, "raw loopback + mic + merged must exceed 2x merged");
    }

    #[test]
    fn detects_ffmpeg_enospc_exit_code() {
        assert!(is_disk_full_failure(Some(-28), ""));
        assert!(!is_disk_full_failure(Some(1), ""));
        assert!(!is_disk_full_failure(None, ""));
    }

    #[test]
    fn detects_disk_full_messages() {
        assert!(is_disk_full_failure(Some(1), "Error writing trailer: No space left on device"));
        assert!(is_disk_full_failure(None, "There is not enough space on the disk."));
        assert!(!is_disk_full_failure(Some(1), "Invalid data found when processing input"));
    }

    #[test]
    fn available_bytes_walks_up_to_an_existing_directory() {
        let missing = std::env::temp_dir().join("audio-rec-no-such-dir").join("file.wav");
        let free = available_bytes(&missing);
        if cfg!(windows) {
            assert!(free.is_some());
        } else {
            assert!(free.is_none());
        }
    }

    #[test]
    fn format_mb_rounds_up() {
        assert_eq!(format_mb(1), "1 MB");
        assert_eq!(format_mb(1024 * 1024), "1 MB");
        assert_eq!(format_mb(1024 * 1024 + 1), "2 MB");
    }
}
