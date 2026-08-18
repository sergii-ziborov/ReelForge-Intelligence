//! Map Intelligence [`RenderGraphIr`] → live `reelforge_render_graph::RenderGraph`.
//!
//! Structural handoff only: no FFmpeg, no encode. Hosts call
//! `schedule_graph` / `run_render_graph` after this bridge.

use crate::error::{IntelError, Result};
use crate::mask_timeline::{mask_timeline_from_resolved, timeline_has_samples};
use crate::render_graph::{GraphNode, GraphNodeKind, RenderGraphIr, graph_from_resolved, op_id};
use crate::resolved::ResolvedEditPlan;
use reelforge_core::Rgba8;
use reelforge_render_graph::{
    BackendClass, ExecutionPlan, GraphOutput, MaskTimeline, MediaAsset, MediaAssetId,
    MediaContract, NodeId, OperationDescriptor, OperationId, OperationRegistry,
    RENDER_GRAPH_VERSION, RedactionStyle, RegionRedaction, RenderGraph, RenderNode, RenderNodeKind,
    SemVer, schedule_graph,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// How Intelligence materializes fused redaction (gaussian is recoverable).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionKind {
    /// Gaussian blur (legacy / preview). Recoverable — not anonymity.
    #[default]
    Gaussian,
    /// Pixelate (preferred privacy default for hosts).
    Pixelate,
    /// Solid fill.
    Solid,
}

impl RedactionKind {
    /// Parse `gaussian` / `pixelate` / `solid` (aliases: `blur`, `mosaic`, `black`).
    ///
    /// # Errors
    ///
    /// Unknown token.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "gaussian" | "blur" => Ok(Self::Gaussian),
            "pixelate" | "mosaic" => Ok(Self::Pixelate),
            "solid" | "black" => Ok(Self::Solid),
            other => Err(IntelError::message(format!(
                "unknown redaction style `{other}` (gaussian|pixelate|solid)"
            ))),
        }
    }
}

/// Options for IR → ReelForge conversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeOptions {
    /// Output URI for the main graph output (e.g. `out.mp4`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_uri: Option<String>,
    /// Gaussian sigma when materializing empty redaction nodes.
    #[serde(default = "default_sigma")]
    pub redaction_sigma: f32,
    /// Redaction appearance. Default stays gaussian for existing callers.
    #[serde(default)]
    pub redaction_kind: RedactionKind,
    /// Pixelate block size (pixels).
    #[serde(default = "default_pixelate_block")]
    pub pixelate_block: u16,
    /// Fallback trim duration (seconds) when IR has no usable range.
    #[serde(default = "default_trim_secs")]
    pub default_trim_duration_secs: f64,
    /// Refuse conversion when privacy approval blocks execute.
    #[serde(default)]
    pub require_approval: bool,
    /// Run `compile_graph` + `schedule_graph` after conversion.
    #[serde(default = "default_true")]
    pub schedule: bool,
}

fn default_sigma() -> f32 {
    12.0
}

fn default_pixelate_block() -> u16 {
    16
}

fn default_trim_secs() -> f64 {
    1.0
}

const fn default_true() -> bool {
    true
}

impl Default for BridgeOptions {
    fn default() -> Self {
        Self {
            output_uri: None,
            redaction_sigma: default_sigma(),
            redaction_kind: RedactionKind::Gaussian,
            pixelate_block: default_pixelate_block(),
            default_trim_duration_secs: default_trim_secs(),
            require_approval: false,
            schedule: true,
        }
    }
}

/// Result of bridging Intelligence IR into a ReelForge graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeResult {
    /// Live ReelForge graph (validates + optional schedule).
    pub graph: RenderGraph,
    /// Canonical JSON of [`Self::graph`].
    pub graph_json: String,
    /// Non-fatal conversion notes (skipped ops, param fills).
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Hybrid execution plan when `schedule` succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_plan: Option<ExecutionPlan>,
}

