use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::Instrument;

use crate::config::RecorderConfig;
use crate::domain::{AudioFormat, RecordingDuration, RecordingSession};
use crate::error::Result;
use crate::output::UserOutput;
use crate::recorder::{merge_audio_streams_smart, RecordingQuality};
use crate::status::{JsonFileObserver, RecordingResult, RecordingStatus, StatusObserver};

#[cfg(windows)]
use crate::wasapi::loopback::WasapiLoopbackRecorder;
#[cfg(windows)]
use crate::wasapi::microphone::WasapiMicrophoneRecorder;

/// Options for the record command
#[derive(Debug, Clone)]
pub struct RecordOptions {
    pub duration: RecordingDuration,
    pub audio_format: AudioFormat,
    pub quality: RecordingQuality,
    pub output_path: Option<PathBuf>,
    pub mic_muted: bool,
    pub trim_silence: bool,
    pub input_device_index: Option<u32>,
    pub output_device_index: Option<u32>,
}

/// Emit a structured JSON event to stdout for the calling tool
fn emit_event(event: &serde_json::Value) {
    if let Ok(json_str) = serde_json::to_string(event) {
        println!("{}", json_str);
    }
}

/// Execute the record command
pub async fn execute(options: RecordOptions, config: RecorderConfig) -> Result<()> {
    execute_with_output(options, config, UserOutput::new()).await
}

/// Execute the record command with custom output
pub async fn execute_with_output(
    options: RecordOptions,
    config: RecorderConfig,
    output: UserOutput,
) -> Result<()> {
    // Check FFmpeg availability before starting
    if let Err(e) = crate::ffmpeg_encoder::check_ffmpeg_available() {
        emit_event(&json!({
            "type": "error",
            "message": e.to_string()
        }));
        return Err(crate::error::RecorderError::Other(e));
    }

    // Validate and resolve device indices early (before creating session)
    #[cfg(windows)]
    let (input_device_name, output_device_name) = {
        use crate::wasapi::device_enumeration::{enumerate_devices, validate_device_index, DeviceDirection};

        let input_name = if let Some(idx) = options.input_device_index {
            let devices = enumerate_devices(DeviceDirection::Input)?;
            let device = validate_device_index(idx, &devices, DeviceDirection::Input)
                .map_err(|e| {
                    emit_event(&json!({
                        "type": "error",
                        "message": e.to_string()
                    }));
                    e
                })?;
            Some(device.name.clone())
        } else {
            None
        };

        let output_name = if let Some(idx) = options.output_device_index {
            let devices = enumerate_devices(DeviceDirection::Output)?;
            let device = validate_device_index(idx, &devices, DeviceDirection::Output)
                .map_err(|e| {
                    emit_event(&json!({
                        "type": "error",
                        "message": e.to_string()
                    }));
                    e
                })?;
            Some(device.name.clone())
        } else {
            None
        };

        (input_name, output_name)
    };

    #[cfg(not(windows))]
    let (input_device_name, output_device_name): (Option<String>, Option<String>) = (None, None);

    // Create recording session
    let session = RecordingSession::new(options.audio_format, options.quality.clone(), options.duration);

    // Determine final output path
    let final_output_path = if let Some(ref path) = options.output_path {
        // User specified a path — ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        path.clone()
    } else {
        // Default: current working directory + auto-generated filename
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        cwd.join(session.filename())
    };

    // Ensure internal directories exist (for status/signals/temp files)
    config.ensure_directories()?;

    // Emit started event (includes device names when specified)
    let mut started_event = json!({
        "type": "started",
        "session_id": session.id.as_str(),
        "file_path": final_output_path.to_string_lossy(),
        "duration": options.duration.to_api_value(),
        "format": options.audio_format.to_string(),
        "quality": options.quality.name,
        "mic_muted": options.mic_muted,
        "trim_silence": options.trim_silence,
    });
    if let Some(ref name) = input_device_name {
        started_event["input_device"] = json!(name);
        started_event["input_device_index"] = json!(options.input_device_index);
    }
    if let Some(ref name) = output_device_name {
        started_event["output_device"] = json!(name);
        started_event["output_device_index"] = json!(options.output_device_index);
    }
    emit_event(&started_event);

    // Now do the actual recording (this blocks, keeping process alive)
    record_worker(
        session,
        config,
        output,
        final_output_path,
        options.mic_muted,
        options.trim_silence,
        options.input_device_index,
        options.output_device_index,
    ).await?;

    Ok(())
}

