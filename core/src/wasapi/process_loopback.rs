//! Process-specific WASAPI loopback recording implementation
//!
//! Captures audio from a specific process (and its child processes) using
//! Windows 10's process-specific loopback API (`ActivateAudioInterfaceAsync`
//! with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`).
//!
//! # Safety
//!
//! This module uses extensive `unsafe` code to interact with WASAPI COM APIs.
//! All unsafe operations follow the same safety invariants as `loopback.rs`.
//!
//! # Why process-specific loopback
//!
//! Standard WASAPI loopback captures ALL system audio. Process loopback
//! captures only audio from a specific process tree, enabling recording
//! of just a meeting app's audio while excluding other system sounds.

use anyhow::{Context, Result};
use crate::audio_utils::{calculate_rms_f32, calculate_rms_i16, calculate_rms_i32};
use crate::wasapi::capture_engine::{
    detect_sample_format, f32_to_i16, i32_to_i16, CaptureTiming, LoopbackCapture, SampleFormat,
    WriteErrorTracker,
};
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::*;

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// How often to check if the target process is still alive (in loop iterations, ~10ms each)
const LIVENESS_CHECK_INTERVAL: u64 = 100;

// ── Raw FFI types for process-specific loopback ──
// These types are needed for ActivateAudioInterfaceAsync with process loopback
// activation parameters. They may not be exposed by the windows crate directly.

/// AUDIOCLIENT_ACTIVATION_TYPE values
#[allow(non_upper_case_globals)]
const AUDIOCLIENT_ACTIVATION_TYPE_DEFAULT: u32 = 0;
#[allow(non_upper_case_globals)]
const AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK: u32 = 1;

/// PROCESS_LOOPBACK_MODE values
#[allow(non_upper_case_globals)]
const PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE: u32 = 0;
#[allow(non_upper_case_globals)]
const PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE: u32 = 1;

/// AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS - parameters for process-specific loopback
#[repr(C)]
#[allow(non_snake_case)]
struct AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
    TargetProcessId: u32,
    ProcessLoopbackMode: u32,
}

/// AUDIOCLIENT_ACTIVATION_PARAMS - union-like struct for activation parameters
#[repr(C)]
#[allow(non_snake_case)]
struct AUDIOCLIENT_ACTIVATION_PARAMS {
    ActivationType: u32,
    ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
}

/// PROPVARIANT for passing activation params to ActivateAudioInterfaceAsync.
///
/// Must be exactly 24 bytes on 64-bit Windows to match the Windows PROPVARIANT layout.
/// Rust's `#[repr(C)]` already inserts the correct 4-byte padding between `cbSize` (u32
/// at offset 8) and `pBlobData` (pointer at offset 16). No manual padding field needed.
#[repr(C)]
#[allow(non_snake_case)]
struct PROPVARIANT_BLOB {
    vt: u16,       // VT_BLOB = 65
    wReserved1: u16,
    wReserved2: u16,
    wReserved3: u16,
    cbSize: u32,
    // repr(C) inserts 4 bytes of padding here to align pBlobData to 8 bytes
    pBlobData: *const u8,
}

// Compile-time assertion: PROPVARIANT_BLOB must be exactly 24 bytes on 64-bit Windows.
// If this fails, ActivateAudioInterfaceAsync will receive a malformed struct and
// silently ignore process loopback activation parameters.
const _: () = assert!(std::mem::size_of::<PROPVARIANT_BLOB>() == 24);

// ActivateAudioInterfaceAsync from mmdevapi
#[link(name = "mmdevapi")]
extern "system" {
    fn ActivateAudioInterfaceAsync(
        deviceInterfacePath: *const u16,
        riid: *const windows::core::GUID,
        activationParams: *const PROPVARIANT_BLOB,
        completionHandler: *const std::ffi::c_void, // IActivateAudioInterfaceCompletionHandler
        activationOperation: *mut *mut std::ffi::c_void, // IActivateAudioInterfaceAsyncOperation
    ) -> windows::core::HRESULT;
}

/// Virtual render device ID for process loopback
const VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK: &str = "VAD\\Process_Loopback";

/// Wrapper to send `IAudioClient` across threads.
///
/// # Safety
///
/// `IAudioClient` is a COM interface pointer (`NonNull<c_void>`) which is `!Send` by default.
/// However, when COM is initialized with `COINIT_MULTITHREADED` (MTA), COM objects are
/// free-threaded and can be safely used from any thread in the same MTA. Both `new()` and
/// `recording_thread()` call `CoInitializeEx(COINIT_MULTITHREADED)`, so this is safe.
struct SendableAudioClient(IAudioClient);

