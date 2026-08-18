//! Semantic edit plan document (intent — not frozen).

use crate::error::{IntelError, Result};
use crate::pii::PiiKind;
use crate::policy::IntelligencePolicy;
use crate::query::EventQuery;
use crate::selector::SubjectSelector;
use crate::time::MediaTime;
use serde::{Deserialize, Serialize};

/// Schema version for [`SemanticEditPlan`].
pub const SEMANTIC_EDIT_PLAN_VERSION: u32 = 2;

/// How to measure "most frequent" subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FrequencyMetric {
    /// Count of appearances / visits.
    #[default]
    AppearanceCount,
    /// Number of distinct sources / cameras.
    SourceCount,
    /// Total duration of presence.
    Duration,
}

/// Framing policy for follow / crop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FramingPolicy {
    /// Host default crop around subject.
    #[default]
    Tight,
    /// Wider framing.
    Medium,
    /// Full frame with optional tracking overlay.
    Wide,
}

/// Query for anomaly-driven reels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AnomalyQuery {
    /// Inclusive minimum hour-of-day (0–23).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_hour_inclusive: Option<u8>,
    /// Exclusive maximum hour-of-day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hour_exclusive: Option<u8>,
    /// Minimum anomaly score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f32>,
    /// Substring match on kind/reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_contains: Option<String>,
}

/// One semantic edit intent (not an executable render node).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SemanticEdit {
    /// Blur a selected subject throughout media.
    BlurSubject {
        /// Subject selector.
        subject: SubjectSelector,
    },
    /// Blur everyone except allowed set / subject.
    BlurEveryoneExcept {
        /// Allowed subjects / set.
        allowed: SubjectSelector,
        /// Uncertainty policy override.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uncertain_identity: Option<crate::policy::UncertaintyPolicy>,
    },
    /// Keep framing on a subject (follow / smart crop).
    FollowSubject {
        /// Subject selector.
        subject: SubjectSelector,
        /// Framing.
        #[serde(default)]
        framing: FramingPolicy,
    },
    /// Build a reel of a subject's appearances.
    BuildSubjectReel {
        /// Subject selector.
        subject: SubjectSelector,
        /// Pre-roll pad.
        #[serde(default)]
        pre_roll: MediaTime,
        /// Post-roll pad.
        #[serde(default)]
        post_roll: MediaTime,
    },
    /// Rank subjects and reel the most frequent.
    BuildMostFrequentSubjectReel {
        /// Frequency metric.
        #[serde(default)]
        metric: FrequencyMetric,
    },
    /// Reel anomaly ranges matching a query.
    BuildAnomalyReel {
        /// Anomaly filter.
        query: AnomalyQuery,
    },
    /// Cut clips around events matching a query (host expands events).
    CreateEventClips {
        /// Event query.
        query: EventQuery,
        /// Pre-roll seconds.
        #[serde(default = "default_pad")]
        pad_before_secs: f64,
        /// Post-roll seconds.
        #[serde(default = "default_pad")]
        pad_after_secs: f64,
    },
    /// Redact non-person PII (plates, screens, text, documents).
    ///
    /// Empty `kinds` means every [`PiiKind`]. Missing evidence is an error —
    /// Intelligence does not invent detections.
    RedactPii {
        /// Kinds to redact. Empty = all kinds.
        #[serde(default)]
        kinds: Vec<PiiKind>,
    },
}

fn default_pad() -> f64 {
    1.0
}

/// Intelligence-owned semantic edit document (**intent** only).
///
/// Must be frozen into [`crate::ResolvedEditPlan`] before final RenderGraph compile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticEditPlan {
    /// Schema version.
    pub version: u32,
    /// Primary media asset id / path key in host project.
    pub media: String,
    /// Optional SightLoom / analysis source ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<String>,
    /// Ordered semantic edits.
    #[serde(default)]
    pub edits: Vec<SemanticEdit>,
    /// Plan-level policy.
    #[serde(default)]
    pub policy: IntelligencePolicy,
    /// Desired output label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_output: Option<String>,
}

impl SemanticEditPlan {
    /// New plan for media.
    #[must_use]
    pub fn new(media: impl Into<String>) -> Self {
        Self {
            version: SEMANTIC_EDIT_PLAN_VERSION,
            media: media.into(),
            analysis: None,
            edits: Vec::new(),
            policy: IntelligencePolicy::default(),
            target_output: None,
        }
    }

    /// Append an edit.
    #[must_use]
    pub fn with_edit(mut self, edit: SemanticEdit) -> Self {
        self.edits.push(edit);
        self
    }

    /// Attach analysis source ref.
    #[must_use]
    pub fn with_analysis(mut self, analysis: impl Into<String>) -> Self {
        self.analysis = Some(analysis.into());
        self
    }

    /// Validate version and non-empty media.
    ///
    /// # Errors
    ///
    /// Unsupported version or empty media.
    pub fn validate(&self) -> Result<()> {
        if self.version == 0 || self.version > SEMANTIC_EDIT_PLAN_VERSION {
            return Err(IntelError::UnsupportedVersion(self.version));
        }
        if self.media.trim().is_empty() {
            return Err(IntelError::message("media id must be non-empty"));
        }
        Ok(())
    }

    /// Pretty JSON.
    ///
    /// # Errors
    ///
    /// Serde.
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| IntelError::message(e.to_string()))
    }

    /// Parse JSON.
    ///
    /// # Errors
    ///
    /// Serde / validate.
    pub fn from_json(text: &str) -> Result<Self> {
        let plan: Self =
            serde_json::from_str(text).map_err(|e| IntelError::message(e.to_string()))?;
        plan.validate()?;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::UncertaintyPolicy;

    #[test]
    fn example_blur_everyone_except_roundtrip() {
        let plan =
            SemanticEditPlan::new("assets/video-1").with_edit(SemanticEdit::BlurEveryoneExcept {
                allowed: SubjectSelector::SubjectSet {
                    name: "family".into(),
                },
                uncertain_identity: Some(UncertaintyPolicy::Blur),
            });
        let json = plan.to_json_pretty().unwrap();
        assert!(json.contains("blur_everyone_except"));
        let back = SemanticEditPlan::from_json(&json).unwrap();
        assert_eq!(back.media, "assets/video-1");
        assert_eq!(back.edits.len(), 1);
    }

    #[test]
    fn redact_pii_roundtrip() {
        let plan = SemanticEditPlan::new("v.mp4").with_edit(SemanticEdit::RedactPii {
            kinds: vec![PiiKind::LicensePlate, PiiKind::Screen],
        });
        let json = plan.to_json_pretty().unwrap();
        assert!(json.contains("redact_pii"));
        assert!(json.contains("license_plate"));
        let back = SemanticEditPlan::from_json(&json).unwrap();
        assert_eq!(back.edits.len(), 1);
    }
}
