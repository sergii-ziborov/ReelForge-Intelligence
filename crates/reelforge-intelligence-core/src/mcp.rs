//! Thin MCP adapter: method name + JSON → [`IntelligenceService`]. No extra logic.

use crate::catalog::HostCatalog;
use crate::edit::SemanticEditPlan;
use crate::error::{IntelError, Result};
use crate::service::IntelligenceService;
use serde_json::Value;

/// Known Intelligence MCP method names (stdio host / tools catalog).
pub const METHODS: &[&str] = &[
    "operations",
    "schemas",
    "list_methods",
    "inspect_media",
    "catalog_scenes",
    "catalog_subjects",
    "check_plan",
    "normalize_plan",
    "repair_plan",
    "rewrite_selectors",
    "compile_plan",
    "resolve_plan",
    "compile_resolved",
    "resolve_and_compile",
    "explain_plan",
    "preview_frame",
    "render",
    "render_resolved",
    "approve_and_render",
    "compile_and_bridge",
    "bridge_graph",
];

/// List supported method names.
#[must_use]
pub fn list_methods() -> &'static [&'static str] {
    METHODS
}

/// JSON-RPC 2.0 protocol version advertised by [`handle_jsonrpc`].
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Handle one JSON-RPC 2.0 MCP message. Notifications return `None`.
///
/// # Errors
///
/// Never — protocol errors are encoded in the JSON-RPC response.
#[must_use]
pub fn handle_jsonrpc(svc: &IntelligenceService, raw: &str) -> Option<Value> {
    let parsed: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return Some(jsonrpc_error(
                &Value::Null,
                -32700,
                format!("parse error: {e}"),
            ));
        }
    };
    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
    let params = parsed.get("params").cloned().unwrap_or(Value::Null);
    let is_notification = parsed.get("id").is_none();

    let result = match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "reelforge-intelligence",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "notifications/initialized" | "initialized" => {
            if is_notification {
                return None;
            }
            Ok(Value::Object(serde_json::Map::new()))
        }
        "ping" => Ok(Value::Object(serde_json::Map::new())),
        "tools/list" => Ok(serde_json::json!({ "tools": mcp_tools() })),
        "tools/call" => match params.get("name").and_then(Value::as_str) {
            None => Err(IntelError::message("tools/call: name required")),
            Some(name) => {
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                dispatch(svc, name, &args).map(|value| {
                    serde_json::json!({
                        "content": [{ "type": "text", "text": value.to_string() }],
                        "structuredContent": value
                    })
                })
            }
        },
        "shutdown" => Ok(Value::Bool(true)),
        "" => Err(IntelError::message("method required")),
        other => {
            // Compatibility: treat unknown JSON-RPC methods as Intelligence tools
            // only when they are in METHODS; otherwise JSON-RPC -32601.
            if METHODS.contains(&other) {
                dispatch(svc, other, &params)
            } else {
                return Some(jsonrpc_error(
                    &id,
                    -32601,
                    format!("method not found: {other}"),
                ));
            }
        }
    };

    if is_notification {
        return None;
    }
    match result {
        Ok(value) => Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": value
        })),
        Err(e) => Some(jsonrpc_error(&id, -32603, e.to_string())),
    }
}

fn mcp_tools() -> Vec<Value> {
    METHODS
        .iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "description": format!("ReelForge Intelligence method `{name}`"),
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": true
                }
            })
        })
        .collect()
}

fn jsonrpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

