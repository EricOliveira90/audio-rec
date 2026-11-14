//! Windows WASAPI loopback recording implementation
//!
//! Thin wrapper around the shared capture engine configured for loopback recording.
//!
//! # Safety
//!
//! This module uses extensive `unsafe` code to interact with the Windows Audio Session API (WASAPI).
//! All unsafe operations follow these safety invariants:
//!
//! - **COM Initialization**: Each thread that uses COM calls `CoInitializeEx` before using COM interfaces
//!   and `CoUninitialize` before exiting. COM interfaces are never used across thread boundaries.
//!
//! - **Pointer Validity**: All pointers obtained from WASAPI (buffer pointers, format structures) are:
//!   1. Only dereferenced while they're valid (between GetBuffer/ReleaseBuffer calls)
//!   2. Properly freed using CoTaskMemFree when appropriate
//!   3. Never used after being released
//!
//! - **Buffer Access**: Audio buffers obtained via GetBuffer are only accessed as slices after
//!   validating the frame count and sample format. The buffer size is calculated based on
//!   `num_frames * channels` to prevent out-of-bounds access.
//!
//! - **Memory Layout**: When casting buffer pointers to typed slices (f32, i16, i32), we rely on
//!   WASAPI's guarantee that buffers are properly aligned and sized for the requested format.

use anyhow::{Context, Result};
use crate::audio_utils::{calculate_rms_f32, calculate_rms_i16, calculate_rms_i32};
use crate::wasapi::capture_engine::{
    classify_capture_fault, detect_sample_format, f32_to_i16, fill_wallclock_silence, i32_to_i16,
    CaptureFault, CaptureHealth, CaptureTiming, FaultOutcome, LoopbackCapture, SampleFormat, WriteErrorTracker,
};
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

const REFTIMES_PER_SEC: i64 = 10_000_000;
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct WasapiLoopbackRecorder {
    is_recording: Arc<AtomicBool>,
    frames_captured: Arc<AtomicU64>,
    has_audio: Arc<AtomicBool>,
    sample_rate: u32,
    start_time: Arc<Mutex<Option<Instant>>>,
    capture_timing: Arc<Mutex<CaptureTiming>>,
    thread_handle: Option<std::thread::JoinHandle<Result<()>>>,
    write_errors: Arc<WriteErrorTracker>,
    health: Arc<CaptureHealth>,
}

impl WasapiLoopbackRecorder {
    pub fn new(filepath: PathBuf) -> Result<Self> {
        Self::with_device_index(filepath, None)
    }

    pub fn with_device_index(filepath: PathBuf, device_index: Option<u32>) -> Result<Self> {
        let is_recording = Arc::new(AtomicBool::new(true));
        let frames_captured = Arc::new(AtomicU64::new(0));
        let has_audio = Arc::new(AtomicBool::new(false));
        let write_errors = Arc::new(WriteErrorTracker::new());
        let start_time: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let capture_timing = Arc::new(Mutex::new(CaptureTiming::new(0)));
        let health = Arc::new(CaptureHealth::new());

        let is_recording_clone = Arc::clone(&is_recording);
        let frames_captured_clone = Arc::clone(&frames_captured);
        let has_audio_clone = Arc::clone(&has_audio);
        let write_errors_clone = Arc::clone(&write_errors);
        let start_time_clone = Arc::clone(&start_time);
        let capture_timing_clone = Arc::clone(&capture_timing);
        let health_clone = Arc::clone(&health);

        // Get sample rate before spawning thread
        let sample_rate = unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM")?;

            let device_enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            let device = if let Some(idx) = device_index {
                let collection = device_enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
                collection.Item(idx).context(format!("Failed to get output device at index {}", idx))?
            } else {
                device_enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?
            };

            let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let mix_format = audio_client.GetMixFormat()?;
            let wf = &*mix_format;

            let sr = wf.nSamplesPerSec;

            CoTaskMemFree(Some(mix_format as *const _ as *const _));

            sr
        };

        // Spawn recording thread that initializes its own COM
        let handle = std::thread::spawn(move || {
            Self::recording_thread(
                filepath,
                is_recording_clone,
                frames_captured_clone,
                has_audio_clone,
                write_errors_clone,
                start_time_clone,
                capture_timing_clone,
                health_clone,
                device_index,
            )
        });

