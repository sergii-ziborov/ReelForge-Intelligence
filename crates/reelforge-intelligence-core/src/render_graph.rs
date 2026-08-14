//! Typed RenderGraph IR for Intelligence → ReelForge handoff.
//!
//! Aligns with ReelForge operation ids (`rf.redaction.region`, `rf.transform.trim`, …)
//! without hard-linking the full `reelforge-render-graph` crate. Hosts can
//! deserialize JSON into ReelForge types or map nodes explicitly.

use crate::edit::SemanticEdit;
use crate::ids::NamespacedId;
use crate::ops::edit_op_id;
use crate::policy::{IntelligencePolicy, UncertaintyPolicy};
use crate::resolved::ResolvedEditPlan;
use crate::time::MediaRange;
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
    /// Concatenate reels.
    pub const TRANSFORM_CONCAT: &str = "rf.transform.concat";
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
}

impl ApprovalRecord {
    /// True when execution is allowed.
    #[must_use]
    pub const fn allows_execute(&self) -> bool {
        !self.required || self.approved
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
    if matches!(
        policy.privacy.missing_mask,
        crate::policy::MissingMaskAction::Review
    ) {
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
    }
}

/// Mark a record approved by operator.
#[must_use]
pub fn approve(mut record: ApprovalRecord, by: impl Into<String>) -> ApprovalRecord {
    record.approved = true;
    record.approved_by = Some(by.into());
    record
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
                params: Some(redaction_params(resolved)),
                semantic: Some("blur".into()),
            });
            prev = id;
        }
    } else {
        for (i, edit) in intent_ops.iter().enumerate() {
            let id = format!("e{i}");
            let (operation, params) = map_edit_to_op(edit, resolved);
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

    // Anomaly / event trims as concat chain
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
            semantic: Some("build_anomaly_reel".into()),
        });
        prev = id;
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
) -> (&'static str, serde_json::Value) {
    match edit {
        SemanticEdit::BlurSubject { .. } | SemanticEdit::BlurEveryoneExcept { .. } => {
            (op_id::REDACTION_REGION, redaction_params(resolved))
        }
        SemanticEdit::FollowSubject { framing, .. } => (
            op_id::TRANSFORM_CROP,
            serde_json::json!({
                "framing": framing,
                "subjects": resolved.resolved_subjects.iter().map(|s| s.id.as_uri()).collect::<Vec<_>>(),
                "ranges": ranges_json(&resolved.resolved_ranges),
            }),
        ),
        SemanticEdit::BuildSubjectReel { .. }
        | SemanticEdit::BuildMostFrequentSubjectReel { .. } => (
            op_id::TRANSFORM_CONCAT,
            serde_json::json!({
                "subjects": resolved.resolved_subjects.iter().map(|s| s.id.as_uri()).collect::<Vec<_>>(),
                "ranges": ranges_json(&resolved.resolved_ranges),
                "mode": "subject_reel",
            }),
        ),
        SemanticEdit::BuildAnomalyReel { .. } | SemanticEdit::CreateEventClips { .. } => (
            op_id::TRANSFORM_TRIM,
            serde_json::json!({
                "events": resolved.resolved_events.len(),
                "ranges": ranges_json(&resolved.resolved_ranges),
            }),
        ),
    }
}

fn redaction_params(resolved: &ResolvedEditPlan) -> serde_json::Value {
    serde_json::json!({
        "style": "blur",
        "subjects": resolved.resolved_subjects.iter().map(|s| s.id.as_uri()).collect::<Vec<_>>(),
        "ranges": ranges_json(&resolved.resolved_ranges),
        "masks": resolved.resolved_masks.len(),
        "privacy": resolved.policy.privacy,
    })
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
            subjects: vec![SubjectEvidence {
                subject_id: 7,
                label: Some("x".into()),
                appearance_count: 9,
                source_ids: vec![1],
                first_ticks: 0,
                last_ticks: 5,
                confidence: Some(0.2),
            }],
            anomalies: Vec::new(),
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
    }
}
