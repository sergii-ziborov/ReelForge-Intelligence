//! CLI / stdio host for ReelForge Intelligence.
//!
//! No FFmpeg encode path and no model weights. Hosts call ReelForge after graph handoff.

#![allow(
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::assigning_clones,
    clippy::collapsible_if,
    clippy::large_enum_variant
)]

use clap::{Parser, Subcommand};
use reelforge_intelligence_core::{
    BridgeOptions, IntelligenceService, MaskFidelity, MaskRequest, RedactionKind, SemanticEditPlan,
    bindings_from_value, dispatch, list_methods, rewrite_selectors,
};
use reelforge_intelligence_sightloom::{export_and_pin_mask_package, load_package};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "reelforge-intelligence",
    version = VERSION,
    about = "Semantic bridge CLI: SightLoom freeze → RenderGraph handoff (no encode)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print version.
    Version,
    /// List supported MCP method names.
    Methods,
    /// One-shot method dispatch (JSON args).
    Dispatch {
        /// Method name (see `methods`).
        #[arg(long, short = 'm')]
        method: String,
        /// JSON object args (inline string).
        #[arg(long, short = 'a', default_value = "{}")]
        args: String,
        /// Optional path to JSON args file (overrides `--args`).
        #[arg(long)]
        args_file: Option<PathBuf>,
    },
    /// JSON-RPC 2.0 MCP host on stdio. Use `--legacy` for the old line protocol.
    Serve {
        /// Previous non-MCP `{id,method,args}` line protocol.
        #[arg(long, default_value_t = false)]
        legacy: bool,
    },
    /// Load package + intent → resolve → bridge → print/write graph.
    ResolveBridge {
        /// Path to VisionIndex package directory.
        #[arg(long)]
        package: PathBuf,
        /// Path to semantic edit plan JSON.
        #[arg(long)]
        plan: PathBuf,
        /// Optional media key override (defaults to plan.media).
        #[arg(long)]
        media: Option<String>,
        /// Output URI embedded in the live graph.
        #[arg(long)]
        output: Option<String>,
        /// Write live graph JSON here.
        #[arg(long)]
        write_graph: Option<PathBuf>,
        /// Write compile report JSON here.
        #[arg(long)]
        write_report: Option<PathBuf>,
        /// Require privacy approval before bridge (default false for offline tooling).
        #[arg(long, default_value_t = false)]
        require_approval: bool,
        /// Mask fidelity: `preview` (bbox) or `final` (true geometry, fail/review if missing).
        #[arg(long, default_value = "preview")]
        mode: String,
        /// Write a ReelForge MaskPackage (`manifest.json` + `masks/*.bin`) here.
        #[arg(long)]
        write_mask_package: Option<PathBuf>,
        /// Host FramePick bindings JSON (array or `{ "bindings": [...] }`).
        #[arg(long)]
        bindings: Option<PathBuf>,
        /// Redaction style: `gaussian` (default), `pixelate`, `solid`.
        #[arg(long, default_value = "gaussian")]
        style: String,
    },
}

#[derive(Debug, Deserialize)]
struct ServeRequest {
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize)]
struct ServeResponse {
    id: Value,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Version => {
            println!("reelforge-intelligence {VERSION}");
            Ok(())
        }
        Commands::Methods => {
            for m in list_methods() {
                println!("{m}");
            }
            Ok(())
        }
        Commands::Dispatch {
            method,
            args,
            args_file,
        } => {
            let args_val = load_args(&args, args_file.as_deref())?;
            let svc = IntelligenceService::new();
            let result = dispatch(&svc, &method, &args_val).map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(ser)?);
            Ok(())
        }
        Commands::Serve { legacy } => {
            if legacy {
                serve_stdio_legacy()
            } else {
                serve_stdio_mcp()
            }
        }
        Commands::ResolveBridge {
            package,
            plan,
            media,
            output,
            write_graph,
            write_report,
            require_approval,
            mode,
            write_mask_package,
            bindings,
            style,
        } => resolve_bridge(
            &package,
            &plan,
            media,
            output,
            write_graph.as_deref(),
            write_report.as_deref(),
            require_approval,
            &mode,
            write_mask_package.as_deref(),
            bindings.as_deref(),
            &style,
        ),
    }
}

