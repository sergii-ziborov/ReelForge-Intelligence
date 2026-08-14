//! Adapter errors.

use std::fmt;

/// Error loading or converting a SightLoom package.
#[derive(Debug)]
pub enum SightLoomAdapterError {
    /// Package I/O or validation.
    Package(String),
    /// Conversion failure.
    Convert(String),
}

impl fmt::Display for SightLoomAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package(m) | Self::Convert(m) => write!(f, "sightloom adapter: {m}"),
        }
    }
}

impl std::error::Error for SightLoomAdapterError {}

impl From<sightloom_index::MemoryError> for SightLoomAdapterError {
    fn from(value: sightloom_index::MemoryError) -> Self {
        Self::Package(value.to_string())
    }
}
