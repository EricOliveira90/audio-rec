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
    detect_sample_format, f32_to_i16, i32_to_i16, SampleFormat, WriteErrorTracker,
};
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

const REFTIMES_PER_SEC: i64 = 10_000_000;
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct WasapiLoopbackRecorder {
    is_recording: Arc<AtomicBool>,
    frames_captured: Arc<AtomicU64>,
    has_audio: Arc<AtomicBool>,
    sample_rate: u32,
    thread_handle: Option<std::thread::JoinHandle<Result<()>>>,
    write_errors: Arc<WriteErrorTracker>,
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

        let is_recording_clone = Arc::clone(&is_recording);
        let frames_captured_clone = Arc::clone(&frames_captured);
        let has_audio_clone = Arc::clone(&has_audio);
        let write_errors_clone = Arc::clone(&write_errors);

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
                device_index,
            )
        });

        Ok(Self {
            is_recording,
            frames_captured,
            has_audio,
            sample_rate,
            thread_handle: Some(handle),
            write_errors,
        })
    }

    fn recording_thread(
        filepath: PathBuf,
        is_recording: Arc<AtomicBool>,
        frames_captured: Arc<AtomicU64>,
        has_audio: Arc<AtomicBool>,
        write_errors: Arc<WriteErrorTracker>,
        device_index: Option<u32>,
    ) -> Result<()> {
        unsafe {
            // Initialize COM for this thread
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM in recording thread")?;

            // Get audio endpoint for rendering (speakers/output)
            let device_enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            let device = if let Some(idx) = device_index {
                tracing::info!("Using output device at index {}", idx);
                let collection = device_enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
                collection.Item(idx).context(format!("Failed to get output device at index {}", idx))?
            } else {
                device_enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?
            };

            // Activate audio client
            let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

            // Get mix format
            let mix_format = audio_client.GetMixFormat()?;
            let wf = &*mix_format;

            // Copy packed struct fields to avoid alignment issues
            let sample_rate = wf.nSamplesPerSec;
            let channels = wf.nChannels;
            let bits_per_sample = wf.wBitsPerSample;
            let format_tag = wf.wFormatTag;

            // Use shared format detection
            let sample_format = detect_sample_format(format_tag, bits_per_sample);

            tracing::info!(
                "WASAPI format: {} Hz, {} channels, {} bits, format tag: 0x{:X}, detected: {:?}",
                sample_rate,
                channels,
                bits_per_sample,
                format_tag,
                sample_format
            );

            // Initialize audio client in loopback mode
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                REFTIMES_PER_SEC, // 1 second buffer
                0,
                mix_format,
                None,
            )?;

            // Get capture client
            let capture_client: IAudioCaptureClient = audio_client.GetService()?;

            // Create WAV writer - always use 16-bit PCM for compatibility
            let spec = WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = WavWriter::create(&filepath, spec)?;
            let mut writer = Some(writer);

            // Start audio client
            audio_client.Start()?;

            // Drain any stale audio data from the render endpoint's buffer.
            // WASAPI loopback capture in shared mode may deliver audio that was
            // already queued in the render buffer before Start() was called.
            // Discarding these initial packets prevents the loopback stream from
            // containing audio that predates the recording start, which would
            // cause it to appear ahead of the microphone stream in the merged output.
            {
                let mut drained_packets = 0u32;
                let mut drained_frames = 0u64;
                loop {
                    let packet_length = capture_client.GetNextPacketSize()?;
                    if packet_length == 0 {
                        break;
                    }
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
                    drained_frames += num_frames as u64;
                    drained_packets += 1;
                    capture_client.ReleaseBuffer(num_frames)?;
                }
                if drained_packets > 0 {
                    tracing::info!(
                        "Drained {} stale loopback packets ({} frames) from render buffer",
                        drained_packets, drained_frames
                    );
                }
            }

            tracing::info!(
                "Recording with: {:?} format",
                sample_format
            );

            // Recording loop
            while is_recording.load(Ordering::Relaxed) {
                // Wait a bit for buffer to fill
                std::thread::sleep(Duration::from_millis(10));

                let packet_length = capture_client.GetNextPacketSize()?;

                if packet_length > 0 {
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
                                        "Unsupported audio format: {} bits, tag: 0x{:X}",
                                        bits_per_sample,
                                        format_tag
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

                    capture_client.ReleaseBuffer(num_frames)?;
                }
            }

            // Stop and cleanup
            audio_client.Stop()?;

            if let Some(writer) = writer.take() {
                writer.finalize()?;
            }

            if write_errors.had_errors() {
                tracing::warn!("Some audio samples could not be written during loopback recording");
            }

            CoTaskMemFree(Some(mix_format as *const _ as *const _));
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

    pub fn stop(&mut self) -> Result<()> {
        self.is_recording.store(false, Ordering::Relaxed);

        // Use JoinHandle with timeout instead of thread::sleep hack
        if let Some(handle) = self.thread_handle.take() {
            // Wait for thread to finish with a timeout
            let start = std::time::Instant::now();
            loop {
                if handle.is_finished() {
                    match handle.join() {
                        Ok(result) => {
                            if let Err(e) = result {
                                tracing::error!("Loopback recording thread error: {}", e);
                            }
                        }
                        Err(_) => {
                            tracing::error!("Loopback recording thread panicked");
                        }
                    }
                    break;
                }
                if start.elapsed() >= STOP_TIMEOUT {
                    tracing::warn!("Loopback recording thread did not finish within {:?}, continuing", STOP_TIMEOUT);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        if self.write_errors.had_errors() {
            tracing::warn!("Some audio samples could not be written during loopback recording");
        }

        Ok(())
    }
}
