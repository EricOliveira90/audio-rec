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

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
