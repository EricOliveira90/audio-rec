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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;
const QPC_TICKS_PER_SECOND: f64 = 10_000_000.0;
const TIMING_ANCHOR_INTERVAL_SECONDS: u64 = 5;
const TIMESTAMP_ERROR_FLAG: u32 = 0x4;
const DATA_DISCONTINUITY_FLAG: u32 = 0x1;
const MAX_MODEL_RESIDUAL_100NS: f64 = 200_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimingAnchor {
    pub(crate) wav_frame: u64,
    pub(crate) device_position: u64,
    pub(crate) qpc_100ns: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct CaptureTiming {
    pub(crate) sample_rate: u32,
    pub(crate) start_time: Option<Instant>,
    pub(crate) anchors: Vec<TimingAnchor>,
    pub(crate) timestamp_error_count: u64,
    last_valid_anchor: Option<TimingAnchor>,
    non_monotonic_count: u64,
    anchor_next_valid_packet: bool,
}

impl CaptureTiming {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            start_time: None,
            anchors: Vec::new(),
            timestamp_error_count: 0,
            last_valid_anchor: None,
            non_monotonic_count: 0,
            anchor_next_valid_packet: false,
        }
    }

    pub(crate) fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
    }

    pub(crate) fn set_start_time(&mut self, start_time: Instant) {
        self.start_time = Some(start_time);
    }

    pub(crate) fn record_packet(
        &mut self,
        wav_frame: u64,
        device_position: u64,
        qpc_100ns: u64,
        flags: u32,
    ) {
        let discontinuity = flags & DATA_DISCONTINUITY_FLAG != 0;
        if flags & TIMESTAMP_ERROR_FLAG != 0 {
            self.timestamp_error_count += 1;
            if discontinuity {
                if let Some(previous) = self.last_valid_anchor {
                    self.push_anchor(previous);
                }
                self.anchor_next_valid_packet = true;
            }
            return;
        }

        let anchor = TimingAnchor {
            wav_frame,
            device_position,
            qpc_100ns,
        };

        if let Some(previous) = self.last_valid_anchor {
            if anchor.wav_frame <= previous.wav_frame
                || anchor.device_position <= previous.device_position
                || anchor.qpc_100ns <= previous.qpc_100ns
            {
                self.non_monotonic_count += 1;
            }
            if discontinuity {
                self.push_anchor(previous);
            }
        }

        let interval_frames = u64::from(self.sample_rate)
            .saturating_mul(TIMING_ANCHOR_INTERVAL_SECONDS);
        let retain_periodic = self
            .anchors
            .last()
            .map(|previous| anchor.wav_frame.saturating_sub(previous.wav_frame) >= interval_frames)
            .unwrap_or(true);
        if retain_periodic || discontinuity || self.anchor_next_valid_packet {
            self.push_anchor(anchor);
        }
        self.anchor_next_valid_packet = false;
        self.last_valid_anchor = Some(anchor);
    }

    pub(crate) fn snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        if let Some(last) = snapshot.last_valid_anchor {
            snapshot.push_anchor(last);
        }
        snapshot
    }

    fn push_anchor(&mut self, anchor: TimingAnchor) {
        if self.anchors.last().copied() != Some(anchor) {
            self.anchors.push(anchor);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ClockModel {
    pub(crate) qpc_origin_100ns: f64,
    pub(crate) qpc_100ns_per_wav_frame: f64,
    pub(crate) span_seconds: f64,
    pub(crate) residual_100ns: f64,
}

pub(crate) fn fit_clock_model(timing: &CaptureTiming) -> std::result::Result<ClockModel, String> {
    let timing = timing.snapshot();
    if timing.sample_rate == 0 {
        return Err("capture sample rate is unavailable".to_string());
    }
    if timing.non_monotonic_count > 0 {
        return Err("capture timestamps are non-monotonic".to_string());
    }
    if timing.anchors.len() < 2 {
        return Err(format!(
            "need at least two valid timing anchors; found {}",
            timing.anchors.len()
        ));
    }

    for pair in timing.anchors.windows(2) {
        if pair[1].wav_frame <= pair[0].wav_frame
            || pair[1].device_position <= pair[0].device_position
            || pair[1].qpc_100ns <= pair[0].qpc_100ns
        {
            return Err("capture timestamps are non-monotonic".to_string());
        }
    }

    let mut adjacent_slopes: Vec<f64> = timing
        .anchors
        .windows(2)
        .map(|pair| {
            (pair[1].qpc_100ns - pair[0].qpc_100ns) as f64
                / (pair[1].wav_frame - pair[0].wav_frame) as f64
        })
        .filter(|slope| slope.is_finite() && *slope > 0.0)
        .collect();
    let seed_slope = median(&mut adjacent_slopes)
        .ok_or_else(|| "capture clock rate is not finite".to_string())?;

    let mut intercepts: Vec<f64> = timing
        .anchors
        .iter()
        .map(|anchor| anchor.qpc_100ns as f64 - seed_slope * anchor.wav_frame as f64)
        .collect();
    let seed_intercept = median(&mut intercepts)
        .ok_or_else(|| "capture clock origin is not finite".to_string())?;
    let mut seed_residuals: Vec<f64> = timing
        .anchors
        .iter()
        .map(|anchor| {
            (anchor.qpc_100ns as f64
                - (seed_intercept + seed_slope * anchor.wav_frame as f64))
                .abs()
        })
        .collect();
    let median_residual = median(&mut seed_residuals).unwrap_or(0.0);
    let retain_threshold = (median_residual * 5.0).max(10_000.0);
    let retained: Vec<TimingAnchor> = timing
        .anchors
        .iter()
        .copied()
        .filter(|anchor| {
            (anchor.qpc_100ns as f64
                - (seed_intercept + seed_slope * anchor.wav_frame as f64))
                .abs()
                <= retain_threshold
        })
        .collect();
    if retained.len() < 2 {
        return Err("too few timing anchors remain after outlier rejection".to_string());
    }

    let x_mean = retained
        .iter()
        .map(|anchor| anchor.wav_frame as f64)
        .sum::<f64>()
        / retained.len() as f64;
    let y_mean = retained
        .iter()
        .map(|anchor| anchor.qpc_100ns as f64)
        .sum::<f64>()
        / retained.len() as f64;
    let variance = retained
        .iter()
        .map(|anchor| (anchor.wav_frame as f64 - x_mean).powi(2))
        .sum::<f64>();
    if variance <= 0.0 || !variance.is_finite() {
        return Err("capture timing span is zero".to_string());
    }
    let covariance = retained
        .iter()
        .map(|anchor| {
            (anchor.wav_frame as f64 - x_mean) * (anchor.qpc_100ns as f64 - y_mean)
        })
        .sum::<f64>();
    let slope = covariance / variance;
    let intercept = y_mean - slope * x_mean;
    if !slope.is_finite() || slope <= 0.0 || !intercept.is_finite() {
        return Err("capture clock model is not finite".to_string());
    }

    let mut residuals: Vec<f64> = retained
        .iter()
        .map(|anchor| {
            (anchor.qpc_100ns as f64 - (intercept + slope * anchor.wav_frame as f64)).abs()
        })
        .collect();
    residuals.sort_by(f64::total_cmp);
    let residual_index = ((residuals.len() - 1) as f64 * 0.95).round() as usize;
    let residual_100ns = residuals[residual_index];
    if residual_100ns > MAX_MODEL_RESIDUAL_100NS {
        return Err(format!(
            "capture clock residual {:.3} ms exceeds 20 ms",
            residual_100ns / 10_000.0
        ));
    }

    let first = retained.first().expect("retained anchors are non-empty");
    let last = retained.last().expect("retained anchors are non-empty");
    let span_seconds = (last.qpc_100ns - first.qpc_100ns) as f64 / QPC_TICKS_PER_SECOND;

    Ok(ClockModel {
        qpc_origin_100ns: intercept,
        qpc_100ns_per_wav_frame: slope,
        span_seconds,
        residual_100ns,
    })
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        Some((values[middle - 1] + values[middle]) / 2.0)
    } else {
        Some(values[middle])
    }
}

// ── LoopbackCapture trait ──

/// Trait abstracting the public interface shared by both loopback recorders
/// (system-wide and process-specific).
///
/// This enables `record.rs` to use `Box<dyn LoopbackCapture>` instead of
/// a concrete recorder type, allowing seamless switching between system
/// and process-specific loopback capture.
///
/// These methods are called once per second in the status check loop,
/// not in the audio hot path — zero performance concern from vtable dispatch.
pub(crate) trait LoopbackCapture: Send {
    /// Stop the recorder and finalize the WAV file.
    fn stop(&mut self) -> anyhow::Result<()>;

    /// Get the number of audio frame batches captured so far.
    fn get_frames_captured(&self) -> u64;

    /// Whether any non-silent audio has been detected.
    fn has_audio_detected(&self) -> bool;

    /// The sample rate of the audio being captured.
    fn get_sample_rate(&self) -> u32;

    /// The wall-clock time when audio capture actually started.
    /// Returns None if the recording thread hasn't started yet.
    fn get_start_time(&self) -> Option<Instant>;

    /// Snapshot capture-clock telemetry after the WAV writer has finalized.
    fn capture_timing(&self) -> CaptureTiming;

    /// Whether the target process has exited during recording.
    ///
    /// Only meaningful for `ProcessLoopbackRecorder`. System loopback
    /// always returns `false` (no target process to monitor).
    ///
    /// Used by the record command's status loop to detect when a
    /// process-specific recorder needs to be swapped for a system
    /// loopback recorder (mid-recording fallback).
    fn is_process_dead(&self) -> bool {
        false // Default: system loopback has no target process
    }

    /// Whether the capture device is currently lost and being rebuilt.
    ///
    /// Process loopback handles device loss through `is_process_dead` and the
    /// system-loopback fallback, so the default is `false`.
    fn is_device_lost(&self) -> bool {
        false
    }

    /// How many times the capture client was rebuilt after device loss.
    fn device_recoveries(&self) -> u64 {
        0
    }
}

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

// ── Capture fault classification (device invalidation / recovery) ──

/// WASAPI `AUDCLNT_E_DEVICE_INVALIDATED`: the audio endpoint is gone (device
/// unplugged/disabled, default-device changed, format changed, or the audio
/// engine reset). The current `IAudioClient` is dead and must be fully rebuilt
/// from a fresh endpoint — restarting the existing client does not work.
pub(crate) const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004u32 as i32;

/// WASAPI `AUDCLNT_E_SERVICE_NOT_RUNNING`: the Windows audio service stopped.
/// Recoverable once the service comes back — same rebuild path as invalidation.
pub(crate) const AUDCLNT_E_SERVICE_NOT_RUNNING: i32 = 0x8889_0010u32 as i32;

/// What a capture loop should do when a per-iteration WASAPI call
/// (`GetNextPacketSize`, `GetBuffer`, `ReleaseBuffer`) fails mid-recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureFault {
    /// Device invalidated or audio service stopped — tear down and rebuild the
    /// client, keep the recording alive.
    Recoverable,
    /// Any other error — stop the loop and surface it immediately.
    Fatal,
}

