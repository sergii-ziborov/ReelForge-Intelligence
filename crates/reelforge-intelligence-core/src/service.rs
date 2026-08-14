//! P4 Intelligence contract. MCP must only dispatch these methods.

use crate::bridge::{
    BridgeOptions, BridgeResult, bridge_resolved, bridge_resolved_for_execute, bridge_to_reelforge,
};
use crate::catalog::{HostCatalog, MediaInspection, SceneHit, SubjectHit};
use crate::compile::{
    CompileReport, approve_compile, attach_reelforge_bridge, compile_and_bridge, compile_resolved,
};
use crate::edit::SemanticEditPlan;
use crate::error::{IntelError, Result};
use crate::mask::{MaskArtifact, MaskRequest};
use crate::ops::{IntelOperation, edit_op_id, operations, schemas};
use crate::policy::{IntelligencePolicy, UncertaintyPolicy};
use crate::provider::AnalysisProvider;
use crate::render_graph::RenderGraphIr;
use crate::resolve::{AnalysisSnapshot, resolve_plan};
use crate::resolved::{ResolvedEditPlan, ResolvedMaskAsset};
use crate::selector::SubjectSelector;
use serde::{Deserialize, Serialize};

/// Work the **host** (`ReelForge` / Capture) must execute. Not done here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HostRequest {
    /// Sample one preview frame.
    PreviewFrame {
        /// Media key.
        media: String,
        /// Time seconds.
        t_secs: f64,
    },
    /// Full render of a compiled graph (host runs `ReelForge`).
    Render {
        /// Media key.
        media: String,
        /// Optional output path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        /// When set, host must use this frozen plan (not re-query VisionIndex).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_plan: Option<Box<ResolvedEditPlan>>,
        /// Live ReelForge `RenderGraph` JSON (after bridge).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reelforge_graph_json: Option<String>,
    },
}

/// In-memory Intelligence service. No FFmpeg. SightLoom evidence via host snapshot.
#[derive(Debug, Clone, Default)]
pub struct IntelligenceService {
    /// Host-provided catalogs keyed by media id.
    catalog: HostCatalog,
}

impl IntelligenceService {
    /// Empty service.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject host catalog (inspect / scenes / subjects).
    #[must_use]
    pub fn with_catalog(mut self, catalog: HostCatalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// List semantic operations.
    #[must_use]
    pub fn operations(&self) -> &'static [IntelOperation] {
        operations()
    }

    /// Parameter schemas keyed by operation id.
    #[must_use]
    pub fn schemas(&self) -> serde_json::Value {
        schemas()
    }

    /// Inspect media from the host catalog.
    ///
    /// # Errors
    ///
    /// Unknown media.
    pub fn inspect_media(&self, media: &str) -> Result<MediaInspection> {
        match &self.catalog.media {
            Some(m) if m.media == media || media.is_empty() => Ok(m.clone()),
            Some(m) => Err(IntelError::message(format!(
                "inspect_media: catalog is for {}, not {media}",
                m.media
            ))),
            None => Err(IntelError::message(
                "inspect_media: host catalog has no media inspection",
            )),
        }
    }

    /// Scene list from the host catalog.
    #[must_use]
    pub fn catalog_scenes(&self) -> &[SceneHit] {
        &self.catalog.scenes
    }

    /// Subject list from the host catalog.
    #[must_use]
    pub fn catalog_subjects(&self) -> &[SubjectHit] {
        &self.catalog.subjects
    }

    /// Validate a plan (version, media, edits present).
    ///
    /// # Errors
    ///
    /// Invalid plan.
    pub fn check_plan(&self, plan: &SemanticEditPlan) -> Result<()> {
        plan.validate()?;
        if plan.edits.is_empty() {
            return Err(IntelError::message("check_plan: plan has no edits"));
        }
        Ok(())
    }

