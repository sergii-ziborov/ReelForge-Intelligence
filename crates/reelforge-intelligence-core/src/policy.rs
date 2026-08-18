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

/// Alias used in product language (`UncertainAction`).
pub type UncertainAction = UncertaintyPolicy;

/// Missing mask behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MissingMaskAction {
    /// Conservative hold: expand bbox / union of last known regions.
    #[default]
    ConservativeHold,
    /// Use last good mask dilated.
    DilateLast,
    /// Skip redaction for that frame (unsafe).
    Skip,
    /// Fail resolve / require review.
    Review,
    /// Hard-fail when true geometry is required and missing.
    Fail,
}

/// Low identity / detection confidence behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LowConfidenceAction {
    /// Treat as uncertain → follow [`PrivacyPolicy::uncertain_identity`].
    #[default]
    TreatAsUncertain,
    /// Blur anyway.
    Blur,
    /// Allow through.
    Allow,
    /// Require review.
    Review,
}

/// Short track gap / occlusion behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GapAction {
    /// Interpolate / dilate mask across short gaps.
    #[default]
    InterpolateDilate,
    /// Hold last box without interpolation.
    HoldLast,
    /// Drop gap frames from redaction.
    Drop,
    /// Require review if gap exceeds threshold.
    ReviewIfLong,
}

/// Privacy-first policy for resolve + materialize.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrivacyPolicy {
    /// Uncertain identity intervals.
    #[serde(default)]
    pub uncertain_identity: UncertaintyPolicy,
    /// Missing true mask payload.
    #[serde(default)]
    pub missing_mask: MissingMaskAction,
    /// Low confidence detections / matches.
    #[serde(default)]
    pub low_confidence: LowConfidenceAction,
    /// Short track gaps.
    #[serde(default)]
    pub track_gap: GapAction,
    /// Max gap ticks treated as "short" for interpolate (`0` = host default).
    #[serde(default)]
    pub short_gap_max_ticks: i64,
    /// Confidence threshold below which low-confidence actions apply.
    #[serde(default = "default_low_conf")]
    pub low_confidence_threshold: f32,
    /// Optional free-form notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn default_low_conf() -> f32 {
    0.5
}

impl Default for PrivacyPolicy {
    /// Privacy-first defaults:
    /// - uncertain → blur
    /// - missing mask → conservative hold
    /// - short gap → interpolate/dilate
    /// - low confidence → treat as uncertain
    fn default() -> Self {
        Self {
            uncertain_identity: UncertaintyPolicy::Blur,
            missing_mask: MissingMaskAction::ConservativeHold,
            low_confidence: LowConfidenceAction::TreatAsUncertain,
            track_gap: GapAction::InterpolateDilate,
            short_gap_max_ticks: 0,
            low_confidence_threshold: default_low_conf(),
            notes: None,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_first_defaults() {
        let p = PrivacyPolicy::default();
        assert_eq!(p.uncertain_identity, UncertaintyPolicy::Blur);
        assert_eq!(p.missing_mask, MissingMaskAction::ConservativeHold);
        assert_eq!(p.track_gap, GapAction::InterpolateDilate);
        assert_eq!(p.low_confidence, LowConfidenceAction::TreatAsUncertain);
    }
}
