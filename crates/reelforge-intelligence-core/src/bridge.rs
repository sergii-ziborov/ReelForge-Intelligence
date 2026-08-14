//! Map Intelligence [`RenderGraphIr`] → live `reelforge_render_graph::RenderGraph`.
//!
//! Structural handoff only: no FFmpeg, no encode. Hosts call
//! `compile_graph` / `schedule_graph` / `run_render_graph` after this bridge.

use crate::error::{IntelError, Result};
use crate::render_graph::{GraphNodeKind, RenderGraphIr, op_id};
use reelforge_render_graph::{
    BackendClass, CapabilitySet, ExecutionPlan, GraphOutput, MaskTimeline, MediaAsset,
    MediaAssetId, MediaContract, NodeId, OperationDescriptor, OperationId, OperationLimits,
    OperationRegistry, RENDER_GRAPH_VERSION, RegionRedaction, RenderGraph, RenderNode,
    RenderNodeKind, SemVer, schedule_graph,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Options for IR → ReelForge conversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeOptions {
    /// Output URI for the main graph output (e.g. `out.mp4`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_uri: Option<String>,
    /// Gaussian sigma when materializing empty redaction nodes.
    #[serde(default = "default_sigma")]
    pub redaction_sigma: f32,
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

/// Convert typed Intelligence IR into a ReelForge `RenderGraph`.
///
/// # Mapping
///
/// | Intelligence | ReelForge |
/// |--------------|-----------|
/// | source / output | `Source` / `Output` + `GraphOutput` |
/// | `rf.adapter.sightloom` | `Op` (params pass-through) |
/// | `rf.redaction.region` | fused `Redaction` (empty masks; host/adapter fills) |
/// | `rf.transform.trim` | `Op` with `start` + `duration` |
/// | `rf.transform.crop` | `Op` when `w`/`h` present; else skip + warn |
/// | `rf.transform.concat` | multi-range → trims + `rf.compose.layers` |
///
/// # Errors
///
/// Approval blocked, empty graph, invalid structure, or ReelForge compile/schedule.
pub fn bridge_to_reelforge(ir: &RenderGraphIr, opts: &BridgeOptions) -> Result<BridgeResult> {
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
                let name = n
                    .name
                    .clone()
                    .unwrap_or_else(|| "main".into());
                let inputs = map_inputs(&n.inputs, &id_map)?;
                nodes.push(RenderNode {
                    id: NodeId(n.id.clone()),
                    body: RenderNodeKind::Output {
                        name: name.clone(),
                    },
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
                        nodes.push(RenderNode {
                            id: NodeId(n.id.clone()),
                            body: RenderNodeKind::Redaction {
                                redaction: RegionRedaction::gaussian(
                                    MaskTimeline::new(),
                                    opts.redaction_sigma,
                                ),
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
                        let (params, note) =
                            normalize_trim_params(n.params.as_ref(), opts.default_trim_duration_secs);
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
                    op_id::TRANSFORM_CONCAT => {
                        let ranges = extract_ranges(n.params.as_ref());
                        if ranges.len() <= 1 {
                            // Single clip: trim is enough; multi-input compose needs ≥2.
                            let (params, note) = if ranges.len() == 1 {
                                trim_from_range(&ranges[0], opts.default_trim_duration_secs)
                            } else {
                                normalize_trim_params(
                                    n.params.as_ref(),
                                    opts.default_trim_duration_secs,
                                )
                            };
                            if let Some(note) = note {
                                warnings.push(format!("{}: {note}", n.id));
                            }
                            warnings.push(format!(
                                "{}: concat→trim (single range / whole span)",
                                n.id
                            ));
                            nodes.push(RenderNode {
                                id: NodeId(n.id.clone()),
                                body: RenderNodeKind::Op {
                                    operation: OperationId::new(op_id::TRANSFORM_TRIM),
                                    params,
                                },
                                inputs,
                            });
                            id_map.insert(n.id.clone(), n.id.clone());
                        } else {
                            let Some(NodeId(src)) = prev else {
                                return Err(IntelError::message(format!(
                                    "bridge: concat `{}` needs an input",
                                    n.id
                                )));
                            };
                            let mut trim_ids = Vec::new();
                            for (i, range) in ranges.iter().enumerate() {
                                let tid = format!("{}_t{i}", n.id);
                                let (params, note) =
                                    trim_from_range(range, opts.default_trim_duration_secs);
                                if let Some(note) = note {
                                    warnings.push(format!("{tid}: {note}"));
                                }
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
                            let layers: Vec<Value> = trim_ids
                                .iter()
                                .enumerate()
                                .map(|(i, _)| json!({ "start": i as f64 }))
                                .collect();
                            nodes.push(RenderNode {
                                id: NodeId(n.id.clone()),
                                body: RenderNodeKind::Op {
                                    operation: OperationId::new("rf.compose.layers"),
                                    params: json!({ "layers": layers }),
                                },
                                inputs: trim_ids,
                            });
                            id_map.insert(n.id.clone(), n.id.clone());
                            warnings.push(format!(
                                "{}: concat expanded to {} trims + rf.compose.layers",
                                n.id,
                                ranges.len()
                            ));
                        }
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

/// Builtins plus Intelligence-facing adapter op (not yet on all crates.io tags).
fn bridge_registry() -> OperationRegistry {
    let mut r = OperationRegistry::with_builtins();
    if r.get(&OperationId::new(op_id::ADAPTER_SIGHTLOOM)).is_err() {
        r.register(OperationDescriptor {
            id: OperationId::new(op_id::ADAPTER_SIGHTLOOM),
            version: SemVer::V1,
            input: MediaContract {
                video: true,
                audio: true,
                masks: false,
                notes: None,
            },
            output: MediaContract {
                video: true,
                audio: true,
                masks: true,
                notes: Some("video passthrough + masks".into()),
            },
            backend: BackendClass::Adapter,
            deterministic: true,
            capabilities: CapabilitySet {
                tags: vec![
                    "adapter".into(),
                    "vision".into(),
                    "privacy".into(),
                    "sightloom".into(),
                ],
            },
            parameter_schema: json!({
                "type": "object",
                "properties": {
                    "subjects": { "type": "array" },
                    "vision_index_generation": { "type": "string" },
                    "vision_index_hash": { "type": "string" },
                    "adapter": { "type": "string" }
                }
            }),
            limits: OperationLimits::default(),
        });
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
pub fn bridge_for_execute(
    ir: &RenderGraphIr,
    output_uri: Option<String>,
) -> Result<BridgeResult> {
    bridge_to_reelforge(
        ir,
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
            subjects: vec![SubjectEvidence {
                subject_id: 7,
                label: Some("x".into()),
                appearance_count: 9,
                source_ids: vec![1],
                first_ticks: 0,
                last_ticks: 5_000_000_000,
                confidence: Some(0.9),
            }],
            anomalies: Vec::new(),
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
        assert!(
            result
                .graph
                .nodes
                .iter()
                .any(|n| matches!(
                    &n.body,
                    RenderNodeKind::Op {
                        operation,
                        ..
                    } if operation.0 == op_id::ADAPTER_SIGHTLOOM
                ))
        );
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
    fn multi_range_concat_expands_to_layers() {
        use crate::render_graph::{
            GraphAsset, GraphNode, GraphNodeKind, INTEL_RENDER_GRAPH_VERSION, ApprovalRecord,
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
                    operation: Some(op_id::TRANSFORM_CONCAT.into()),
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
        assert!(
            result
                .graph
                .nodes
                .iter()
                .any(|n| matches!(
                    &n.body,
                    RenderNodeKind::Op { operation, .. } if operation.0 == "rf.compose.layers"
                ))
        );
        assert!(result.execution_plan.is_some());
    }
}
