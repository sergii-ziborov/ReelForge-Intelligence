//! Privacy and intelligence policies for semantic edits.

use serde::{Deserialize, Serialize};

/// What to do when identity is uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UncertaintyPolicy {
    /// Blur uncertain subjects (default, conservative privacy).
    #[default]
    Blur,
    /// Leave uncertain subjects unblurred.
    Allow,
    /// Require manual review before render.
    Review,
}

/// High-level privacy policy (nested under [`IntelligencePolicy`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PrivacyPolicy {
    /// Uncertainty handling.
    #[serde(default)]
    pub uncertain_identity: UncertaintyPolicy,
    /// Optional free-form notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Plan-level Intelligence policy (intent + freeze).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IntelligencePolicy {
    /// Privacy / uncertainty defaults.
    #[serde(default)]
    pub privacy: PrivacyPolicy,
    /// When true, missing selectors fail resolve hard (no soft empty freeze).
    #[serde(default)]
    pub fail_on_empty_resolution: bool,
    /// Require human approve on [`UncertaintyPolicy::Review`] before final compile.
    #[serde(default = "default_true")]
    pub require_approve_on_review: bool,
}

fn default_true() -> bool {
    true
}