fn materialize_redaction(timeline: MaskTimeline, opts: &BridgeOptions) -> RegionRedaction {
    match opts.redaction_kind {
        RedactionKind::Gaussian => RegionRedaction::gaussian(timeline, opts.redaction_sigma),
        RedactionKind::Pixelate => RegionRedaction {
            masks: timeline,
            style: RedactionStyle::Pixelate {
                block_size: opts.pixelate_block.max(2),
            },
        },
        RedactionKind::Solid => RegionRedaction {
            masks: timeline,
            style: RedactionStyle::Solid {
                color: Rgba8::BLACK,
            },
        },
    }
}

/// Convert typed Intelligence IR into a ReelForge `RenderGraph`.
///
/// Prefer [`bridge_resolved`] when you still have the freeze (fills masks).
///
/// # Mapping
///
/// | Intelligence | ReelForge |
/// |--------------|-----------|
/// | source / output | `Source` / `Output` + `GraphOutput` |
/// | `rf.adapter.sightloom` | `Op` (params pass-through) |
/// | `rf.redaction.region` | fused `Redaction` (`MaskTimeline` from freeze when present) |
/// | `rf.transform.trim` | `Op` with `start` + `duration` |
/// | `rf.transform.crop` | `Op` when `w`/`h` present; else skip + warn |
/// | `rf.timeline.concat` | multi-range → trims + sequential concat |
///
/// # Errors
///
/// Approval blocked, empty graph, invalid structure, or ReelForge schedule.
pub fn bridge_to_reelforge(ir: &RenderGraphIr, opts: &BridgeOptions) -> Result<BridgeResult> {
    bridge_to_reelforge_with_masks(ir, opts, None)
}

