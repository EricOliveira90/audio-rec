use serde::{Deserialize, Serialize};

use crate::config::RecorderConfig;
use crate::error::Result;

#[cfg(not(windows))]
use serde_json::json;

/// Represents the enhanced status output for an active recording session
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusOutput {
    #[serde(rename = "type")]
    pub event_type: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_mic_muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loopback_has_audio: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_has_audio: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loopback_frames: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_frames: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ffmpeg_progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_speed: Option<String>,
    // Process-specific capture fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_window: Option<String>,
}

impl StatusOutput {
    /// Create a "no active session" status output
    pub fn no_active_session() -> Self {
        Self {
            event_type: "status".to_string(),
            active: false,
            message: Some("No active recording session found".to_string()),
            session_id: None,
            filename: None,
            elapsed: None,
            duration: None,
            format: None,
            quality: None,
            is_mic_muted: None,
            device: None,
            sample_rate: None,
            loopback_has_audio: None,
            mic_has_audio: None,
            loopback_frames: None,
            mic_frames: None,
            processing_stage: None,
            ffmpeg_progress: None,
            processing_speed: None,
            capture_mode: None,
            app: None,
            app_window: None,
        }
    }
}

/// Format elapsed seconds into a human-readable string (e.g., "2m 34s")
pub fn format_elapsed(seconds: u64) -> String {
    if seconds == 0 {
        return "0s".to_string();
    }

    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 {
        parts.push(format!("{}m", minutes));
    }
    if secs > 0 || parts.is_empty() {
        parts.push(format!("{}s", secs));
    }

    parts.join(" ")
}

/// Format recording duration setting into a human-readable string
pub fn format_duration_setting(duration_secs: u64) -> String {
    if duration_secs == 0 {
        "indefinite".to_string()
    } else {
        format!("{}s", duration_secs)
    }
}

/// Parse a status JSON file and determine if it represents an active session
/// Returns None if the session is completed, cancelled, or the file can't be parsed
pub fn parse_status_file(content: &str) -> Option<StatusOutput> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;

    let status = value.get("status")?.as_str()?;

    // Only show recording or processing sessions as active
    match status {
        "recording" | "processing" | "compressing" => {}
        _ => return None, // completed, cancelled, etc.
    }

    let session_id = value.get("session_id")?.as_str()?.to_string();
    let filename = value
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let elapsed_secs = value.get("elapsed").and_then(|v| v.as_u64()).unwrap_or(0);
    let duration_secs = value.get("duration").and_then(|v| v.as_u64()).unwrap_or(0);

    let quality = value
        .get("quality")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let device = value
        .get("device")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let sample_rate = value.get("sample_rate").and_then(|v| v.as_u64()).map(|v| v as u32);
    let is_mic_muted = value.get("is_mic_muted").and_then(|v| v.as_bool());
    let loopback_has_audio = value.get("loopback_has_audio").and_then(|v| v.as_bool());
    let mic_has_audio = value.get("mic_has_audio").and_then(|v| v.as_bool());
    let loopback_frames = value.get("loopback_frames").and_then(|v| v.as_u64());
    let mic_frames = value.get("mic_frames").and_then(|v| v.as_u64());

    // Processing-specific fields
    let processing_stage = value
        .get("stage_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let ffmpeg_progress = value
        .get("ffmpeg_progress")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8);
    let processing_speed = value
        .get("processing_speed")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Determine format from filename extension
    let format = if filename.ends_with(".m4a") {
        Some("m4a".to_string())
    } else if filename.ends_with(".wav") {
        Some("wav".to_string())
    } else {
        None
    };

    // Process-specific capture fields
    let capture_mode = value
        .get("capture_mode")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let app = value
        .get("app")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let app_window = value
        .get("app_window")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(StatusOutput {
        event_type: "status".to_string(),
        active: true,
        message: None,
        session_id: Some(session_id),
        filename: Some(filename),
        elapsed: Some(format_elapsed(elapsed_secs)),
        duration: Some(format_duration_setting(duration_secs)),
        format,
        quality,
        is_mic_muted,
        device,
        sample_rate,
        loopback_has_audio,
        mic_has_audio,
        loopback_frames,
        mic_frames,
        processing_stage,
        ffmpeg_progress,
        processing_speed,
        capture_mode,
        app,
        app_window,
    })
}

