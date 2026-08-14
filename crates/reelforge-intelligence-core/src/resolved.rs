//! Frozen resolution document: only this compiles to a final RenderGraph.
//!
//! A [`crate::SemanticEditPlan`] is **intent**. Between preview and final render
//! the VisionIndex may change; hosts must freeze evidence into [`ResolvedEditPlan`].

use crate::edit::SemanticEditPlan;
use crate::error::{IntelError, Result};
use crate::ids::NamespacedId;
use crate::mask::{MaskArtifact, MaskFidelity};
use crate::policy::IntelligencePolicy;
use crate::time::{MediaRange, MediaTime};
use serde::{Deserialize, Serialize};

/// Schema version for [`ResolvedEditPlan`].
pub const RESOLVED_EDIT_PLAN_VERSION: u32 = 2;

/// One subject identity frozen from SightLoom (or host catalog).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSubject {
    /// Namespaced subject id for ReelForge (`sightloom://…/subjects/184`).
    pub id: NamespacedId,
    /// Raw numeric id inside the VisionIndex (debug / host map).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_subject_id: Option<u64>,
    /// Optional human label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Sources where the subject appears (raw source numbers).
    #[serde(default)]
    pub source_ids: Vec<u32>,
    /// Namespaced source ids when frozen.
    #[serde(default)]
    pub source_uris: Vec<NamespacedId>,
    /// Appearance span when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<MediaRange>,
    /// Peak identity confidence when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// One event / anomaly / visit frozen for the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedEvent {
    /// Event id (preferably namespaced URI).
    pub event_id: String,
    /// Kind tag (`appearance`, `anomaly`, `visit`, …).
    pub kind: String,
    /// Optional namespaced subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<NamespacedId>,
    /// Time range.
    pub range: MediaRange,
}

/// Mask / redaction asset frozen from host or VisionIndex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedMaskAsset {
    /// Namespaced mask id when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_id: Option<NamespacedId>,
    /// Host or index mask ref / path key (legacy string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_ref: Option<String>,
    /// Optional namespaced subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<NamespacedId>,
    /// Valid range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<MediaRange>,
    /// Fidelity used for this asset.
    #[serde(default)]
    pub fidelity: MaskFidelity,
    /// Materialized artifact (preview bbox or true geometry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<MaskArtifact>,
}

/// Why a subject/event was chosen or rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolutionDecision {
    /// Machine code (`most_frequent`, `selector_ids`, `anomaly_filter`, …).
    pub code: String,
    /// Human explanation.
    pub message: String,
    /// Related edit index in the intent plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_index: Option<usize>,
}

/// Non-fatal resolution note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolutionWarning {
    /// Message.
    pub message: String,
    /// Related edit index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_index: Option<usize>,
}

/// Frozen, reproducible resolution of a semantic intent against a VisionIndex snapshot.
///
/// **Only** this document is compiled into a final ReelForge `RenderGraph`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedEditPlan {
    /// Schema version.
    pub version: u32,
    /// Content hash of the media source (host-defined).
    pub source_hash: String,
    /// Active VisionIndex package generation (`gen-00000042`, …).
    pub vision_index_generation: String,
    /// Hash / checksum of the VisionIndex snapshot used for resolution.
    pub vision_index_hash: String,
    /// Copy of intent media key for host routing.
    pub media: String,
    /// Frozen subjects.
    #[serde(default)]
    pub resolved_subjects: Vec<ResolvedSubject>,
    /// Frozen events.
    #[serde(default)]
    pub resolved_events: Vec<ResolvedEvent>,
    /// Frozen masks.
    #[serde(default)]
    pub resolved_masks: Vec<ResolvedMaskAsset>,
    /// Time ranges for reels / redactions / follows.
    #[serde(default)]
    pub resolved_ranges: Vec<MediaRange>,
    /// Decisions audit trail.
    #[serde(default)]
    pub decisions: Vec<ResolutionDecision>,
    /// Warnings.
    #[serde(default)]
    pub warnings: Vec<ResolutionWarning>,
    /// Policy snapshot at freeze time.
    #[serde(default)]
    pub policy: IntelligencePolicy,
    /// Original intent (optional, for explainability).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<SemanticEditPlan>,
    /// When this freeze was taken (host wall clock ns, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_at_ns: Option<i64>,
}

impl ResolvedEditPlan {
    /// Empty shell with required hash fields.
    #[must_use]
    pub fn new(
        media: impl Into<String>,
        source_hash: impl Into<String>,
        vision_index_generation: impl Into<String>,
        vision_index_hash: impl Into<String>,
    ) -> Self {
        Self {
            version: RESOLVED_EDIT_PLAN_VERSION,
            source_hash: source_hash.into(),
            vision_index_generation: vision_index_generation.into(),
            vision_index_hash: vision_index_hash.into(),
            media: media.into(),
            resolved_subjects: Vec::new(),
            resolved_events: Vec::new(),
            resolved_masks: Vec::new(),
            resolved_ranges: Vec::new(),
            decisions: Vec::new(),
            warnings: Vec::new(),
            policy: IntelligencePolicy::default(),
            intent: None,
            frozen_at_ns: None,
        }
    }

    /// Validate freeze integrity.
    ///
    /// # Errors
    ///
    /// Missing hashes / empty media.
    pub fn validate(&self) -> Result<()> {
        if self.version == 0 || self.version > RESOLVED_EDIT_PLAN_VERSION {
            return Err(IntelError::UnsupportedVersion(self.version));
        }
        if self.media.trim().is_empty() {
            return Err(IntelError::message("resolved plan: media empty"));
        }
        if self.source_hash.trim().is_empty() {
            return Err(IntelError::message("resolved plan: source_hash empty"));
        }
        if self.vision_index_hash.trim().is_empty() {
            return Err(IntelError::message(
                "resolved plan: vision_index_hash empty",
            ));
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

/// Helper: whole-media range when timescale known.
#[must_use]
pub fn whole_media_range(duration_ticks: i64, timescale: u32) -> MediaRange {
    MediaRange::new(
        MediaTime::new(0, timescale),
        MediaTime::new(duration_ticks, timescale),
    )
}
