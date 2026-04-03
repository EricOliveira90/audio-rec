//! Shared WASAPI capture engine
//!
//! Extracts the ~70% shared logic from `loopback.rs` and `microphone.rs` into a
//! reusable capture engine. Both recorders become thin wrappers that configure
//! the shared engine differently.
//!
//! # Safety
//!
//! This module uses `unsafe` code to interact with the Windows Audio Session API (WASAPI).
//! All unsafe operations follow the same safety invariants documented in `loopback.rs`.

use std::sync::atomic::{AtomicBool, Ordering};

// ── Sample format detection ──

/// Detected audio sample format from WASAPI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// 32-bit IEEE float (most common for modern WASAPI)
    Float32,
    /// 32-bit signed integer
    Int32,
    /// 16-bit signed integer (legacy)
    Int16,
    /// Unsupported format — will write silence
    Unsupported,
}

/// Detect the sample format from WASAPI format parameters.
///
/// WASAPI usually returns WAVEFORMATEXTENSIBLE (format_tag=0xFFFE) for modern formats.
/// For WAVEFORMATEXTENSIBLE, we check the SubFormat GUID via bits_per_sample.
/// For legacy formats, we check the format_tag directly.
pub fn detect_sample_format(format_tag: u16, bits_per_sample: u16) -> SampleFormat {
    if format_tag == 0xFFFE {
        // WAVE_FORMAT_EXTENSIBLE — check bits_per_sample to determine actual format
        match bits_per_sample {
            32 => SampleFormat::Float32, // Assume 32-bit is float in WASAPI shared mode
            16 => SampleFormat::Int16,
            _ => SampleFormat::Unsupported,
        }
    } else if format_tag == 3 {
        // WAVE_FORMAT_IEEE_FLOAT
        SampleFormat::Float32
    } else if format_tag == 1 {
        // WAVE_FORMAT_PCM
        match bits_per_sample {
            32 => SampleFormat::Int32,
            16 => SampleFormat::Int16,
            _ => SampleFormat::Unsupported,
        }
    } else {
        SampleFormat::Unsupported
    }
}

// ── Sample conversion ──

/// Convert a 32-bit float sample to 16-bit integer.
/// Clamps to [-1.0, 1.0] range before conversion.
#[inline]
pub fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// Convert a 32-bit integer sample to 16-bit integer.
/// Shifts right by 16 bits (equivalent to dividing by 65536).
#[inline]
pub fn i32_to_i16(sample: i32) -> i16 {
    (sample >> 16) as i16
}

// ── Write error tracking ──

/// Tracks sample write errors during recording.
/// On first failure, logs a warning. Recording continues (best effort).
/// On stop, the caller can check if any writes failed.
#[derive(Debug)]
pub struct WriteErrorTracker {
    has_error: AtomicBool,
}

impl WriteErrorTracker {
    pub fn new() -> Self {
        Self {
            has_error: AtomicBool::new(false),
        }
    }

    /// Record a write error. Logs on first occurrence.
    pub fn record_error(&self, err: &dyn std::fmt::Display) {
        if !self.has_error.swap(true, Ordering::Relaxed) {
            tracing::warn!("Sample write error: {}. Recording may be incomplete.", err);
        }
    }

    /// Check if any write errors occurred during recording.
    pub fn had_errors(&self) -> bool {
        self.has_error.load(Ordering::Relaxed)
    }
}

impl Default for WriteErrorTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ── Capture configuration ──

/// Configuration for the capture engine, determining how it behaves
/// for loopback vs microphone recording.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// If true, use AUDCLNT_STREAMFLAGS_LOOPBACK for system audio capture
    pub loopback: bool,
    /// If true, support muting (microphone only)
    pub mute_support: bool,
    /// Optional audio processing pipeline callback for mic (Phase 5)
    /// Placeholder for future real-time audio processing injection
    pub process_audio: bool,
}

impl CaptureConfig {
    /// Configuration for loopback (system audio) capture
    pub fn loopback() -> Self {
        Self {
            loopback: true,
            mute_support: false,
            process_audio: false,
        }
    }