fn load_args(inline: &str, file: Option<&Path>) -> Result<Value, String> {
    if let Some(path) = file {
        let text = fs::read_to_string(path).map_err(|e| format!("read args: {e}"))?;
        return serde_json::from_str(&text).map_err(|e| format!("parse args file: {e}"));
    }
    serde_json::from_str(inline).map_err(|e| format!("parse --args: {e}"))
}

fn serve_stdio_mcp() -> Result<(), String> {
    use reelforge_intelligence_core::handle_jsonrpc;
    let svc = IntelligenceService::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("stdin: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(resp) = handle_jsonrpc(&svc, line) {
            if resp.get("result").and_then(Value::as_bool) == Some(true)
                && line.contains("\"shutdown\"")
            {
                let text = serde_json::to_string(&resp).map_err(ser)?;
                writeln!(stdout, "{text}").map_err(|e| format!("stdout: {e}"))?;
                stdout.flush().map_err(|e| format!("stdout flush: {e}"))?;
                break;
            }
            let text = serde_json::to_string(&resp).map_err(ser)?;
            writeln!(stdout, "{text}").map_err(|e| format!("stdout: {e}"))?;
            stdout.flush().map_err(|e| format!("stdout flush: {e}"))?;
        }
    }
    Ok(())
}

fn serve_stdio_legacy() -> Result<(), String> {
    let svc = IntelligenceService::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("stdin: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: ServeRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let resp = ServeResponse {
                    id: Value::Null,
                    ok: false,
                    result: None,
                    error: Some(format!("bad request json: {e}")),
                };
                write_line(&mut stdout, &resp)?;
                continue;
            }
        };
        if req.method == "shutdown" || req.method == "exit" {
            let resp = ServeResponse {
                id: req.id,
                ok: true,
                result: Some(Value::Bool(true)),
                error: None,
            };
            write_line(&mut stdout, &resp)?;
            break;
        }
        let resp = match dispatch(&svc, &req.method, &req.args) {
            Ok(result) => ServeResponse {
                id: req.id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(e) => ServeResponse {
                id: req.id,
                ok: false,
                result: None,
                error: Some(e.to_string()),
            },
        };
        write_line(&mut stdout, &resp)?;
    }
    Ok(())
}

