use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("file too large: {0} bytes (limit {1})")]
    TooLarge(u64, u64),
    #[error("corrupted file: {0}")]
    Corrupted(String),
}