/// Same as [`bridge_to_reelforge`], injecting a pre-built [`MaskTimeline`] into
/// every redaction node (fused privacy ROI).
///
/// # Errors
///
/// Same as [`bridge_to_reelforge`].
pub fn bridge_to_reelforge_with_masks(
    ir: &RenderGraphIr,
    opts: &BridgeOptions,
    masks: Option<&MaskTimeline>,
) -> Result<BridgeResult> {
    if opts.require_approval && !ir.approval.allows_execute() {
        return Err(IntelError::message(format!(
            "bridge: approval required ({})",
            ir.approval.reasons.join(", ")
        )));
    }

    let mut warnings = Vec::new();
    if ir.approval.required && !ir.approval.approved {
        warnings.push(format!(
            "privacy approval not granted ({}); graph is for inspection only",
            ir.approval.reasons.join(", ")
        ));
    }
    if !ir.final_graph {
        warnings.push("IR is preview-only (final_graph=false)".into());
    }

    let assets: Vec<MediaAsset> = ir
        .assets
        .iter()
        .map(|a| MediaAsset {
            id: MediaAssetId(a.id.clone()),
            uri: a.uri.clone(),
            duration: None,
            role: Some("video".into()),
        })
        .collect();

    let mut nodes: Vec<RenderNode> = Vec::new();
    // Maps IR node id → last emitted ReelForge node id (after expansions / skips).
    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut main_output: Option<(String, String)> = None; // (node_id, name)

    for n in &ir.nodes {
        match n.kind {
            GraphNodeKind::Source => {
                let asset = n.asset.clone().ok_or_else(|| {
                    IntelError::message(format!("bridge: source node `{}` missing asset", n.id))
                })?;
                nodes.push(RenderNode {
                    id: NodeId(n.id.clone()),
                    body: RenderNodeKind::Source {
                        asset: MediaAssetId(asset),
                    },
                    inputs: Vec::new(),
                });
                id_map.insert(n.id.clone(), n.id.clone());
            }
            GraphNodeKind::Output => {
                let name = n.name.clone().unwrap_or_else(|| "main".into());
                let inputs = map_inputs(&n.inputs, &id_map)?;
                nodes.push(RenderNode {
                    id: NodeId(n.id.clone()),
                    body: RenderNodeKind::Output { name: name.clone() },
                    inputs,
                });
                id_map.insert(n.id.clone(), n.id.clone());
                main_output = Some((n.id.clone(), name));
            }
            GraphNodeKind::Op => {
                let op = n.operation.as_deref().ok_or_else(|| {
                    IntelError::message(format!("bridge: op node `{}` missing operation", n.id))
                })?;
                let inputs = map_inputs(&n.inputs, &id_map)?;
                let prev = inputs.first().cloned();
                match op {
                    op_id::REDACTION_REGION => {
                        let timeline = masks.cloned().unwrap_or_else(MaskTimeline::new);
                        if timeline_has_samples(&timeline) {
                            warnings.push(format!(
                                "{}: redaction with {} mask samples",
                                n.id,
                                timeline.samples.len()
                            ));
                        } else {
                            warnings.push(format!(
                                "{}: redaction empty MaskTimeline (host/adapter should fill)",
                                n.id
                            ));
                        }
                        nodes.push(RenderNode {
                            id: NodeId(n.id.clone()),
                            body: RenderNodeKind::Redaction {
                                redaction: materialize_redaction(timeline, opts),
                            },
                            inputs,
                        });
                        id_map.insert(n.id.clone(), n.id.clone());
                    }
                    op_id::ADAPTER_SIGHTLOOM => {
                        let params = adapter_params(n.params.as_ref());
                        nodes.push(RenderNode {
                            id: NodeId(n.id.clone()),
                            body: RenderNodeKind::Op {
                                operation: OperationId::new(op),
                                params,
                            },
                            inputs,
                        });
                        id_map.insert(n.id.clone(), n.id.clone());
                    }
                    op_id::TRANSFORM_TRIM => {
                        let (params, note) = normalize_trim_params(
                            n.params.as_ref(),
                            opts.default_trim_duration_secs,
                        );
                        if let Some(note) = note {
                            warnings.push(format!("{}: {note}", n.id));
                        }
                        nodes.push(RenderNode {
                            id: NodeId(n.id.clone()),
                            body: RenderNodeKind::Op {
                                operation: OperationId::new(op_id::TRANSFORM_TRIM),
                                params,
                            },
                            inputs,
                        });
                        id_map.insert(n.id.clone(), n.id.clone());
                    }
                    op_id::TRANSFORM_CROP => {
                        if let Some(params) = crop_params(n.params.as_ref()) {
                            nodes.push(RenderNode {
                                id: NodeId(n.id.clone()),
                                body: RenderNodeKind::Op {
                                    operation: OperationId::new(op_id::TRANSFORM_CROP),
                                    params,
                                },
                                inputs,
                            });
                            id_map.insert(n.id.clone(), n.id.clone());
                        } else {
                            // Framing without pixel crop — host uses adapter tracks.
                            warnings.push(format!(
                                "{}: crop skipped (no w/h); framing left for host/adapter",
                                n.id
                            ));
                            if let Some(NodeId(ref p)) = prev {
                                id_map.insert(n.id.clone(), p.clone());
                            } else {
                                return Err(IntelError::message(format!(
                                    "bridge: crop skip on `{}` has no input",
                                    n.id
                                )));
                            }
                        }
                    }
                    op_id::TIMELINE_CONCAT | "rf.transform.concat" => {
                        emit_timeline_concat(
                            n,
                            &inputs,
                            prev.as_ref(),
                            opts,
                            &mut nodes,
                            &mut id_map,
                            &mut warnings,
                        )?;
                    }
                    other => {
                        // Pass through if registry knows the op; else skip.
                        let reg = bridge_registry();
                        let oid = OperationId::new(other);
                        if reg.get(&oid).is_ok() {
                            nodes.push(RenderNode {
                                id: NodeId(n.id.clone()),
                                body: RenderNodeKind::Op {
                                    operation: oid,
                                    params: n.params.clone().unwrap_or(json!({})),
                                },
                                inputs,
                            });
                            id_map.insert(n.id.clone(), n.id.clone());
                        } else {
                            warnings.push(format!("{}: unknown op `{other}` skipped", n.id));
                            if let Some(NodeId(ref p)) = prev {
                                id_map.insert(n.id.clone(), p.clone());
                            } else {
                                return Err(IntelError::message(format!(
                                    "bridge: cannot skip `{}` with no input",
                                    n.id
                                )));
                            }
                        }
                    }
                }
            }
        }
    }

    if nodes.is_empty() {
        return Err(IntelError::message("bridge: no nodes produced"));
    }

    let (out_node, out_name) = main_output.unwrap_or_else(|| {
        // No explicit output — attach one on the last node.
        let last = nodes
            .last()
            .map_or_else(|| "out".into(), |n| n.id.0.clone());
        (last, "main".into())
    });

    // Ensure an Output node exists if IR only had ops.
    let has_output = nodes
        .iter()
        .any(|n| matches!(n.body, RenderNodeKind::Output { .. }));
    if !has_output {
        nodes.push(RenderNode {
            id: NodeId("out".into()),
            body: RenderNodeKind::Output {
                name: out_name.clone(),
            },
            inputs: vec![NodeId(out_node.clone())],
        });
    }

    let out_bind = nodes
        .iter()
        .find_map(|n| match &n.body {
            RenderNodeKind::Output { name } => Some((n.id.0.clone(), name.clone())),
            _ => None,
        })
        .unwrap_or((out_node, out_name));

    let graph = RenderGraph {
        version: RENDER_GRAPH_VERSION,
        assets,
        nodes,
        outputs: vec![GraphOutput {
            name: out_bind.1,
            node: NodeId(out_bind.0),
            uri: opts.output_uri.clone(),
        }],
    };

    graph
        .validate()
        .map_err(|e| IntelError::message(format!("bridge validate: {e}")))?;

    let mut execution_plan = None;
    if opts.schedule {
        let reg = bridge_registry();
        let plan = schedule_graph(&graph, &reg)
            .map_err(|e| IntelError::message(format!("bridge schedule_graph: {e}")))?;
        execution_plan = Some(plan);
    }

    let graph_json = graph
        .to_json_pretty()
        .map_err(|e| IntelError::message(format!("bridge serialize: {e}")))?;

    Ok(BridgeResult {
        graph,
        graph_json,
        warnings,
        execution_plan,
    })
}