/// Wait for a file to be fully written and readable
/// Checks both that the file exists and that its size hasn't changed
async fn wait_for_file_ready(path: &std::path::PathBuf, timeout_ms: u64) -> Result<()> {
    let start = tokio::time::Instant::now();
    let timeout = Duration::from_millis(timeout_ms);

    loop {
        // Check if file exists and is readable
        if let Ok(metadata) = std::fs::metadata(path) {
            let size = metadata.len();

            // Wait a bit more and check size again
            tokio::time::sleep(Duration::from_millis(50)).await;

            if let Ok(metadata2) = std::fs::metadata(path) {
                // If size hasn't changed, file is done writing
                if size == metadata2.len() && size > 0 {
                    tracing::debug!("File ready: {:?} ({} bytes)", path, size);
                    return Ok(());
                }
            }
        }

        if start.elapsed() > timeout {
            tracing::warn!("Timeout waiting for file: {:?} (continuing anyway)", path);
            return Ok(()); // Continue anyway, file might be ready
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn record_worker(
    session: RecordingSession,
    config: RecorderConfig,
    output: UserOutput,
    final_output_path: PathBuf,
    mic_muted: bool,
    trim_silence: bool,
    input_device_index: Option<u32>,
    output_device_index: Option<u32>,
) -> Result<()> {
    // Capture values for the span before moving session
    let session_id = session.id.clone();
    let format = session.format;
    let duration_secs = session.duration.effective_duration();

    async move {
        let observer = Arc::new(JsonFileObserver::new(config.status_dir.clone()));
        let effective_duration = session.duration.effective_duration();

        // Use WASAPI dual-channel recording on Windows (loopback + microphone)
        #[cfg(windows)]
        {
        tracing::info!(
            "Starting WASAPI dual-channel recording: {} ({} seconds, mic_muted: {})",
            session.temp_filename(),
            effective_duration,
            mic_muted
        );

        // Create temporary filenames for separate recordings
        let loopback_temp = config.recordings_dir.join(format!(
            "{}_loopback.wav",
            session.id.as_str()
        ));
        let mic_temp = config.recordings_dir.join(format!(
            "{}_mic.wav",
            session.id.as_str()
        ));

        // Initialize loopback recorder (system audio) - REQUIRED
        let loopback_recorder = if let Some(idx) = output_device_index {
            tracing::info!("Using output device index {} for loopback recording", idx);
            WasapiLoopbackRecorder::with_device_index(loopback_temp.clone(), Some(idx))?
        } else {
            WasapiLoopbackRecorder::new(loopback_temp.clone())?
        };

        // Get loopback sample rate to match microphone
        let target_sample_rate = loopback_recorder.get_sample_rate();
        tracing::info!("System audio sample rate: {} Hz - will match microphone to this", target_sample_rate);

        // Create mute flag for microphone (start muted if requested)
        let is_mic_muted = Arc::new(AtomicBool::new(mic_muted));

        // Initialize microphone recorder with matched sample rate and mute flag
        let mut mic_recorder = match if let Some(idx) = input_device_index {
            tracing::info!("Using input device index {} for microphone recording", idx);
            WasapiMicrophoneRecorder::with_device_index(
                mic_temp.clone(),
                target_sample_rate,
                is_mic_muted.clone(),
                Some(idx),
            )
        } else {
            WasapiMicrophoneRecorder::new(
                mic_temp.clone(),
                target_sample_rate,
                is_mic_muted.clone(),
            )
        } {
            Ok(recorder) => {
                tracing::info!("Microphone recorder initialized successfully with matched sample rate");
                if mic_muted {
                    tracing::info!("Microphone starting in MUTED state");
                }
                Some(recorder)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize microphone recorder: {}. Continuing with loopback only.", e);
                output.warning("Microphone unavailable - recording system audio only");
                None
            }
        };

        let start_time = std::time::Instant::now();

        // Write initial status
        observer.on_progress(RecordingStatus {
            session_id: session.id.as_str().to_string(),
            filename: session.temp_filename(),
            duration: effective_duration,
            elapsed: 0,
            progress: 0,
            quality: session.quality.name.clone(),
            device: if mic_recorder.is_some() {
                "Dual-Channel (System + Microphone)".to_string()
            } else {
                "System Audio Only (WASAPI Loopback)".to_string()
            },
            sample_rate: loopback_recorder.get_sample_rate(),
            channels: 2, // Always stereo output
            frames_captured: loopback_recorder.get_frames_captured(),
            has_audio: loopback_recorder.has_audio_detected(),
            status: "recording".to_string(),
            // Per-channel data (initial state)
            loopback_frames: Some(loopback_recorder.get_frames_captured()),
            loopback_has_audio: Some(loopback_recorder.has_audio_detected()),
            mic_frames: mic_recorder.as_ref().map(|m| m.get_frames_captured()),
            mic_has_audio: mic_recorder.as_ref().map(|m| m.has_audio_detected()),
            is_mic_muted: Some(is_mic_muted.load(Ordering::Relaxed)),
            // FFmpeg progress (not applicable during recording)
            ffmpeg_progress: None,
            processing_speed: None,
        })?;

        // Track previous state for change detection (only emit events on state changes)
        let mut prev_loopback_has_audio = loopback_recorder.has_audio_detected();
        let mut prev_mic_has_audio = mic_recorder.as_ref().map(|m| m.has_audio_detected());
        let mut prev_mic_muted = is_mic_muted.load(Ordering::Relaxed);

        // Check for signals and state changes every second
        let check_interval = config.signal_check_interval;
        loop {
            tokio::time::sleep(check_interval).await;

            let elapsed = start_time.elapsed().as_secs();

            // Check for time limit (0 = indefinite)
            if effective_duration > 0 && elapsed >= effective_duration {
                break;
            }

            // Check for cancel signal (skip processing)
            let cancel_signal = config
                .signals_dir
                .join(format!("{}.cancel", session.id.as_str()));
            if cancel_signal.exists() {
                tracing::info!("Cancel signal received for session {}", session.id);
                let _ = std::fs::remove_file(cancel_signal);

                // Stop both recorders
                let mut loopback_recorder = loopback_recorder;
                loopback_recorder.stop()?;
                if let Some(mut mic) = mic_recorder.take() {
                    mic.stop()?;
                }

                // Cleanup temporary files
                let _ = std::fs::remove_file(&loopback_temp);
                let _ = std::fs::remove_file(&mic_temp);
                tracing::info!("Recording cancelled, temporary files cleaned up");

                // Write cancelled status
                observer.on_complete(RecordingResult {
                    session_id: session.id.as_str().to_string(),
                    filename: "".to_string(),
                    file_path: None,
                    duration: 0,
                    file_size_mb: "0 MB".to_string(),
                    format: session.format.to_string(),
                    codec: "".to_string(),
                    status: "cancelled".to_string(),
                    message: "Recording cancelled by user".to_string(),
                })?;

                emit_event(&json!({
                    "type": "cancelled",
                    "session_id": session.id.as_str(),
                    "message": "Recording cancelled by user"
                }));

                output.warning("Recording cancelled by user");
                return Ok(());
            }

            // Check for stop signal
            let stop_signal = config
                .signals_dir
                .join(format!("{}.stop", session.id.as_str()));
            if stop_signal.exists() {
                tracing::info!("Stop signal received for session {}", session.id);
                let _ = std::fs::remove_file(stop_signal);
                break;
            }

            // Check for mute signal
            let mute_signal = config.signals_dir.join(format!("{}.mute", session.id.as_str()));
            if mute_signal.exists() {
                let _ = std::fs::remove_file(&mute_signal);
                if mic_recorder.is_some() {
                    let was_muted = is_mic_muted.fetch_xor(true, Ordering::Relaxed);
                    let now_muted = !was_muted;

                    emit_event(&json!({
                        "type": "muted",
                        "session_id": session.id.as_str(),
                        "is_muted": now_muted
                    }));

                    if now_muted {
                        tracing::info!("Microphone muted for session {}", session.id);
                        output.prefixed("Mute", "Microphone muted");
                    } else {
                        tracing::info!("Microphone unmuted for session {}", session.id);
                        output.prefixed("Mute", "Microphone unmuted");
                    }
                } else {
                    tracing::warn!("Mute signal ignored - no microphone is active");
                }
            }

            // Detect state changes and emit events only when something changes
            let cur_loopback_has_audio = loopback_recorder.has_audio_detected();
            let cur_mic_has_audio = mic_recorder.as_ref().map(|m| m.has_audio_detected());
            let cur_mic_muted = is_mic_muted.load(Ordering::Relaxed);

            let state_changed = cur_loopback_has_audio != prev_loopback_has_audio
                || cur_mic_has_audio != prev_mic_has_audio
                || cur_mic_muted != prev_mic_muted;

            if state_changed {
                emit_event(&json!({
                    "type": "audio_state",
                    "session_id": session.id.as_str(),
                    "loopback_has_audio": cur_loopback_has_audio,
                    "mic_has_audio": cur_mic_has_audio,
                    "is_mic_muted": cur_mic_muted,
                }));

                // Calculate progress for status file
                let progress = if effective_duration > 0 {
                    ((elapsed as f64 / effective_duration as f64) * 100.0).min(100.0) as u8
                } else {
                    0
                };

                // Update status file only on state change
                observer.on_progress(RecordingStatus {
                    session_id: session.id.as_str().to_string(),
                    filename: session.temp_filename(),
                    duration: effective_duration,
                    elapsed,
                    progress,
                    quality: session.quality.name.clone(),
                    device: if mic_recorder.is_some() {
                        "Dual-Channel (System + Microphone)".to_string()
                    } else {
                        "System Audio Only (WASAPI Loopback)".to_string()
                    },
                    sample_rate: loopback_recorder.get_sample_rate(),
                    channels: 2,
                    frames_captured: loopback_recorder.get_frames_captured(),
                    has_audio: cur_loopback_has_audio,
                    status: "recording".to_string(),
                    loopback_frames: Some(loopback_recorder.get_frames_captured()),
                    loopback_has_audio: Some(cur_loopback_has_audio),
                    mic_frames: mic_recorder.as_ref().map(|m| m.get_frames_captured()),
                    mic_has_audio: cur_mic_has_audio,
                    is_mic_muted: Some(cur_mic_muted),
                    ffmpeg_progress: None,
                    processing_speed: None,
                })?;

                // Update tracked state
                prev_loopback_has_audio = cur_loopback_has_audio;
                prev_mic_has_audio = cur_mic_has_audio;
                prev_mic_muted = cur_mic_muted;
            }
        }

        // Stop both recorders
        let mut loopback_recorder = loopback_recorder;
        loopback_recorder.stop()?;
        if let Some(mut mic) = mic_recorder.take() {
            mic.stop()?;
        }

        let actual_elapsed = start_time.elapsed().as_secs();
        tracing::info!("Recording completed after {}s, starting post-processing", actual_elapsed);
        output.success("Recording completed successfully!");

        emit_event(&json!({
            "type": "processing",
            "session_id": session.id.as_str(),
            "message": "Post-processing started",
            "elapsed_secs": actual_elapsed,
        }));

        // Wait for temporary files to be fully written and readable before processing
        tracing::info!("Waiting for temporary files to be ready...");
        let _ = wait_for_file_ready(&loopback_temp, 2000).await;
        let _ = wait_for_file_ready(&mic_temp, 2000).await;
        tracing::info!("Temporary files are ready for processing");

        // Determine total steps based on format and trimming
        let has_trim = trim_silence;
        let total_steps = match (session.format, has_trim) {
            (AudioFormat::M4a, true) => 5,   // Analyze -> Merge -> Encode -> Trim -> Finalize
            (AudioFormat::M4a, false) => 4,  // Analyze -> Merge -> Encode -> Finalize
            (AudioFormat::Wav, true) => 4,   // Analyze -> Merge -> Trim -> Finalize
            (AudioFormat::Wav, false) => 3,  // Analyze -> Merge -> Finalize
        };

        // Stage 1: Analyzing Audio
        let loopback_has_audio = loopback_recorder.has_audio_detected();
        let mic_has_audio = mic_recorder.as_ref().map(|m| m.has_audio_detected()).unwrap_or(false);

        let analysis_message = if loopback_has_audio && mic_has_audio {
            "Detected system audio and microphone"
        } else if loopback_has_audio {
            "Detected system audio only"
        } else if mic_has_audio {
            "Detected microphone only"
        } else {
            "No audio detected"
        };

        tracing::info!("Stage 1/{}: Analyzing Audio - {}", total_steps, analysis_message);
        let _ = observer.write_processing_status_v2(
            session.id.as_str(),
            analysis_message,
            Some(1),
            Some(total_steps),
            Some("analyzing"),
            None,
            None,
        );

        tracing::info!("Post-processing started");

        // Determine the merge output path (temp location in recordings dir)
        let merge_output_path = if matches!(session.format, AudioFormat::M4a) {
            config.recordings_dir.join(format!("{}_merged.m4a", session.id.as_str()))
        } else {
            config.recordings_dir.join(format!("{}_merged.wav", session.id.as_str()))
        };

        // Merge audio channels (and encode to M4A if requested)
        {
            let loopback_has_audio_flag = loopback_recorder.has_audio_detected();
            let mic_has_audio_flag = mic_recorder.as_ref().map(|m| m.has_audio_detected()).unwrap_or(false);

            async {
                if matches!(session.format, AudioFormat::M4a) {
                    output.prefixed("Processing", "Merging and encoding to M4A...");
                } else {
                    output.prefixed("Processing", "Merging audio channels...");
                }

                // Small delay to ensure status file was written before merge starts
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Get audio detection flags
                let loopback_has_audio = loopback_recorder.has_audio_detected();
                let mic_has_audio = mic_recorder.as_ref()
                    .map(|m| m.has_audio_detected())
                    .unwrap_or(false);

                // Merge audio streams using FFmpeg
                merge_audio_streams_smart(
                    &loopback_temp,
                    &mic_temp,
                    &merge_output_path,
                    loopback_has_audio,
                    mic_has_audio,
                    &session.quality,
                    session.format,
                    Some(session.id.as_str()),
                    Some(observer.clone()),
                    total_steps,
                )
                .await?;

                tracing::info!("Audio processing completed: {:?}", merge_output_path);
                output.success("Audio processing completed!");
                Ok::<(), crate::error::RecorderError>(())
            }
            .instrument(tracing::info_span!(
                "merge_audio",
                loopback_has_audio = loopback_has_audio_flag,
                mic_has_audio = mic_has_audio_flag
            ))
            .await?;
        }

        // Cleanup temporary recording files
        let _ = std::fs::remove_file(&loopback_temp);
        let _ = std::fs::remove_file(&mic_temp);
        tracing::info!("Temporary recording files cleaned up");

        // Clean up stale mute signal
        let mute_signal = config.signals_dir.join(format!("{}.mute", session.id.as_str()));
        let _ = std::fs::remove_file(&mute_signal);

        // Silence trimming (if enabled)
        if trim_silence {
            tracing::info!("Trimming silence from recording...");
            output.prefixed("Processing", "Trimming silence...");

            emit_event(&json!({
                "type": "processing",
                "session_id": session.id.as_str(),
                "message": "Trimming silence",
            }));

            // Trim with -50dB threshold and 2s max internal silence
            let _ = crate::ffmpeg_encoder::trim_silence(
                &merge_output_path,
                -50.0,
                2.0,
            ).await;

            tracing::info!("Silence trimming completed");
        }

        // Move the processed file to the final output path
        if let Some(parent) = final_output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        // Copy then remove (works across drives, unlike rename)
        std::fs::copy(&merge_output_path, &final_output_path)?;
        let _ = std::fs::remove_file(&merge_output_path);

        tracing::info!("Final file saved to: {:?}", final_output_path);
        }

        #[cfg(not(windows))]
        {
            return Err(crate::error::RecorderError::Other(anyhow::anyhow!(
                "Recording is currently only supported on Windows. Cross-platform support (Linux/macOS) is planned for a future release."
            )));
        }

        // Get final file info
        let file_size_mb = if final_output_path.exists() {
            let size = std::fs::metadata(&final_output_path)?.len();
            (size as f64) / (1024.0 * 1024.0)
        } else {
            0.0
        };

        let final_filename = final_output_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Write final status to status file
        observer.on_complete(RecordingResult {
            status: "completed".to_string(),
            session_id: session.id.as_str().to_string(),
            filename: final_filename.clone(),
            file_path: Some(final_output_path.to_string_lossy().to_string()),
            duration: session.duration.to_api_value(),
            file_size_mb: format!("{:.2}", file_size_mb),
            format: session.format.to_string(),
            codec: session.format.codec().to_string(),
            message: "Recording completed successfully".to_string(),
        })?;

        // Emit completed event to stdout (this is what the calling tool reads)
        emit_event(&json!({
            "type": "completed",
            "session_id": session.id.as_str(),
            "file_path": final_output_path.to_string_lossy(),
            "filename": final_filename,
            "file_size_mb": format!("{:.2}", file_size_mb),
            "format": session.format.to_string(),
            "duration_secs": session.duration.to_api_value(),
            "message": "Recording completed successfully"
        }));

        Ok(())
    }
    .instrument(tracing::info_span!(
        "recording_session",
        session_id = %session_id,
        format = ?format,
        duration_secs = duration_secs
    ))
    .await
}