// SAFETY: The IAudioClient was created in an MTA apartment and will only be used
// in another MTA apartment thread. MTA COM objects are free-threaded.
unsafe impl Send for SendableAudioClient {}

pub struct ProcessLoopbackRecorder {
    is_recording: Arc<AtomicBool>,
    frames_captured: Arc<AtomicU64>,
    has_audio: Arc<AtomicBool>,
    sample_rate: u32,
    start_time: Arc<Mutex<Option<Instant>>>,
    capture_timing: Arc<Mutex<CaptureTiming>>,
    thread_handle: Option<std::thread::JoinHandle<Result<()>>>,
    write_errors: Arc<WriteErrorTracker>,
    /// Set to true when the target process exits during recording
    process_dead: Arc<AtomicBool>,
    pid: u32,
}

impl ProcessLoopbackRecorder {
    /// Create a new process-specific loopback recorder.
    ///
    /// Eagerly activates the process-specific loopback audio client on the calling
    /// thread. If the API is not available (e.g., `E_NOTIMPL` on unsupported Windows
    /// builds), this returns `Err(...)` immediately so the caller can fall back to
    /// system-wide loopback. The activated `IAudioClient` is then passed to the
    /// recording thread for capture.
    pub fn new(filepath: PathBuf, pid: u32) -> Result<Self> {
        let is_recording = Arc::new(AtomicBool::new(true));
        let frames_captured = Arc::new(AtomicU64::new(0));
        let has_audio = Arc::new(AtomicBool::new(false));
        let write_errors = Arc::new(WriteErrorTracker::new());
        let start_time: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let capture_timing = Arc::new(Mutex::new(CaptureTiming::new(0)));
        let process_dead = Arc::new(AtomicBool::new(false));

        // Eagerly activate process loopback on the calling thread.
        // This ensures that if the API is unavailable (E_NOTIMPL, wrong Windows build,
        // etc.), we fail HERE — not silently in the recording thread where the error
        // would be swallowed and the caller's fallback logic would never trigger.
        let (sendable_client, sample_rate) = unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM")?;

            let audio_client = Self::activate_process_loopback(pid)
                .context("activate_process_loopback failed")?;

            tracing::info!("Process loopback audio client activated for PID {}", pid);

            // The process loopback virtual device does NOT support GetMixFormat() —
            // it returns E_NOTIMPL. Instead, get the mix format from the default
            // render endpoint (the system's output device), which is what the process
            // loopback client will use internally.
            let device_enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let render_device = device_enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let render_client: IAudioClient = render_device.Activate(CLSCTX_ALL, None)?;
            let mix_format = render_client.GetMixFormat()
                .context("GetMixFormat failed on default render device")?;
            let wf = &*mix_format;
            let sr = wf.nSamplesPerSec;

            tracing::info!("Process loopback will use render device format: {} Hz", sr);

            // Initialize the process loopback audio client with the render device's format.
            // No stream flags needed — process-specific filtering is configured via
            // the activation parameters passed to ActivateAudioInterfaceAsync.
            let stream_flags = AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                stream_flags,
                10_000_000, // 1 second buffer
                0,
                mix_format,
                None,
            ).context("Initialize failed on process loopback audio client")?;

            tracing::info!("Process loopback audio client initialized for PID {}", pid);

            CoTaskMemFree(Some(mix_format as *const _ as *const _));