        Ok(Self {
            is_recording,
            frames_captured,
            has_audio,
            sample_rate,
            start_time,
            capture_timing,
            thread_handle: Some(handle),
            write_errors,
            health,
        })
    }

    fn recording_thread(
        filepath: PathBuf,
        is_recording: Arc<AtomicBool>,
        frames_captured: Arc<AtomicU64>,
        has_audio: Arc<AtomicBool>,
        write_errors: Arc<WriteErrorTracker>,
        start_time: Arc<Mutex<Option<Instant>>>,
        capture_timing: Arc<Mutex<CaptureTiming>>,
        health: Arc<CaptureHealth>,
        device_index: Option<u32>,
    ) -> Result<()> {
        unsafe {
            // Initialize COM for this thread
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM in recording thread")?;

            // Activate the loopback capture client from the default (or selected)
            // render endpoint. Extracted into a helper so it can be re-invoked to
            // rebuild the client if the device is invalidated mid-recording.
            let mut active = activate_loopback_client(device_index)
                .context("Failed to activate loopback audio client")?;

            let sample_rate = active.sample_rate;
            let channels = active.channels;
            let sample_format = active.sample_format;
            let channels_usize = channels as usize;

            // Create WAV writer - always use 16-bit PCM for compatibility.
            // The spec is fixed at the initial device's format; if a replacement
            // device after recovery has a different format we keep this spec and
            // fill silence rather than corrupt the stream (see handle_loopback_fault).
            let spec = WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = WavWriter::create(&filepath, spec)?;
            let mut writer = Some(writer);

            // Start audio client
            active.audio_client.Start()?;

            // Record the wall-clock time when capture actually started.
            // Used to compute the leading silence needed to align loopback and mic.
            let capture_start = Instant::now();
            if let Ok(mut st) = start_time.lock() {
                *st = Some(capture_start);
            }
            if let Ok(mut timing) = capture_timing.lock() {
                timing.set_sample_rate(sample_rate);
                timing.set_start_time(capture_start);
            }

            tracing::info!(
                "Recording with: {:?} format",
                sample_format
            );

            // Track total frames written to the WAV file and the recording start time.
            // When WASAPI loopback delivers no packets (no system audio playing), we
            // calculate how many silence frames are needed to keep the WAV duration
            // aligned with wall-clock time. This is more accurate than writing a fixed
            // amount per iteration, since thread::sleep(10ms) on Windows can actually
            // sleep 15-16ms due to the default timer resolution. The same silence-fill
            // bridges the gap while a device is being rebuilt after invalidation, so
            // recovered audio lands at the correct wall-clock position.
            let mut total_frames_written: u64 = 0;
            let recording_start = Instant::now();
            let mut last_recovery_attempt: Option<Instant> = None;

            // Recording loop
            while is_recording.load(Ordering::Relaxed) {
                // Wait a bit for buffer to fill
                std::thread::sleep(Duration::from_millis(10));

                let packet_length = match active.capture_client.GetNextPacketSize() {
                    Ok(len) => len,
                    Err(e) => {
                        match handle_loopback_fault(&e, &mut active, device_index, &mut last_recovery_attempt, &health) {
                            FaultOutcome::Continue => {
                                // Keep the WAV aligned to wall-clock while the device
                                // is down / being rebuilt, then retry on the next tick.
                                fill_wallclock_silence(
                                    &mut writer,
                                    &mut total_frames_written,
                                    recording_start,
                                    sample_rate,
                                    channels_usize,
                                    &write_errors,
                                );
                                continue;
                            }
                            FaultOutcome::Fatal => {
                                if let Some(writer) = writer.take() {
                                    let _ = writer.finalize();
                                }
                                return Err(anyhow::anyhow!(
                                    "Loopback capture failed with unrecoverable error: {}",
                                    e
                                ));
                            }
                        }
                    }
                };

                if packet_length > 0 {
                    let mut buffer: *mut u8 = std::ptr::null_mut();
                    let mut num_frames = 0u32;
                    let mut flags = 0u32;
                    let mut device_position = 0u64;
                    let mut qpc_100ns = 0u64;
                    let wav_frame = total_frames_written;

                    if let Err(e) = active.capture_client.GetBuffer(
                        &mut buffer,
                        &mut num_frames,
                        &mut flags,
                        Some(&mut device_position),
                        Some(&mut qpc_100ns),
                    ) {
                        match handle_loopback_fault(&e, &mut active, device_index, &mut last_recovery_attempt, &health) {
                            FaultOutcome::Continue => {
                                fill_wallclock_silence(
                                    &mut writer,
                                    &mut total_frames_written,
                                    recording_start,
                                    sample_rate,
                                    channels_usize,
                                    &write_errors,
                                );
                                continue;
                            }
                            FaultOutcome::Fatal => {
                                if let Some(writer) = writer.take() {
                                    let _ = writer.finalize();
                                }
                                return Err(anyhow::anyhow!(
                                    "Loopback capture failed with unrecoverable error: {}",
                                    e
                                ));
                            }
                        }
                    }

                    if let Ok(mut timing) = capture_timing.lock() {
                        timing.record_packet(wav_frame, device_position, qpc_100ns, flags);
                    }

                    if num_frames > 0 {
                        frames_captured.fetch_add(1, Ordering::Relaxed);
                        total_frames_written += num_frames as u64;
                        let total_samples = (num_frames * channels as u32) as usize;

                        // Check if buffer contains silence (AUDCLNT_BUFFERFLAGS_SILENT = 0x2)
                        let is_silent = (flags & 0x2) != 0;

                        if is_silent {
                            if let Some(writer) = writer.as_mut() {
                                for _ in 0..total_samples {
                                    if let Err(e) = writer.write_sample(0i16) {
                                        write_errors.record_error(&e);
                                    }
                                }
                            }
                        } else {
                            match sample_format {
                                SampleFormat::Float32 => {
                                    let samples = std::slice::from_raw_parts(
                                        buffer as *const f32,
                                        total_samples,
                                    );

                                    if !has_audio.load(Ordering::Relaxed) {
                                        let rms = calculate_rms_f32(samples);
                                        if rms > 0.01 {
                                            has_audio.store(true, Ordering::Relaxed);
                                            tracing::info!("Audio detected! Level: {:.4}", rms);
                                        }
                                    }

                                    if let Some(writer) = writer.as_mut() {
                                        for &sample in samples {
                                            let sample_i16 = f32_to_i16(sample);
                                            if let Err(e) = writer.write_sample(sample_i16) {
                                                write_errors.record_error(&e);
                                            }
                                        }
                                    }
                                }
                                SampleFormat::Int32 => {
                                    let samples = std::slice::from_raw_parts(
                                        buffer as *const i32,
                                        total_samples,
                                    );

                                    if !has_audio.load(Ordering::Relaxed) {
                                        let rms = calculate_rms_i32(samples);
                                        if rms > 100000.0 {
                                            has_audio.store(true, Ordering::Relaxed);
                                            tracing::info!("Audio detected! Level: {:.2}", rms);
                                        }
                                    }

                                    if let Some(writer) = writer.as_mut() {
                                        for &sample in samples {
                                            let sample_i16 = i32_to_i16(sample);
                                            if let Err(e) = writer.write_sample(sample_i16) {
                                                write_errors.record_error(&e);
                                            }
                                        }
                                    }
                                }
                                SampleFormat::Int16 => {
                                    let samples = std::slice::from_raw_parts(
                                        buffer as *const i16,
                                        total_samples,
                                    );

                                    if !has_audio.load(Ordering::Relaxed) {
                                        let rms = calculate_rms_i16(samples);
                                        if rms > 100.0 {
                                            has_audio.store(true, Ordering::Relaxed);
                                            tracing::info!("Audio detected! Level: {:.2}", rms);
                                        }
                                    }

                                    if let Some(writer) = writer.as_mut() {
                                        for &sample in samples {
                                            if let Err(e) = writer.write_sample(sample) {
                                                write_errors.record_error(&e);
                                            }
                                        }
                                    }
                                }
                                SampleFormat::Unsupported => {
                                    tracing::warn!(
                                        "Unsupported audio format: {:?}, writing silence",
                                        sample_format
                                    );
                                    // Write silence for unsupported formats
                                    if let Some(writer) = writer.as_mut() {
                                        for _ in 0..total_samples {
                                            if let Err(e) = writer.write_sample(0i16) {
                                                write_errors.record_error(&e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // A ReleaseBuffer failure on an invalidated device is expected;
                    // the client is torn down and rebuilt rather than killing the thread.
                    if let Err(e) = active.capture_client.ReleaseBuffer(num_frames) {
                        match handle_loopback_fault(&e, &mut active, device_index, &mut last_recovery_attempt, &health) {
                            FaultOutcome::Continue => continue,
                            FaultOutcome::Fatal => {
                                if let Some(writer) = writer.take() {
                                    let _ = writer.finalize();
                                }
                                return Err(anyhow::anyhow!(
                                    "Loopback capture failed with unrecoverable error: {}",
                                    e
                                ));
                            }
                        }
                    }
                } else {
                    // No packets available: no system audio is playing.
                    // Write wall-clock silence to keep the WAV duration aligned.
                    fill_wallclock_silence(
                        &mut writer,
                        &mut total_frames_written,
                        recording_start,
                        sample_rate,
                        channels_usize,
                        &write_errors,
                    );
                }
            }

            // Stop and cleanup. The client may already be dead (invalidated), so
            // ignore a Stop error — the WAV still needs finalizing either way.
            let _ = active.audio_client.Stop();

            if let Some(writer) = writer.take() {
                writer.finalize()?;
            }

            if write_errors.had_errors() {
                tracing::warn!("Some audio samples could not be written during loopback recording");
            }

            CoUninitialize();

            Ok(())
        }
    }

    pub fn get_frames_captured(&self) -> u64 {
        self.frames_captured.load(Ordering::Relaxed)
    }

    pub fn has_audio_detected(&self) -> bool {
        self.has_audio.load(Ordering::Relaxed)
    }

    pub fn get_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Whether the output device is currently lost and being rebuilt.
    pub fn is_device_lost(&self) -> bool {
        self.health.is_device_lost()
    }

    /// How many times the capture client was rebuilt after device loss.
    pub fn device_recoveries(&self) -> u64 {
        self.health.recoveries()
    }

    /// Get the wall-clock time when audio_client.Start() was called.
    /// Returns None if the recording thread hasn't started yet.
    pub fn get_start_time(&self) -> Option<Instant> {
        self.start_time.lock().ok().and_then(|guard| *guard)
    }
    pub(crate) fn capture_timing(&self) -> CaptureTiming {
        self.capture_timing
            .lock()
            .map(|timing| timing.snapshot())
            .unwrap_or_else(|_| CaptureTiming::new(self.sample_rate))
    }

    pub fn stop(&mut self) -> Result<()> {
        self.is_recording.store(false, Ordering::Relaxed);

        // Use JoinHandle with timeout instead of thread::sleep hack
        if let Some(handle) = self.thread_handle.take() {
            // Wait for thread to finish with a timeout
            let start = std::time::Instant::now();
            loop {
                if handle.is_finished() {
                    match handle.join() {
                        Ok(result) => result?,
                        Err(_) => {
                            return Err(anyhow::anyhow!("Loopback recording thread panicked"));
                        }
                    }
                    break;
                }
                if start.elapsed() >= STOP_TIMEOUT {
                    return Err(anyhow::anyhow!("Loopback recording thread did not finish within {:?}", STOP_TIMEOUT));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        self.write_errors.ensure_complete("Loopback")
    }
}

/// A live loopback capture client plus the format fields the recording loop needs.
/// Rebuilt as a unit when the device is invalidated mid-recording.
struct ActiveLoopbackClient {
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    sample_rate: u32,
    channels: u16,
    sample_format: SampleFormat,
}

/// Activate a loopback capture client from the default (or indexed) render
/// endpoint and initialize it, returning the ready-to-start client.
///
/// # Safety
/// Must be called on a thread that has initialized COM (`CoInitializeEx`). The
/// returned COM interfaces are released on drop; the `WAVEFORMATEX` from
/// `GetMixFormat` is freed here before returning, so no raw pointer escapes.
unsafe fn activate_loopback_client(device_index: Option<u32>) -> Result<ActiveLoopbackClient> {
    let device_enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

    let device = if let Some(idx) = device_index {
        tracing::info!("Using output device at index {}", idx);
        let collection = device_enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        collection
            .Item(idx)
            .context(format!("Failed to get output device at index {}", idx))?
    } else {
        device_enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?
    };

    let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

    let mix_format = audio_client.GetMixFormat()?;
    let wf = &*mix_format;
    // Copy packed struct fields to avoid alignment issues.
    let sample_rate = wf.nSamplesPerSec;
    let channels = wf.nChannels;
    let bits_per_sample = wf.wBitsPerSample;
    let format_tag = wf.wFormatTag;
    let sample_format = detect_sample_format(format_tag, bits_per_sample);

    tracing::info!(
        "WASAPI format: {} Hz, {} channels, {} bits, format tag: 0x{:X}, detected: {:?}",
        sample_rate,
        channels,
        bits_per_sample,
        format_tag,
        sample_format
    );

    let init_result = audio_client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK,
        REFTIMES_PER_SEC, // 1 second buffer
        0,
        mix_format,
        None,
    );
    // Free the mix format regardless of whether Initialize succeeded.
    CoTaskMemFree(Some(mix_format as *const _ as *const _));
    init_result?;

    let capture_client: IAudioCaptureClient = audio_client.GetService()?;

    Ok(ActiveLoopbackClient {
        audio_client,
        capture_client,
        sample_rate,
        channels,
        sample_format,
    })
}

/// Handle a WASAPI error raised by a capture call mid-recording.
///
/// Fatal errors stop the loop. Recoverable errors (device invalidated / audio
/// service restarted) tear down the dead client and rebuild it from a fresh
/// endpoint, at most once per second so a persistently-missing device does not
/// spin. If the rebuilt device's format differs from the WAV spec we do not
/// adopt it (that would corrupt the stream) and keep filling silence until a
/// compatible device returns.
///
/// # Safety
/// Must be called on the COM-initialized recording thread.
unsafe fn handle_loopback_fault(
    err: &windows::core::Error,
    active: &mut ActiveLoopbackClient,
    device_index: Option<u32>,
    last_recovery_attempt: &mut Option<Instant>,
    health: &CaptureHealth,
) -> FaultOutcome {
    let hr = err.code().0;
    if classify_capture_fault(hr) == CaptureFault::Fatal {
        tracing::error!(
            "Loopback capture failed with unrecoverable error 0x{:08X}: {}",
            hr as u32,
            err
        );
        return FaultOutcome::Fatal;
    }

    if health.mark_lost() {
        tracing::error!(
            "Loopback device lost (0x{:08X}): {}. System audio is not being captured until the device is rebuilt",
            hr as u32,
            err
        );
    }

    // Throttle rebuild attempts to once per second while the device is missing.
    let now = Instant::now();
    if let Some(prev) = *last_recovery_attempt {
        if now.duration_since(prev) < Duration::from_secs(1) {
            return FaultOutcome::Continue;
        }
    }
    *last_recovery_attempt = Some(now);

    tracing::warn!(
        "Loopback device invalidated (0x{:08X}); rebuilding capture client to resume recording",
        hr as u32
    );

    let rebuilt = match activate_loopback_client(device_index) {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("Loopback client rebuild failed: {}; will retry", e);
            return FaultOutcome::Continue;
        }
    };

    if rebuilt.sample_rate != active.sample_rate || rebuilt.channels != active.channels {
        tracing::warn!(
            "Replacement audio device format ({} Hz, {} ch) differs from the recording \
             ({} Hz, {} ch); filling silence until a compatible device is available",
            rebuilt.sample_rate,
            rebuilt.channels,
            active.sample_rate,
            active.channels
        );
        return FaultOutcome::Continue;
    }

    match rebuilt.audio_client.Start() {
        Ok(()) => {
            tracing::info!("Loopback capture client rebuilt and restarted; recording resumed");
            *active = rebuilt;
            health.mark_recovered();
            FaultOutcome::Continue
        }
        Err(e) => {
            tracing::warn!("Rebuilt loopback client failed to start: {}; will retry", e);
            FaultOutcome::Continue
        }
    }
}

/// Implement LoopbackCapture trait for WasapiLoopbackRecorder.
/// This is a purely additive change — all methods already exist on the struct.
/// Delegates to the existing public methods.
impl LoopbackCapture for WasapiLoopbackRecorder {
    fn stop(&mut self) -> anyhow::Result<()> {
        WasapiLoopbackRecorder::stop(self)
    }

    fn get_frames_captured(&self) -> u64 {
        WasapiLoopbackRecorder::get_frames_captured(self)
    }

    fn has_audio_detected(&self) -> bool {
        WasapiLoopbackRecorder::has_audio_detected(self)
    }

    fn get_sample_rate(&self) -> u32 {
        WasapiLoopbackRecorder::get_sample_rate(self)
    }

    fn get_start_time(&self) -> Option<Instant> {
        WasapiLoopbackRecorder::get_start_time(self)
    }
    fn capture_timing(&self) -> CaptureTiming {
        WasapiLoopbackRecorder::capture_timing(self)
    }

    fn is_device_lost(&self) -> bool {
        WasapiLoopbackRecorder::is_device_lost(self)
    }

    fn device_recoveries(&self) -> u64 {
        WasapiLoopbackRecorder::device_recoveries(self)
    }
}
