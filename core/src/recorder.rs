use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Audio quality constants
const PROFESSIONAL_SAMPLE_RATE: u32 = 48000;
const PROFESSIONAL_CHANNELS: u16 = 2;

// File size estimation constants
// Professional quality: 48kHz stereo 16-bit = 48000 * 2 channels * 2 bytes = 192,000 bytes/sec
const PROFESSIONAL_BYTES_PER_SECOND: u64 = 192_000;
const DEFAULT_DURATION_FALLBACK_MS: u64 = 300_000; // 5 minutes
const DURATION_ESTIMATE_BUFFER_FACTOR: f64 = 1.2; // 20% buffer to prevent premature 100%

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingQuality {
    pub name: String,
    pub description: String,
    pub size_per_min: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl RecordingQuality {
    #[must_use]
    pub fn professional() -> Self {
        Self {
            name: "Professional (48kHz Stereo)".to_string(),
            description: "Professional quality for meetings".to_string(),
            size_per_min: "11 MB/min".to_string(),
            sample_rate: PROFESSIONAL_SAMPLE_RATE,
            channels: PROFESSIONAL_CHANNELS,
        }
    }

    #[must_use]
    pub fn quick() -> Self {
        Self {
            name: "Quick (16kHz Mono)".to_string(),
            description: "Smaller files, good for voice notes".to_string(),
            size_per_min: "2 MB/min".to_string(),
            sample_rate: 16000,
            channels: 1,
        }
    }

    #[must_use]
    pub fn standard() -> Self {
        Self {
            name: "Standard (44.1kHz Stereo)".to_string(),
            description: "CD quality, balanced file size".to_string(),
            size_per_min: "10 MB/min".to_string(),
            sample_rate: 44100,
            channels: 2,
        }
    }

    #[must_use]
    pub fn high() -> Self {
        Self {
            name: "High (96kHz Stereo)".to_string(),
            description: "Maximum quality, larger files".to_string(),
            size_per_min: "22 MB/min".to_string(),
            sample_rate: 96000,
            channels: 2,
        }
    }
}

/// Convert WAV file to M4A using FFmpeg with optimized AAC encoding
pub async fn convert_wav_to_m4a(wav_path: &PathBuf, m4a_path: &PathBuf) -> Result<()> {
    crate::ffmpeg_encoder::convert_wav_to_m4a_optimized(wav_path, m4a_path).await
}

/// Merge two audio streams (loopback and microphone) into a single stereo file
/// Uses FFmpeg to handle sample rate mismatches and audio synchronization
/// Output format: Dual-mono stereo (Left=system audio, Right=microphone)
/// Supports direct M4A encoding (merge + encode in one pass for 50-70% faster processing)
///
/// `start_offset_ms` compensates for the time difference between when each recorder's
/// `audio_client.Start()` was called. Positive = loopback started first (common case),
/// so we delay the loopback with `-itsoffset`. Negative = mic started first (rare).
/// Zero = no compensation needed.
pub async fn merge_audio_streams_smart(
    loopback_wav: &PathBuf,
    mic_wav: &PathBuf,
    output_path: &PathBuf,
    loopback_has_audio: bool,
    mic_has_audio: bool,
    quality: &RecordingQuality,
    output_format: crate::domain::AudioFormat,
    session_id: Option<&str>,
    observer: Option<std::sync::Arc<crate::status::JsonFileObserver>>,
    total_steps: u8,
    start_offset_ms: i64,
) -> Result<()> {
    use std::process::Stdio;
    use std::time::Instant;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    let format_str = match output_format {
        crate::domain::AudioFormat::Wav => "WAV",
        crate::domain::AudioFormat::M4a => "M4A (AAC)",
    };

    tracing::info!("═══════════════════════════════════════════════════════════");
    tracing::info!("🎧 AUDIO MERGE PROCESS STARTED");
    tracing::info!("═══════════════════════════════════════════════════════════");
    tracing::info!("📊 Merge Configuration:");
    tracing::info!(
        "    • Loopback (System Audio): {}",
        if loopback_has_audio { "✓ Present" } else { "✗ Silent" }
    );
    tracing::info!(
        "    • Microphone (User Audio):  {}",
        if mic_has_audio { "✓ Present" } else { "✗ Silent" }
    );
    tracing::info!("    • Output Format:          {}", format_str);
    tracing::info!("    • Output Sample Rate:     {} Hz", quality.sample_rate);
    tracing::info!("    • Output Channels:        {} (Stereo)", quality.channels);
    if start_offset_ms != 0 {
        tracing::info!("    • Start offset:           {} ms ({})",
            start_offset_ms.abs(),
            if start_offset_ms > 0 { "loopback started first, delaying loopback" }
            else { "mic started first, delaying mic" }
        );
    }

    // Check if FFmpeg is available (cached on first run)
    use std::sync::OnceLock;
    static FFMPEG_AVAILABLE: OnceLock<bool> = OnceLock::new();

    let ffmpeg_available = FFMPEG_AVAILABLE.get_or_init(|| {
        // Perform synchronous FFmpeg availability check
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.arg("-version");

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: Approved for FFmpeg to prevent console window flashing on Windows
            cmd.creation_flags(0x08000000);
        }

        let check_result = cmd.output();
        check_result.is_ok()
    });

    if !ffmpeg_available {
        anyhow::bail!("FFmpeg is not installed or not in PATH. Please install FFmpeg for dual-channel recording.");
    }

    let target_sample_rate = quality.sample_rate.to_string();

    // Helper function to create FFmpeg command with hidden console on Windows
    #[cfg(windows)]
    fn setup_ffmpeg_command() -> Command {
        let mut cmd = Command::new("ffmpeg");
        // CREATE_NO_WINDOW: Approved for FFmpeg to prevent console window flashing on Windows
        cmd.creation_flags(0x08000000);
        cmd
    }

    #[cfg(not(windows))]
    fn setup_ffmpeg_command() -> Command {
        Command::new("ffmpeg")
    }

    // Helper function to add encoding parameters to FFmpeg command
    fn add_encoding_params(
        cmd: &mut Command,
        output_format: crate::domain::AudioFormat,
    ) {
        match output_format {
            crate::domain::AudioFormat::M4a => {
                // Optimized software AAC encoding (20-50x real-time performance)
                cmd.arg("-c:a").arg("aac");
                cmd.arg("-b:a").arg("192k"); // Explicit bitrate for consistent quality
                cmd.arg("-movflags").arg("faststart"); // Streaming-friendly
                cmd.arg("-threads").arg("auto"); // Use all CPU cores for encoding
            }
            crate::domain::AudioFormat::Wav => {
                // WAV output - no encoding needed, PCM passthrough
            }
        }
    }

    // Validate input files before merge — empty/corrupt WAV files cause FFmpeg to fail
    // with unhelpful errors. Log file sizes for diagnostics.
    let loopback_file_size = std::fs::metadata(loopback_wav).map(|m| m.len()).unwrap_or(0);
    let mic_file_size = std::fs::metadata(mic_wav).map(|m| m.len()).unwrap_or(0);

    tracing::info!("📁 Input file sizes:");
    tracing::info!("    • Loopback WAV: {} bytes ({:.2} KB)", loopback_file_size, loopback_file_size as f64 / 1024.0);
    tracing::info!("    • Microphone WAV: {} bytes ({:.2} KB)", mic_file_size, mic_file_size as f64 / 1024.0);

    // A valid WAV file with any audio data should be at least ~100 bytes (44-byte header + data).
    // Files under 100 bytes are effectively empty (header only or corrupt).
    const MIN_VALID_WAV_SIZE: u64 = 100;

    let loopback_file_valid = loopback_file_size >= MIN_VALID_WAV_SIZE;
    let mic_file_valid = mic_file_size >= MIN_VALID_WAV_SIZE;

    if !loopback_file_valid {
        tracing::warn!("⚠ Loopback WAV file is too small ({} bytes) — likely empty or corrupt", loopback_file_size);
    }
    if !mic_file_valid {
        tracing::warn!("⚠ Microphone WAV file is too small ({} bytes) — likely empty or corrupt", mic_file_size);
    }

    // Determine merge strategy based on audio detection flags
    tracing::info!("───────────────────────────────────────────────────────────");
    tracing::info!("⏳ Starting merge operation...");

    let start_time = Instant::now();

    // Emit interim status: preparing merge
    if let (Some(session_id), Some(observer)) = (session_id, observer.as_ref()) {
        tracing::info!("📝 Stage 2/{}: Preparing merge (detecting durations)...", total_steps);
        let _ = observer.write_processing_status_v2(
            session_id,
            "Preparing merge operation...",
            Some(2),
            Some(total_steps),
            Some("merging"),
            None,
            None,
        );
    }

    // Get audio duration from BOTH input files and use the maximum
    // On Windows, we merge two files (loopback + mic) which may have different durations
    // Run duration detection in parallel to reduce setup overhead (50% faster)
    let (loopback_duration_ms, mic_duration_ms) = tokio::join!(
        async { crate::ffmpeg_encoder::get_audio_duration_ms(loopback_wav).await.unwrap_or(0) },
        async { crate::ffmpeg_encoder::get_audio_duration_ms(mic_wav).await.unwrap_or(0) }
    );

    // Use the longer duration to ensure we don't show 100% prematurely
    let audio_duration_ms = std::cmp::max(loopback_duration_ms, mic_duration_ms);

    tracing::info!("📊 Duration detection:");
    tracing::info!("  ├─ Loopback: {} ms", loopback_duration_ms);
    tracing::info!("  ├─ Microphone: {} ms", mic_duration_ms);
    tracing::info!("  └─ Using maximum: {} ms", audio_duration_ms);

    // ALWAYS enable progress monitoring when we have a session and observer
    let enable_progress = session_id.is_some() && observer.is_some();

    // If duration detection failed for both, estimate from the larger file
    // Professional quality: 48kHz stereo 16-bit = 192,000 bytes/second
    let effective_duration_ms = if audio_duration_ms > 0 {
        audio_duration_ms
    } else {
        tracing::warn!("⚠ Duration detection failed for both files, estimating from larger file size");
        let loopback_size = std::fs::metadata(loopback_wav).map(|m| m.len()).unwrap_or(0);
        let mic_size = std::fs::metadata(mic_wav).map(|m| m.len()).unwrap_or(0);
        let larger_size = std::cmp::max(loopback_size, mic_size);

        if larger_size > 0 {
            let estimated_secs = larger_size / PROFESSIONAL_BYTES_PER_SECOND;
            let estimated_ms = estimated_secs * 1000;
            // Be conservative: add buffer to prevent premature 100%
            let buffered_ms = (estimated_ms as f64 * DURATION_ESTIMATE_BUFFER_FACTOR) as u64;
            tracing::warn!("  ├─ Loopback file: {} bytes", loopback_size);
            tracing::warn!("  ├─ Microphone file: {} bytes", mic_size);
            tracing::warn!("  ├─ Estimated duration: {} ms", estimated_ms);
            tracing::warn!("  └─ Buffered estimate ({}% extra): {} ms",
                (DURATION_ESTIMATE_BUFFER_FACTOR - 1.0) * 100.0, buffered_ms);
            buffered_ms
        } else {
            tracing::warn!("  └─ Files unavailable, using default {} seconds",
                DEFAULT_DURATION_FALLBACK_MS / 1000);
            DEFAULT_DURATION_FALLBACK_MS
        }
    };

    tracing::info!("Duration detection result: {} ms", audio_duration_ms);
    tracing::info!("Progress monitoring: {} (using effective duration: {} ms)",
        if enable_progress { "ENABLED" } else { "DISABLED" },
        effective_duration_ms);

    // Clone observer for the closure to own
    let observer_for_closure = observer.clone();

    // Helper closure to execute FFmpeg with or without progress monitoring
    let execute_ffmpeg = |mut cmd: Command| async move {
        if enable_progress {
            let session_id = session_id.unwrap();
            let observer = observer_for_closure.as_ref().unwrap();

            // Add progress flag
            cmd.arg("-progress").arg("pipe:2");
            cmd.stderr(Stdio::piped());

            let mut child = cmd.spawn()?;
            let stderr = child.stderr.take().unwrap();
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();

            // Parse progress in real-time
            let mut current_speed = None;
            let mut current_speed_float: f64 = 0.0;
            let mut encoding_stage_emitted = false;
            let encoding_start = std::time::Instant::now();

            // Track checkpoints (reset per recording)
            let mut checkpoint_25_logged = false;
            let mut checkpoint_50_logged = false;
            let mut checkpoint_75_logged = false;
            let mut ffmpeg_diagnostics = Vec::new();

            // Log estimated processing time at the start
            // Typical processing speed is 4-6x real-time for M4A encoding
            let estimated_processing_secs = effective_duration_ms / 1000 / 5; // Conservative 5x estimate
            tracing::info!("⏱️  Estimated processing time: ~{} minutes (based on {}s audio at ~5x speed)",
                estimated_processing_secs / 60,
                effective_duration_ms / 1000);
            tracing::info!("    Audio duration: {}ms, Progress monitoring: ACTIVE", effective_duration_ms);

            while let Ok(Some(line)) = lines.next_line().await {
                let is_progress_line = line
                    .split_once('=')
                    .map(|(key, _)| {
                        matches!(
                            key,
                            "bitrate"
                                | "total_size"
                                | "out_time_us"
                                | "out_time_ms"
                                | "out_time"
                                | "dup_frames"
                                | "drop_frames"
                                | "speed"
                                | "progress"
                        )
                    })
                    .unwrap_or(false);
                if !is_progress_line {
                    ffmpeg_diagnostics.push(line.clone());
                }

                if line.starts_with("out_time_ms=") {
                    if let Ok(time_us) = line.split('=').nth(1).unwrap_or("0").parse::<u64>() {
                        // IMPORTANT: FFmpeg's out_time_ms is actually in MICROSECONDS despite the name
                        // Convert to milliseconds for all calculations
                        let time_ms = time_us / 1000;

                        // Use effective_duration_ms (never 0) for reliable progress calculation
                        let progress_pct = ((time_ms as f64 / effective_duration_ms as f64) * 100.0).min(100.0) as u8;

                        // Calculate estimated remaining time based on current processing speed
                        let remaining_audio_ms = effective_duration_ms.saturating_sub(time_ms);
                        let estimated_remaining_secs = if current_speed_float > 0.0 {
                            // Use actual measured speed
                            (remaining_audio_ms as f64 / 1000.0 / current_speed_float) as u64
                        } else {
                            // Fallback to elapsed-based estimate
                            let elapsed = encoding_start.elapsed().as_secs_f64();
                            if elapsed > 0.0 && time_ms > 0 {
                                let actual_speed = time_ms as f64 / 1000.0 / elapsed;
                                (remaining_audio_ms as f64 / 1000.0 / actual_speed) as u64
                            } else {
                                remaining_audio_ms / 1000 / 5 // Conservative fallback
                            }
                        };

                        // For M4A format, emit encoding stage when we're past merging (at 30% progress)
                        if matches!(output_format, crate::domain::AudioFormat::M4a)
                            && !encoding_stage_emitted
                            && progress_pct >= 30 {
                            tracing::info!("📝 Stage 3/{}: Encoding to M4A", total_steps);
                            tracing::info!("    └─ Estimated remaining: ~{} min {} sec",
                                estimated_remaining_secs / 60,
                                estimated_remaining_secs % 60);
                            let _ = observer.write_processing_status_v2(
                                session_id,
                                "Converting to M4A format...",
                                Some(3),
                                Some(total_steps),
                                Some("encoding"),
                                None,
                                None,
                            );
                            encoding_stage_emitted = true;
                        }

                        let _ = observer.update_ffmpeg_progress(
                            session_id,
                            progress_pct,
                            current_speed.clone(),
                            Some(effective_duration_ms),
                            Some(time_ms),
                            Some(estimated_remaining_secs),
                        );

                        // Log progress checkpoints at approximately 25%, 50%, 75%
                        // Use ranges to avoid missing checkpoints due to progress jumps
                        let elapsed = encoding_start.elapsed();
                        if progress_pct >= 25 && progress_pct < 30 && !checkpoint_25_logged {
                            checkpoint_25_logged = true;
                            tracing::info!("    📊 Progress checkpoint: {}% | Elapsed: {:.0}s | ETA: ~{} min {} sec",
                                progress_pct, elapsed.as_secs_f64(), estimated_remaining_secs / 60, estimated_remaining_secs % 60);
                        } else if progress_pct >= 50 && progress_pct < 55 && !checkpoint_50_logged {
                            checkpoint_50_logged = true;
                            tracing::info!("    📊 Progress checkpoint: {}% | Elapsed: {:.0}s | ETA: ~{} min {} sec",
                                progress_pct, elapsed.as_secs_f64(), estimated_remaining_secs / 60, estimated_remaining_secs % 60);
                        } else if progress_pct >= 75 && progress_pct < 80 && !checkpoint_75_logged {
                            checkpoint_75_logged = true;
                            tracing::info!("    📊 Progress checkpoint: {}% | Elapsed: {:.0}s | ETA: ~{} min {} sec",
                                progress_pct, elapsed.as_secs_f64(), estimated_remaining_secs / 60, estimated_remaining_secs % 60);
                        }

                        tracing::debug!(
                            "FFmpeg merge progress: {}% ({}/{} ms, speed: {:?}, ETA: {}s)",
                            progress_pct,
                            time_ms,
                            effective_duration_ms,
                            current_speed,
                            estimated_remaining_secs
                        );
                    }
                } else if line.starts_with("speed=") {
                    current_speed = line.split('=').nth(1).map(|s| s.to_string());
                    // Parse speed as float for ETA calculation (e.g., "4.75x" -> 4.75)
                    if let Some(speed_str) = current_speed.as_ref() {
                        current_speed_float = speed_str.trim_end_matches('x').parse().unwrap_or(0.0);
                    }
                }
            }

            let mut output = child.wait_with_output().await?;
            output.stderr = ffmpeg_diagnostics.join("\n").into_bytes();
            Ok(output)
        } else {
            cmd.output().await
        }
    };

    // Calculate max duration in seconds for use in filters (to pad shorter input)
    let max_duration_secs = (effective_duration_ms as f64 / 1000.0).ceil() as u64;

    let output = if loopback_has_audio && mic_has_audio {
        // Scenario A: Both have audio - Create dual-mono stereo (L=loopback, R=mic)
        // Use apad with whole_dur to pad the shorter input to match the longer one,
        // preventing amerge from truncating to the shortest input.
        tracing::info!("📋 Merge Strategy: Dual-mono stereo (L=loopback, R=microphone)");

        // Pre-merge LUFS measurement and mic gain compensation
        // Measure loudness of both streams in parallel to determine if mic needs boosting
        let (loopback_lufs_result, mic_lufs_result) = tokio::join!(
            crate::ffmpeg_encoder::measure_lufs(loopback_wav),
            crate::ffmpeg_encoder::measure_lufs(mic_wav)
        );

        let mic_gain_db = match (&loopback_lufs_result, &mic_lufs_result) {
            (Ok(loopback_lufs), Ok(mic_lufs)) => {
                tracing::info!("🔊 LUFS Measurement:");
                tracing::info!("    • Loopback (system): {:.1} LUFS", loopback_lufs);
                tracing::info!("    • Microphone (user): {:.1} LUFS", mic_lufs);

                let gain = crate::ffmpeg_encoder::calculate_mic_gain_db(*loopback_lufs, *mic_lufs);

                if gain != 0.0 {
                    tracing::info!("    • Mic gain compensation: {:.1} dB", gain);
                } else {
                    tracing::info!("    • Mic gain compensation: skipped (streams already balanced)");
                }

                gain
            }
            _ => {
                // Graceful fallback: if LUFS measurement fails, proceed without gain compensation
                if let Err(e) = &loopback_lufs_result {
                    tracing::warn!("⚠ Failed to measure loopback LUFS: {}. Proceeding without gain compensation.", e);
                }
                if let Err(e) = &mic_lufs_result {
                    tracing::warn!("⚠ Failed to measure mic LUFS: {}. Proceeding without gain compensation.", e);
                }
                0.0
            }
        };

        // Build mic input filter chain — optionally prepend volume adjustment
        let mic_filter = if mic_gain_db != 0.0 {
            format!(
                "[1:a]volume={:.1}dB,aformat=channel_layouts=mono,apad=whole_dur={},asplit=2[ml][mr]",
                mic_gain_db, max_duration_secs
            )
        } else {
            format!(
                "[1:a]aformat=channel_layouts=mono,apad=whole_dur={},asplit=2[ml][mr]",
                max_duration_secs
            )
        };

        let filter = format!(
            "[0:a]aformat=channel_layouts=stereo,apad=whole_dur={}[left];{};[left][ml][mr]amerge=inputs=3,pan=stereo|c0<c0+c2|c1<c1+c2[aout]",
            max_duration_secs, mic_filter
        );

        // Convert offset to seconds for FFmpeg -itsoffset flag.
        // Positive offset = loopback started first, so delay loopback to align with mic.
        // Negative offset = mic started first, so delay mic to align with loopback.
        let offset_secs = start_offset_ms.abs() as f64 / 1000.0;
        let offset_str = format!("{:.3}", offset_secs);

        let mut cmd = setup_ffmpeg_command();
        cmd.arg("-hide_banner")
            .arg("-loglevel").arg("error");

        if start_offset_ms > 0 {
            // Loopback started first: delay loopback input
            cmd.arg("-itsoffset").arg(&offset_str);
        }
        cmd.arg("-i").arg(loopback_wav);

        if start_offset_ms < 0 {
            // Mic started first (rare): delay mic input
            cmd.arg("-itsoffset").arg(&offset_str);
        }
        cmd.arg("-i").arg(mic_wav);

        cmd.arg("-filter_complex")
            .arg(&filter)
            .arg("-filter_threads").arg("auto")  // Enable parallel filter processing
            .arg("-map").arg("[aout]")
            .arg("-ar").arg(&target_sample_rate);

        add_encoding_params(&mut cmd, output_format);

        cmd.arg("-y")
            .arg(output_path);

        // Update status before FFmpeg starts
        if let (Some(session_id), Some(observer)) = (session_id, observer.as_ref()) {
            tracing::info!("🔀 Starting FFmpeg merge and encode...");
            let _ = observer.write_processing_status_v2(
                session_id,
                "Combining audio streams...",
                Some(2),
                Some(total_steps),
                Some("merging"),
                None,
                None,
            );
        }

        let result = execute_ffmpeg(cmd).await?;

        result
    } else if loopback_has_audio && !mic_has_audio {
        // Scenario B: Loopback only - Convert to stereo (duplicate to both channels)
        tracing::info!("📋 Merge Strategy: Using loopback only (duplicate system audio to stereo)");

        let mut cmd = setup_ffmpeg_command();
        cmd.arg("-hide_banner")
            .arg("-loglevel").arg("error")
            .arg("-i")
            .arg(loopback_wav)
            .arg("-ac").arg("2")  // Direct channel conversion to stereo (faster than filter_complex)
            .arg("-ar")
            .arg(&target_sample_rate);

        add_encoding_params(&mut cmd, output_format);

        cmd.arg("-y")
            .arg(output_path);

        let result = execute_ffmpeg(cmd).await?;

        result
    } else if !loopback_has_audio && mic_has_audio {
        // Scenario C: Mic only - Convert mono to stereo (duplicate to both channels)
        tracing::info!("📋 Merge Strategy: Using microphone only (duplicate user audio to stereo)");

        let mut cmd = setup_ffmpeg_command();
        cmd.arg("-hide_banner")
            .arg("-loglevel").arg("error")
            .arg("-i").arg(mic_wav)
            .arg("-filter_complex")
            .arg("[0:a]aformat=channel_layouts=mono,asplit=2[l][r];[l][r]amerge=inputs=2,pan=stereo|c0=c0|c1=c1[aout]")
            .arg("-filter_threads").arg("auto")  // Enable parallel filter processing
            .arg("-map").arg("[aout]")
            .arg("-ar").arg(&target_sample_rate);

        add_encoding_params(&mut cmd, output_format);

        cmd.arg("-y")
            .arg(output_path);

        let result = execute_ffmpeg(cmd).await?;

        result
    } else {
        // Scenario D: Neither has audio — create a silent output file.
        //
        // If the loopback WAV is valid, use it as input (preserves original duration).
        // If it's empty/corrupt (common with process loopback when the target process
        // doesn't produce audio), generate silence using FFmpeg's anullsrc filter
        // to avoid "Invalid data found when processing input" errors.
        tracing::info!("📋 Merge Strategy: Both channels silent (creating silent stereo file)");

        let mut cmd = setup_ffmpeg_command();
        cmd.arg("-hide_banner")
            .arg("-loglevel").arg("error");

        if loopback_file_valid {
            // Loopback WAV is valid — use it as input
            tracing::info!("    Using loopback WAV as silent stereo source");
            cmd.arg("-i")
                .arg(loopback_wav)
                .arg("-filter_complex")
                .arg("[0:a]aformat=channel_layouts=stereo[aout]")
                .arg("-map")
                .arg("[aout]");
        } else if mic_file_valid {
            // Loopback WAV is invalid but mic WAV is valid — use mic as source
            tracing::info!("    Loopback WAV invalid, using microphone WAV as silent stereo source");
            cmd.arg("-i")
                .arg(mic_wav)
                .arg("-filter_complex")
                .arg("[0:a]aformat=channel_layouts=stereo[aout]")
                .arg("-map")
                .arg("[aout]");
        } else {
            // Both files are invalid — generate silence with anullsrc.
            // Use the effective duration (from recording time) or fall back to 1 second.
            let silence_duration_secs = if effective_duration_ms > 0 {
                effective_duration_ms as f64 / 1000.0
            } else {
                1.0
            };
            tracing::warn!(
                "    Both WAV files are invalid/empty. Generating {:.1}s of silence with anullsrc.",
                silence_duration_secs
            );
            cmd.arg("-f").arg("lavfi")
                .arg("-i")
                .arg(format!(
                    "anullsrc=channel_layout=stereo:sample_rate={}",
                    &target_sample_rate
                ))
                .arg("-t")
                .arg(format!("{:.3}", silence_duration_secs));
        }

        cmd.arg("-ar")
            .arg(&target_sample_rate);

        add_encoding_params(&mut cmd, output_format);

        cmd.arg("-y")
            .arg(output_path);

        let result = execute_ffmpeg(cmd).await?;

        result
    };

    let elapsed = start_time.elapsed();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // When progress monitoring is enabled, stderr is consumed by the progress parser,
        // so the error message may be empty. Include file diagnostics to help debug.
        let stderr_msg = if stderr.trim().is_empty() {
            format!(
                "(stderr consumed by progress parser — check logs above for details). \
                 Input file sizes: loopback={} bytes, mic={} bytes. \
                 Audio detected: loopback={}, mic={}. \
                 Exit code: {:?}",
                loopback_file_size, mic_file_size,
                loopback_has_audio, mic_has_audio,
                output.status.code()
            )
        } else {
            stderr.to_string()
        };
        anyhow::bail!("FFmpeg merge failed: {}", stderr_msg);
    }

    // Mark FFmpeg merge/encoding as complete to ensure UI transitions properly
    if let Some(session_id) = session_id {
        if let Some(observer) = &observer {
            // Emit finalizing stage
            let final_step = if matches!(output_format, crate::domain::AudioFormat::M4a) {
                4
            } else {
                3
            };
            tracing::info!("📝 Stage {}/{}: Finalizing", final_step, total_steps);
            let _ = observer.write_processing_status_v2(
                session_id,
                "Saving recording...",
                Some(final_step),
                Some(total_steps),
                Some("finalizing"),
                None,
                None,
            );

            // Small delay to show finalization stage
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

            // Mark as complete
            let _ = observer.mark_ffmpeg_complete(session_id);
        }
    }

    // Log FFmpeg output for debugging
    let ffmpeg_stdout = String::from_utf8_lossy(&output.stdout);
    let ffmpeg_stderr = String::from_utf8_lossy(&output.stderr);
    if !ffmpeg_stdout.is_empty() {
        tracing::debug!("FFmpeg stdout: {}", ffmpeg_stdout);
    }
    if !ffmpeg_stderr.is_empty() {
        tracing::debug!("FFmpeg stderr: {}", ffmpeg_stderr);
    }

    // Get output file information
    let output_metadata = std::fs::metadata(output_path)?;
    let output_size_mb = output_metadata.len() as f64 / (1024.0 * 1024.0);

    tracing::info!("📊 Merge Results:");
    tracing::info!("    • Output file:     {:?}", output_path.file_name().unwrap_or_default());
    tracing::info!("    • Output format:   {}", format_str);
    tracing::info!("    • Output size:     {:.2} MB", output_size_mb);
    tracing::info!("    • Time elapsed:    {:.2}s", elapsed.as_secs_f64());
    tracing::info!("    • Sample rate:     {} Hz", quality.sample_rate);

    if matches!(output_format, crate::domain::AudioFormat::M4a) {
        tracing::info!("    • Encoder:         AAC (software, multi-threaded)");
        tracing::info!("    • Bitrate:         192 kbps");
    }

    tracing::info!("═══════════════════════════════════════════════════════════");
    Ok(())
}