            (SendableAudioClient(audio_client), sr)
        };

        tracing::info!(
            "Process loopback ready for PID {} (sample rate: {} Hz)",
            pid, sample_rate
        );

        let is_recording_clone = Arc::clone(&is_recording);
        let frames_captured_clone = Arc::clone(&frames_captured);
        let has_audio_clone = Arc::clone(&has_audio);
        let write_errors_clone = Arc::clone(&write_errors);
        let start_time_clone = Arc::clone(&start_time);
        let capture_timing_clone = Arc::clone(&capture_timing);
        let process_dead_clone = Arc::clone(&process_dead);

        let handle = std::thread::spawn(move || {
            Self::recording_thread(
                filepath,
                pid,
                is_recording_clone,
                frames_captured_clone,
                has_audio_clone,
                write_errors_clone,
                start_time_clone,
                capture_timing_clone,
                process_dead_clone,
                sendable_client,
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
            process_dead,
            pid,
        })
    }

    fn recording_thread(
        filepath: PathBuf,
        pid: u32,
        is_recording: Arc<AtomicBool>,
        frames_captured: Arc<AtomicU64>,
        has_audio: Arc<AtomicBool>,
        write_errors: Arc<WriteErrorTracker>,
        start_time: Arc<Mutex<Option<Instant>>>,
        capture_timing: Arc<Mutex<CaptureTiming>>,
        process_dead: Arc<AtomicBool>,
        sendable_client: SendableAudioClient,
    ) -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM in process loopback thread")?;

            // Unwrap the pre-activated and pre-initialized audio client from new().
            let audio_client = sendable_client.0;

            // Get format info from the default render device (same as new()).
            // Process loopback virtual device doesn't support GetMixFormat().
            let device_enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let render_device = device_enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let render_client: IAudioClient = render_device.Activate(CLSCTX_ALL, None)?;
            let mix_format = render_client.GetMixFormat()?;
            let wf = &*mix_format;

            let sample_rate = wf.nSamplesPerSec;
            let channels = wf.nChannels;
            let bits_per_sample = wf.wBitsPerSample;
            let format_tag = wf.wFormatTag;

            let sample_format = detect_sample_format(format_tag, bits_per_sample);

            tracing::info!(
                "Process loopback WASAPI format: {} Hz, {} channels, {} bits, tag: 0x{:X}, detected: {:?} (PID: {})",
                sample_rate, channels, bits_per_sample, format_tag, sample_format, pid
            );

            // Audio client was already initialized in new() — go straight to capture
            let capture_client: IAudioCaptureClient = audio_client.GetService()?;

            // Create WAV writer
            let spec = WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = WavWriter::create(&filepath, spec)?;
            let mut writer = Some(writer);

            audio_client.Start()?;

            let capture_start = Instant::now();
            if let Ok(mut st) = start_time.lock() {
                *st = Some(capture_start);
            }
            if let Ok(mut timing) = capture_timing.lock() {
                timing.set_sample_rate(sample_rate);
                timing.set_start_time(capture_start);
            }

            tracing::info!("Process loopback recording started for PID {}", pid);

            let mut total_frames_written: u64 = 0;
            let recording_start = Instant::now();
            let channels_usize = channels as usize;
            let mut loop_count: u64 = 0;

            while is_recording.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                loop_count += 1;

                // Periodic liveness check (~1 second interval)
                if loop_count % LIVENESS_CHECK_INTERVAL == 0 {
                    if !Self::is_process_alive(pid) {
                        tracing::warn!("Target process (PID {}) has exited", pid);
                        process_dead.store(true, Ordering::Relaxed);
                        break;
                    }
                }

                let packet_length = match capture_client.GetNextPacketSize() {
                    Ok(len) => len,
                    Err(e) => {
                        // AUDCLNT_E_DEVICE_INVALIDATED or similar — process likely exited
                        tracing::warn!("GetNextPacketSize failed (process may have exited): {}", e);
                        process_dead.store(true, Ordering::Relaxed);
                        break;
                    }
                };

                if packet_length > 0 {
                    let mut buffer: *mut u8 = std::ptr::null_mut();
                    let mut num_frames = 0u32;
                    let mut flags = 0u32;
                    let mut device_position = 0u64;
                    let mut qpc_100ns = 0u64;
                    let wav_frame = total_frames_written;

                    match capture_client.GetBuffer(
                        &mut buffer,
                        &mut num_frames,
                        &mut flags,
                        Some(&mut device_position),
                        Some(&mut qpc_100ns),
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::warn!("GetBuffer failed (process may have exited): {}", e);
                            process_dead.store(true, Ordering::Relaxed);
                            break;
                        }
                    }

                    if let Ok(mut timing) = capture_timing.lock() {
                        timing.record_packet(wav_frame, device_position, qpc_100ns, flags);
                    }

                    if num_frames > 0 {
                        frames_captured.fetch_add(1, Ordering::Relaxed);
                        total_frames_written += num_frames as u64;
                        let total_samples = (num_frames * channels as u32) as usize;

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
                                            tracing::info!("Process audio detected! Level: {:.4}", rms);
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

                    let _ = capture_client.ReleaseBuffer(num_frames);
                } else {
                    // No packets: fill silence to keep WAV aligned with wall-clock time
                    let elapsed_secs = recording_start.elapsed().as_secs_f64();
                    let expected_frames = (elapsed_secs * sample_rate as f64) as u64;

                    if expected_frames > total_frames_written {
                        let gap_frames = (expected_frames - total_frames_written) as usize;
                        let gap_samples = gap_frames * channels_usize;

                        if let Some(writer) = writer.as_mut() {
                            for _ in 0..gap_samples {
                                if let Err(e) = writer.write_sample(0i16) {
                                    write_errors.record_error(&e);
                                }
                            }
                        }
                        total_frames_written += gap_frames as u64;
                    }
                }
            }

            // Stop and cleanup
            let _ = audio_client.Stop();

            // Ensure the WAV file has valid data even if no audio was captured.
            // If the recording loop ran but received zero packets (e.g., process wasn't
            // producing audio through WASAPI), the WAV file would have a header but zero
            // data bytes, causing FFmpeg to fail during post-processing.
            // Write at least 1 second of silence to guarantee a valid WAV file.
            if total_frames_written == 0 {
                tracing::warn!(
                    "Process loopback captured zero audio frames for PID {}. \
                     Writing 1 second of silence to ensure valid WAV file.",
                    pid
                );
                let silence_frames = sample_rate as u64;
                let silence_samples = silence_frames * channels as u64;
                if let Some(writer) = writer.as_mut() {
                    for _ in 0..silence_samples {
                        if let Err(e) = writer.write_sample(0i16) {
                            write_errors.record_error(&e);
                        }
                    }
                }
                #[allow(unused_assignments)]
                {
                    total_frames_written = silence_frames;
                }
            }

            if let Some(writer) = writer.take() {
                writer.finalize()?;
            }

            if write_errors.had_errors() {
                tracing::warn!("Some audio samples could not be written during process loopback recording");
            }

            CoTaskMemFree(Some(mix_format as *const _ as *const _));
            CoUninitialize();

            Ok(())
        }
    }

    /// Activate an audio client for process-specific loopback capture.
    ///
    /// Uses `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`
    /// and `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` to get an IAudioClient
    /// that captures only audio from the specified process and its child processes.
    ///
    /// This is the Windows 10 Build 20348+ API for process-specific audio capture.
    /// See: https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
    unsafe fn activate_process_loopback(pid: u32) -> Result<IAudioClient> {
        use std::sync::mpsc;

        tracing::info!("Activating process-specific loopback for PID {} using ActivateAudioInterfaceAsync", pid);

        // Build the activation parameters
        let params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        };

        // Wrap in a PROPVARIANT (VT_BLOB)
        let params_ptr = &params as *const AUDIOCLIENT_ACTIVATION_PARAMS as *const u8;
        let params_size = std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32;

        let prop = PROPVARIANT_BLOB {
            vt: 65, // VT_BLOB
            wReserved1: 0,
            wReserved2: 0,
            wReserved3: 0,
            cbSize: params_size,
            pBlobData: params_ptr,
        };

        // Create a Win32 event for synchronization
        let event = windows::Win32::System::Threading::CreateEventW(None, true, false, None)?;

        // We need a completion handler. Use a simple approach: create a channel
        // and signal the event from the completion handler.
        let (tx, _rx) = mpsc::channel::<()>();

        // Build the completion handler as a COM object.
        // The #[implement] macro creates a reference-counted COM object.
        // We convert it to a raw IActivateAudioInterfaceCompletionHandler pointer
        // for the FFI call. COM reference counting manages the lifetime — we do NOT
        // manually reclaim it with Box::from_raw.
        let handler: IActivateAudioInterfaceCompletionHandler =
            CompletionHandler::new(event, tx).into();
        let handler_raw = handler.as_raw();

        // Device interface path for process loopback
        let device_path: Vec<u16> = VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // IID_IAudioClient
        let iid_audio_client = windows::core::GUID::from_values(
            0x1CB9AD4C, 0xDBFA, 0x4c32,
            [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
        );

        let mut operation: *mut std::ffi::c_void = std::ptr::null_mut();

        let hr = ActivateAudioInterfaceAsync(
            device_path.as_ptr(),
            &iid_audio_client,
            &prop,
            handler_raw as *const std::ffi::c_void,
            &mut operation,
        );

        if hr.is_err() {
            CloseHandle(event)?;
            return Err(anyhow::anyhow!(
                "ActivateAudioInterfaceAsync failed for PID {}: HRESULT 0x{:08X}",
                pid, hr.0
            ));
        }

        // Wait for the async operation to complete (timeout: 5 seconds)
        let wait_result = WaitForSingleObject(event, 5000);
        CloseHandle(event)?;

        if wait_result != WAIT_EVENT(0) {
            return Err(anyhow::anyhow!(
                "ActivateAudioInterfaceAsync timed out for PID {}",
                pid
            ));
        }

        // Get the result from the async operation
        let op: IActivateAudioInterfaceAsyncOperation =
            IActivateAudioInterfaceAsyncOperation::from_raw(operation);

        let mut activate_hr = windows::core::HRESULT(0);
        let mut activated_interface: Option<windows::core::IUnknown> = None;
        op.GetActivateResult(&mut activate_hr, &mut activated_interface)?;

        if activate_hr.is_err() {
            return Err(anyhow::anyhow!(
                "Process loopback activation failed for PID {}: HRESULT 0x{:08X}. \
                 Ensure the process is running and producing audio.",
                pid, activate_hr.0
            ));
        }

        let unknown = activated_interface.ok_or_else(|| {
            anyhow::anyhow!("ActivateAudioInterfaceAsync returned null interface for PID {}", pid)
        })?;

        let audio_client: IAudioClient = unknown.cast()?;

        tracing::info!("Successfully activated process-specific loopback for PID {}", pid);

        Ok(audio_client)
    }

    /// Check if a process is still alive using OpenProcess + WaitForSingleObject.
    /// Non-blocking check (timeout = 0), takes <1μs.
    unsafe fn is_process_alive(pid: u32) -> bool {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(handle) => {
                let result = WaitForSingleObject(handle, 0);
                let _ = CloseHandle(handle);
                result == WAIT_EVENT(258) // WAIT_TIMEOUT
            }
            Err(_) => false,
        }
    }

    /// Check if the target process has exited during recording.
    pub fn is_process_dead(&self) -> bool {
        self.process_dead.load(Ordering::Relaxed)
    }

    /// Get the PID of the target process.
    pub fn pid(&self) -> u32 {
        self.pid
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

        if let Some(handle) = self.thread_handle.take() {
            let start = std::time::Instant::now();
            loop {
                if handle.is_finished() {
                    match handle.join() {
                        Ok(result) => result?,
                        Err(_) => {
                            return Err(anyhow::anyhow!("Process loopback recording thread panicked"));
                        }
                    }
                    break;
                }
                if start.elapsed() >= STOP_TIMEOUT {
                    return Err(anyhow::anyhow!("Process loopback recording thread did not finish within {:?}", STOP_TIMEOUT));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        self.write_errors.ensure_complete("Process loopback")
    }
}

/// Implement LoopbackCapture trait for ProcessLoopbackRecorder.
impl LoopbackCapture for ProcessLoopbackRecorder {
    fn stop(&mut self) -> anyhow::Result<()> {
        ProcessLoopbackRecorder::stop(self)
    }

    fn get_frames_captured(&self) -> u64 {
        ProcessLoopbackRecorder::get_frames_captured(self)
    }

    fn has_audio_detected(&self) -> bool {
        ProcessLoopbackRecorder::has_audio_detected(self)
    }

    fn get_sample_rate(&self) -> u32 {
        ProcessLoopbackRecorder::get_sample_rate(self)
    }

    fn get_start_time(&self) -> Option<Instant> {
        ProcessLoopbackRecorder::get_start_time(self)
    }
    fn capture_timing(&self) -> CaptureTiming {
        ProcessLoopbackRecorder::capture_timing(self)
    }

    fn is_process_dead(&self) -> bool {
        ProcessLoopbackRecorder::is_process_dead(self)
    }
}

// ── Completion Handler for ActivateAudioInterfaceAsync ──

#[windows::core::implement(IActivateAudioInterfaceCompletionHandler)]
struct CompletionHandler {
    event: HANDLE,
    _tx: std::sync::mpsc::Sender<()>,
}

impl CompletionHandler {
    fn new(event: HANDLE, tx: std::sync::mpsc::Sender<()>) -> Self {
        Self { event, _tx: tx }
    }
}

impl IActivateAudioInterfaceCompletionHandler_Impl for CompletionHandler_Impl {
    fn ActivateCompleted(
        &self,
        _activate_operation: Option<&IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        unsafe {
            let _ = SetEvent(self.event);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propvariant_blob_is_24_bytes_on_64bit() {
        assert_eq!(
            std::mem::size_of::<PROPVARIANT_BLOB>(),
            24,
            "PROPVARIANT_BLOB must be exactly 24 bytes to match Windows PROPVARIANT layout"
        );
    }

    #[test]
    fn propvariant_blob_has_repr_c_layout() {
        let blob = PROPVARIANT_BLOB {
            vt: 65,
            wReserved1: 0,
            wReserved2: 0,
            wReserved3: 0,
            cbSize: 42,
            pBlobData: std::ptr::null(),
        };
        assert_eq!(std::mem::size_of_val(&blob), 24);
    }

    #[test]
    fn audioclient_activation_params_layout() {
        assert_eq!(
            std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>(),
            12,
            "AUDIOCLIENT_ACTIVATION_PARAMS should be 12 bytes"
        );
    }
}