    /// Drop empty selectors / clamp pads. Does not invent subjects.
    #[must_use]
    pub fn normalize_plan(&self, mut plan: SemanticEditPlan) -> SemanticEditPlan {
        if plan.version == 0 {
            plan.version = crate::edit::SEMANTIC_EDIT_PLAN_VERSION;
        }
        for edit in &mut plan.edits {
            if let crate::edit::SemanticEdit::CreateEventClips {
                pad_before_secs,
                pad_after_secs,
                ..
            } = edit
            {
                if !pad_before_secs.is_finite() || *pad_before_secs < 0.0 {
                    *pad_before_secs = 1.0;
                }
                if !pad_after_secs.is_finite() || *pad_after_secs < 0.0 {
                    *pad_after_secs = 1.0;
                }
            }
        }
        plan
    }

    /// Conservative repairs.
    #[must_use]
    pub fn repair_plan(&self, mut plan: SemanticEditPlan) -> SemanticEditPlan {
        plan = self.normalize_plan(plan);
        if matches!(
            plan.policy.privacy.uncertain_identity,
            UncertaintyPolicy::Allow
        ) && plan
            .edits
            .iter()
            .any(|e| matches!(e, crate::edit::SemanticEdit::BlurEveryoneExcept { .. }))
        {
            plan.policy.privacy.uncertain_identity = UncertaintyPolicy::Blur;
        }
        plan
    }

    /// Freeze intent against a host-provided SightLoom analysis snapshot.
    ///
    /// # Errors
    ///
    /// Resolve / validate failures.
    pub fn resolve_plan(
        &self,
        intent: &SemanticEditPlan,
        analysis: &AnalysisSnapshot,
    ) -> Result<ResolvedEditPlan> {
        let intent = self.repair_plan(intent.clone());
        self.check_plan(&intent)?;
        let policy = intent.policy.clone();
        resolve_plan(&intent, analysis, policy)
    }

    /// **Final** compile: only frozen plans.
    ///
    /// # Errors
    ///
    /// Invalid freeze / compile failure.
    pub fn compile_resolved(&self, resolved: &ResolvedEditPlan) -> Result<CompileReport> {
        compile_resolved(resolved)
    }

    /// Preview compile from raw intent (not final — may change if VisionIndex updates).
    ///
    /// # Errors
    ///
    /// Failed check after repair.
    pub fn compile_plan(&self, plan: &SemanticEditPlan) -> Result<CompileReport> {
        let plan = self.repair_plan(plan.clone());
        self.check_plan(&plan)?;
        Ok(build_preview_report(&plan))
    }

    /// Human explanation of the plan.
    #[must_use]
    pub fn explain_plan(&self, plan: &SemanticEditPlan) -> String {
        let mut lines = vec![
            format!("media: {}", plan.media),
            format!("edits: {}", plan.edits.len()),
            format!("uncertain: {:?}", plan.policy.privacy.uncertain_identity),
        ];
        for (i, e) in plan.edits.iter().enumerate() {
            lines.push(format!("  [{i}] {}", edit_op_id(e)));
        }
        lines.join("\n")
    }

    /// Ask the host to preview a frame (Intelligence does not decode).
    ///
    /// # Errors
    ///
    /// Invalid plan.
    pub fn preview_frame(&self, plan: &SemanticEditPlan, t_secs: f64) -> Result<HostRequest> {
        plan.validate()?;
        if !t_secs.is_finite() || t_secs < 0.0 {
            return Err(IntelError::message("preview_frame: t_secs must be >= 0"));
        }
        Ok(HostRequest::PreviewFrame {
            media: plan.media.clone(),
            t_secs,
        })
    }

    /// Preview render request (no freeze). Prefer [`Self::render_resolved`].
    ///
    /// # Errors
    ///
    /// Compile failure.
    pub fn render(&self, plan: &SemanticEditPlan) -> Result<HostRequest> {
        let report = self.compile_plan(plan)?;
        if !report.ok {
            return Err(IntelError::message("render: compile_plan failed"));
        }
        Ok(HostRequest::Render {
            media: plan.media.clone(),
            output: plan.target_output.clone(),
            resolved_plan: None,
            reelforge_graph_json: None,
        })
    }

