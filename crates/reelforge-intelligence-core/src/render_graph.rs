//! Typed RenderGraph IR for Intelligence → ReelForge handoff.
//!
//! Aligns with ReelForge operation ids (`rf.redaction.region`, `rf.transform.trim`, …)
//! without hard-linking the full `reelforge-render-graph` crate. Hosts can
//! deserialize JSON into ReelForge types or map nodes explicitly.

use crate::edit::{FramingPolicy, SemanticEdit};
use crate::framing::{self, CropRect};
use crate::ids::{EntityKind, NamespacedId};
use crate::mask::RegionSample;
use crate::ops::edit_op_id;
use crate::policy::{IntelligencePolicy, UncertaintyPolicy};
use crate::resolved::{ResolvedEditPlan, ResolvedMaskAsset, ResolvedOperation};
use crate::time::{MediaRange, MediaTime};
use serde::{Deserialize, Serialize};

/// Schema version for Intelligence-produced graphs.
pub const INTEL_RENDER_GRAPH_VERSION: u32 = 1;

/// Stable ReelForge-facing operation id strings.
pub mod op_id {
    /// Region redaction / privacy blur.
    pub const REDACTION_REGION: &str = "rf.redaction.region";
    /// Timeline trim / subclip.
    pub const TRANSFORM_TRIM: &str = "rf.transform.trim";
    /// Crop / follow framing.
    pub const TRANSFORM_CROP: &str = "rf.transform.crop";
    /// Deprecated alias for [`TIMELINE_CONCAT`].
    pub const TRANSFORM_CONCAT: &str = "rf.timeline.concat";
    /// Sequential clip concatenation (not layer composition).
    pub const TIMELINE_CONCAT: &str = "rf.timeline.concat";
    /// Adapter stage (SightLoom materialization).
    pub const ADAPTER_SIGHTLOOM: &str = "rf.adapter.sightloom";
}

/// Input media asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphAsset {
    /// Asset id inside the graph (`in`, …).
    pub id: String,
    /// Host URI / path key.
    pub uri: String,
    /// Content hash when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

/// Kind of DAG node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    /// Media source.
    Source,
    /// Typed operation.
    Op,
    /// Graph output.
    Output,
}

/// One node in the executable DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Node id.
    pub id: String,
    /// Kind.
    pub kind: GraphNodeKind,
    /// Operation id when [`GraphNodeKind::Op`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Input node ids.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Asset id when source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Output name when output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Free-form params (tracks, ranges, subjects, styles).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    /// Semantic op that produced this node (audit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
}

/// Deterministic media DAG for ReelForge hosts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderGraphIr {
    /// Schema version.
    pub version: u32,
    /// Final (frozen) vs preview.
    pub final_graph: bool,
    /// VisionIndex generation pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_index_generation: Option<String>,
    /// VisionIndex content hash pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_index_hash: Option<String>,
    /// Approval status required / recorded.
    #[serde(default)]
    pub approval: ApprovalRecord,
    /// Assets.
    pub assets: Vec<GraphAsset>,
    /// Nodes in topological order (source → ops → output).
    pub nodes: Vec<GraphNode>,
    /// Human note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl RenderGraphIr {
    /// Pretty JSON for hosts / MCP.
    ///
    /// # Errors
    ///
    /// Serde.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Compact JSON string.
    ///
    /// # Errors
    ///
    /// Serde.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Approval gate for privacy-first final renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ApprovalRecord {
    /// Whether policy requires human approve before final execute.
    #[serde(default)]
    pub required: bool,
    /// Whether an operator has approved.
    #[serde(default)]
    pub approved: bool,
    /// Optional approver id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    /// Reason codes that triggered the gate.
    #[serde(default)]
    pub reasons: Vec<String>,
    /// Bound graph fingerprint (SHA-256 of canonical live graph JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_fingerprint: Option<String>,
    /// Bound IR fingerprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_fingerprint: Option<String>,
    /// Bound resolved-plan fingerprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_fingerprint: Option<String>,
    /// Bound policy hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_hash: Option<String>,
    /// Bound output URI hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_uri_hash: Option<String>,
    /// Approval unix timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at_unix: Option<i64>,
    /// Optional expiration unix timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix: Option<i64>,
    /// HMAC-SHA256 hex of the bound fingerprints when `RF_INTEL_APPROVAL_HMAC` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ApprovalRecord {
    /// True when execution is allowed.
    ///
    /// False when approval is required and not granted, or when
    /// `expires_at_unix` is set and already in the past. A missing expiry
    /// does not expire.
    #[must_use]
    pub fn allows_execute(&self) -> bool {
        !self.is_expired() && (!self.required || self.approved)
    }

    /// `expires_at_unix` is set and strictly before now.
    #[must_use]
    pub(crate) fn is_expired(&self) -> bool {
        self.expires_at_unix.is_some_and(|t| t < now_unix())
    }
}