fn emit_timeline_concat(
    n: &GraphNode,
    inputs: &[NodeId],
    prev: Option<&NodeId>,
    opts: &BridgeOptions,
    nodes: &mut Vec<RenderNode>,
    id_map: &mut HashMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let ranges = extract_ranges(n.params.as_ref());
    if ranges.len() <= 1 {
        let (params, note) = if ranges.len() == 1 {
            trim_from_range(&ranges[0], opts.default_trim_duration_secs)
        } else {
            normalize_trim_params(n.params.as_ref(), opts.default_trim_duration_secs)
        };
        if let Some(note) = note {
            warnings.push(format!("{}: {note}", n.id));
        }
        warnings.push(format!("{}: concat→trim (single range / whole span)", n.id));
        nodes.push(RenderNode {
            id: NodeId(n.id.clone()),
            body: RenderNodeKind::Op {
                operation: OperationId::new(op_id::TRANSFORM_TRIM),
                params,
            },
            inputs: inputs.to_vec(),
        });
        id_map.insert(n.id.clone(), n.id.clone());
        return Ok(());
    }

    let Some(NodeId(src)) = prev else {
        return Err(IntelError::message(format!(
            "bridge: concat `{}` needs an input",
            n.id
        )));
    };
    let mut trim_ids = Vec::new();
    let mut clips = Vec::new();
    for (i, range) in ranges.iter().enumerate() {
        let tid = format!("{}_t{i}", n.id);
        let (params, note) = trim_from_range(range, opts.default_trim_duration_secs);
        if let Some(note) = note {
            warnings.push(format!("{tid}: {note}"));
        }
        clips.push(params.clone());
        nodes.push(RenderNode {
            id: NodeId(tid.clone()),
            body: RenderNodeKind::Op {
                operation: OperationId::new(op_id::TRANSFORM_TRIM),
                params,
            },
            inputs: vec![NodeId(src.clone())],
        });
        trim_ids.push(NodeId(tid));
    }
    nodes.push(RenderNode {
        id: NodeId(n.id.clone()),
        body: RenderNodeKind::Op {
            operation: OperationId::new(op_id::TIMELINE_CONCAT),
            params: json!({ "clips": clips }),
        },
        inputs: trim_ids,
    });
    id_map.insert(n.id.clone(), n.id.clone());
    warnings.push(format!(
        "{}: concat expanded to {} sequential trims + rf.timeline.concat",
        n.id,
        ranges.len()
    ));
    Ok(())
}