/// Classify a WASAPI HRESULT raised by a capture call into a recovery action.
///
/// Pure and dependency-free so the recovery policy can be unit-tested without a
/// real audio device (the capture loop itself requires live WASAPI hardware).
pub(crate) fn classify_capture_fault(hr: i32) -> CaptureFault {
    match hr {
        AUDCLNT_E_DEVICE_INVALIDATED | AUDCLNT_E_SERVICE_NOT_RUNNING => CaptureFault::Recoverable,
        _ => CaptureFault::Fatal,
    }
}

/// Live device-health state for one capture stream, shared between the capture
/// thread (writer) and the record command's status loop (reader).
///
/// `device_lost` is true from the first recoverable fault until a rebuilt client
/// starts. `recoveries` counts successful rebuilds over the session.
#[derive(Debug, Default)]
pub(crate) struct CaptureHealth {
    device_lost: AtomicBool,
    recoveries: AtomicU64,
}

impl CaptureHealth {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Mark the device as lost. Returns true only on the transition from
    /// healthy to lost, so the caller can log the loss once.
    pub(crate) fn mark_lost(&self) -> bool {
        !self.device_lost.swap(true, Ordering::Relaxed)
    }

    /// Mark a successful rebuild: clear the lost flag and count the recovery.
    pub(crate) fn mark_recovered(&self) {
        self.device_lost.store(false, Ordering::Relaxed);
        self.recoveries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn is_device_lost(&self) -> bool {
        self.device_lost.load(Ordering::Relaxed)
    }

    pub(crate) fn recoveries(&self) -> u64 {
        self.recoveries.load(Ordering::Relaxed)
    }
}

/// What a recording loop should do after a WASAPI capture fault was handled.
pub(crate) enum FaultOutcome {
    /// Keep looping: the client was rebuilt, or a rebuild is pending/throttled
    /// and the loop should keep filling wall-clock silence until it succeeds.
    Continue,
    /// Stop: the error is unrecoverable and must be surfaced immediately.
    Fatal,
}

/// Fill a 16-bit PCM WAV with silence up to the frame count that *should* have
/// been written by now based on wall-clock time. Keeps the stream duration
/// aligned whether the device is simply idle (no packets) or down and being
/// rebuilt after invalidation, so post-recovery audio lands at the correct
/// position instead of collapsing the gap and desyncing the tracks.
pub(crate) fn fill_wallclock_silence<W: std::io::Write + std::io::Seek>(
    writer: &mut Option<hound::WavWriter<W>>,
    total_frames_written: &mut u64,
    recording_start: Instant,
    sample_rate: u32,
    channels_usize: usize,
    write_errors: &WriteErrorTracker,
) {
    let elapsed_secs = recording_start.elapsed().as_secs_f64();
    let expected_frames = (elapsed_secs * sample_rate as f64) as u64;

    if expected_frames > *total_frames_written {
        let gap_frames = (expected_frames - *total_frames_written) as usize;
        let gap_samples = gap_frames * channels_usize;

        if let Some(writer) = writer.as_mut() {
            for _ in 0..gap_samples {
                if let Err(e) = writer.write_sample(0i16) {
                    write_errors.record_error(&e);
                }
            }
        }
        *total_frames_written += gap_frames as u64;
    }
}

// ── Write error tracking ──

/// Tracks sample write errors during recording.
/// On first failure, logs a warning. Recording continues (best effort).
/// On stop, the caller can check if any writes failed.
#[derive(Debug)]
pub struct WriteErrorTracker {
    has_error: AtomicBool,
    first_error: Mutex<Option<String>>,
}

impl WriteErrorTracker {
    pub fn new() -> Self {
        Self {
            has_error: AtomicBool::new(false),
            first_error: Mutex::new(None),
        }
    }

