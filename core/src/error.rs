pub type Result<T> = std::result::Result<T, RecorderError>;

#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