    /// Final render request bound to a frozen resolution.
    ///
    /// Fails when privacy policy requires approval and it is not granted —
    /// call [`Self::approve_and_render`] after operator sign-off.
    ///
    /// # Errors
    ///
    /// Compile failure or approval gate.
    pub fn render_resolved(&self, resolved: &ResolvedEditPlan) -> Result<HostRequest> {
        let report = self.compile_resolved(resolved)?;
        if !report.ok || !report.final_graph {
            return Err(IntelError::message("render_resolved: compile failed"));
        }
        if !report.allows_execute() {
            return Err(IntelError::message(format!(
                "render_resolved: approval required ({})",
                report.approval.reasons.join(", ")
            )));
        }
        let output = resolved
            .intent
            .as_ref()
            .and_then(|i| i.target_output.clone());
        let reelforge_graph_json = bridge_resolved_for_execute(resolved, output.clone())
            .ok()
            .map(|b| b.graph_json);
        Ok(HostRequest::Render {
            media: resolved.media.clone(),
            output,
            resolved_plan: Some(Box::new(resolved.clone())),
            reelforge_graph_json,
        })
    }

    /// Approve a final compile report (operator gate).
    #[must_use]
    pub fn approve_report(&self, report: CompileReport, by: impl Into<String>) -> CompileReport {
        approve_compile(report, by)
    }

    /// Compile + approve + host render request in one call.
    ///
    /// Bridges to a live ReelForge graph when possible.
    ///
    /// # Errors
    ///
    /// Compile failure.
    pub fn approve_and_render(
        &self,
        resolved: &ResolvedEditPlan,
        by: impl Into<String>,
    ) -> Result<(CompileReport, HostRequest)> {
        let report = self.compile_resolved(resolved)?;
        let mut report = approve_compile(report, by);
        if !report.allows_execute() {
            return Err(IntelError::message("approve_and_render: still blocked"));
        }
        let output = resolved
            .intent
            .as_ref()
            .and_then(|i| i.target_output.clone());
        match bridge_resolved_for_execute(resolved, output.clone()) {
            Ok(bridged) => {
                report.reelforge_graph_json = Some(bridged.graph_json.clone());
                report.bridge_warnings = bridged.warnings;
                let req = HostRequest::Render {
                    media: resolved.media.clone(),
                    output,
                    resolved_plan: Some(Box::new(resolved.clone())),
                    reelforge_graph_json: Some(bridged.graph_json),
                };
                Ok((report, req))
            }
            Err(e) => {
                report.bridge_warnings.push(format!("bridge: {e}"));
                let req = HostRequest::Render {
                    media: resolved.media.clone(),
                    output,
                    resolved_plan: Some(Box::new(resolved.clone())),
                    reelforge_graph_json: report.reelforge_graph_json.clone(),
                };
                Ok((report, req))
            }
        }
    }

    /// Bridge a typed IR into a live ReelForge graph (+ optional schedule).
    ///
    /// # Errors
    ///
    /// Bridge / validate / schedule failure.
    pub fn bridge_graph(&self, ir: &RenderGraphIr, opts: &BridgeOptions) -> Result<BridgeResult> {
        bridge_to_reelforge(ir, opts)
    }

    /// Bridge from frozen plan (fills `MaskTimeline` from resolved mask artifacts).
    ///
    /// # Errors
    ///
    /// Bridge failure.
    pub fn bridge_resolved(
        &self,
        resolved: &ResolvedEditPlan,
        opts: &BridgeOptions,
    ) -> Result<BridgeResult> {
        bridge_resolved(resolved, opts)
    }

    /// Compile frozen plan and bridge to ReelForge in one step.
    ///
    /// # Errors
    ///
    /// Compile or bridge failure.
    pub fn compile_and_bridge(
        &self,
        resolved: &ResolvedEditPlan,
        opts: &BridgeOptions,
    ) -> Result<(CompileReport, BridgeResult)> {
        compile_and_bridge(resolved, opts)
    }

    /// Attach a ReelForge graph onto an existing compile report.
    ///
    /// # Errors
    ///
    /// Bridge failure.
    pub fn attach_bridge(
        &self,
        report: CompileReport,
        opts: &BridgeOptions,
    ) -> Result<CompileReport> {
        attach_reelforge_bridge(report, opts)
    }