/// Builtins plus Intelligence-facing adapter op (not yet on all crates.io tags).
fn bridge_registry() -> OperationRegistry {
    let mut r = OperationRegistry::with_builtins();
    if r.get(&OperationId::new(op_id::ADAPTER_SIGHTLOOM)).is_err() {
        r.register(
            OperationDescriptor::new(
                op_id::ADAPTER_SIGHTLOOM,
                SemVer::V1,
                MediaContract::video_av(),
                MediaContract::video_av()
                    .with_masks()
                    .with_notes("video passthrough + masks"),
                BackendClass::Adapter,
            )
            .with_capabilities(["adapter", "vision", "privacy", "sightloom"])
            .with_parameter_schema(json!({
                "type": "object",
                "properties": {
                    "subjects": { "type": "array" },
                    "vision_index_generation": { "type": "string" },
                    "vision_index_hash": { "type": "string" },
                    "adapter": { "type": "string" }
                }
            })),
        );
    }
    if r.get(&OperationId::new(op_id::TIMELINE_CONCAT)).is_err() {
        r.register(
            OperationDescriptor::nary(
                op_id::TIMELINE_CONCAT,
                SemVer::V1,
                MediaContract::video_av().with_notes("n-ary sequential concat"),
                MediaContract::video_av().with_notes("duration = sum of inputs"),
                BackendClass::Rust,
            )
            .with_capabilities(["edit", "timeline", "concat"])
            .with_parameter_schema(json!({
                "type": "object",
                "properties": {
                    "clips": { "type": "array" },
                    "ranges": { "type": "array" }
                }
            })),
        );
    }
    r
}

fn map_inputs(inputs: &[String], id_map: &HashMap<String, String>) -> Result<Vec<NodeId>> {
    inputs
        .iter()
        .map(|id| {
            id_map
                .get(id)
                .map(|mapped| NodeId(mapped.clone()))
                .ok_or_else(|| IntelError::message(format!("bridge: unknown input `{id}`")))
        })
        .collect()
}

fn adapter_params(raw: Option<&Value>) -> Value {
    let mut params = raw.cloned().unwrap_or_else(|| json!({}));
    if let Some(obj) = params.as_object_mut() {
        obj.entry("adapter".to_string())
            .or_insert_with(|| json!("sightloom"));
    }
    params
}

/// Prefer explicit crop geometry; framing-only → None.
fn crop_params(raw: Option<&Value>) -> Option<Value> {
    let params = raw?;
    let width = params.get("w").and_then(Value::as_u64)?;
    let height = params.get("h").and_then(Value::as_u64)?;
    let left = params.get("x").and_then(Value::as_u64).unwrap_or(0);
    let top = params.get("y").and_then(Value::as_u64).unwrap_or(0);
    Some(json!({ "x": left, "y": top, "w": width, "h": height }))
}

fn normalize_trim_params(raw: Option<&Value>, default_secs: f64) -> (Value, Option<String>) {
    if let Some(v) = raw {
        if v.get("duration").is_some() {
            let start = v.get("start").cloned().unwrap_or(json!(0.0));
            return (
                json!({ "start": start, "duration": v.get("duration").cloned().unwrap() }),
                None,
            );
        }
        if let Some(range) = v.get("range") {
            return trim_from_range(range, default_secs);
        }
        if let Some(ranges) = v.get("ranges").and_then(Value::as_array)
            && let Some(first) = ranges.first()
        {
            let (p, note) = trim_from_range(first, default_secs);
            return (
                p,
                Some(note.unwrap_or_else(|| "using first of ranges[]".into())),
            );
        }
    }
    (
        json!({ "start": 0.0, "duration": default_secs }),
        Some(format!("trim defaulted to duration={default_secs}s")),
    )
}

fn extract_ranges(raw: Option<&Value>) -> Vec<Value> {
    let Some(v) = raw else {
        return Vec::new();
    };
    if let Some(arr) = v.get("ranges").and_then(Value::as_array) {
        return arr.clone();
    }
    if let Some(r) = v.get("range") {
        return vec![r.clone()];
    }
    Vec::new()
}

