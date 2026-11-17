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
use crate::recorder::{merge_audio_streams_smart, AudioAlignment, RecordingQuality};
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
    // Quality feature flags (all ON by default)
    pub normalize: bool,
    pub limiter: bool,
    pub denoise: bool,
    pub noise_gate: bool,
    // Quality feature configuration
    pub target_loudness: f64,    // LUFS, default -16.0
    pub gate_threshold: f64,     // VAD probability 0.0-1.0 or dB, default 0.5
    // Process-specific capture
    pub app: Option<String>,         // --app <name>
    pub app_window: Option<String>,  // --app-window <title>
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
    // Fix invalid filenames now, not after the recording: the final copy is the last step.
    let requested_output_path = options.output_path.as_ref().map(|path| {
        match sanitize_output_file_name(path) {
            Some(cleaned) => {
                output.warning(format!(
                    "Output name {:?} has characters Windows does not allow in filenames. Saving as {:?}",
                    path, cleaned
                ));
                cleaned
            }
            None => path.clone(),
        }
    });

    let final_output_path = if let Some(ref path) = requested_output_path.as_ref().filter(|p| p.is_dir()) {
        // User passed a directory: copying onto a directory fails with "Access is denied"
        // on Windows, so put the auto-generated filename inside it.
        path.join(session.filename())
    } else if let Some(ref path) = requested_output_path {
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

    // Emit started event (includes device names and capture mode when specified)
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
    // Include process-specific capture info in started event
    if let Some(ref app_name) = options.app {
        started_event["capture_mode"] = json!("process");
        started_event["app"] = json!(app_name);
    } else if let Some(ref window_title) = options.app_window {
        started_event["capture_mode"] = json!("process");
        started_event["app_window"] = json!(window_title);
    } else {
        started_event["capture_mode"] = json!("system");
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
        options.app,
        options.app_window,
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

/// Characters Windows does not allow in a file name. A ':' is the worst one: NTFS reads
/// "a:b.wav" as alternate data stream "b.wav" on file "a". The final copy then fails
/// with "Access is denied", but only after the whole recording has been processed.
const INVALID_FILE_NAME_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Returns a path with invalid file name characters replaced by '-', and trailing dots
/// and spaces removed. Returns `None` if the file name is already valid.
/// Only the last path component is changed, so a drive prefix such as "C:" is kept.
fn sanitize_output_file_name(path: &std::path::Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let replaced: String = name
        .chars()
        .map(|c| if INVALID_FILE_NAME_CHARS.contains(&c) || c.is_control() { '-' } else { c })
        .collect();
    let cleaned = replaced.trim_end_matches(['.', ' ']);
    if cleaned == name || cleaned.is_empty() {
        return None;
    }
    Some(path.with_file_name(cleaned))
}

/// Copy the processed temporary file to its official destination, then remove it.
/// A successful return guarantees there is no second merged output left behind.
fn finalize_output(merge_output_path: &PathBuf, final_output_path: &PathBuf) -> Result<()> {
    if let Some(parent) = final_output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Rename first: on the same volume it needs no extra disk space. A full-size copy
    // here once doubled the space a long recording needed at its peak.
    if std::fs::rename(merge_output_path, final_output_path).is_ok() {
        return Ok(());
    }

    // Rename fails across drives, so copy then remove.
    // On failure the processed file stays in place, so name both paths for recovery.
    std::fs::copy(merge_output_path, final_output_path).map_err(|error| {
        anyhow::anyhow!(
            "Could not write final recording to {} ({}). The processed recording is kept at {}",
            final_output_path.display(),
            error,
            merge_output_path.display()
        )
    })?;
    std::fs::remove_file(merge_output_path)?;

    Ok(())
}

fn finalize_recording_files(
    merge_output_path: &PathBuf,
    final_output_path: &PathBuf,
    raw_tracks: &[&PathBuf],
) -> Result<()> {
    finalize_output(merge_output_path, final_output_path)?;
    for raw_track in raw_tracks {
        match std::fs::remove_file(raw_track) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Warns at recording start when the recordings drive may not hold the whole recording
/// (raw tracks + merge + optional trim). Indefinite recordings are checked for one hour.
#[cfg(windows)]
fn warn_if_disk_space_low(
    session: &RecordingSession,
    config: &RecorderConfig,
    trim_silence: bool,
    output: &UserOutput,
) {
    use crate::disk_space;

    let Some(free) = disk_space::available_bytes(&config.recordings_dir) else {
        return;
    };
    let planned_secs = match session.duration.effective_duration() {
        0 => disk_space::INDEFINITE_ESTIMATE_SECS,
        secs => secs,
    };
    let needed = disk_space::estimate_recording_peak_bytes(
        planned_secs,
        session.quality.sample_rate,
        session.quality.channels,
        matches!(session.format, AudioFormat::M4a),
        trim_silence,
    );
    tracing::info!(
        "Disk space: {} free in {:?}, about {} needed for {} min",
        disk_space::format_mb(free),
        config.recordings_dir,
        disk_space::format_mb(needed),
        planned_secs / 60
    );
    if free < needed {
        output.warning(format!(
            "Low disk space: {} free on the recordings drive, but a {}-minute recording needs about {}. \
             Free up space now, or the recording may fail when it is saved.",
            disk_space::format_mb(free),
            planned_secs / 60,
            disk_space::format_mb(needed)
        ));
    }
}

/// Returns an error before the merge starts if the recordings drive cannot hold the
/// merged output (and the trimmed copy, when trimming is on). The raw tracks are kept,
/// so the recording can be merged again after the user frees space.
#[cfg(windows)]
fn ensure_space_for_post_processing(
    session: &RecordingSession,
    config: &RecorderConfig,
    raw_tracks: &[&PathBuf],
    alignment: &AudioAlignment,
    trim_silence: bool,
) -> Result<()> {
    use crate::disk_space;

    let Some(free) = disk_space::available_bytes(&config.recordings_dir) else {
        tracing::warn!("Could not read free disk space; skipping the pre-merge space check");
        return Ok(());
    };
    let longest_track_secs = raw_tracks
        .iter()
        .filter_map(|path| hound::WavReader::open(path).ok())
        .map(|reader| {
            let rate = reader.spec().sample_rate.max(1);
            f64::from(reader.duration()) / f64::from(rate)
        })
        .fold(0.0_f64, f64::max);
    let output_secs = longest_track_secs + alignment.start_offset_seconds().abs();
    let merged = disk_space::estimate_merged_output_bytes(
        output_secs,
        session.quality.sample_rate,
        session.quality.channels,
        matches!(session.format, AudioFormat::M4a),
    );
    let needed = disk_space::required_post_processing_bytes(merged, trim_silence);
    tracing::info!(
        "Pre-merge disk check: {} free, {} needed ({:.0} s of audio, trim: {})",
        disk_space::format_mb(free),
        disk_space::format_mb(needed),
        output_secs,
        trim_silence
    );
    if free >= needed {
        return Ok(());
    }

    let kept: Vec<String> = raw_tracks.iter().map(|p| p.display().to_string()).collect();
    Err(crate::error::RecorderError::Other(anyhow::anyhow!(
        "Not enough disk space to save the recording: {} free on the drive of {}, but the merge needs about {}. \
         Nothing was lost: the raw tracks are kept at {}. Free up space, then merge them again.",
        disk_space::format_mb(free),
        config.recordings_dir.display(),
        disk_space::format_mb(needed),
        kept.join(" and ")
    )))
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
    app: Option<String>,
    app_window: Option<String>,
) -> Result<()> {
    // Capture values for the span before moving session
    let session_id = session.id.clone();
    let format = session.format;
    let duration_secs = session.duration.effective_duration();

    async move {
        let observer = Arc::new(JsonFileObserver::new(config.status_dir.clone()));
        let effective_duration = session.duration.effective_duration();

        // Why a capture stream stopped early, one entry per stream. Empty when
        // both streams ran to the end. Drives the final message and event.
        #[allow(unused_mut)]
        let mut stream_failures: Vec<String> = Vec::new();
        // How many times a lost device was rebuilt; each left a few silent seconds.
        #[allow(unused_mut)]
        let mut total_device_recoveries: u64 = 0;

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

        warn_if_disk_space_low(&session, &config, trim_silence, &output);

        // Determine capture mode: process-specific or system-wide
        let mut capture_mode: String;
        let _capture_app = app.clone();
        let _capture_app_window = app_window.clone();

        // Initialize loopback recorder — either process-specific or system-wide
        let mut loopback_recorder: Box<dyn crate::wasapi::capture_engine::LoopbackCapture> =
            if let Some(ref app_name) = app {
                // Process-specific capture via --app
                capture_mode = "process".to_string();

                // Check Windows version
                if !crate::wasapi::process_resolver::check_windows_version() {
                    emit_event(&json!({
                        "type": "warning",
                        "session_id": session.id.as_str(),
                        "code": "unsupported_version",
                        "message": "Windows version does not support process-specific loopback (requires Build 20348+). Falling back to system audio capture."
                    }));
                    tracing::warn!("Windows version too old for process loopback, falling back to system audio");
                    capture_mode = "system".to_string();
                    Box::new(WasapiLoopbackRecorder::new(loopback_temp.clone())?)
                } else {
                    // Resolve PID
                    let pid = crate::wasapi::process_resolver::resolve_by_process_name(app_name)?;
                    tracing::info!("Resolved process '{}' to PID {}", app_name, pid);

                    // Warn if --output-device is also specified (ignored for process loopback)
                    if output_device_index.is_some() {
                        emit_event(&json!({
                            "type": "warning",
                            "session_id": session.id.as_str(),
                            "code": "output_device_ignored",
                            "message": "Output device selection is ignored when using --app (process-specific capture)."
                        }));
                    }

                    match crate::wasapi::process_loopback::ProcessLoopbackRecorder::new(
                        loopback_temp.clone(),
                        pid,
                    ) {
                        Ok(recorder) => {
                            tracing::info!("Process loopback recorder created for PID {}", pid);
                            Box::new(recorder)
                        }
                        Err(e) => {
                            tracing::error!("Failed to create process loopback recorder: {}. Falling back to system audio.", e);
                            emit_event(&json!({
                                "type": "warning",
                                "session_id": session.id.as_str(),
                                "code": "process_loopback_failed",
                                "message": format!("Process loopback failed: {}. Falling back to system audio.", e)
                            }));
                            capture_mode = "system".to_string();
                            Box::new(WasapiLoopbackRecorder::new(loopback_temp.clone())?)
                        }
                    }
                }
            } else if let Some(ref window_title) = app_window {
                // Process-specific capture via --app-window
                capture_mode = "process".to_string();

                if !crate::wasapi::process_resolver::check_windows_version() {
                    emit_event(&json!({
                        "type": "warning",
                        "session_id": session.id.as_str(),
                        "code": "unsupported_version",
                        "message": "Windows version does not support process-specific loopback (requires Build 20348+). Falling back to system audio capture."
                    }));
                    tracing::warn!("Windows version too old for process loopback, falling back to system audio");
                    capture_mode = "system".to_string();
                    Box::new(WasapiLoopbackRecorder::new(loopback_temp.clone())?)
                } else {
                    let pid = crate::wasapi::process_resolver::resolve_by_window_title(window_title)?;
                    tracing::info!("Resolved window title '{}' to PID {}", window_title, pid);

                    if output_device_index.is_some() {
                        emit_event(&json!({
                            "type": "warning",
                            "session_id": session.id.as_str(),
                            "code": "output_device_ignored",
                            "message": "Output device selection is ignored when using --app-window (process-specific capture)."
                        }));
                    }

                    match crate::wasapi::process_loopback::ProcessLoopbackRecorder::new(
                        loopback_temp.clone(),
                        pid,
                    ) {
                        Ok(recorder) => {
                            tracing::info!("Process loopback recorder created for PID {}", pid);
                            Box::new(recorder)
                        }
                        Err(e) => {
                            tracing::error!("Failed to create process loopback recorder: {}. Falling back to system audio.", e);
                            emit_event(&json!({
                                "type": "warning",
                                "session_id": session.id.as_str(),
                                "code": "process_loopback_failed",
                                "message": format!("Process loopback failed: {}. Falling back to system audio.", e)
                            }));
                            capture_mode = "system".to_string();
                            Box::new(WasapiLoopbackRecorder::new(loopback_temp.clone())?)
                        }
                    }
                }
            } else {
                // Standard system-wide loopback
                capture_mode = "system".to_string();
                if let Some(idx) = output_device_index {
                    tracing::info!("Using output device index {} for loopback recording", idx);
                    Box::new(WasapiLoopbackRecorder::with_device_index(loopback_temp.clone(), Some(idx))?)
                } else {
                    Box::new(WasapiLoopbackRecorder::new(loopback_temp.clone())?)
                }
            };

        // capture_mode may change during recording (e.g., process exits → system fallback)
        let mut capture_mode = capture_mode;

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
            // Process-specific capture fields
            capture_mode: Some(capture_mode.clone()),
            app: _capture_app.clone(),
            app_window: _capture_app_window.clone(),
        })?;

        // Track previous state for change detection (only emit events on state changes)
        let mut prev_loopback_has_audio = loopback_recorder.has_audio_detected();
        let mut prev_mic_has_audio = mic_recorder.as_ref().map(|m| m.has_audio_detected());
        let mut prev_mic_muted = is_mic_muted.load(Ordering::Relaxed);
        let mut prev_loopback_device_lost = false;
        let mut prev_mic_device_lost: Option<bool> = mic_recorder.as_ref().map(|_| false);
        let mut prev_device_recoveries: u64 = 0;
        let mut reported_loopback_failure = false;
        let mut reported_mic_failure = false;

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

            // ── Mid-recording fallback: process exit detection ──
            // If the target process has exited, fall back to system-wide loopback
            if capture_mode == "process" && loopback_recorder.is_process_dead() {
                tracing::warn!("Target process exited during recording, falling back to system-wide loopback");

                // Emit warning event
                emit_event(&json!({
                    "type": "warning",
                    "session_id": session.id.as_str(),
                    "code": "process_exited",
                    "message": "Target process has exited. Falling back to system-wide audio capture."
                }));

                // Stop the process-specific recorder
                loopback_recorder.stop()?;

                // Attempt to create a system-wide loopback recorder writing to the same temp file
                match WasapiLoopbackRecorder::new(loopback_temp.clone()) {
                    Ok(new_recorder) => {
                        loopback_recorder = Box::new(new_recorder);
                        capture_mode = "system".to_string();
                        tracing::info!("Successfully fell back to system-wide loopback recording");
                    }
                    Err(e) => {
                        // Fallback recorder creation failed (rare: audio device disconnected)
                        // Continue with mic-only recording
                        tracing::error!("Failed to create fallback system loopback recorder: {}. Continuing with mic-only.", e);
                        emit_event(&json!({
                            "type": "error",
                            "session_id": session.id.as_str(),
                            "code": "fallback_failed",
                            "message": format!("Failed to create fallback recorder: {}. Continuing with microphone only.", e)
                        }));
                        capture_mode = "system".to_string();
                        // Don't break — mic recording continues
                    }
                }
            }

            // Detect state changes and emit events only when something changes
            let cur_loopback_has_audio = loopback_recorder.has_audio_detected();
            let cur_mic_has_audio = mic_recorder.as_ref().map(|m| m.has_audio_detected());
            let cur_mic_muted = is_mic_muted.load(Ordering::Relaxed);
            // Device health: a lost device is being rebuilt by its capture thread.
            // Surface it live so a dead stream is never hidden until stop.
            let cur_loopback_device_lost = loopback_recorder.is_device_lost();
            let cur_mic_device_lost = mic_recorder.as_ref().map(|m| m.is_device_lost());
            let cur_device_recoveries = loopback_recorder.device_recoveries()
                + mic_recorder.as_ref().map(|m| m.device_recoveries()).unwrap_or(0);

            // A stream that stopped for good is reported once, as it happens.
            for (stream, failure, reported) in [
                ("system audio", loopback_recorder.capture_failure(), &mut reported_loopback_failure),
                (
                    "microphone",
                    mic_recorder.as_ref().and_then(|m| m.capture_failure()),
                    &mut reported_mic_failure,
                ),
            ] {
                if let Some(reason) = failure {
                    if !*reported {
                        *reported = true;
                        tracing::error!("The {} stream stopped early: {}", stream, reason);
                        emit_event(&json!({
                            "type": "warning",
                            "session_id": session.id.as_str(),
                            "code": "stream_failed",
                            "message": format!("The {} recording stopped: {}. The rest of the session is recorded without it.", stream, reason)
                        }));
                        output.warning(&format!("{} recording stopped: {}", stream, reason));
                    }
                }
            }

            for (stream, was_lost, is_lost) in [
                ("system audio", prev_loopback_device_lost, cur_loopback_device_lost),
                (
                    "microphone",
                    prev_mic_device_lost.unwrap_or(false),
                    cur_mic_device_lost.unwrap_or(false),
                ),
            ] {
                if is_lost && !was_lost {
                    emit_event(&json!({
                        "type": "warning",
                        "session_id": session.id.as_str(),
                        "code": "device_lost",
                        "message": format!("The {} device was lost. Reconnecting; silence is recorded until it is back.", stream)
                    }));
                    output.warning(&format!("{} device lost - reconnecting...", stream));
                } else if was_lost && !is_lost {
                    emit_event(&json!({
                        "type": "warning",
                        "session_id": session.id.as_str(),
                        "code": "device_recovered",
                        "message": format!("The {} device is back. Recording resumed.", stream)
                    }));
                    output.prefixed("Recovery", &format!("{} device reconnected, recording resumed", stream));
                }
            }

            let state_changed = cur_loopback_has_audio != prev_loopback_has_audio
                || cur_mic_has_audio != prev_mic_has_audio
                || cur_mic_muted != prev_mic_muted
                || cur_loopback_device_lost != prev_loopback_device_lost
                || cur_mic_device_lost != prev_mic_device_lost
                || cur_device_recoveries != prev_device_recoveries;

            if state_changed {
                emit_event(&json!({
                    "type": "audio_state",
                    "session_id": session.id.as_str(),
                    "loopback_has_audio": cur_loopback_has_audio,
                    "mic_has_audio": cur_mic_has_audio,
                    "is_mic_muted": cur_mic_muted,
                    "capture_mode": &capture_mode,
                    "loopback_device_lost": cur_loopback_device_lost,
                    "mic_device_lost": cur_mic_device_lost,
                    "device_recoveries": cur_device_recoveries,
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
                    capture_mode: Some(capture_mode.clone()),
                    app: _capture_app.clone(),
                    app_window: _capture_app_window.clone(),
                })?;

                // Update tracked state
                prev_loopback_has_audio = cur_loopback_has_audio;
                prev_mic_has_audio = cur_mic_has_audio;
                prev_mic_muted = cur_mic_muted;
                prev_loopback_device_lost = cur_loopback_device_lost;
                prev_mic_device_lost = cur_mic_device_lost;
                prev_device_recoveries = cur_device_recoveries;
            }
        }

        // Save audio detection state and start times BEFORE stopping/taking recorders
        // (mic_recorder.take() sets it to None, making has_audio_detected() inaccessible)
        let final_loopback_has_audio = loopback_recorder.has_audio_detected();
        let final_mic_has_audio = mic_recorder.as_ref()
            .map(|m| m.has_audio_detected())
            .unwrap_or(false);

        // Stop and finalize both writers before taking immutable timing snapshots.
        let mut loopback_recorder = loopback_recorder;
        let loopback_stop_result = loopback_recorder.stop();
        let loopback_timing = loopback_recorder.capture_timing();
        stream_failures.extend(loopback_recorder.capture_failure());
        if loopback_recorder.is_device_lost() {
            stream_failures.push(
                "the system audio device was still disconnected when the recording stopped".to_string(),
            );
        }
        total_device_recoveries += loopback_recorder.device_recoveries();
        let (mic_stop_result, mic_timing) = if let Some(mut mic) = mic_recorder.take() {
            let stop_result = mic.stop();
            stream_failures.extend(mic.capture_failure());
            if mic.is_device_lost() {
                stream_failures.push(
                    "the microphone was still disconnected when the recording stopped".to_string(),
                );
            }
            total_device_recoveries += mic.device_recoveries();
            (stop_result, Some(mic.capture_timing()))
        } else {
            (Ok(()), None)
        };
        loopback_stop_result?;
        mic_stop_result?;
        let alignment = mic_timing
            .as_ref()
            .map(|timing| AudioAlignment::from_captures(&loopback_timing, timing))
            .unwrap_or_else(|| AudioAlignment::none("microphone capture unavailable"));

        let actual_elapsed = start_time.elapsed().as_secs();
        tracing::info!("Recording completed after {}s, starting post-processing", actual_elapsed);
        if stream_failures.is_empty() && total_device_recoveries == 0 {
            output.success("Recording completed successfully!");
        } else if stream_failures.is_empty() {
            output.success(format!(
                "Recording completed. The audio device dropped and reconnected {} time(s); a few seconds are silent each time.",
                total_device_recoveries
            ));
        } else {
            // Never claim success when a stream stopped early: the file will be
            // missing that stream from the point it stopped.
            output.warning(&format!(
                "Recording stopped, but part of it is missing: {}",
                stream_failures.join("; ")
            ));
        }

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
        // Use the saved values from before stop/take (mic_recorder is None after take())
        let loopback_has_audio = final_loopback_has_audio;
        let mic_has_audio = final_mic_has_audio;

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

        // Fail before FFmpeg starts if the merge cannot fit on disk. Otherwise FFmpeg
        // writes for minutes, then dies with ENOSPC and leaves a truncated file.
        ensure_space_for_post_processing(
            &session,
            &config,
            &[&loopback_temp, &mic_temp],
            &alignment,
            trim_silence,
        )?;

        // Merge audio channels (and encode to M4A if requested)
        {
            // Use saved values (mic_recorder was already taken above)
            let loopback_has_audio_flag = final_loopback_has_audio;
            let mic_has_audio_flag = final_mic_has_audio;

            async {
                if matches!(session.format, AudioFormat::M4a) {
                    output.prefixed("Processing", "Merging and encoding to M4A...");
                } else {
                    output.prefixed("Processing", "Merging audio channels...");
                }

                // Small delay to ensure status file was written before merge starts
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Merge audio streams using FFmpeg (with desync compensation)
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
                    &alignment,
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

        finalize_recording_files(
            &merge_output_path,
            &final_output_path,
            &[&loopback_temp, &mic_temp],
        )?;
        tracing::info!("Temporary recording files cleaned up");

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

        let completion_message = if stream_failures.is_empty() && total_device_recoveries == 0 {
            "Recording completed successfully".to_string()
        } else if stream_failures.is_empty() {
            format!(
                "Recording completed. The audio device dropped and reconnected {} time(s); a few seconds are silent each time",
                total_device_recoveries
            )
        } else {
            format!(
                "Recording saved, but part of it is missing: {}",
                stream_failures.join("; ")
            )
        };

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
            message: completion_message.clone(),
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
            "message": completion_message,
            "warnings": stream_failures,
            "device_recoveries": total_device_recoveries,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn sanitize_replaces_colon_in_file_name() {
        let cleaned = sanitize_output_file_name(std::path::Path::new("2026-11-07_1:1_Nic.wav"));
        assert_eq!(cleaned, Some(PathBuf::from("2026-11-07_1-1_Nic.wav")));
    }

    #[test]
    fn sanitize_keeps_drive_prefix_and_valid_names() {
        assert_eq!(sanitize_output_file_name(std::path::Path::new(r"C:\rec\meeting.wav")), None);
        assert_eq!(
            sanitize_output_file_name(std::path::Path::new(r"C:\rec\a?b.wav")),
            Some(PathBuf::from(r"C:\rec\a-b.wav"))
        );
    }

    #[test]
    fn finalize_output_keeps_only_official_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::current_dir()
            .unwrap()
            .join("target/test-artifacts")
            .join(format!("audio-rec-finalize-{unique}"));
        std::fs::create_dir_all(&test_dir).unwrap();

        let merged = test_dir.join("session_merged.wav");
        let official = test_dir.join("recording.wav");
        std::fs::write(&merged, b"recording data").unwrap();

        finalize_output(&merged, &official).unwrap();

        assert!(official.exists());
        assert!(!merged.exists());
        assert_eq!(std::fs::read(&official).unwrap(), b"recording data");

        std::fs::remove_dir_all(test_dir).unwrap();
    }
    #[test]
    fn failed_finalization_retains_raw_tracks_for_recovery() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_dir = std::env::current_dir()
            .unwrap()
            .join("target/test-artifacts")
            .join(format!("audio-rec-recovery-{unique}"));
        std::fs::create_dir_all(&test_dir).unwrap();

        let missing_merged = test_dir.join("missing_merged.wav");
        let official = test_dir.join("recording.wav");
        let loopback = test_dir.join("loopback.wav");
        let microphone = test_dir.join("microphone.wav");
        std::fs::write(&loopback, b"loopback recovery data").unwrap();
        std::fs::write(&microphone, b"microphone recovery data").unwrap();

        let result = finalize_recording_files(
            &missing_merged,
            &official,
            &[&loopback, &microphone],
        );

        let message = result.unwrap_err().to_string();
        assert!(message.contains("recording.wav"), "{message}");
        assert!(message.contains("missing_merged.wav"), "{message}");
        assert!(loopback.exists());
        assert!(microphone.exists());
        assert!(!official.exists());

        std::fs::remove_dir_all(test_dir).unwrap();
    }
}
