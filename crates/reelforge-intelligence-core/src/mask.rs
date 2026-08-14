//! Mask domains: preview bbox proxy vs final RLE/dense/polygon materialization.
//!
//! SightLoom stores mask handles + payloads. ReelForge consumes time-varying
//! regional samples. Intelligence chooses fidelity per phase.

use crate::ids::NamespacedId;
use crate::time::{MediaRange, MediaTime};
use serde::{Deserialize, Serialize};

/// When / how masks are materialised for ReelForge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MaskFidelity {
    /// Fast axis-aligned box proxy (preview scrub / draft).
    #[default]
    BBoxProxy,
    /// True geometry: RLE / dense / polygon from SightLoom.
    TrueGeometry,
}

impl MaskFidelity {
    /// Preview path default.
    #[must_use]
    pub const fn preview() -> Self {
        Self::BBoxProxy
    }

    /// Final render path default.
    #[must_use]
    pub const fn final_render() -> Self {
        Self::TrueGeometry
    }
}

/// Request masks for subjects / tracks over ranges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskRequest {
    /// Namespaced subjects to materialize.
    #[serde(default)]
    pub subjects: Vec<NamespacedId>,
    /// Optional explicit mask handles.
    #[serde(default)]
    pub mask_ids: Vec<NamespacedId>,
    /// Time ranges of interest.
    #[serde(default)]
    pub ranges: Vec<MediaRange>,
    /// Fidelity.
    #[serde(default)]
    pub fidelity: MaskFidelity,
    /// Sample step in media ticks (`0` = host default).
    #[serde(default)]
    pub sample_step_ticks: i64,
}

impl MaskRequest {
    /// Preview request: bbox proxy for subjects over one span.
    #[must_use]
    pub fn preview_subjects(subjects: Vec<NamespacedId>, span: MediaRange) -> Self {
        Self {
            subjects,
            mask_ids: Vec::new(),
            ranges: vec![span],
            fidelity: MaskFidelity::preview(),
            sample_step_ticks: 0,
        }
    }

    /// Final request: true geometry.
    #[must_use]
    pub fn final_subjects(subjects: Vec<NamespacedId>, ranges: Vec<MediaRange>) -> Self {
        Self {
            subjects,
            mask_ids: Vec::new(),
            ranges,
            fidelity: MaskFidelity::final_render(),
            sample_step_ticks: 0,
        }
    }
}

/// One regional sample for ReelForge MaskTimeline / RegionRedaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionSample {
    /// Media time of this sample.
    pub at: MediaTime,
    /// Axis-aligned box: left, top, right, bottom.
    pub box_xyxy: [f32; 4],
    /// Optional subject this region belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<NamespacedId>,
    /// Confidence of the region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// Encoded true-geometry payload (opaque to ReelForge until host decodes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "encoding", rename_all = "snake_case")]
pub enum MaskGeometry {
    /// COCO-style RLE counts (host-defined packing).
    Rle {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
        /// Counts.
        counts: Vec<u32>,
    },
    /// Dense binary mask row-major.
    Dense {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
        /// 0/1 bytes.
        #[serde(with = "serde_bytes_opt")]
        data: Vec<u8>,
    },
    /// Polygon ring(s) in image coordinates.
    Polygon {
        /// Rings: each ring is [[x,y], …].
        rings: Vec<Vec<[f32; 2]>>,
    },
    /// Only bbox available.
    BBox {
        /// Box.
        box_xyxy: [f32; 4],
    },
}

mod serde_bytes_opt {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
        // JSON-friendly base64-less: array of numbers is fine for small masks in tests.
        s.collect_seq(data.iter().copied())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Vec::<u8>::deserialize(d)
    }
}

/// Materialized mask artifact returned by [`crate::AnalysisProvider::materialize_masks`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskArtifact {
    /// Fidelity actually produced.
    pub fidelity: MaskFidelity,
    /// Regional samples for ReelForge (always present; may be bbox-only).
    #[serde(default)]
    pub regions: Vec<RegionSample>,
    /// True geometry when fidelity is [`MaskFidelity::TrueGeometry`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<MaskGeometry>,
    /// Namespaced mask ids involved.
    #[serde(default)]
    pub mask_ids: Vec<NamespacedId>,
    /// Notes (e.g. "fell back to bbox — no dense mask").
    #[serde(default)]
    pub notes: Vec<String>,
}

impl MaskArtifact {
    /// Empty bbox-proxy artifact.
    #[must_use]
    pub fn empty_preview() -> Self {
        Self {
            fidelity: MaskFidelity::BBoxProxy,
            regions: Vec::new(),
            geometry: None,
            mask_ids: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Build bbox-proxy artifact from samples.
    #[must_use]
    pub fn from_regions(regions: Vec<RegionSample>) -> Self {
        Self {
            fidelity: MaskFidelity::BBoxProxy,
            regions,
            geometry: None,
            mask_ids: Vec::new(),
            notes: Vec::new(),
        }
    }
}
