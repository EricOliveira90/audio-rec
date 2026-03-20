use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::{RecorderError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Wav,
    M4a,
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioFormat::Wav => write!(f, "wav"),
            AudioFormat::M4a => write!(f, "m4a"),
        }
    }
}

impl FromStr for AudioFormat {
    type Err = RecorderError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "wav" => Ok(AudioFormat::Wav),
            "m4a" => Ok(AudioFormat::M4a),
            _ => Err(RecorderError::InvalidParameter(format!(
                "Unsupported audio format '{}'. Supported formats: wav, m4a",
                s
            ))),
        }
    }
}

impl AudioFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::M4a => "m4a",
        }
    }

    pub fn codec(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "pcm",
            AudioFormat::M4a => "aac",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingDuration {
    Fixed(u64),          // seconds
    Manual { max: u64 }, // -1 becomes this
}

impl RecordingDuration {
    pub fn from_secs(duration: i64, max_manual_duration: u64) -> Result<Self> {
        match duration {
            -1 => Ok(RecordingDuration::Manual {
                max: max_manual_duration,
            }),
            d if d > 0 => Ok(RecordingDuration::Fixed(d as u64)),
            d => Err(RecorderError::InvalidParameter(format!(
                "Duration must be -1 (manual mode) or a positive number, got: {}",
                d
            ))),
        }
    }

    pub fn effective_duration(&self) -> u64 {
        match self {
            RecordingDuration::Fixed(d) => *d,
            RecordingDuration::Manual { max } => *max,
        }
    }

    pub fn to_api_value(&self) -> i64 {
        match self {
            RecordingDuration::Fixed(d) => *d as i64,
            RecordingDuration::Manual { .. } => -1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn new() -> Self {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        Self(format!("rec-{}", timestamp))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct RecordingSession {
    pub id: SessionId,
    pub format: AudioFormat,
    pub quality: crate::recorder::RecordingQuality,
    pub duration: RecordingDuration,
    pub started_at: chrono::DateTime<Local>,
}

impl RecordingSession {
    pub fn new(format: AudioFormat, quality: crate::recorder::RecordingQuality, duration: RecordingDuration) -> Self {
        Self {
            id: SessionId::new(),
            format,
            quality,
            duration,
            started_at: Local::now(),
        }
    }

    pub fn filename(&self) -> String {
        let timestamp = self.started_at.format("%Y-%m-%d-%H_%M");
        format!("{}_recording.{}", timestamp, self.format.extension())
    }

    pub fn temp_filename(&self) -> String {
        let timestamp = self.started_at.format("%Y-%m-%d-%H_%M");
        format!("{}_recording.wav", timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_parse_wav() {
        let format: AudioFormat = "wav".parse().unwrap();
        assert_eq!(format, AudioFormat::Wav);
    }

    #[test]
    fn test_audio_format_parse_m4a() {
        let format: AudioFormat = "m4a".parse().unwrap();
        assert_eq!(format, AudioFormat::M4a);
    }

    #[test]
    fn test_audio_format_parse_case_insensitive() {
        let format: AudioFormat = "WAV".parse().unwrap();
        assert_eq!(format, AudioFormat::Wav);
        let format: AudioFormat = "M4A".parse().unwrap();
        assert_eq!(format, AudioFormat::M4a);
    }

    #[test]
    fn test_audio_format_parse_invalid() {
        let result: std::result::Result<AudioFormat, _> = "mp3".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_audio_format_extension() {
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert_eq!(AudioFormat::M4a.extension(), "m4a");
    }

    #[test]
    fn test_audio_format_codec() {
        assert_eq!(AudioFormat::Wav.codec(), "pcm");
        assert_eq!(AudioFormat::M4a.codec(), "aac");
    }

    #[test]
    fn test_audio_format_display() {
        assert_eq!(format!("{}", AudioFormat::Wav), "wav");
        assert_eq!(format!("{}", AudioFormat::M4a), "m4a");
    }

    #[test]
    fn test_recording_duration_fixed() {
        let duration = RecordingDuration::from_secs(60, 7200).unwrap();
        assert_eq!(duration, RecordingDuration::Fixed(60));
        assert_eq!(duration.effective_duration(), 60);
        assert_eq!(duration.to_api_value(), 60);
    }

    #[test]
    fn test_recording_duration_manual() {
        let duration = RecordingDuration::from_secs(-1, 7200).unwrap();
        assert_eq!(duration, RecordingDuration::Manual { max: 7200 });
        assert_eq!(duration.effective_duration(), 7200);
        assert_eq!(duration.to_api_value(), -1);
    }

    #[test]
    fn test_recording_duration_invalid_zero() {
        let result = RecordingDuration::from_secs(0, 7200);
        assert!(result.is_err());
    }

    #[test]
    fn test_recording_duration_invalid_negative() {
        let result = RecordingDuration::from_secs(-2, 7200);
        assert!(result.is_err());
    }

    #[test]
    fn test_session_id_format() {
        let id = SessionId::new();
        let s = id.as_str();
        assert!(s.starts_with("rec-"));
        // Format: rec-YYYYMMDD_HHMMSS
        assert!(s.len() >= 19); // "rec-" (4) + "YYYYMMDD_HHMMSS" (15) = 19
    }

    #[test]
    fn test_session_id_from_string() {
        let id = SessionId::from("test-session".to_string());
        assert_eq!(id.as_str(), "test-session");
    }

    #[test]
    fn test_session_id_from_str() {
        let id = SessionId::from("test-session");
        assert_eq!(id.as_str(), "test-session");
    }

    #[test]
    fn test_session_id_display() {
        let id = SessionId::from("my-session");
        assert_eq!(format!("{}", id), "my-session");
    }
}
