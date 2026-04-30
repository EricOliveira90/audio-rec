use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
    pub session_id: String,
    pub filename: String,
    pub duration: u64,
    pub elapsed: u64,
    pub progress: u8,
    pub quality: String,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames_captured: u64,
    pub has_audio: bool,
    pub status: String,
    pub loopback_frames: Option<u64>,
    pub loopback_has_audio: Option<bool>,
    pub mic_frames: Option<u64>,
    pub mic_has_audio: Option<bool>,
    pub is_mic_muted: Option<bool>,
    pub ffmpeg_progress: Option<u8>,
    pub processing_speed: Option<String>,
    // Process-specific capture fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_window: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingResult {
    pub session_id: String,
    pub filename: String,
    pub file_path: Option<String>,
    pub duration: i64,
    pub file_size_mb: String,
    pub format: String,
    pub codec: String,
    pub status: String,
    pub message: String,
}

pub trait StatusObserver: Send + Sync {
    fn on_progress(&self, status: RecordingStatus) -> crate::error::Result<()>;
    fn on_complete(&self, result: RecordingResult) -> crate::error::Result<()>;
}

pub struct JsonFileObserver {
    status_dir: PathBuf,
}

impl JsonFileObserver {
    pub fn new(status_dir: PathBuf) -> Self {
        Self { status_dir }
    }

    pub fn write_processing_status_v2(
        &self,
        session_id: &str,
        message: &str,
        current_stage: Option<u8>,
        total_stages: Option<u8>,
        stage_name: Option<&str>,
        ffmpeg_progress: Option<u8>,
        processing_speed: Option<String>,
    ) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "processing",
            "message": message,
            "current_stage": current_stage,
            "total_stages": total_stages,
            "stage_name": stage_name,
            "ffmpeg_progress": ffmpeg_progress,
            "processing_speed": processing_speed,
        });
        self.write_status_file(session_id, &status)
    }

    pub fn update_ffmpeg_progress(
        &self,
        session_id: &str,
        progress_pct: u8,
        speed: Option<String>,
        total_duration_ms: Option<u64>,
        current_time_ms: Option<u64>,
        estimated_remaining_secs: Option<u64>,
    ) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "processing",
            "ffmpeg_progress": progress_pct,
            "processing_speed": speed,
            "total_duration_ms": total_duration_ms,
            "current_time_ms": current_time_ms,
            "estimated_remaining_secs": estimated_remaining_secs,
        });
        self.write_status_file(session_id, &status)
    }

    pub fn write_compression_progress(
        &self,
        session_id: &str,
        progress_pct: u8,
        speed: Option<String>,
        total_duration_ms: Option<u64>,
        current_time_ms: Option<u64>,
        estimated_remaining_secs: Option<u64>,
    ) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "compressing",
            "compression_progress": progress_pct,
            "processing_speed": speed,
            "total_duration_ms": total_duration_ms,
            "current_time_ms": current_time_ms,
            "estimated_remaining_secs": estimated_remaining_secs,
        });
        self.write_status_file(session_id, &status)
    }

    pub fn mark_ffmpeg_complete(&self, session_id: &str) -> crate::error::Result<()> {
        let status = serde_json::json!({
            "session_id": session_id,
            "status": "processing",
            "ffmpeg_complete": true,
            "ffmpeg_progress": 100,
        });
        self.write_status_file(session_id, &status)
    }

    fn write_status_file(
        &self,
        session_id: &str,
        value: &serde_json::Value,
    ) -> crate::error::Result<()> {
        let path = self.status_dir.join(format!("{}.json", session_id));
        let content = serde_json::to_string_pretty(value)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}

impl StatusObserver for JsonFileObserver {
    fn on_progress(&self, status: RecordingStatus) -> crate::error::Result<()> {
        let value = serde_json::to_value(&status).map_err(crate::error::RecorderError::Json)?;
        self.write_status_file(&status.session_id, &value)
    }

    fn on_complete(&self, result: RecordingResult) -> crate::error::Result<()> {
        let value = serde_json::to_value(&result).map_err(crate::error::RecorderError::Json)?;
        self.write_status_file(&result.session_id, &value)
    }
}
