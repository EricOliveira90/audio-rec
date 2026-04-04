use thiserror::Error;

pub type Result<T> = std::result::Result<T, RecorderError>;

#[derive(Debug, Error)]
pub enum RecorderError {
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Audio device disconnected: {0}")]
    DeviceDisconnected(String),

    #[error("Audio device reconnected: {0}")]
    DeviceReconnected(String),

    #[error("Process not found: {0}")]
    ProcessNotFound(String),

    #[error("Unsupported Windows version: {0}")]
    UnsupportedWindowsVersion(String),

    #[error("Sample write error: {0}")]
    WriteError(String),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
