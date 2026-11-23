use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("binary file, no text extracted")]
    Binary,
}