    /// Record a write error. Logs on first occurrence.
    pub fn record_error(&self, err: &dyn std::fmt::Display) {
        if !self.has_error.swap(true, Ordering::Relaxed) {
            let message = err.to_string();
            if let Ok(mut first_error) = self.first_error.lock() {
                *first_error = Some(message.clone());
            }
            tracing::warn!("Sample write error: {}. Recording may be incomplete.", message);
        }
    }

    /// Check if any write errors occurred during recording.
    pub fn had_errors(&self) -> bool {
        self.has_error.load(Ordering::Relaxed)
    }

    pub fn ensure_complete(&self, stream_name: &str) -> anyhow::Result<()> {
        if !self.had_errors() {
            return Ok(());
        }

        let detail = self
            .first_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .unwrap_or_else(|| "unknown write error".to_string());
        anyhow::bail!(
            "{} audio samples could not be written: {}. The recording is incomplete; raw tracks were retained for recovery",
            stream_name,
            detail
        );
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
    #[test]
    fn capture_timing_keeps_first_last_and_excludes_timestamp_errors() {
        let mut timing = CaptureTiming::new(100);
        timing.record_packet(0, 1_000, 10_000, 0);
        timing.record_packet(100, 1_100, 20_000, 0);
        timing.record_packet(500, 1_500, 60_000, 0);
        timing.record_packet(600, 1_600, 70_000, 0);
        timing.record_packet(700, 1_700, 80_000, TIMESTAMP_ERROR_FLAG);

        let snapshot = timing.snapshot();

        assert_eq!(snapshot.timestamp_error_count, 1);
        assert_eq!(
            snapshot
                .anchors
                .iter()
                .map(|anchor| anchor.wav_frame)
                .collect::<Vec<_>>(),
            vec![0, 500, 600]
        );
    }

    #[test]
    fn capture_timing_keeps_anchors_around_discontinuity() {
        let mut timing = CaptureTiming::new(100);
        timing.record_packet(0, 1_000, 10_000, 0);
        timing.record_packet(100, 1_100, 20_000, 0);
        timing.record_packet(200, 1_200, 30_000, DATA_DISCONTINUITY_FLAG);
        timing.record_packet(300, 1_300, 40_000, 0);

        let snapshot = timing.snapshot();

        assert_eq!(
            snapshot
                .anchors
                .iter()
                .map(|anchor| anchor.wav_frame)
                .collect::<Vec<_>>(),
            vec![0, 100, 200, 300]
        );
    }
    #[test]
    fn timestamp_error_discontinuity_keeps_surrounding_valid_anchors() {
        let mut timing = CaptureTiming::new(100);
        timing.record_packet(0, 1_000, 10_000, 0);
        timing.record_packet(100, 1_100, 20_000, 0);
        timing.record_packet(
            200,
            1_200,
            30_000,
            TIMESTAMP_ERROR_FLAG | DATA_DISCONTINUITY_FLAG,
        );
        timing.record_packet(300, 1_300, 40_000, 0);

        let snapshot = timing.snapshot();

        assert_eq!(snapshot.timestamp_error_count, 1);
        assert_eq!(
            snapshot
                .anchors
                .iter()
                .map(|anchor| anchor.wav_frame)
                .collect::<Vec<_>>(),
            vec![0, 100, 300]
        );
    }
    #[test]
    fn clock_model_recovers_affine_mapping_with_sub_frame_endpoint_error() {
        let mut timing = CaptureTiming::new(48_000);
        let origin = 50_000_000u64;
        let ticks_per_frame = 200u64;
        for frame in [0, 480_000, 960_000, 1_440_000] {
            timing.record_packet(
                frame,
                10_000 + frame,
                origin + frame * ticks_per_frame,
                0,
            );
        }

        let model = fit_clock_model(&timing).unwrap();
        let endpoint = 1_920_000u64;
        let predicted = model.qpc_origin_100ns
            + model.qpc_100ns_per_wav_frame * endpoint as f64;
        let expected = (origin + endpoint * ticks_per_frame) as f64;
        let endpoint_error_frames =
            (predicted - expected).abs() / model.qpc_100ns_per_wav_frame;

        assert!(endpoint_error_frames <= 1.0);
        assert_eq!(model.qpc_100ns_per_wav_frame, ticks_per_frame as f64);
        assert_eq!(model.residual_100ns, 0.0);
    }

    #[test]
    fn clock_model_rejects_non_monotonic_capture_positions() {
        let mut timing = CaptureTiming::new(48_000);
        timing.record_packet(0, 1_000, 10_000, 0);
        timing.record_packet(480_000, 900, 110_000, 0);

        let error = fit_clock_model(&timing).unwrap_err();

        assert!(error.contains("non-monotonic"));
    }

    #[test]
    fn clock_model_rejects_a_single_large_outlier() {
        let mut timing = CaptureTiming::new(48_000);
        let origin = 50_000_000u64;
        for second in (0..=60).step_by(10) {
            let frame = second * 48_000;
            let outlier = if second == 30 { 5_000_000 } else { 0 };
            timing.record_packet(
                frame,
                10_000 + frame,
                origin + frame * 200 + outlier,
                0,
            );
        }

        let model = fit_clock_model(&timing).unwrap();

        assert!((model.qpc_origin_100ns - origin as f64).abs() < 1.0);
        assert!((model.qpc_100ns_per_wav_frame - 200.0).abs() < 1e-9);
    }

    #[test]
    fn clock_model_rejects_excessive_residual_jitter() {
        let mut timing = CaptureTiming::new(48_000);
        let origin = 50_000_000u64;
        for second in (0..=60).step_by(10) {
            let frame = second * 48_000;
            let interval = second / 10;
            let jitter = interval * interval * 100_000;
            timing.record_packet(
                frame,
                10_000 + frame,
                origin + frame * 200 + jitter,
                0,
            );
        }

        let error = fit_clock_model(&timing).unwrap_err();

        assert!(error.contains("residual"));
    }
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

    #[test]
    fn write_error_tracker_returns_capture_failure() {
        let tracker = WriteErrorTracker::new();
        tracker.record_error(&std::io::Error::from_raw_os_error(112));

        let error = tracker
            .ensure_complete("microphone")
            .expect_err("disk write errors must fail the recording");

        assert!(error.to_string().contains("microphone"));
        assert!(error.to_string().contains("could not be written"));
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

    // ── Capture fault classification ──

    #[test]
    fn device_invalidated_is_recoverable() {
        // 0x88890004 is the exact HRESULT that silently killed the capture
        // threads in a 35-minute recording on 2026-11-13 (only 3 minutes saved).
        assert_eq!(
            classify_capture_fault(AUDCLNT_E_DEVICE_INVALIDATED),
            CaptureFault::Recoverable
        );
        assert_eq!(classify_capture_fault(0x8889_0004u32 as i32), CaptureFault::Recoverable);
    }

    #[test]
    fn service_not_running_is_recoverable() {
        assert_eq!(
            classify_capture_fault(AUDCLNT_E_SERVICE_NOT_RUNNING),
            CaptureFault::Recoverable
        );
    }

    #[test]
    fn unknown_error_is_fatal() {
        // E_POINTER and a generic unknown HRESULT must not trigger a rebuild loop.
        assert_eq!(classify_capture_fault(0x8000_4003u32 as i32), CaptureFault::Fatal);
        assert_eq!(classify_capture_fault(0x8889_0001u32 as i32), CaptureFault::Fatal);
    }

    #[test]
    fn capture_health_reports_loss_transition_once() {
        let health = CaptureHealth::new();
        assert!(!health.is_device_lost());

        assert!(health.mark_lost(), "first loss is a transition");
        assert!(!health.mark_lost(), "repeated faults while lost are not");
        assert!(health.is_device_lost());
        assert_eq!(health.recoveries(), 0);
    }

    #[test]
    fn capture_health_counts_recoveries_and_clears_loss() {
        let health = CaptureHealth::new();
        health.mark_lost();
        health.mark_recovered();

        assert!(!health.is_device_lost());
        assert_eq!(health.recoveries(), 1);
        assert!(health.mark_lost(), "a second loss after recovery is a new transition");
    }

    #[test]
    fn success_hresult_is_fatal_not_recoverable() {
        // S_OK should never be classified recoverable; a zero HRESULT reaching
        // the classifier means something else went wrong, so treat it as fatal.
        assert_eq!(classify_capture_fault(0), CaptureFault::Fatal);
    }
}
