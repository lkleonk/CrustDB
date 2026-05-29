use thiserror::Error;

pub type Result<T> = std::result::Result<T, CrustDbError>;

#[derive(Debug, Error)]
pub enum CrustDbError {
    #[error("{0}")]
    Validation(String),

    #[error("{0}")]
    UniqueConstraint(String),

    #[error("{0}")]
    IncompatibleSchema(String),

    #[error("Unknown model: {0}")]
    UnknownModel(String),

    #[error("Storage format error: {0}")]
    StorageFormat(String),

    #[error("Storage error: {0}")]
    Storage(#[from] std::io::Error),
}