/// Compute approval requirements from policy + resolved plan.
#[must_use]
pub fn approval_for_resolved(
    resolved: &ResolvedEditPlan,
    policy: &IntelligencePolicy,
) -> ApprovalRecord {
    let mut reasons = Vec::new();
    if policy.require_approve_on_review
        && matches!(policy.privacy.uncertain_identity, UncertaintyPolicy::Review)
    {
        reasons.push("uncertain_identity=review".into());
    }
    // The policy enum alone is not evidence. Review applies when resolve
    // actually recorded a subject with no mask sample and no box.
    if matches!(
        policy.privacy.missing_mask,
        crate::policy::MissingMaskAction::Review
    ) && resolved
        .warnings
        .iter()
        .any(|warning| warning.message.to_lowercase().contains("missing mask"))
    {
        reasons.push("missing_mask=review".into());
    }
    if resolved
        .warnings
        .iter()
        .any(|w| w.message.to_lowercase().contains("review"))
    {
        reasons.push("resolution_warning_review".into());
    }
    // Uncertain subjects with low confidence under TreatAsUncertain + Review path
    if matches!(policy.privacy.uncertain_identity, UncertaintyPolicy::Review) {
        for s in &resolved.resolved_subjects {
            if s.confidence
                .is_some_and(|c| c < policy.privacy.low_confidence_threshold)
            {
                reasons.push(format!("low_confidence:{}", s.id.as_uri()));
            }
        }
    }
    reasons.sort();
    reasons.dedup();
    ApprovalRecord {
        required: !reasons.is_empty() && policy.require_approve_on_review,
        approved: false,
        approved_by: None,
        reasons,
        graph_fingerprint: None,
        ir_fingerprint: None,
        resolved_fingerprint: None,
        policy_hash: None,
        output_uri_hash: None,
        approved_at_unix: None,
        expires_at_unix: None,
        signature: None,
    }
}

/// Mark a record approved by operator (does not bind fingerprints).
#[must_use]
pub fn approve(mut record: ApprovalRecord, by: impl Into<String>) -> ApprovalRecord {
    record.approved = true;
    record.approved_by = Some(by.into());
    record.approved_at_unix = Some(now_unix());
    record
}

