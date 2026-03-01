//! Canonical error type for the trusty-izzie workspace.

use thiserror::Error;

/// The top-level error type shared across the workspace.
///
/// Library crates return `TrustyError`; application crates may use
/// `anyhow::Error` to wrap it with additional context.
#[derive(Debug, Error)]
pub enum TrustyError {
    /// Configuration file was missing, malformed, or had invalid values.
    #[error("configuration error: {0}")]
    Config(String),

    /// An I/O error from the standard library.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A storage backend (LanceDB, Kuzu, or SQLite) returned an error.
    #[error("storage error: {0}")]
    Storage(String),

    /// An error occurred while calling the OpenRouter or Gmail API.
    #[error("HTTP error: {0}")]
    Http(String),

    /// JSON serialisation or deserialisation failed.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Entity extraction produced an invalid or unusable result.
    #[error("extraction error: {0}")]
    Extraction(String),

    /// The requested resource was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An operation was attempted without valid authentication.
    #[error("authentication error: {0}")]
    Auth(String),

    /// An error propagated from the embedding layer.
    #[error("embedding error: {0}")]
    Embedding(String),

    /// An IPC communication error with the daemon.
    #[error("IPC error: {0}")]
    Ipc(String),

    /// A catch-all for unexpected errors from dependencies.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Convenience alias used throughout the workspace.
pub type Result<T> = std::result::Result<T, TrustyError>;
