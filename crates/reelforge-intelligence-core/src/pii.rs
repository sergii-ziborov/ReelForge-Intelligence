//! PII kinds Intelligence can redact. Detection lives on the host / SightLoom.

use crate::error::{IntelError, Result};
use serde::{Deserialize, Serialize};

/// Class of non-identity privacy target.
///
/// People stay on [`crate::SemanticEdit::BlurSubject`] /
/// [`crate::SemanticEdit::BlurEveryoneExcept`]. These kinds are objects
/// (plates, screens, text) the host must project onto the freeze.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiKind {
    /// Vehicle registration plate.
    LicensePlate,
    /// Monitor / laptop / phone screen.
    Screen,
    /// Burned-in or OCR text.
    Text,
    /// ID card / passport / document face.
    Document,
}

impl PiiKind {
    /// All kinds, stable order.
    pub const ALL: [Self; 4] = [
        Self::LicensePlate,
        Self::Screen,
        Self::Text,
        Self::Document,
    ];

    /// Snake_case token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::LicensePlate => "license_plate",
            Self::Screen => "screen",
            Self::Text => "text",
            Self::Document => "document",
        }
    }

    /// Parse a host / detector label. Unknown tokens error (no silent drop).
    ///
    /// # Errors
    ///
    /// Unknown token.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "license_plate" | "plate" | "number_plate" | "numberplate" => Ok(Self::LicensePlate),
            "screen" | "monitor" | "display" | "tv" | "laptop" => Ok(Self::Screen),
            "text" | "ocr" | "caption" => Ok(Self::Text),
            "document" | "id_card" | "idcard" | "passport" => Ok(Self::Document),
            other => Err(IntelError::message(format!(
                "unknown PII kind `{other}` (license_plate|screen|text|document)"
            ))),
        }
    }
}

impl std::fmt::Display for PiiKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aliases() {
        assert_eq!(PiiKind::parse("plate").unwrap(), PiiKind::LicensePlate);
        assert_eq!(PiiKind::parse("TV").unwrap(), PiiKind::Screen);
        assert!(PiiKind::parse("person").is_err());
    }
}