/// Bind fingerprints onto an approved record.
#[must_use]
pub fn bind_approval(
    mut record: ApprovalRecord,
    graph_fingerprint: impl Into<String>,
    ir_fingerprint: impl Into<String>,
    resolved_fingerprint: impl Into<String>,
    policy_hash: impl Into<String>,
    output_uri_hash: impl Into<String>,
) -> ApprovalRecord {
    record.graph_fingerprint = Some(graph_fingerprint.into());
    record.ir_fingerprint = Some(ir_fingerprint.into());
    record.resolved_fingerprint = Some(resolved_fingerprint.into());
    record.policy_hash = Some(policy_hash.into());
    record.output_uri_hash = Some(output_uri_hash.into());
    let material = crate::digest::approval_material(
        record.graph_fingerprint.as_deref().unwrap_or(""),
        record.ir_fingerprint.as_deref().unwrap_or(""),
        record.resolved_fingerprint.as_deref().unwrap_or(""),
        record.policy_hash.as_deref().unwrap_or(""),
        record.output_uri_hash.as_deref().unwrap_or(""),
    );
    record.signature = crate::digest::maybe_sign_approval(&material);
    record
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Build typed graph from frozen resolution (final).
#[must_use]
pub fn graph_from_resolved(resolved: &ResolvedEditPlan) -> RenderGraphIr {
    let policy = &resolved.policy;
    let approval = approval_for_resolved(resolved, policy);
    let intent_ops: Vec<&SemanticEdit> = resolved
        .intent
        .as_ref()
        .map(|i| i.edits.iter().collect())
        .unwrap_or_default();

    let mut nodes = vec![GraphNode {
        id: "src".into(),
        kind: GraphNodeKind::Source,
        operation: None,
        inputs: Vec::new(),
        asset: Some("in".into()),
        name: None,
        params: Some(serde_json::json!({
            "source_hash": resolved.source_hash,
        })),
        semantic: None,
    }];
    let mut prev = "src".to_string();

    // Adapter stage: host materializes tracks/masks from frozen subjects.
    if !resolved.resolved_subjects.is_empty() || !resolved.resolved_masks.is_empty() {
        let id = "adapter_sightloom".to_string();
        let subjects: Vec<String> = resolved
            .resolved_subjects
            .iter()
            .map(|s| s.id.as_uri())
            .collect();
        nodes.push(GraphNode {
            id: id.clone(),
            kind: GraphNodeKind::Op,
            operation: Some(op_id::ADAPTER_SIGHTLOOM.into()),
            inputs: vec![prev.clone()],
            asset: None,
            name: None,
            params: Some(serde_json::json!({
                "subjects": subjects,
                "vision_index_generation": resolved.vision_index_generation,
                "vision_index_hash": resolved.vision_index_hash,
                "package_id": resolved.mask_package_id,
                "ranges": ranges_json(&resolved.resolved_ranges),
            })),
            semantic: Some("materialize".into()),
        });
        prev = id;
    }

    // Map intent semantics → ops when present; else default redaction/reel from freeze.
    if intent_ops.is_empty() {
        if !resolved.resolved_subjects.is_empty() || !resolved.resolved_ranges.is_empty() {
            let id = "redact".to_string();
            nodes.push(GraphNode {
                id: id.clone(),
                kind: GraphNodeKind::Op,
                operation: Some(op_id::REDACTION_REGION.into()),
                inputs: vec![prev.clone()],
                asset: None,
                name: None,
                params: Some(redaction_params(resolved, None)),
                semantic: Some("blur".into()),
            });
            prev = id;
        }
    } else {
        for (i, edit) in intent_ops.iter().enumerate() {
            let id = format!("e{i}");
            let (operation, params) = map_edit_to_op(edit, resolved, i);
            nodes.push(GraphNode {
                id: id.clone(),
                kind: GraphNodeKind::Op,
                operation: Some(operation.into()),
                inputs: vec![prev.clone()],
                asset: None,
                name: None,
                params: Some(params),
                semantic: Some(edit_op_id(edit).into()),
            });
            prev = id;
        }
    }

    // Event / anomaly ranges: independent trims off the same parent, then concat.
    // Skip when intent already mapped those edits (avoids double concat).
    let events_already_mapped = intent_ops.iter().any(|e| {
        matches!(
            e,
            SemanticEdit::BuildAnomalyReel { .. } | SemanticEdit::CreateEventClips { .. }
        )
    });
    if !events_already_mapped && !resolved.resolved_events.is_empty() {
        let mut trim_ids = Vec::new();
        for (i, ev) in resolved.resolved_events.iter().enumerate() {
            let id = format!("ev{i}");
            nodes.push(GraphNode {
                id: id.clone(),
                kind: GraphNodeKind::Op,
                operation: Some(op_id::TRANSFORM_TRIM.into()),
                inputs: vec![prev.clone()],
                asset: None,
                name: None,
                params: Some(serde_json::json!({
                    "event_id": ev.event_id,
                    "kind": ev.kind,
                    "subject": ev.subject.as_ref().map(NamespacedId::as_uri),
                    "range": range_json(&ev.range),
                })),
                semantic: Some("event_trim".into()),
            });
            trim_ids.push(id);
        }
        if trim_ids.len() == 1 {
            prev = trim_ids.remove(0);
        } else {
            let id = "events_concat".to_string();
            nodes.push(GraphNode {
                id: id.clone(),
                kind: GraphNodeKind::Op,
                operation: Some(op_id::TIMELINE_CONCAT.into()),
                inputs: trim_ids,
                asset: None,
                name: None,
                params: Some(serde_json::json!({
                    "ranges": ranges_json(&resolved.resolved_events.iter().map(|e| e.range).collect::<Vec<_>>()),
                    "mode": "event_reel",
                })),
                semantic: Some("event_concat".into()),
            });
            prev = id;
        }
    }

    nodes.push(GraphNode {
        id: "out".into(),
        kind: GraphNodeKind::Output,
        operation: None,
        inputs: vec![prev],
        asset: None,
        name: Some(
            resolved
                .intent
                .as_ref()
                .and_then(|i| i.target_output.clone())
                .unwrap_or_else(|| "main".into()),
        ),
        params: None,
        semantic: None,
    });

    RenderGraphIr {
        version: INTEL_RENDER_GRAPH_VERSION,
        final_graph: true,
        vision_index_generation: Some(resolved.vision_index_generation.clone()),
        vision_index_hash: Some(resolved.vision_index_hash.clone()),
        approval,
        assets: vec![GraphAsset {
            id: "in".into(),
            uri: resolved.media.clone(),
            source_hash: Some(resolved.source_hash.clone()),
        }],
        nodes,
        note: Some("Intelligence typed RenderGraphIr — host maps to reelforge::RenderGraph".into()),
    }
}

fn map_edit_to_op(
    edit: &SemanticEdit,
    resolved: &ResolvedEditPlan,
    edit_index: usize,
) -> (&'static str, serde_json::Value) {
    match edit {
        SemanticEdit::BlurSubject { .. }
        | SemanticEdit::BlurEveryoneExcept { .. }
        | SemanticEdit::RedactPii { .. } => (
            op_id::REDACTION_REGION,
            redaction_params(resolved, Some(edit_index)),
        ),
        SemanticEdit::FollowSubject { framing, .. } => {
            let subjects = subject_uris(resolved, Some(edit_index));
            let ranges = ranges_for(resolved, Some(edit_index));
            let mut params = serde_json::json!({
                "framing": framing,
                "subjects": subjects,
                "ranges": ranges_json(&ranges),
            });
            if let Some(crop) = follow_crop_params(resolved, *framing, edit_index)
                && let Some(obj) = params.as_object_mut()
                && let Some(crop_obj) = crop.as_object()
            {
                for (key, value) in crop_obj {
                    obj.insert(key.clone(), value.clone());
                }
            }
            (op_id::TRANSFORM_CROP, params)
        }
        SemanticEdit::BuildSubjectReel { .. }
        | SemanticEdit::BuildMostFrequentSubjectReel { .. } => {
            let subjects = subject_uris(resolved, Some(edit_index));
            let ranges = ranges_for(resolved, Some(edit_index));
            (
                op_id::TIMELINE_CONCAT,
                serde_json::json!({
                    "subjects": subjects,
                    "ranges": ranges_json(&ranges),
                    "mode": "subject_reel",
                }),
            )
        }
        SemanticEdit::BuildAnomalyReel { .. } | SemanticEdit::CreateEventClips { .. } => {
            let events = operation_at(resolved, edit_index)
                .map_or(resolved.resolved_events.len(), |operation| {
                    operation.ranges.len()
                });
            let ranges = ranges_for(resolved, Some(edit_index));
            (
                op_id::TIMELINE_CONCAT,
                serde_json::json!({
                    "events": events,
                    "ranges": ranges_json(&ranges),
                    "mode": "event_reel",
                }),
            )
        }
    }
}

/// Per-edit record. `None` means a plan frozen before operation scope existed.
fn operation_at(resolved: &ResolvedEditPlan, edit_index: usize) -> Option<&ResolvedOperation> {
    resolved
        .operations
        .iter()
        .find(|operation| operation.edit_index == edit_index)
}

/// Subjects this edit may change. An existing record wins even when it is empty.
fn subject_uris(resolved: &ResolvedEditPlan, edit_index: Option<usize>) -> Vec<String> {
    if let Some(edit_index) = edit_index
        && let Some(operation) = operation_at(resolved, edit_index)
    {
        return operation.subjects.clone();
    }
    resolved
        .resolved_subjects
        .iter()
        .map(|subject| subject.id.as_uri())
        .collect()
}

/// Ranges this edit may change. An existing record wins even when it is empty.
fn ranges_for(resolved: &ResolvedEditPlan, edit_index: Option<usize>) -> Vec<MediaRange> {
    if let Some(edit_index) = edit_index
        && let Some(operation) = operation_at(resolved, edit_index)
    {
        return operation.ranges.clone();
    }
    resolved.resolved_ranges.clone()
}

fn id_in_operation(operation: &ResolvedOperation, subject: &NamespacedId) -> bool {
    let uri = subject.as_uri();
    operation.subjects.contains(&uri)
        || (subject.kind == EntityKind::Subject
            && operation.local_subject_ids.contains(&subject.id))
}

fn mask_in_operation(mask: &ResolvedMaskAsset, operation: &ResolvedOperation) -> bool {
    mask.subject
        .as_ref()
        .is_some_and(|subject| id_in_operation(operation, subject))
}

fn mask_count(resolved: &ResolvedEditPlan, edit_index: Option<usize>) -> usize {
    let Some(edit_index) = edit_index else {
        return resolved.resolved_masks.len();
    };
    let Some(operation) = operation_at(resolved, edit_index) else {
        return resolved.resolved_masks.len();
    };
    resolved
        .resolved_masks
        .iter()
        .filter(|mask| mask_in_operation(mask, operation))
        .count()
}

fn redaction_params(resolved: &ResolvedEditPlan, edit_index: Option<usize>) -> serde_json::Value {
    let subjects = subject_uris(resolved, edit_index);
    let ranges = ranges_for(resolved, edit_index);
    serde_json::json!({
        "style": "blur",
        "subjects": subjects,
        "ranges": ranges_json(&ranges),
        "masks": mask_count(resolved, edit_index),
        "privacy": resolved.policy.privacy,
    })
}

fn follow_crop_params(
    resolved: &ResolvedEditPlan,
    framing: FramingPolicy,
    edit_index: usize,
) -> Option<serde_json::Value> {
    let width = resolved.frame_width?;
    let height = resolved.frame_height?;
    let frame = framing::FrameSize::new(width, height)?;
    let boxes = scoped_region_boxes(resolved, edit_index);
    if boxes.is_empty() {
        return None;
    }
    framing::compute_follow_crop(&boxes, framing, frame, framing::FramingOptions::default())
        .ok()
        .map(CropRect::to_params)
}

/// Mask samples and snapshot boxes that belong to this edit.
///
/// A plan with no operation record keeps every box. A record that names no
/// subject yields no boxes, so another edit's geometry cannot move the crop.
fn scoped_region_boxes(resolved: &ResolvedEditPlan, edit_index: usize) -> Vec<RegionSample> {
    let operation = operation_at(resolved, edit_index);
    let mut boxes = Vec::new();
    for mask in &resolved.resolved_masks {
        if let Some(operation) = operation
            && !mask_in_operation(mask, operation)
        {
            continue;
        }
        let Some(artifact) = mask.artifact.as_ref() else {
            continue;
        };
        for region in &artifact.regions {
            if let Some(operation) = operation
                && let Some(subject) = region.subject.as_ref()
                && !id_in_operation(operation, subject)
            {
                continue;
            }
            boxes.push(region.clone());
        }
    }
    if !boxes.is_empty() {
        return boxes;
    }
    let scoped = ranges_for(resolved, Some(edit_index));
    let timescale = scoped
        .first()
        .or(resolved.resolved_ranges.first())
        .map_or(1_000_000_000, |range| range.start.timescale);
    resolved
        .subject_boxes
        .iter()
        .filter(|(id, _)| match operation {
            Some(operation) => operation.local_subject_ids.contains(id),
            None => true,
        })
        .map(|(_, xyxy)| RegionSample {
            at: MediaTime::new(0, timescale.max(1)),
            box_xyxy: *xyxy,
            subject: None,
            confidence: None,
            geometry: None,
        })
        .collect()
}

fn ranges_json(ranges: &[MediaRange]) -> Vec<serde_json::Value> {
    ranges.iter().map(range_json).collect()
}

fn range_json(r: &MediaRange) -> serde_json::Value {
    serde_json::json!({
        "start": { "ticks": r.start.ticks, "timescale": r.start.timescale },
        "end": { "ticks": r.end.ticks, "timescale": r.end.timescale },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{FrequencyMetric, SemanticEdit, SemanticEditPlan};
    use crate::policy::{IntelligencePolicy, UncertaintyPolicy};
    use crate::resolve::{AnalysisSnapshot, SubjectEvidence, resolve_plan};

    fn snap() -> AnalysisSnapshot {
        AnalysisSnapshot {
            media: "cam1".into(),
            source_hash: "src".into(),
            vision_index_generation: "gen-1".into(),
            vision_index_hash: "idx".into(),
            timescale: 1_000_000_000,
            subjects: vec![
                SubjectEvidence {
                    subject_id: 7,
                    label: Some("x".into()),
                    appearance_count: 9,
                    source_ids: vec![1],
                    first_ticks: 0,
                    last_ticks: 5,
                    confidence: Some(0.2),
                    ..SubjectEvidence::default()
                }
                .with_visit(0, 5),
            ],
            anomalies: Vec::new(),
            ..AnalysisSnapshot::default()
        }
    }

    #[test]
    fn graph_has_adapter_and_concat_for_most_frequent() {
        let intent =
            SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
                metric: FrequencyMetric::AppearanceCount,
            });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        let graph = graph_from_resolved(&resolved);
        assert!(graph.final_graph);
        assert!(
            graph
                .nodes
                .iter()
                .any(|n| n.operation.as_deref() == Some(op_id::ADAPTER_SIGHTLOOM))
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|n| n.operation.as_deref() == Some(op_id::TRANSFORM_CONCAT))
        );
        let json = graph.to_json_pretty().unwrap();
        assert!(json.contains("sightloom://gen-1/subjects/7"));
    }

    #[test]
    fn review_policy_requires_approval() {
        let intent =
            SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
                metric: FrequencyMetric::AppearanceCount,
            });
        let mut policy = IntelligencePolicy::default();
        policy.privacy.uncertain_identity = UncertaintyPolicy::Review;
        policy.require_approve_on_review = true;
        let resolved = resolve_plan(&intent, &snap(), policy).unwrap();
        let graph = graph_from_resolved(&resolved);
        assert!(graph.approval.required);
        assert!(!graph.approval.allows_execute());
        let approved = approve(graph.approval.clone(), "operator-1");
        assert!(approved.allows_execute());
        let mut expired = approved.clone();
        expired.expires_at_unix = Some(1);
        assert!(!expired.allows_execute());
        let mut future = approved.clone();
        future.expires_at_unix = Some(i64::MAX);
        assert!(future.allows_execute());
    }

    #[test]
    fn follow_crop_emits_pixel_geometry_from_snapshot_frame() {
        let mut analysis = snap();
        analysis.frame_width = Some(1920);
        analysis.frame_height = Some(1080);
        analysis.subject_boxes = vec![(7, [100.0, 100.0, 200.0, 200.0])];
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::FollowSubject {
            subject: crate::selector::SubjectSelector::SubjectIds { ids: vec![7] },
            framing: crate::edit::FramingPolicy::Tight,
        });
        let resolved = resolve_plan(&intent, &analysis, IntelligencePolicy::default()).unwrap();
        let graph = graph_from_resolved(&resolved);
        let crop = graph
            .nodes
            .iter()
            .find(|n| n.operation.as_deref() == Some(op_id::TRANSFORM_CROP))
            .and_then(|n| n.params.as_ref())
            .expect("crop node");
        assert!(crop.get("w").and_then(serde_json::Value::as_u64).unwrap() >= 100);
        assert!(crop.get("h").and_then(serde_json::Value::as_u64).unwrap() >= 100);
        assert!(crop.get("x").is_some());
        assert!(crop.get("y").is_some());
    }

    fn two_photos() -> AnalysisSnapshot {
        AnalysisSnapshot {
            media: "cam1".into(),
            source_hash: "src".into(),
            vision_index_generation: "gen-1".into(),
            vision_index_hash: "idx".into(),
            timescale: 1_000_000_000,
            frame_width: Some(1920),
            frame_height: Some(1080),
            subjects: vec![
                SubjectEvidence {
                    subject_id: 1,
                    label: Some("near-origin".into()),
                    appearance_count: 1,
                    source_ids: vec![1],
                    confidence: Some(0.95),
                    ..SubjectEvidence::default()
                }
                .with_visit(0, 1_000_000_000),
                SubjectEvidence {
                    subject_id: 2,
                    label: Some("right-side".into()),
                    appearance_count: 1,
                    source_ids: vec![2],
                    confidence: Some(0.95),
                    ..SubjectEvidence::default()
                }
                .with_visit(2_000_000_000, 3_000_000_000),
            ],
            subject_boxes: vec![
                (1, [8.0, 8.0, 32.0, 40.0]),
                (2, [400.0, 180.0, 520.0, 420.0]),
            ],
            ..AnalysisSnapshot::default()
        }
    }

    fn blur_and_follow() -> SemanticEditPlan {
        SemanticEditPlan::new("cam1")
            .with_edit(SemanticEdit::BlurSubject {
                subject: crate::selector::SubjectSelector::SubjectIds { ids: vec![1] },
            })
            .with_edit(SemanticEdit::FollowSubject {
                subject: crate::selector::SubjectSelector::SubjectIds { ids: vec![2] },
                framing: FramingPolicy::Tight,
            })
    }

    fn node_params<'a>(graph: &'a RenderGraphIr, operation: &str) -> &'a serde_json::Value {
        graph
            .nodes
            .iter()
            .find(|node| node.operation.as_deref() == Some(operation))
            .and_then(|node| node.params.as_ref())
            .unwrap_or_else(|| panic!("missing {operation}"))
    }

    fn string_list(value: &serde_json::Value, key: &str) -> Vec<String> {
        value[key]
            .as_array()
            .unwrap_or_else(|| panic!("missing {key}"))
            .iter()
            .map(|item| item.as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn range_ticks(value: &serde_json::Value) -> Vec<(i64, i64)> {
        value["ranges"]
            .as_array()
            .expect("ranges")
            .iter()
            .map(|range| {
                (
                    range["start"]["ticks"].as_i64().unwrap_or_default(),
                    range["end"]["ticks"].as_i64().unwrap_or_default(),
                )
            })
            .collect()
    }

    #[test]
    fn blur_of_one_subject_does_not_receive_the_follow_target() {
        let resolved = resolve_plan(
            &blur_and_follow(),
            &two_photos(),
            IntelligencePolicy::default(),
        )
        .unwrap();
        assert_eq!(
            resolved
                .resolved_subjects
                .iter()
                .find(|subject| subject.local_subject_id == Some(1))
                .unwrap()
                .source_ids,
            vec![1]
        );
        assert_eq!(
            resolved
                .resolved_subjects
                .iter()
                .find(|subject| subject.local_subject_id == Some(2))
                .unwrap()
                .source_ids,
            vec![2]
        );
        let graph = graph_from_resolved(&resolved);
        let blur = node_params(&graph, op_id::REDACTION_REGION);
        let follow = node_params(&graph, op_id::TRANSFORM_CROP);
        let adapter = node_params(&graph, op_id::ADAPTER_SIGHTLOOM);
        assert_eq!(
            string_list(blur, "subjects"),
            vec!["sightloom://gen-1/subjects/1".to_string()]
        );
        assert_eq!(range_ticks(blur), vec![(0, 1_000_000_000)]);
        assert_eq!(
            string_list(follow, "subjects"),
            vec!["sightloom://gen-1/subjects/2".to_string()]
        );
        assert_eq!(range_ticks(follow), vec![(2_000_000_000, 3_000_000_000)]);
        let crop_x = follow["x"].as_u64().expect("scoped crop x");
        assert!(
            crop_x >= 200,
            "follow crop must sit on subject 2, x={crop_x}"
        );
        assert_eq!(string_list(adapter, "subjects").len(), 2);
        assert_eq!(range_ticks(adapter).len(), 2);

        let mut legacy_json = serde_json::to_value(&resolved).unwrap();
        legacy_json.as_object_mut().unwrap().remove("operations");
        let legacy: ResolvedEditPlan = serde_json::from_value(legacy_json).unwrap();
        assert!(legacy.operations.is_empty());
        let legacy_graph = graph_from_resolved(&legacy);
        let legacy_blur = node_params(&legacy_graph, op_id::REDACTION_REGION);
        let legacy_follow = node_params(&legacy_graph, op_id::TRANSFORM_CROP);
        assert_eq!(string_list(legacy_blur, "subjects").len(), 2);
        assert_eq!(range_ticks(legacy_follow).len(), 2);
        let legacy_x = legacy_follow["x"].as_u64().expect("legacy crop x");
        assert!(
            legacy_x < crop_x,
            "union crop x={legacy_x} must stay left of the scoped crop x={crop_x}"
        );
    }

    #[test]
    fn skipped_subject_stays_off_the_redaction_node() {
        let mut policy = IntelligencePolicy::default();
        policy.privacy.missing_mask = crate::policy::MissingMaskAction::Skip;
        let mut analysis = two_photos();
        analysis.subject_boxes.clear();
        analysis.subjects.retain(|subject| subject.subject_id == 1);
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: crate::selector::SubjectSelector::SubjectIds { ids: vec![1] },
        });
        let resolved = resolve_plan(&intent, &analysis, policy).unwrap();
        assert!(
            resolved
                .decisions
                .iter()
                .any(|decision| decision.code == "missing_mask_skip")
        );
        assert!(resolved.operations[0].subjects.is_empty());
        assert_eq!(resolved.resolved_subjects.len(), 1);
        let graph = graph_from_resolved(&resolved);
        let blur = node_params(&graph, op_id::REDACTION_REGION);
        assert!(string_list(blur, "subjects").is_empty());
        let adapter = node_params(&graph, op_id::ADAPTER_SIGHTLOOM);
        assert_eq!(
            string_list(adapter, "subjects"),
            vec!["sightloom://gen-1/subjects/1".to_string()]
        );
    }

    #[test]
    fn missing_mask_review_requires_approval_only_without_evidence() {
        let mut policy = IntelligencePolicy::default();
        policy.privacy.missing_mask = crate::policy::MissingMaskAction::Review;
        policy.require_approve_on_review = true;
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: crate::selector::SubjectSelector::SubjectIds { ids: vec![1] },
        });
        let mut bare = two_photos();
        bare.subject_boxes.clear();
        bare.subjects.retain(|subject| subject.subject_id == 1);
        let missing = resolve_plan(&intent, &bare, policy.clone()).unwrap();
        let approval = approval_for_resolved(&missing, &missing.policy);
        assert!(approval.required);
        assert!(
            approval
                .reasons
                .iter()
                .any(|reason| reason == "missing_mask=review")
        );

        let mut with_box = bare;
        with_box.subject_boxes = vec![(1, [10.0, 10.0, 40.0, 40.0])];
        let present = resolve_plan(&intent, &with_box, policy).unwrap();
        let approval = approval_for_resolved(&present, &present.policy);
        assert!(!approval.required);
        assert!(
            !approval
                .reasons
                .iter()
                .any(|reason| reason == "missing_mask=review")
        );
    }
}