    /// Full pipeline: resolve → final compile.
    ///
    /// # Errors
    ///
    /// Resolve / compile.
    pub fn resolve_and_compile(
        &self,
        intent: &SemanticEditPlan,
        analysis: &AnalysisSnapshot,
    ) -> Result<(ResolvedEditPlan, CompileReport)> {
        let resolved = self.resolve_plan(intent, analysis)?;
        let report = self.compile_resolved(&resolved)?;
        Ok((resolved, report))
    }

    /// Materialize masks via any [`AnalysisProvider`] (preview vs final fidelity).
    ///
    /// # Errors
    ///
    /// Provider errors.
    pub fn materialize_masks<P: AnalysisProvider>(
        &self,
        provider: &P,
        request: &MaskRequest,
    ) -> Result<MaskArtifact> {
        provider.materialize_masks(request)
    }

    /// Attach a mask artifact onto a frozen plan (host step after resolve).
    #[must_use]
    pub fn with_mask_artifact(
        &self,
        mut resolved: ResolvedEditPlan,
        artifact: MaskArtifact,
    ) -> ResolvedEditPlan {
        let subjects: Vec<_> = resolved
            .resolved_subjects
            .iter()
            .map(|s| s.id.clone())
            .collect();
        resolved.resolved_masks.push(ResolvedMaskAsset {
            mask_id: subjects.first().cloned(),
            mask_ref: None,
            subject: subjects.first().cloned(),
            range: resolved.resolved_ranges.first().copied(),
            fidelity: artifact.fidelity,
            artifact: Some(artifact),
        });
        resolved
    }
}

fn build_preview_report(plan: &SemanticEditPlan) -> CompileReport {
    let mut report = CompileReport::success();
    report.final_graph = false;
    report.providers_used.push("intelligence-preview".into());
    report.warnings.push(crate::compile::CompileWarning {
        message: "preview compile only — freeze via resolve_plan before final render".into(),
        edit_index: None,
    });
    let mut nodes = vec![serde_json::json!({
        "id": "src",
        "kind": "source",
        "asset": "in"
    })];
    let mut prev = "src".to_string();
    for (i, edit) in plan.edits.iter().enumerate() {
        let id = format!("e{i}");
        let op = match edit {
            crate::edit::SemanticEdit::FollowSubject { .. } => "rf.transform.crop",
            crate::edit::SemanticEdit::CreateEventClips { .. }
            | crate::edit::SemanticEdit::BuildSubjectReel { .. }
            | crate::edit::SemanticEdit::BuildMostFrequentSubjectReel { .. }
            | crate::edit::SemanticEdit::BuildAnomalyReel { .. } => "rf.transform.trim",
            _ => "rf.redaction.region",
        };
        nodes.push(serde_json::json!({
            "id": id,
            "kind": "op",
            "operation": op,
            "inputs": [prev],
            "semantic": edit_op_id(edit),
        }));
        prev = format!("e{i}");
    }
    nodes.push(serde_json::json!({
        "id": "out",
        "kind": "output",
        "name": plan.target_output.clone().unwrap_or_else(|| "main".into()),
        "inputs": [prev]
    }));
    let graph = serde_json::json!({
        "version": 1,
        "final": false,
        "assets": [{ "id": "in", "uri": plan.media }],
        "nodes": nodes,
        "note": "PREVIEW only — VisionIndex may change; use ResolvedEditPlan for final"
    });
    report.render_graph_json = Some(graph.to_string());
    if plan.edits.iter().any(|e| {
        matches!(
            e,
            crate::edit::SemanticEdit::BlurSubject {
                subject: SubjectSelector::FramePick { .. }
            } | crate::edit::SemanticEdit::BlurEveryoneExcept {
                allowed: SubjectSelector::FramePick { .. },
                ..
            }
        )
    }) {
        report.warnings.push(crate::compile::CompileWarning {
            message: "frame_pick requires host SightLoom materialization".into(),
            edit_index: Some(0),
        });
    }
    let _ = IntelligencePolicy::default();
    report
}
