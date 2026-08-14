//! Analysis provider interface — SightLoom first, more providers later.

use crate::error::Result;
use crate::ids::NamespacedId;
use crate::mask::{MaskArtifact, MaskRequest};
use crate::query::EventQuery;
use crate::selector::SubjectSelector;
use crate::time::MediaRange;
use serde::{Deserialize, Serialize};

/// Generation pin for freeze reproducibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisGeneration {
    /// Provider id (`sightloom`, `transcript`, …).
    pub provider_id: String,
    /// VisionIndex package generation / stream revision.
    pub generation: String,
    /// Content hash of the analysis snapshot.
    pub content_hash: String,
    /// Optional source media hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

/// Subject query against an analysis provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SubjectQuery {
    /// Optional selector (ids / set / most-frequent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<SubjectSelector>,
    /// Limit results (`0` = unlimited).
    #[serde(default)]
    pub limit: usize,
    /// Minimum confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f32>,
}

/// One subject row from a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectResult {
    /// Namespaced subject id.
    pub id: NamespacedId,
    /// Optional label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Appearance count.
    #[serde(default)]
    pub appearance_count: u64,
    /// Source ids (raw u32 within the index).
    #[serde(default)]
    pub source_ids: Vec<u32>,
    /// Presence span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<MediaRange>,
    /// Confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// One event / anomaly row from a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventResult {
    /// Namespaced or opaque event id string.
    pub event_id: String,
    /// Kind.
    pub kind: String,
    /// Optional subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<NamespacedId>,
    /// Range.
    pub range: MediaRange,
    /// Hour of day when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour_of_day: Option<u8>,
    /// Score.
    #[serde(default)]
    pub score: f32,
}

/// Provider of analysis evidence. First impl: [`crate::SightLoomProvider`].
///
/// Later: TranscriptProvider, OCRProvider, CaptureEventProvider, AudioActivityProvider.
pub trait AnalysisProvider {
    /// Pin / generation for freeze.
    fn generation(&self) -> AnalysisGeneration;

    /// Query subjects.
    ///
    /// # Errors
    ///
    /// Provider failures.
    fn query_subjects(&self, query: &SubjectQuery) -> Result<Vec<SubjectResult>>;

    /// Query events / anomalies.
    ///
    /// # Errors
    ///
    /// Provider failures.
    fn query_events(&self, query: &EventQuery) -> Result<Vec<EventResult>>;

    /// Materialize masks for ReelForge (bbox preview or true geometry).
    ///
    /// # Errors
    ///
    /// Provider failures.
    fn materialize_masks(&self, request: &MaskRequest) -> Result<MaskArtifact>;
}

/// Descriptor for catalogs / MCP (not the trait).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisProviderInfo {
    /// Provider id.
    pub id: String,
    /// Human label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Capability tags.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl AnalysisProviderInfo {
    /// SightLoom provider descriptor.
    #[must_use]
    pub fn sightloom() -> Self {
        Self {
            id: "sightloom".into(),
            label: Some("SightLoom VisionIndex".into()),
            capabilities: vec![
                "subjects".into(),
                "tracks".into(),
                "masks".into(),
                "anomalies".into(),
                "ranking".into(),
            ],
        }
    }
}