/// Dispatch one MCP-style call. Unknown methods error.
///
/// # Errors
///
/// Unknown method, bad args, or service error.
pub fn dispatch(svc: &IntelligenceService, method: &str, args: &Value) -> Result<Value> {
    match method {
        "operations" => Ok(serde_json::to_value(svc.operations()).unwrap_or(Value::Null)),
        "list_methods" => Ok(serde_json::to_value(list_methods()).unwrap_or(Value::Null)),
        "schemas" => Ok(svc.schemas()),
        "inspect_media" => {
            let media = arg_str(args, "media")?;
            Ok(serde_json::to_value(svc.inspect_media(media)?).unwrap_or(Value::Null))
        }
        "catalog_scenes" => Ok(serde_json::to_value(svc.catalog_scenes()).unwrap_or(Value::Null)),
        "catalog_subjects" => {
            Ok(serde_json::to_value(svc.catalog_subjects()).unwrap_or(Value::Null))
        }
        "check_plan" => {
            let plan = plan_arg(args)?;
            svc.check_plan(&plan)?;
            Ok(Value::Bool(true))
        }
        "normalize_plan" => {
            let plan = plan_arg(args)?;
            Ok(serde_json::to_value(svc.normalize_plan(plan)).unwrap_or(Value::Null))
        }
        "repair_plan" => {
            let plan = plan_arg(args)?;
            Ok(serde_json::to_value(svc.repair_plan(plan)).unwrap_or(Value::Null))
        }
        "rewrite_selectors" => {
            let plan = plan_arg(args)?;
            let bindings =
                crate::rewrite::bindings_from_value(args.get("bindings").unwrap_or(&Value::Null))?;
            Ok(
                serde_json::to_value(svc.rewrite_selectors(plan, &bindings)?)
                    .unwrap_or(Value::Null),
            )
        }
        "compile_plan" => {
            let plan = plan_arg(args)?;
            Ok(serde_json::to_value(svc.compile_plan(&plan)?).unwrap_or(Value::Null))
        }
        "resolve_plan" => {
            let plan = plan_arg(args)?;
            let analysis = analysis_arg(args)?;
            Ok(serde_json::to_value(svc.resolve_plan(&plan, &analysis)?).unwrap_or(Value::Null))
        }
        "compile_resolved" => {
            let resolved = resolved_arg(args)?;
            Ok(serde_json::to_value(svc.compile_resolved(&resolved)?).unwrap_or(Value::Null))
        }
        "resolve_and_compile" => {
            let plan = plan_arg(args)?;
            let analysis = analysis_arg(args)?;
            let (resolved, report) = svc.resolve_and_compile(&plan, &analysis)?;
            Ok(serde_json::json!({ "resolved": resolved, "report": report }))
        }
        "explain_plan" => {
            let plan = plan_arg(args)?;
            Ok(Value::String(svc.explain_plan(&plan)))
        }
        "preview_frame" => {
            let plan = plan_arg(args)?;
            let t = args
                .get("t_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| IntelError::message("preview_frame: t_secs required"))?;
            Ok(serde_json::to_value(svc.preview_frame(&plan, t)?).unwrap_or(Value::Null))
        }
        "render" => {
            let plan = plan_arg(args)?;
            Ok(serde_json::to_value(svc.render(&plan)?).unwrap_or(Value::Null))
        }
        "render_resolved" => {
            let resolved = resolved_arg(args)?;
            Ok(serde_json::to_value(svc.render_resolved(&resolved)?).unwrap_or(Value::Null))
        }
        "approve_and_render" => {
            let resolved = resolved_arg(args)?;
            let by = args.get("by").and_then(Value::as_str).unwrap_or("operator");
            let (report, req) = svc.approve_and_render(&resolved, by)?;
            Ok(serde_json::json!({ "report": report, "request": req }))
        }
        "compile_and_bridge" => {
            let resolved = resolved_arg(args)?;
            let opts = bridge_opts_from_args(args)?;
            let (report, bridged) = svc.compile_and_bridge(&resolved, &opts)?;
            Ok(serde_json::json!({
                "report": report,
                "graph_json": bridged.graph_json,
                "warnings": bridged.warnings,
                "has_execution_plan": bridged.execution_plan.is_some(),
            }))
        }
        "bridge_graph" => {
            let ir = args
                .get("graph")
                .ok_or_else(|| IntelError::message("bridge_graph: graph required"))?;
            let ir: crate::render_graph::RenderGraphIr = if let Some(s) = ir.as_str() {
                serde_json::from_str(s).map_err(|e| IntelError::message(e.to_string()))?
            } else {
                serde_json::from_value(ir.clone())
                    .map_err(|e| IntelError::message(e.to_string()))?
            };
            let opts = bridge_opts_from_args(args)?;
            let bridged = svc.bridge_graph(&ir, &opts)?;
            Ok(serde_json::json!({
                "graph_json": bridged.graph_json,
                "warnings": bridged.warnings,
                "has_execution_plan": bridged.execution_plan.is_some(),
            }))
        }
        other => Err(IntelError::message(format!(
            "unknown intelligence method `{other}`"
        ))),
    }
}

/// Load a catalog into a new service (MCP bootstrap).
#[must_use]
pub fn service_with_catalog(catalog: HostCatalog) -> IntelligenceService {
    IntelligenceService::new().with_catalog(catalog)
}

fn bridge_opts_from_args(args: &Value) -> Result<crate::bridge::BridgeOptions> {
    let output = args
        .get("output")
        .and_then(Value::as_str)
        .map(str::to_string);
    let require_approval = args
        .get("require_approval")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut opts = crate::bridge::BridgeOptions {
        output_uri: output,
        require_approval,
        ..crate::bridge::BridgeOptions::default()
    };
    if let Some(style) = args.get("style").and_then(Value::as_str) {
        opts.redaction_kind = crate::bridge::RedactionKind::parse(style)?;
    }
    if let Some(block) = args.get("pixelate_block").and_then(Value::as_u64) {
        opts.pixelate_block = u16::try_from(block).unwrap_or(u16::MAX).max(2);
    }
    if let Some(sigma) = args.get("redaction_sigma").and_then(Value::as_f64) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        {
            opts.redaction_sigma = sigma as f32;
        }
    }
    Ok(opts)
}

fn plan_arg(args: &Value) -> Result<SemanticEditPlan> {
    let raw = args
        .get("plan")
        .ok_or_else(|| IntelError::message("plan required"))?;
    if let Some(s) = raw.as_str() {
        return SemanticEditPlan::from_json(s);
    }
    let plan: SemanticEditPlan =
        serde_json::from_value(raw.clone()).map_err(|e| IntelError::message(e.to_string()))?;
    plan.validate()?;
    Ok(plan)
}

