pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("resource not found")]
    NotFound,
    #[error("database operation failed")]
    Database(#[source] rusqlite::Error),
    #[error("stored node data is invalid")]
    Serialization(#[source] serde_json::Error),
    #[error("upstream operation failed: {0}")]
    Upstream(String),
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        Self::Upstream(error.to_string())
    }
}
