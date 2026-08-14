//! Intelligence contract errors.

use thiserror::Error;

/// Result alias.
pub type Result<T> = std::result::Result<T, IntelError>;

/// Contract / validation errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IntelError {
    /// Generic message.
    #[error("intelligence: {0}")]
    Message(String),
    /// Unsupported schema version.
    #[error("unsupported SemanticEditPlan version {0}")]
    UnsupportedVersion(u32),
}

impl IntelError {
    /// Message helper.
    #[must_use]
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}