fn analysis_arg(args: &Value) -> Result<crate::AnalysisSnapshot> {
    let raw = args
        .get("analysis")
        .ok_or_else(|| IntelError::message("analysis required"))?;
    serde_json::from_value(raw.clone()).map_err(|e| IntelError::message(e.to_string()))
}

fn resolved_arg(args: &Value) -> Result<crate::ResolvedEditPlan> {
    let raw = args
        .get("resolved")
        .ok_or_else(|| IntelError::message("resolved required"))?;
    if let Some(s) = raw.as_str() {
        return crate::ResolvedEditPlan::from_json(s);
    }
    let plan: crate::ResolvedEditPlan =
        serde_json::from_value(raw.clone()).map_err(|e| IntelError::message(e.to_string()))?;
    plan.validate()?;
    Ok(plan)
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| IntelError::message(format!("{key} required")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{HostCatalog, MediaInspection};
    use crate::edit::SemanticEdit;
    use crate::selector::SubjectSelector;

    fn plan() -> SemanticEditPlan {
        SemanticEditPlan::new("assets/video-1").with_edit(SemanticEdit::BlurSubject {
            subject: SubjectSelector::SubjectSet {
                name: "family".into(),
            },
        })
    }

    #[test]
    fn dispatch_operations_and_compile() {
        let svc = IntelligenceService::new();
        let ops = dispatch(&svc, "operations", &Value::Null).unwrap();
        assert!(ops.as_array().unwrap().len() >= 4);
        let args = serde_json::json!({ "plan": plan() });
        let report = dispatch(&svc, "compile_plan", &args).unwrap();
        assert_eq!(report["ok"], true);
        assert!(
            report["render_graph_json"]
                .as_str()
                .unwrap()
                .contains("src")
        );
        let explain = dispatch(&svc, "explain_plan", &args).unwrap();
        assert!(explain.as_str().unwrap().contains("blur_subject"));
    }

    #[test]
    fn inspect_needs_catalog() {
        let svc = IntelligenceService::new().with_catalog(HostCatalog {
            media: Some(MediaInspection {
                media: "assets/video-1".into(),
                duration_secs: Some(12.0),
                size: Some((1920, 1080)),
                has_video: true,
                has_audio: true,
            }),
            ..HostCatalog::default()
        });
        let v = dispatch(
            &svc,
            "inspect_media",
            &serde_json::json!({ "media": "assets/video-1" }),
        )
        .unwrap();
        assert_eq!(v["duration_secs"], 12.0);
    }

    #[test]
    fn preview_is_host_request() {
        let svc = IntelligenceService::new();
        let args = serde_json::json!({ "plan": plan(), "t_secs": 1.5 });
        let req = dispatch(&svc, "preview_frame", &args).unwrap();
        assert_eq!(req["action"], "preview_frame");
        assert_eq!(req["t_secs"], 1.5);
    }

    #[test]
    fn dispatch_rewrite_selectors() {
        let svc = IntelligenceService::new();
        let args = serde_json::json!({
            "plan": {
                "version": 2,
                "media": "cam",
                "edits": [{
                    "type": "blur_subject",
                    "subject": {
                        "kind": "frame_pick",
                        "media": "cam",
                        "frame_index": 0,
                        "box_xyxy": [1.0, 2.0, 3.0, 4.0]
                    }
                }]
            },
            "bindings": [{
                "media": "cam",
                "frame_index": 0,
                "box_xyxy": [1.0, 2.0, 3.0, 4.0],
                "ids": [9]
            }]
        });
        let out = dispatch(&svc, "rewrite_selectors", &args).unwrap();
        assert_eq!(out["edits"][0]["subject"]["kind"], "subject_ids");
        assert_eq!(out["edits"][0]["subject"]["ids"][0], 9);
    }

    #[test]
    fn unknown_method_errors() {
        let svc = IntelligenceService::new();
        assert!(dispatch(&svc, "hack_the_planet", &Value::Null).is_err());
    }

    #[test]
    fn jsonrpc_initialize_and_tools_call() {
        let svc = IntelligenceService::new();
        let init = handle_jsonrpc(
            &svc,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        let listed =
            handle_jsonrpc(&svc, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert!(listed["result"]["tools"].as_array().unwrap().len() >= 4);
        let call = handle_jsonrpc(
            &svc,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_methods","arguments":{}}}"#,
        )
        .unwrap();
        assert!(
            call["result"]["structuredContent"]
                .as_array()
                .unwrap()
                .len()
                >= 4
        );
        let missing = handle_jsonrpc(&svc, r#"{"jsonrpc":"2.0","id":4,"method":"nope"}"#).unwrap();
        assert_eq!(missing["error"]["code"], -32601);
        let bad = handle_jsonrpc(&svc, "not-json").unwrap();
        assert_eq!(bad["error"]["code"], -32700);
    }
}
