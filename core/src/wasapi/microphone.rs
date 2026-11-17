//! Windows WASAPI microphone recording implementation
//!
//! Thin wrapper around the shared capture engine configured for microphone recording.
//!
//! # Safety
//!
//! This module uses extensive `unsafe` code to interact with the Windows Audio Session API (WASAPI).
//! All unsafe operations follow the same safety invariants as the loopback module:
//!
//! - **COM Initialization**: Proper initialization and cleanup on each thread
//! - **Pointer Validity**: All WASAPI pointers are used only while valid
//! - **Buffer Access**: Audio buffers are validated before casting to typed slices
//! - **Memory Layout**: Relies on WASAPI's alignment guarantees for audio buffers

use anyhow::{Context, Result};
use crate::audio_utils::{calculate_rms_f32, calculate_rms_i16};
use crate::wasapi::capture_engine::{
    classify_capture_fault, detect_sample_format, f32_to_i16, fill_wallclock_silence, CaptureFault,
    CaptureHealth, CaptureTiming, FaultOutcome, SampleFormat, WriteErrorTracker,
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

pub struct WasapiMicrophoneRecorder {
    is_recording: Arc<AtomicBool>,
    frames_captured: Arc<AtomicU64>,
    has_audio: Arc<AtomicBool>,
    is_muted: Arc<AtomicBool>,
    start_time: Arc<Mutex<Option<Instant>>>,
    capture_timing: Arc<Mutex<CaptureTiming>>,
    thread_handle: Option<std::thread::JoinHandle<Result<()>>>,
    write_errors: Arc<WriteErrorTracker>,
    health: Arc<CaptureHealth>,
}

impl WasapiMicrophoneRecorder {
    pub fn new(filepath: PathBuf, target_sample_rate: u32, is_muted: Arc<AtomicBool>) -> Result<Self> {
        Self::with_device_index(filepath, target_sample_rate, is_muted, None)
    }

    pub fn with_device_index(filepath: PathBuf, target_sample_rate: u32, is_muted: Arc<AtomicBool>, device_index: Option<u32>) -> Result<Self> {
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
        let is_muted_clone = Arc::clone(&is_muted);
        let write_errors_clone = Arc::clone(&write_errors);
        let start_time_clone = Arc::clone(&start_time);
        let capture_timing_clone = Arc::clone(&capture_timing);
        let health_clone = Arc::clone(&health);

        // Spawn recording thread that initializes its own COM
        let failure_health = Arc::clone(&health);
        let handle = std::thread::spawn(move || {
            let result = Self::recording_thread(
                filepath,
                target_sample_rate,
                is_recording_clone,
                frames_captured_clone,
                has_audio_clone,
                is_muted_clone,
                write_errors_clone,
                start_time_clone,
                capture_timing_clone,
                health_clone,
                device_index,
            );
            // Report a failed start live; the status loop would otherwise only
            // see it when the user presses stop.
            if let Err(e) = &result {
                failure_health.mark_failed(format!("microphone capture could not run: {:#}", e));
            }
            result
        });

        Ok(Self {
            is_recording,
            frames_captured,
            has_audio,
            is_muted,
            start_time,
            capture_timing,
            thread_handle: Some(handle),
            write_errors,
            health,
        })
    }

    fn recording_thread(
        filepath: PathBuf,
        target_sample_rate: u32,
        is_recording: Arc<AtomicBool>,
        frames_captured: Arc<AtomicU64>,
        has_audio: Arc<AtomicBool>,
        is_muted: Arc<AtomicBool>,
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

            // Activate the microphone capture client. Extracted into a helper so it
            // can be re-invoked to rebuild the client if the device is invalidated
            // mid-recording (device unplugged, default-device change, service reset).
            tracing::info!(
                "Using native microphone format - FFmpeg will resample to {} Hz during merge",
                target_sample_rate
            );
            let mut active = activate_microphone_client(device_index)
                .context("Failed to activate microphone audio client")?;

            // Record at native sample rate - FFmpeg will resample during merge
            let sample_rate = active.sample_rate;
            let channels = active.channels;
            let sample_format = active.sample_format;
            let channels_usize = channels as usize;

            // Create WAV writer - always use 16-bit PCM for compatibility.
            // Spec is fixed at the initial device's format; a replacement device
            // after recovery with a different format is rejected (silence filled)
            // rather than corrupting the stream (see handle_microphone_fault).
            tracing::info!("Creating WAV writer for microphone...");
            let spec = WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = WavWriter::create(&filepath, spec)?;
            let mut writer = Some(writer);
            tracing::info!("WAV writer created successfully");

            // Start audio client
            tracing::info!("Starting microphone audio client...");
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

            tracing::info!("Audio client started successfully");

            tracing::info!(
                "Recording microphone: {} Hz, {} channels, {:?} format",
                sample_rate,
                channels,
                sample_format
            );
            tracing::info!("Microphone audio client started, entering recording loop...");

            // Track total frames written to the WAV file and the recording start time.
            // When WASAPI microphone delivers no packets (rare but possible), we
            // calculate how many silence frames are needed to keep the WAV duration
            // aligned with wall-clock time. This matches the loopback recorder's
            // approach and prevents progressive drift between the two tracks over
            // long recordings caused by hardware clock differences.
            let mut total_frames_written: u64 = 0;
            let recording_start = Instant::now();
            let mut last_recovery_attempt: Option<Instant> = None;

            // Recording loop
            let mut loop_count = 0u64;
            let mut total_packets_received = 0u64;
            while is_recording.load(Ordering::Relaxed) {
                // Wait a bit for buffer to fill
                std::thread::sleep(Duration::from_millis(10));

                let packet_length = match active.capture_client.GetNextPacketSize() {
                    Ok(len) => len,
                    Err(e) => {
                        match handle_microphone_fault(&e, &mut active, device_index, &mut last_recovery_attempt, &health, &capture_timing) {
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
                                // End this stream only. The WAV is finalized below so
                                // the audio captured so far is kept and merged.
                                health.mark_failed(format!("microphone capture stopped: {}", e));
                                break;
                            }
                        }
                    }
                };

                // Log every 100 iterations (~1 second) to track packet availability
                if loop_count % 100 == 0 {
                    tracing::debug!(
                        "Mic loop #{}: packet_length={}, total_packets={}",
                        loop_count,
                        packet_length,
                        total_packets_received
                    );
                }
                loop_count += 1;

                if packet_length > 0 {
                    total_packets_received += 1;
                    tracing::debug!("Mic packet available: {} frames", packet_length);
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
                        match handle_microphone_fault(&e, &mut active, device_index, &mut last_recovery_attempt, &health, &capture_timing) {
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
                                // End this stream only. The WAV is finalized below so
                                // the audio captured so far is kept and merged.
                                health.mark_failed(format!("microphone capture stopped: {}", e));
                                break;
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

                        if is_muted.load(Ordering::Relaxed) || is_silent {
                            // Write silence when muted or buffer is silent
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
                                            tracing::info!(
                                                "Microphone audio detected! Level: {:.4}",
                                                rms
                                            );
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
                                SampleFormat::Int16 => {
                                    let samples = std::slice::from_raw_parts(
                                        buffer as *const i16,
                                        total_samples,
                                    );

                                    if !has_audio.load(Ordering::Relaxed) {
                                        let rms = calculate_rms_i16(samples);
                                        if rms > 327.0 {
                                            has_audio.store(true, Ordering::Relaxed);
                                            tracing::info!(
                                                "Microphone audio detected! Level: {:.4}",
                                                rms / 32767.0
                                            );
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
                                SampleFormat::Int32 | SampleFormat::Unsupported => {
                                    // Write silence for unsupported or i32 mic formats
                                    tracing::warn!(
                                        "Unsupported microphone format: {:?}, writing silence",
                                        sample_format
                                    );
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
                        match handle_microphone_fault(&e, &mut active, device_index, &mut last_recovery_attempt, &health, &capture_timing) {
                            FaultOutcome::Continue => continue,
                            FaultOutcome::Fatal => {
                                // End this stream only. The WAV is finalized below so
                                // the audio captured so far is kept and merged.
                                health.mark_failed(format!("microphone capture stopped: {}", e));
                                break;
                            }
                        }
                    }
                } else {
                    // No packets available from microphone.
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

            // Stop and cleanup
            tracing::info!(
                "Microphone recording loop finished: {} iterations, {} packets received, {} frames captured",
                loop_count,
                total_packets_received,
                frames_captured.load(Ordering::Relaxed)
            );

            // The client may already be dead (invalidated); ignore a Stop error —
            // the WAV still needs finalizing either way.
            let _ = active.audio_client.Stop();

            if let Some(writer) = writer.take() {
                writer.finalize()?;
            }

            if write_errors.had_errors() {
                tracing::warn!("Some audio samples could not be written during microphone recording");
            }

            // Native format was already freed after Initialize()

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

    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::Relaxed)
    }

    /// Whether the input device is currently lost and being rebuilt.
    pub fn is_device_lost(&self) -> bool {
        self.health.is_device_lost()
    }

    /// How many times the capture client was rebuilt after device loss.
    pub fn device_recoveries(&self) -> u64 {
        self.health.recoveries()
    }

    /// Why the capture stream stopped early, if it did.
    pub fn capture_failure(&self) -> Option<String> {
        self.health.failure()
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
            .unwrap_or_else(|_| CaptureTiming::new(0))
    }

    pub fn stop(&mut self) -> Result<()> {
        self.is_recording.store(false, Ordering::Relaxed);

        // Use JoinHandle with timeout instead of thread::sleep hack
        if let Some(handle) = self.thread_handle.take() {
            let start = std::time::Instant::now();
            loop {
                if handle.is_finished() {
                    match handle.join() {
                        Ok(result) => result?,
                        Err(_) => {
                            return Err(anyhow::anyhow!("Microphone recording thread panicked"));
                        }
                    }
                    break;
                }
                if start.elapsed() >= STOP_TIMEOUT {
                    return Err(anyhow::anyhow!("Microphone recording thread did not finish within {:?}", STOP_TIMEOUT));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        self.write_errors.ensure_complete("Microphone")
    }
}

/// A live microphone capture client plus the format fields the recording loop
/// needs. Rebuilt as a unit when the device is invalidated mid-recording.
struct ActiveMicrophoneClient {
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    sample_rate: u32,
    channels: u16,
    sample_format: SampleFormat,
}

/// Activate a microphone capture client from the default (or indexed) capture
/// endpoint and initialize it at the device's native format, returning the
/// ready-to-start client.
///
/// # Safety
/// Must be called on a thread that has initialized COM (`CoInitializeEx`). The
/// returned COM interfaces are released on drop; the `WAVEFORMATEX` from
/// `GetMixFormat` is freed here before returning, so no raw pointer escapes.
unsafe fn activate_microphone_client(device_index: Option<u32>) -> Result<ActiveMicrophoneClient> {
    let device_enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

    let device = if let Some(idx) = device_index {
        tracing::info!("Using input device at index {}", idx);
        let collection = device_enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        collection
            .Item(idx)
            .context(format!("Failed to get input device at index {}", idx))?
    } else {
        device_enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?
    };

    let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

    // Get microphone's native format. Using the native format avoids
    // AUDCLNT_E_UNSUPPORTED_FORMAT; FFmpeg resamples during the merge.
    let native_format = audio_client.GetMixFormat()?;
    let native_wf = &*native_format;
    // Copy packed struct fields to avoid alignment issues.
    let channels = native_wf.nChannels;
    let sample_rate = native_wf.nSamplesPerSec;
    let bits_per_sample = native_wf.wBitsPerSample;
    let format_tag = native_wf.wFormatTag;
    let sample_format = detect_sample_format(format_tag, bits_per_sample);

    tracing::info!(
        "WASAPI Microphone native format: {} Hz, {} channels, {} bits, tag: 0x{:X}, detected: {:?}",
        sample_rate,
        channels,
        bits_per_sample,
        format_tag,
        sample_format
    );

    tracing::info!("Initializing audio client with native format...");
    let init_result = audio_client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        0,                // No loopback flag for microphone capture
        REFTIMES_PER_SEC, // 1 second buffer
        0,
        native_format,
        None,
    );
    // Free the native format regardless of whether Initialize succeeded.
    CoTaskMemFree(Some(native_format as *const _ as *const _));
    init_result?;
    tracing::info!("Audio client initialized successfully");

    tracing::info!("Getting capture client service...");
    let capture_client: IAudioCaptureClient = audio_client.GetService()?;
    tracing::info!("Capture client obtained successfully");

    Ok(ActiveMicrophoneClient {
        audio_client,
        capture_client,
        sample_rate,
        channels,
        sample_format,
    })
}

/// Handle a WASAPI error raised by a microphone capture call mid-recording.
///
/// Fatal errors stop the loop. Recoverable errors (device invalidated / audio
/// service restarted) tear down the dead client and rebuild it from a fresh
/// endpoint, at most once per second. If the rebuilt device's native format
/// differs from the WAV spec it is not adopted (that would corrupt the stream);
/// silence is filled until a compatible device returns.
///
/// # Safety
/// Must be called on the COM-initialized recording thread.
unsafe fn handle_microphone_fault(
    err: &windows::core::Error,
    active: &mut ActiveMicrophoneClient,
    device_index: Option<u32>,
    last_recovery_attempt: &mut Option<Instant>,
    health: &CaptureHealth,
    capture_timing: &Mutex<CaptureTiming>,
) -> FaultOutcome {
    let hr = err.code().0;
    if classify_capture_fault(hr) == CaptureFault::Fatal {
        tracing::error!(
            "Microphone capture failed with unrecoverable error 0x{:08X}: {}",
            hr as u32,
            err
        );
        return FaultOutcome::Fatal;
    }

    if health.mark_lost() {
        tracing::error!(
            "Microphone device lost (0x{:08X}): {}. Your voice is not being captured until the device is rebuilt",
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
        "Microphone device invalidated (0x{:08X}); rebuilding capture client to resume recording",
        hr as u32
    );

    let rebuilt = match activate_microphone_client(device_index) {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!("Microphone client rebuild failed: {}; will retry", e);
            return FaultOutcome::Continue;
        }
    };

    if rebuilt.sample_rate != active.sample_rate || rebuilt.channels != active.channels {
        tracing::warn!(
            "Replacement microphone format ({} Hz, {} ch) differs from the recording \
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
            tracing::info!("Microphone capture client rebuilt and restarted; recording resumed");
            *active = rebuilt;
            health.mark_recovered();
            // The new client restarts its device position at zero.
            if let Ok(mut timing) = capture_timing.lock() {
                timing.begin_new_stream();
            }
            FaultOutcome::Continue
        }
        Err(e) => {
            tracing::warn!("Rebuilt microphone client failed to start: {}; will retry", e);
            FaultOutcome::Continue
        }
    }
}
