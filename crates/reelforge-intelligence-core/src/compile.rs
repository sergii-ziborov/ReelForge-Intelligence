//! Compile reports: **only** [`crate::ResolvedEditPlan`] → final RenderGraph.
//!
//! Compiling raw [`crate::SemanticEditPlan`] without freeze is for **preview**
//! only and is marked non-final.

use crate::bridge::{BridgeOptions, BridgeResult, bridge_resolved, bridge_to_reelforge};
use crate::error::IntelError;
use crate::render_graph::{
    ApprovalRecord, RenderGraphIr, approval_for_resolved, approve, graph_from_resolved,
};
use crate::resolved::ResolvedEditPlan;
use serde::{Deserialize, Serialize};

// Analysis provider **trait** lives in `provider.rs`.
// Re-export info type for catalogs under the old name for compatibility.
pub use crate::provider::AnalysisProviderInfo as AnalysisProvider;

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
/// Prefer [`compile_resolved`] for final renders.
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
    /// Opaque `RenderGraph` JSON (serialized [`RenderGraphIr`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_graph_json: Option<String>,
    /// Typed graph when final compile succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_graph: Option<RenderGraphIr>,
    /// Providers consulted.
    #[serde(default)]
    pub providers_used: Vec<String>,
    /// Frozen generation echoed for audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_index_generation: Option<String>,
    /// Frozen index hash echoed for audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_index_hash: Option<String>,
    /// Approval gate (final only).
    #[serde(default)]
    pub approval: ApprovalRecord,
    /// Live ReelForge graph JSON (after optional bridge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reelforge_graph_json: Option<String>,
    /// Bridge warnings when reelforge graph was produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bridge_warnings: Vec<String>,
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

    /// True when host may execute (final + approval ok).
    #[must_use]
    pub fn allows_execute(&self) -> bool {
        self.ok && self.final_graph && self.approval.allows_execute()
    }
}

/// Compile a **frozen** plan into a typed + JSON RenderGraph.
///
/// # Errors
///
/// Invalid resolved plan.
pub fn compile_resolved(resolved: &ResolvedEditPlan) -> crate::Result<CompileReport> {
    resolved.validate()?;
    let graph = graph_from_resolved(resolved);
    let mut report = CompileReport::success();
    report.final_graph = true;
    report.providers_used.push("intelligence-resolved".into());
    report.vision_index_generation = Some(resolved.vision_index_generation.clone());
    report.vision_index_hash = Some(resolved.vision_index_hash.clone());
    report.approval = graph.approval.clone();
    report.render_graph_json = Some(
        graph
            .to_json()
            .map_err(|e| IntelError::message(e.to_string()))?,
    );
    report.render_graph = Some(graph);
    for w in &resolved.warnings {
        report.warnings.push(CompileWarning {
            message: w.message.clone(),
            edit_index: w.edit_index,
        });
    }
    if report.approval.required && !report.approval.approved {
        report.warnings.push(CompileWarning {
            message: format!(
                "approval required before execute: {}",
                report.approval.reasons.join(", ")
            ),
            edit_index: None,
        });
    }
    Ok(report)
}

/// Attach operator approval onto a final compile report (and embedded graph).
#[must_use]
pub fn approve_compile(mut report: CompileReport, by: impl Into<String>) -> CompileReport {
    let by = by.into();
    report.approval = approve(report.approval, by.clone());
    if let Some(ref mut g) = report.render_graph {
        g.approval = report.approval.clone();
        if let Ok(json) = g.to_json() {
            report.render_graph_json = Some(json);
        }
    }
    report
}

/// Recompute approval only (without rebuilding nodes).
#[must_use]
pub fn approval_status(resolved: &ResolvedEditPlan) -> ApprovalRecord {
    approval_for_resolved(resolved, &resolved.policy)
}

/// Attach a live ReelForge graph onto a final compile report.
///
/// # Errors
///
/// Bridge / validate / schedule failure.
pub fn attach_reelforge_bridge(
    mut report: CompileReport,
    opts: &BridgeOptions,
) -> crate::Result<CompileReport> {
    let Some(ref ir) = report.render_graph else {
        return Err(IntelError::message(
            "attach_reelforge_bridge: compile report has no typed RenderGraphIr",
        ));
    };
    let bridged = bridge_to_reelforge(ir, opts)?;
    report.reelforge_graph_json = Some(bridged.graph_json);
    report.bridge_warnings = bridged.warnings;
    Ok(report)
}

/// Compile frozen plan and immediately bridge to ReelForge.
///
/// Injects `MaskTimeline` samples from `resolved.resolved_masks` when present.
///
/// # Errors
///
/// Compile or bridge failure.
pub fn compile_and_bridge(
    resolved: &ResolvedEditPlan,
    opts: &BridgeOptions,
) -> crate::Result<(CompileReport, BridgeResult)> {
    let mut report = compile_resolved(resolved)?;
    let bridged = bridge_resolved(resolved, opts)?;
    report.reelforge_graph_json = Some(bridged.graph_json.clone());
    report.bridge_warnings.clone_from(&bridged.warnings);
    Ok((report, bridged))
}