fn trim_from_range(range: &Value, default_secs: f64) -> (Value, Option<String>) {
    let start = range.get("start");
    let end = range.get("end");
    match (start, end) {
        (Some(s), Some(e)) => {
            if let (Some(st), Some(et), Some(ts_s), Some(ts_e)) = (
                s.get("ticks").and_then(Value::as_i64),
                e.get("ticks").and_then(Value::as_i64),
                s.get("timescale").and_then(Value::as_u64),
                e.get("timescale").and_then(Value::as_u64),
            ) {
                if ts_s == ts_e && ts_s > 0 {
                    let dur = (et - st).max(0);
                    return (
                        json!({
                            "start": { "ticks": st, "timescale": ts_s },
                            "duration": { "ticks": dur, "timescale": ts_s },
                        }),
                        None,
                    );
                }
            }
            // Fallback: seconds approximation via f64 fields or nested.
            let start_secs = media_time_secs(s).unwrap_or(0.0);
            let end_secs = media_time_secs(e).unwrap_or(start_secs + default_secs);
            let dur = (end_secs - start_secs).max(0.0);
            (
                json!({ "start": start_secs, "duration": if dur > 0.0 { dur } else { default_secs } }),
                Some("trim range converted via seconds".into()),
            )
        }
        _ => (
            json!({ "start": 0.0, "duration": default_secs }),
            Some(format!("incomplete range; duration={default_secs}s")),
        ),
    }
}

fn media_time_secs(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    let ticks = v.get("ticks").and_then(Value::as_i64)?;
    let ts = v.get("timescale").and_then(Value::as_u64)?;
    if ts == 0 {
        return None;
    }
    Some(ticks as f64 / ts as f64)
}

/// Convenience: bridge with default options (schedule on).
///
/// # Errors
///
/// Same as [`bridge_to_reelforge`].
pub fn bridge_default(ir: &RenderGraphIr) -> Result<BridgeResult> {
    bridge_to_reelforge(ir, &BridgeOptions::default())
}

/// Bridge only when approval allows execute.
///
/// # Errors
///
/// Approval or conversion failure.
pub fn bridge_for_execute(ir: &RenderGraphIr, output_uri: Option<String>) -> Result<BridgeResult> {
    bridge_to_reelforge(
        ir,
        &BridgeOptions {
            output_uri,
            require_approval: true,
            ..BridgeOptions::default()
        },
    )
}

/// Compile-path from freeze: build IR + inject mask samples from resolved artifacts.
///
/// # Errors
///
/// Validation / bridge failure.
pub fn bridge_resolved(resolved: &ResolvedEditPlan, opts: &BridgeOptions) -> Result<BridgeResult> {
    resolved.validate()?;
    let ir = graph_from_resolved(resolved);
    let masks = mask_timeline_from_resolved(resolved);
    let mask_ref = if timeline_has_samples(&masks) {
        Some(&masks)
    } else {
        None
    };
    bridge_to_reelforge_with_masks(&ir, opts, mask_ref)
}