/// Find the most recent active session status file in the status directory
pub fn find_active_session(status_dir: &std::path::Path) -> Option<StatusOutput> {
    let entries = std::fs::read_dir(status_dir).ok()?;

    let mut most_recent: Option<(std::time::SystemTime, StatusOutput)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let modified = entry.metadata().ok()?.modified().ok()?;
        let content = std::fs::read_to_string(&path).ok()?;

        if let Some(status) = parse_status_file(&content) {
            match &most_recent {
                Some((prev_time, _)) if modified > *prev_time => {
                    most_recent = Some((modified, status));
                }
                None => {
                    most_recent = Some((modified, status));
                }
                _ => {}
            }
        }
    }

    most_recent.map(|(_, status)| status)
}

/// Execute the status command
pub async fn execute() -> Result<()> {
    let config = RecorderConfig::new();
    execute_with_config(config).await
}

/// Execute the status command with a specific config (for testing)
pub async fn execute_with_config(config: RecorderConfig) -> Result<()> {
    #[cfg(windows)]
    {
        let status = find_active_session(&config.status_dir)
            .unwrap_or_else(StatusOutput::no_active_session);

        let json_str = serde_json::to_string(&status)?;
        println!("{}", json_str);
    }

    #[cfg(not(windows))]
    {
        let result = json!({
            "type": "error",
            "message": "Recording is currently only supported on Windows. Cross-platform support is planned for a future release."
        });
        println!("{}", serde_json::to_string(&result)?);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Elapsed time formatting ──

    #[test]
    fn test_format_elapsed_zero_seconds() {
        assert_eq!(format_elapsed(0), "0s");
    }

    #[test]
    fn test_format_elapsed_seconds_only() {
        assert_eq!(format_elapsed(45), "45s");
    }

    #[test]
    fn test_format_elapsed_minutes_and_seconds() {
        assert_eq!(format_elapsed(154), "2m 34s");
    }

    #[test]
    fn test_format_elapsed_exact_minutes() {
        assert_eq!(format_elapsed(120), "2m");
    }

    #[test]
    fn test_format_elapsed_hours_minutes_seconds() {
        assert_eq!(format_elapsed(3661), "1h 1m 1s");
    }

    #[test]
    fn test_format_elapsed_hours_only() {
        assert_eq!(format_elapsed(7200), "2h");
    }

    // ── Duration setting formatting ──

    #[test]
    fn test_format_duration_setting_indefinite() {
        assert_eq!(format_duration_setting(0), "indefinite");
    }

    #[test]
    fn test_format_duration_setting_fixed() {
        assert_eq!(format_duration_setting(60), "60s");
    }

    // ── Status file parsing ──

    #[test]
    fn test_parse_recording_status_file() {
        let content = r#"{
            "session_id": "rec-20260329_120000",
            "filename": "2026-03-29-12_00_recording.wav",
            "duration": 60,
            "elapsed": 34,
            "progress": 56,
            "quality": "Professional (48kHz Stereo)",
            "device": "Dual-Channel (System + Microphone)",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 1500,
            "has_audio": true,
            "status": "recording",
            "loopback_frames": 1500,
            "loopback_has_audio": true,
            "mic_frames": 1200,
            "mic_has_audio": true,
            "is_mic_muted": false,
            "ffmpeg_progress": null,
            "processing_speed": null
        }"#;

        let status = parse_status_file(content).unwrap();

        assert!(status.active);
        assert_eq!(status.event_type, "status");
        assert_eq!(status.session_id.unwrap(), "rec-20260329_120000");
        assert_eq!(status.filename.unwrap(), "2026-03-29-12_00_recording.wav");
        assert_eq!(status.elapsed.unwrap(), "34s");
        assert_eq!(status.duration.unwrap(), "60s");
        assert_eq!(status.quality.unwrap(), "Professional (48kHz Stereo)");
        assert_eq!(status.device.unwrap(), "Dual-Channel (System + Microphone)");
        assert_eq!(status.sample_rate.unwrap(), 48000);
        assert_eq!(status.is_mic_muted.unwrap(), false);
        assert_eq!(status.loopback_has_audio.unwrap(), true);
        assert_eq!(status.mic_has_audio.unwrap(), true);
        assert_eq!(status.loopback_frames.unwrap(), 1500);
        assert_eq!(status.mic_frames.unwrap(), 1200);
        assert_eq!(status.format.unwrap(), "wav");
    }

    #[test]
    fn test_parse_processing_status_file() {
        let content = r#"{
            "session_id": "rec-20260329_120000",
            "status": "processing",
            "message": "Converting to M4A format...",
            "current_stage": 3,
            "total_stages": 5,
            "stage_name": "encoding",
            "ffmpeg_progress": 45,
            "processing_speed": "4.5x"
        }"#;

        let status = parse_status_file(content).unwrap();

        assert!(status.active);
        assert_eq!(status.session_id.unwrap(), "rec-20260329_120000");
        assert_eq!(status.processing_stage.unwrap(), "encoding");
        assert_eq!(status.ffmpeg_progress.unwrap(), 45);
        assert_eq!(status.processing_speed.unwrap(), "4.5x");
    }

    #[test]
    fn test_parse_completed_status_returns_none() {
        let content = r#"{
            "session_id": "rec-20260329_120000",
            "filename": "recording.wav",
            "file_path": "C:\\recordings\\recording.wav",
            "duration": 60,
            "file_size_mb": "2.34",
            "format": "wav",
            "codec": "pcm",
            "status": "completed",
            "message": "Recording completed successfully"
        }"#;

        let result = parse_status_file(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_cancelled_status_returns_none() {
        let content = r#"{
            "session_id": "rec-20260329_120000",
            "filename": "",
            "file_path": null,
            "duration": 0,
            "file_size_mb": "0 MB",
            "format": "wav",
            "codec": "",
            "status": "cancelled",
            "message": "Recording cancelled by user"
        }"#;

        let result = parse_status_file(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_invalid_json_returns_none() {
        let result = parse_status_file("not valid json");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_missing_status_field_returns_none() {
        let content = r#"{"session_id": "test"}"#;
        let result = parse_status_file(content);
        assert!(result.is_none());
    }

    // ── No active session output ──

    #[test]
    fn test_no_active_session_output() {
        let status = StatusOutput::no_active_session();

        assert_eq!(status.event_type, "status");
        assert!(!status.active);
        assert_eq!(
            status.message.unwrap(),
            "No active recording session found"
        );
        assert!(status.session_id.is_none());
    }

    #[test]
    fn test_no_active_session_serializes_correctly() {
        let status = StatusOutput::no_active_session();
        let json = serde_json::to_value(&status).unwrap();

        assert_eq!(json["type"], "status");
        assert_eq!(json["active"], false);
        assert_eq!(json["message"], "No active recording session found");
        // Optional fields should not be present
        assert!(json.get("session_id").is_none());
        assert!(json.get("elapsed").is_none());
    }

    // ── Active session serialization ──

    #[test]
    fn test_active_status_serializes_as_valid_json() {
        let content = r#"{
            "session_id": "rec-20260329_120000",
            "filename": "recording.wav",
            "duration": 0,
            "elapsed": 154,
            "progress": 0,
            "quality": "Professional (48kHz Stereo)",
            "device": "Dual-Channel (System + Microphone)",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 500,
            "has_audio": true,
            "status": "recording",
            "loopback_frames": 500,
            "loopback_has_audio": true,
            "mic_frames": 400,
            "mic_has_audio": false,
            "is_mic_muted": true,
            "ffmpeg_progress": null,
            "processing_speed": null
        }"#;

        let status = parse_status_file(content).unwrap();
        let json_str = serde_json::to_string(&status).unwrap();

        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "status");
        assert_eq!(parsed["active"], true);
        assert_eq!(parsed["elapsed"], "2m 34s");
        assert_eq!(parsed["duration"], "indefinite");
        assert_eq!(parsed["is_mic_muted"], true);
    }

    // ── find_active_session with temp directory ──

    #[test]
    fn test_find_active_session_empty_directory() {
        let temp_dir = std::env::temp_dir().join("audio_rec_test_empty_status");
        let _ = std::fs::create_dir_all(&temp_dir);

        // Clean up any existing files
        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        let result = find_active_session(&temp_dir);
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_active_session_with_recording() {
        let temp_dir = std::env::temp_dir().join("audio_rec_test_active_status");
        let _ = std::fs::create_dir_all(&temp_dir);

        // Clean up any existing files
        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        let status_content = r#"{
            "session_id": "rec-test-123",
            "filename": "test.wav",
            "duration": 0,
            "elapsed": 10,
            "progress": 0,
            "quality": "Professional (48kHz Stereo)",
            "device": "System Audio",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 100,
            "has_audio": true,
            "status": "recording",
            "loopback_frames": 100,
            "loopback_has_audio": true,
            "mic_frames": null,
            "mic_has_audio": null,
            "is_mic_muted": false,
            "ffmpeg_progress": null,
            "processing_speed": null
        }"#;

        std::fs::write(temp_dir.join("rec-test-123.json"), status_content).unwrap();

        let result = find_active_session(&temp_dir);
        assert!(result.is_some());
        let status = result.unwrap();
        assert!(status.active);
        assert_eq!(status.session_id.unwrap(), "rec-test-123");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_active_session_ignores_completed() {
        let temp_dir = std::env::temp_dir().join("audio_rec_test_completed_status");
        let _ = std::fs::create_dir_all(&temp_dir);

        // Clean up any existing files
        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        let completed_content = r#"{
            "session_id": "rec-completed",
            "filename": "done.wav",
            "file_path": "C:\\done.wav",
            "duration": 60,
            "file_size_mb": "2.34",
            "format": "wav",
            "codec": "pcm",
            "status": "completed",
            "message": "Recording completed successfully"
        }"#;

        std::fs::write(temp_dir.join("rec-completed.json"), completed_content).unwrap();

        let result = find_active_session(&temp_dir);
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // ── Process-specific capture fields ──

    #[test]
    fn test_parse_status_with_capture_mode_and_app() {
        let content = r#"{
            "session_id": "rec-process-test",
            "filename": "recording.wav",
            "duration": 0,
            "elapsed": 10,
            "progress": 0,
            "quality": "Professional (48kHz Stereo)",
            "device": "Dual-Channel (System + Microphone)",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 100,
            "has_audio": true,
            "status": "recording",
            "capture_mode": "process",
            "app": "Zoom.exe"
        }"#;

        let status = parse_status_file(content).unwrap();
        assert_eq!(status.capture_mode.unwrap(), "process");
        assert_eq!(status.app.unwrap(), "Zoom.exe");
        assert!(status.app_window.is_none());
    }

    #[test]
    fn test_parse_status_with_app_window() {
        let content = r#"{
            "session_id": "rec-window-test",
            "filename": "recording.wav",
            "duration": 0,
            "elapsed": 5,
            "progress": 0,
            "quality": "Professional",
            "device": "System",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 50,
            "has_audio": true,
            "status": "recording",
            "capture_mode": "process",
            "app_window": "Zoom Meeting"
        }"#;

        let status = parse_status_file(content).unwrap();
        assert_eq!(status.capture_mode.unwrap(), "process");
        assert!(status.app.is_none());
        assert_eq!(status.app_window.unwrap(), "Zoom Meeting");
    }

    #[test]
    fn test_parse_status_system_mode_no_app_fields() {
        let content = r#"{
            "session_id": "rec-system-test",
            "filename": "recording.wav",
            "duration": 0,
            "elapsed": 5,
            "progress": 0,
            "quality": "Professional",
            "device": "System",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 50,
            "has_audio": true,
            "status": "recording",
            "capture_mode": "system"
        }"#;

        let status = parse_status_file(content).unwrap();
        assert_eq!(status.capture_mode.unwrap(), "system");
        assert!(status.app.is_none());
        assert!(status.app_window.is_none());
    }

    #[test]
    fn test_status_output_serializes_capture_mode() {
        let content = r#"{
            "session_id": "rec-serialize-test",
            "filename": "recording.wav",
            "duration": 0,
            "elapsed": 5,
            "progress": 0,
            "quality": "Professional",
            "device": "System",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 50,
            "has_audio": true,
            "status": "recording",
            "capture_mode": "process",
            "app": "Teams.exe"
        }"#;

        let status = parse_status_file(content).unwrap();
        let json = serde_json::to_value(&status).unwrap();

        assert_eq!(json["capture_mode"], "process");
        assert_eq!(json["app"], "Teams.exe");
        // app_window should not be present (skip_serializing_if = None)
        assert!(json.get("app_window").is_none());
    }

    #[test]
    fn test_no_active_session_has_no_capture_fields() {
        let status = StatusOutput::no_active_session();
        let json = serde_json::to_value(&status).unwrap();

        assert!(json.get("capture_mode").is_none());
        assert!(json.get("app").is_none());
        assert!(json.get("app_window").is_none());
    }

    #[test]
    fn test_parse_m4a_format_from_filename() {
        let content = r#"{
            "session_id": "rec-test",
            "filename": "recording.m4a",
            "duration": 0,
            "elapsed": 5,
            "progress": 0,
            "quality": "Professional",
            "device": "System",
            "sample_rate": 48000,
            "channels": 2,
            "frames_captured": 50,
            "has_audio": true,
            "status": "recording"
        }"#;

        let status = parse_status_file(content).unwrap();
        assert_eq!(status.format.unwrap(), "m4a");
    }
}
