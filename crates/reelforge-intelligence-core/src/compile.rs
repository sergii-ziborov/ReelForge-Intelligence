//! Compile reports: **only** [`crate::ResolvedEditPlan`] → final RenderGraph stub.
//!
//! Compiling raw [`crate::SemanticEditPlan`] without freeze is for **preview**
//! only and is marked non-final.

use crate::resolved::ResolvedEditPlan;
use serde::{Deserialize, Serialize};

/// Host-provided analysis capability (`SightLoom`, activity, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisProvider {
    /// Provider id (`sightloom`, `capture_events`, …).
    pub id: String,
    /// Human label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Capability tags.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Non-fatal compile note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileWarning {
    /// Message.
    pub message: String,
    /// Related edit index if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_index: Option<usize>,
}

/// Result of compiling toward a `RenderGraph`.
///
/// The actual graph body lives in `ReelForge`; this report is the Intelligence
/// side contract. Prefer [`compile_resolved`] for final renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CompileReport {
    /// Whether compilation succeeded.
    pub ok: bool,
    /// When true, graph is final (from frozen resolution). When false, preview-only.
    #[serde(default)]
    pub final_graph: bool,
    /// Warnings.
    #[serde(default)]
    pub warnings: Vec<CompileWarning>,
    /// Opaque `RenderGraph` JSON (optional until executor lands).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_graph_json: Option<String>,
    /// Providers consulted.
    #[serde(default)]
    pub providers_used: Vec<String>,
    /// Frozen generation echoed for audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_index_generation: Option<String>,
    /// Frozen index hash echoed for audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_index_hash: Option<String>,
}

impl CompileReport {
    /// Successful empty report.
    #[must_use]
    pub fn success() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }

    /// Failure with a warning-as-error message.
    #[must_use]
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            warnings: vec![CompileWarning {
                message: message.into(),
                edit_index: None,
            }],
            ..Self::default()
        }
    }
}

/// Compile a **frozen** plan into a ReelForge-shaped RenderGraph JSON stub.
///
/// # Errors
///
/// Invalid resolved plan.
pub fn compile_resolved(resolved: &ResolvedEditPlan) -> crate::Result<CompileReport> {
    resolved.validate()?;
    let mut report = CompileReport::success();
    report.final_graph = true;
    report.providers_used.push("intelligence-resolved".into());
    report.vision_index_generation = Some(resolved.vision_index_generation.clone());
    report.vision_index_hash = Some(resolved.vision_index_hash.clone());

    let mut nodes = vec![serde_json::json!({
        "id": "src",
        "kind": "source",
        "asset": "in",
        "source_hash": resolved.source_hash,
    })];
    let mut prev = "src".to_string();

    if !resolved.resolved_subjects.is_empty()
        || !resolved.resolved_ranges.is_empty()
        || !resolved.resolved_masks.is_empty()
    {
        let id = "redact_or_reel".to_string();
        nodes.push(serde_json::json!({
            "id": id,
            "kind": "op",
            "operation": "rf.redaction.region",
            "inputs": [prev],
            "subjects": resolved.resolved_subjects.iter().map(|s| s.subject_id).collect::<Vec<_>>(),
            "ranges": resolved.resolved_ranges.len(),
            "masks": resolved.resolved_masks.len(),
            "note": "host binds TrackTimeline / MaskTimeline from frozen resolution",
        }));
        prev = "redact_or_reel".into();
    }

    for (i, ev) in resolved.resolved_events.iter().enumerate() {
        let id = format!("ev{i}");
        nodes.push(serde_json::json!({
            "id": id,
            "kind": "op",
            "operation": "rf.transform.trim",
            "inputs": [prev],
            "event_id": ev.event_id,
            "kind": ev.kind,
        }));
        prev = format!("ev{i}");
    }

    nodes.push(serde_json::json!({
        "id": "out",
        "kind": "output",
        "name": "main",
        "inputs": [prev],
    }));

    let graph = serde_json::json!({
        "version": 1,
        "final": true,
        "vision_index_generation": resolved.vision_index_generation,
        "vision_index_hash": resolved.vision_index_hash,
        "assets": [{ "id": "in", "uri": resolved.media, "source_hash": resolved.source_hash }],
        "nodes": nodes,
        "decisions": resolved.decisions,
        "warnings": resolved.warnings,
        "note": "Final compile from ResolvedEditPlan — host executes via ReelForge",
    });
    report.render_graph_json = Some(graph.to_string());
    for w in &resolved.warnings {
        report.warnings.push(CompileWarning {
            message: w.message.clone(),
            edit_index: w.edit_index,
        });
    }
    Ok(report)
}