/// Approval-gated bridge from freeze (for host execute).
///
/// # Errors
///
/// Approval or bridge failure.
pub fn bridge_resolved_for_execute(
    resolved: &ResolvedEditPlan,
    output_uri: Option<String>,
) -> Result<BridgeResult> {
    bridge_resolved(
        resolved,
        &BridgeOptions {
            output_uri,
            require_approval: true,
            ..BridgeOptions::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{FrequencyMetric, SemanticEdit, SemanticEditPlan};
    use crate::policy::{IntelligencePolicy, UncertaintyPolicy};
    use crate::render_graph::{approve, graph_from_resolved};
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
                    last_ticks: 5_000_000_000,
                    confidence: Some(0.9),
                    ..SubjectEvidence::default()
                }
                .with_visit(0, 5_000_000_000),
            ],
            anomalies: Vec::new(),
            ..AnalysisSnapshot::default()
        }
    }

    #[test]
    fn bridge_most_frequent_compiles_and_schedules() {
        let intent =
            SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
                metric: FrequencyMetric::AppearanceCount,
            });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        let ir = graph_from_resolved(&resolved);
        let result = bridge_to_reelforge(
            &ir,
            &BridgeOptions {
                output_uri: Some("out.mp4".into()),
                ..BridgeOptions::default()
            },
        )
        .unwrap();
        assert!(!result.graph.nodes.is_empty());
        assert!(result.execution_plan.is_some());
        assert!(result.graph.nodes.iter().any(|n| matches!(
            &n.body,
            RenderNodeKind::Op {
                operation,
                ..
            } if operation.0 == op_id::ADAPTER_SIGHTLOOM
        )));
        let json = &result.graph_json;
        assert!(json.contains("rf.adapter.sightloom") || json.contains("adapter"));
        assert_eq!(result.graph.outputs[0].uri.as_deref(), Some("out.mp4"));
    }

    #[test]
    fn bridge_redaction_is_fused_node() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: crate::selector::SubjectSelector::MostFrequent {
                metric: FrequencyMetric::AppearanceCount,
            },
        });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        let ir = graph_from_resolved(&resolved);
        let result = bridge_default(&ir).unwrap();
        assert!(
            result
                .graph
                .nodes
                .iter()
                .any(|n| matches!(n.body, RenderNodeKind::Redaction { .. }))
        );
    }

    #[test]
    fn redaction_kind_parse() {
        assert_eq!(
            RedactionKind::parse("blur").unwrap(),
            RedactionKind::Gaussian
        );
        assert_eq!(
            RedactionKind::parse("MOSAIC").unwrap(),
            RedactionKind::Pixelate
        );
        assert_eq!(RedactionKind::parse("black").unwrap(), RedactionKind::Solid);
        assert!(RedactionKind::parse("swirl").is_err());
    }

    #[test]
    fn bridge_pixelate_is_not_gaussian() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: crate::selector::SubjectSelector::MostFrequent {
                metric: FrequencyMetric::AppearanceCount,
            },
        });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        let ir = graph_from_resolved(&resolved);
        let opts = BridgeOptions {
            redaction_kind: RedactionKind::Pixelate,
            pixelate_block: 24,
            ..BridgeOptions::default()
        };
        let result = bridge_to_reelforge(&ir, &opts).unwrap();
        let style = result.graph.nodes.iter().find_map(|n| match &n.body {
            RenderNodeKind::Redaction { redaction } => Some(&redaction.style),
            _ => None,
        });
        assert!(
            matches!(style, Some(RedactionStyle::Pixelate { block_size: 24 })),
            "{style:?}"
        );
    }

    #[test]
    fn bridge_resolved_fills_mask_samples() {
        use crate::mask::{MaskArtifact, RegionSample};
        use crate::resolved::ResolvedMaskAsset;
        use crate::time::{MediaRange, MediaTime};

        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: crate::selector::SubjectSelector::MostFrequent {
                metric: FrequencyMetric::AppearanceCount,
            },
        });
        let mut resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        resolved.resolved_masks.push(ResolvedMaskAsset {
            mask_id: None,
            mask_ref: None,
            subject: resolved.resolved_subjects.first().map(|s| s.id.clone()),
            range: Some(MediaRange::new(
                MediaTime::new(0, 1_000_000_000),
                MediaTime::new(5_000_000_000, 1_000_000_000),
            )),
            fidelity: crate::mask::MaskFidelity::BBoxProxy,
            artifact: Some(MaskArtifact::from_regions(vec![RegionSample {
                at: MediaTime::new(0, 1_000_000_000),
                box_xyxy: [100.0, 100.0, 200.0, 300.0],
                subject: resolved.resolved_subjects.first().map(|s| s.id.clone()),
                confidence: Some(0.95),
                geometry: None,
            }])),
        });
        let result = bridge_resolved(&resolved, &BridgeOptions::default()).unwrap();
        let redaction = result.graph.nodes.iter().find_map(|n| match &n.body {
            RenderNodeKind::Redaction { redaction } => Some(redaction),
            _ => None,
        });
        let redaction = redaction.expect("redaction node");
        assert_eq!(redaction.masks.samples.len(), 1);
        assert_eq!(redaction.masks.samples[0].left, Some(100.0));
        assert!(result.warnings.iter().any(|w| w.contains("mask samples")));
    }

    #[test]
    fn bridge_for_execute_blocks_without_approval() {
        let intent =
            SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
                metric: FrequencyMetric::AppearanceCount,
            });
        let mut policy = IntelligencePolicy::default();
        policy.privacy.uncertain_identity = UncertaintyPolicy::Review;
        policy.require_approve_on_review = true;
        let resolved = resolve_plan(&intent, &snap(), policy).unwrap();
        let mut ir = graph_from_resolved(&resolved);
        assert!(ir.approval.required);
        assert!(bridge_for_execute(&ir, None).is_err());
        ir.approval = approve(ir.approval, "ops");
        assert!(bridge_for_execute(&ir, Some("out.mp4".into())).is_ok());
    }

    #[test]
    fn multi_range_concat_is_sequential() {
        use crate::render_graph::{
            ApprovalRecord, GraphAsset, GraphNode, GraphNodeKind, INTEL_RENDER_GRAPH_VERSION,
        };
        let ir = RenderGraphIr {
            version: INTEL_RENDER_GRAPH_VERSION,
            final_graph: true,
            vision_index_generation: None,
            vision_index_hash: None,
            approval: ApprovalRecord::default(),
            assets: vec![GraphAsset {
                id: "in".into(),
                uri: "cam1".into(),
                source_hash: None,
            }],
            nodes: vec![
                GraphNode {
                    id: "src".into(),
                    kind: GraphNodeKind::Source,
                    operation: None,
                    inputs: vec![],
                    asset: Some("in".into()),
                    name: None,
                    params: None,
                    semantic: None,
                },
                GraphNode {
                    id: "reel".into(),
                    kind: GraphNodeKind::Op,
                    operation: Some(op_id::TIMELINE_CONCAT.into()),
                    inputs: vec!["src".into()],
                    asset: None,
                    name: None,
                    params: Some(json!({
                        "ranges": [
                            {
                                "start": { "ticks": 0, "timescale": 1_000_000_000u64 },
                                "end": { "ticks": 1_000_000_000i64, "timescale": 1_000_000_000u64 }
                            },
                            {
                                "start": { "ticks": 2_000_000_000i64, "timescale": 1_000_000_000u64 },
                                "end": { "ticks": 3_000_000_000i64, "timescale": 1_000_000_000u64 }
                            }
                        ]
                    })),
                    semantic: Some("build_subject_reel".into()),
                },
                GraphNode {
                    id: "out".into(),
                    kind: GraphNodeKind::Output,
                    operation: None,
                    inputs: vec!["reel".into()],
                    asset: None,
                    name: Some("main".into()),
                    params: None,
                    semantic: None,
                },
            ],
            note: None,
        };
        let result = bridge_default(&ir).unwrap();
        assert!(result.graph.nodes.iter().any(|n| matches!(
            &n.body,
            RenderNodeKind::Op { operation, .. } if operation.0 == op_id::TIMELINE_CONCAT
        )));
        assert!(!result.graph.nodes.iter().any(|n| matches!(
            &n.body,
            RenderNodeKind::Op { operation, .. } if operation.0 == "rf.compose.layers"
        )));
        let concat = result
            .graph
            .nodes
            .iter()
            .find_map(|n| match &n.body {
                RenderNodeKind::Op { operation, params }
                    if operation.0 == op_id::TIMELINE_CONCAT =>
                {
                    Some(params)
                }
                _ => None,
            })
            .expect("concat node");
        let clips = concat
            .get("clips")
            .and_then(Value::as_array)
            .expect("clips");
        assert_eq!(clips.len(), 2);
        assert_eq!(
            result
                .graph
                .nodes
                .iter()
                .filter(|n| matches!(
                    &n.body,
                    RenderNodeKind::Op { operation, .. } if operation.0 == op_id::TRANSFORM_TRIM
                ))
                .count(),
            2
        );
        assert!(result.execution_plan.is_some());
    }
}
