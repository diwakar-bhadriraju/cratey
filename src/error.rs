//! Application-wide error type.

/// Unified error type for Cratey.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Desktop integration error: {0}")]
    Integration(String),
    #[error("Update error: {0}")]
    Update(String),
    #[error("AppImage error: {0}")]
    AppImage(String),
}

/// Convenience result alias. The error type defaults to [`AppError`] but callers
/// may substitute a precise error when needed.
pub type Result<T, E = AppError> = std::result::Result<T, E>;

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Config(format!("JSON error: {e}"))
    }
}

impl From<toml::de::Error> for AppError {
    fn from(e: toml::de::Error) -> Self {
        AppError::Config(format!("TOML parse error: {e}"))
    }
}

impl From<image::ImageError> for AppError {
    fn from(e: image::ImageError) -> Self {
        AppError::Io(format!("image error: {e}"))
    }
}
