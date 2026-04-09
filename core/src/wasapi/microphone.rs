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
    detect_sample_format, f32_to_i16, SampleFormat, WriteErrorTracker,
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
    thread_handle: Option<std::thread::JoinHandle<Result<()>>>,
    write_errors: Arc<WriteErrorTracker>,
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

        let is_recording_clone = Arc::clone(&is_recording);
        let frames_captured_clone = Arc::clone(&frames_captured);
        let has_audio_clone = Arc::clone(&has_audio);
        let is_muted_clone = Arc::clone(&is_muted);
        let write_errors_clone = Arc::clone(&write_errors);
        let start_time_clone = Arc::clone(&start_time);

        // Spawn recording thread that initializes its own COM
        let handle = std::thread::spawn(move || {
            Self::recording_thread(
                filepath,
                target_sample_rate,
                is_recording_clone,
                frames_captured_clone,
                has_audio_clone,
                is_muted_clone,
                write_errors_clone,
                start_time_clone,
                device_index,
            )
        });

        Ok(Self {
            is_recording,
            frames_captured,
            has_audio,
            is_muted,
            start_time,
            thread_handle: Some(handle),
            write_errors,
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
        device_index: Option<u32>,
    ) -> Result<()> {
        unsafe {
            // Initialize COM for this thread
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM in recording thread")?;

            // Get audio endpoint for capture (microphone/input)
            let device_enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            let device = if let Some(idx) = device_index {
                tracing::info!("Using input device at index {}", idx);
                let collection = device_enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
                collection.Item(idx).context(format!("Failed to get input device at index {}", idx))?
            } else {
                device_enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?
            };

            // Activate audio client
            let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

            // Get microphone's native format
            let native_format = audio_client.GetMixFormat()?;
            let native_wf = &*native_format;

            // Copy packed struct fields to avoid alignment issues
            let channels = native_wf.nChannels;
            let native_sample_rate = native_wf.nSamplesPerSec;
            let bits_per_sample = native_wf.wBitsPerSample;
            let format_tag = native_wf.wFormatTag;

            // Use shared format detection
            let sample_format = detect_sample_format(format_tag, bits_per_sample);

            tracing::info!(
                "WASAPI Microphone native format: {} Hz, {} channels, {} bits, tag: 0x{:X}, detected: {:?}",
                native_sample_rate,
                channels,
                bits_per_sample,
                format_tag,
                sample_format
            );

            // IMPORTANT: Use native format to avoid AUDCLNT_E_UNSUPPORTED_FORMAT errors.
            // FFmpeg will handle resampling during the merge to match system audio.
            tracing::info!(
                "Using native microphone format - FFmpeg will resample to {} Hz during merge",
                target_sample_rate
            );

            // Initialize audio client with native format
            tracing::info!("Initializing audio client with native format...");
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                0,                // No loopback flag for microphone capture
                REFTIMES_PER_SEC, // 1 second buffer
                0,
                native_format,
                None,
            )?;
            tracing::info!("Audio client initialized successfully");

            // Free native format (we're done with it after Initialize)
            CoTaskMemFree(Some(native_format as *const _ as *const _));

            // Record at native sample rate - FFmpeg will resample during merge
            let sample_rate = native_sample_rate;

            // Get capture client
            tracing::info!("Getting capture client service...");
            let capture_client: IAudioCaptureClient = audio_client.GetService()?;
            tracing::info!("Capture client obtained successfully");

            // Create WAV writer - always use 16-bit PCM for compatibility
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
            audio_client.Start()?;

            // Record the wall-clock time when capture actually started.
            // Used to compute the offset between loopback and mic for FFmpeg -itsoffset alignment.
            if let Ok(mut st) = start_time.lock() {
                *st = Some(Instant::now());
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
            let channels_usize = channels as usize;

            // Recording loop
            let mut loop_count = 0u64;
            let mut total_packets_received = 0u64;
            while is_recording.load(Ordering::Relaxed) {
                // Wait a bit for buffer to fill
                std::thread::sleep(Duration::from_millis(10));

                let packet_length = capture_client.GetNextPacketSize()?;

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

                    capture_client.GetBuffer(
                        &mut buffer,
                        &mut num_frames,
                        &mut flags,
                        None,
                        None,
                    )?;

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

                    capture_client.ReleaseBuffer(num_frames)?;
                } else {
                    // No packets available from microphone.
                    // Calculate how many frames SHOULD have been written by now based on
                    // wall-clock time, then write silence to fill the gap. This keeps the
                    // mic WAV duration aligned with wall-clock time, matching the loopback
                    // recorder's approach and preventing progressive drift between the two
                    // tracks over long recordings.
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
            tracing::info!(
                "Microphone recording loop finished: {} iterations, {} packets received, {} frames captured",
                loop_count,
                total_packets_received,
                frames_captured.load(Ordering::Relaxed)
            );

            audio_client.Stop()?;

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

    /// Get the wall-clock time when audio_client.Start() was called.
    /// Returns None if the recording thread hasn't started yet.
    pub fn get_start_time(&self) -> Option<Instant> {
        self.start_time.lock().ok().and_then(|guard| *guard)
    }

    pub fn stop(&mut self) -> Result<()> {
        self.is_recording.store(false, Ordering::Relaxed);

        // Use JoinHandle with timeout instead of thread::sleep hack
        if let Some(handle) = self.thread_handle.take() {
            let start = std::time::Instant::now();
            loop {
                if handle.is_finished() {
                    match handle.join() {
                        Ok(result) => {
                            if let Err(e) = result {
                                tracing::error!("Microphone recording thread error: {}", e);
                            }
                        }
                        Err(_) => {
                            tracing::error!("Microphone recording thread panicked");
                        }
                    }
                    break;
                }
                if start.elapsed() >= STOP_TIMEOUT {
                    tracing::warn!("Microphone recording thread did not finish within {:?}, continuing", STOP_TIMEOUT);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        if self.write_errors.had_errors() {
            tracing::warn!("Some audio samples could not be written during microphone recording");
        }

        Ok(())
    }
}