fn write_line(out: &mut impl Write, resp: &ServeResponse) -> Result<(), String> {
    let line = serde_json::to_string(resp).map_err(ser)?;
    writeln!(out, "{line}").map_err(|e| format!("stdout: {e}"))?;
    out.flush().map_err(|e| format!("stdout flush: {e}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn resolve_bridge(
    package: &Path,
    plan_path: &Path,
    media: Option<String>,
    output: Option<String>,
    graph_out: Option<&Path>,
    report_out: Option<&Path>,
    require_approval: bool,
    mode: &str,
    mask_package_out: Option<&Path>,
    bindings_path: Option<&Path>,
    style: &str,
) -> Result<(), String> {
    let mut loaded = load_package(package).map_err(|e| format!("package: {e}"))?;
    if let Some(dest) = mask_package_out {
        export_and_pin_mask_package(&mut loaded, dest).map_err(|e| format!("mask package: {e}"))?;
    }
    let plan_text = fs::read_to_string(plan_path).map_err(|e| format!("read plan: {e}"))?;
    let mut plan = SemanticEditPlan::from_json(&plan_text).map_err(|e| e.to_string())?;
    if let Some(m) = media {
        plan.media = m;
    } else if plan.media.trim().is_empty() {
        plan.media.clone_from(&loaded.snapshot.media);
    }
    if let Some(path) = bindings_path {
        let text = fs::read_to_string(path).map_err(|e| format!("read bindings: {e}"))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("parse bindings: {e}"))?;
        let bindings = bindings_from_value(&value).map_err(|e| e.to_string())?;
        plan = rewrite_selectors(plan, &bindings).map_err(|e| e.to_string())?;
    }
    let svc = IntelligenceService::new();
    let mut resolved = svc
        .resolve_plan(&plan, &loaded.snapshot)
        .map_err(|e| e.to_string())?;

    // Masks: preview = bbox proxy; final = true geometry with fail/review.
    let final_mode = mode.eq_ignore_ascii_case("final");
    if !resolved.resolved_subjects.is_empty() {
        let subjects: Vec<_> = resolved
            .resolved_subjects
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let ranges = if resolved.resolved_ranges.is_empty() {
            resolved
                .resolved_subjects
                .iter()
                .filter_map(|s| s.span)
                .collect()
        } else {
            resolved.resolved_ranges.clone()
        };
        if !ranges.is_empty() {
            let request = if final_mode {
                MaskRequest::final_subjects(subjects, ranges)
            } else if ranges.len() == 1 {
                MaskRequest::preview_subjects(subjects, ranges[0])
            } else {
                MaskRequest {
                    subjects,
                    mask_ids: Vec::new(),
                    ranges,
                    fidelity: MaskFidelity::BBoxProxy,
                    sample_step_ticks: 0,
                }
            };
            let provider = loaded.provider();
            match svc.materialize_masks(&provider, &request) {
                Ok(artifact) => {
                    if final_mode && !artifact.carries_true_geometry() {
                        match resolved.policy.privacy.missing_mask {
                            reelforge_intelligence_core::MissingMaskAction::Fail => {
                                return Err(
                                    "final mode: true mask geometry unavailable (missing_mask=fail)"
                                        .into(),
                                );
                            }
                            reelforge_intelligence_core::MissingMaskAction::Review => {
                                resolved.warnings.push(
                                    reelforge_intelligence_core::ResolutionWarning {
                                        message:
                                            "final mode: true geometry missing — review required"
                                                .into(),
                                        edit_index: None,
                                    },
                                );
                            }
                            _ => {}
                        }
                    }
                    if !artifact.regions.is_empty() || artifact.geometry.is_some() {
                        resolved = svc.with_mask_artifact(resolved, artifact);
                    }
                }
                Err(e) if final_mode => return Err(format!("final masks: {e}")),
                Err(_) => {}
            }
        }
    }

    let redaction_kind = RedactionKind::parse(style).map_err(|e| e.to_string())?;
    let opts = BridgeOptions {
        output_uri: output,
        require_approval,
        redaction_kind,
        ..BridgeOptions::default()
    };
    let (report, bridged) = svc
        .compile_and_bridge(&resolved, &opts)
        .map_err(|e| e.to_string())?;

    let graph_written = if let Some(path) = graph_out {
        fs::write(path, &bridged.graph_json).map_err(|e| format!("write graph: {e}"))?;
        true
    } else {
        false
    };
    if let Some(path) = report_out {
        let text = serde_json::to_string_pretty(&report).map_err(ser)?;
        fs::write(path, text).map_err(|e| format!("write report: {e}"))?;
    }

    let summary = serde_json::json!({
        "ok": report.ok,
        "final_graph": report.final_graph,
        "allows_execute": report.allows_execute(),
        "subjects": resolved.resolved_subjects.len(),
        "masks": resolved.resolved_masks.len(),
        "mask_samples": resolved.resolved_masks.iter()
            .filter_map(|m| m.artifact.as_ref())
            .map(|a| a.regions.len())
            .sum::<usize>(),
        "bridge_warnings": bridged.warnings,
        "has_execution_plan": bridged.execution_plan.is_some(),
        "mask_package_id": resolved.mask_package_id,
        "mask_package_uri": resolved.mask_package_uri,
        "graph_nodes": bridged.graph.nodes.len(),
        "graph_json_preview": if graph_written {
            Value::String("(written to --write-graph)".into())
        } else {
            Value::String(bridged.graph_json)
        },
    });
    println!("{}", serde_json::to_string_pretty(&summary).map_err(ser)?);
    Ok(())
}

fn ser(err: serde_json::Error) -> String {
    format!("json: {err}")
}