    /// Configuration for microphone capture
    pub fn microphone() -> Self {
        Self {
            loopback: false,
            mute_support: true,
            process_audio: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Sample conversion: f32 → i16 ──

    #[test]
    fn f32_to_i16_converts_full_scale_positive() {
        assert_eq!(f32_to_i16(1.0), 32767);
    }

    #[test]
    fn f32_to_i16_converts_full_scale_negative() {
        assert_eq!(f32_to_i16(-1.0), -32767);
    }

    #[test]
    fn f32_to_i16_converts_silence() {
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn f32_to_i16_clamps_above_one() {
        assert_eq!(f32_to_i16(1.5), 32767);
    }

    #[test]
    fn f32_to_i16_clamps_below_negative_one() {
        assert_eq!(f32_to_i16(-1.5), -32767);
    }

    #[test]
    fn f32_to_i16_converts_half_scale() {
        let result = f32_to_i16(0.5);
        // 0.5 * 32767 = 16383.5, truncated to 16383
        assert_eq!(result, 16383);
    }

    // ── Sample conversion: i32 → i16 ──

    #[test]
    fn i32_to_i16_converts_full_scale_positive() {
        // i32 max = 2147483647, >> 16 = 32767
        assert_eq!(i32_to_i16(i32::MAX), 32767);
    }

    #[test]
    fn i32_to_i16_converts_full_scale_negative() {
        // i32 min = -2147483648, >> 16 = -32768
        assert_eq!(i32_to_i16(i32::MIN), -32768);
    }

    #[test]
    fn i32_to_i16_converts_silence() {
        assert_eq!(i32_to_i16(0), 0);
    }

    #[test]
    fn i32_to_i16_converts_mid_range() {
        // 1048576 = 0x100000, >> 16 = 16
        assert_eq!(i32_to_i16(1048576), 16);
    }

    // ── Format detection: WAVEFORMATEXTENSIBLE ──

    #[test]
    fn detect_format_extensible_32bit_is_float() {
        assert_eq!(
            detect_sample_format(0xFFFE, 32),
            SampleFormat::Float32
        );
    }

    #[test]
    fn detect_format_extensible_16bit_is_int16() {
        assert_eq!(
            detect_sample_format(0xFFFE, 16),
            SampleFormat::Int16
        );
    }

    #[test]
    fn detect_format_extensible_24bit_is_unsupported() {
        assert_eq!(
            detect_sample_format(0xFFFE, 24),
            SampleFormat::Unsupported
        );
    }

    // ── Format detection: Legacy format tags ──

    #[test]
    fn detect_format_ieee_float_tag() {
        // WAVE_FORMAT_IEEE_FLOAT = 3
        assert_eq!(
            detect_sample_format(3, 32),
            SampleFormat::Float32
        );
    }

    #[test]
    fn detect_format_pcm_16bit() {
        // WAVE_FORMAT_PCM = 1
        assert_eq!(
            detect_sample_format(1, 16),
            SampleFormat::Int16
        );
    }

    #[test]
    fn detect_format_pcm_32bit() {
        assert_eq!(
            detect_sample_format(1, 32),
            SampleFormat::Int32
        );
    }

    #[test]
    fn detect_format_unknown_tag_is_unsupported() {
        assert_eq!(
            detect_sample_format(99, 16),
            SampleFormat::Unsupported
        );
    }

    // ── Write error tracking ──

    #[test]
    fn write_error_tracker_starts_clean() {
        let tracker = WriteErrorTracker::new();
        assert!(!tracker.had_errors());
    }

    #[test]
    fn write_error_tracker_records_first_error() {
        let tracker = WriteErrorTracker::new();
        tracker.record_error(&"test error");
        assert!(tracker.had_errors());
    }

    #[test]
    fn write_error_tracker_stays_true_after_multiple_errors() {
        let tracker = WriteErrorTracker::new();
        tracker.record_error(&"error 1");
        tracker.record_error(&"error 2");
        assert!(tracker.had_errors());
    }

    // ── CaptureConfig ──

    #[test]
    fn capture_config_loopback_has_correct_flags() {
        let config = CaptureConfig::loopback();
        assert!(config.loopback);
        assert!(!config.mute_support);
    }

    #[test]
    fn capture_config_microphone_has_correct_flags() {
        let config = CaptureConfig::microphone();
        assert!(!config.loopback);
        assert!(config.mute_support);
    }
}
