use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("checksum mismatch for {url}: expected {expected}, got {actual}")]
    Checksum {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn other(msg: impl Into<String>) -> Error {
    Error::Other(msg.into())
}
