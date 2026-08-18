# Context handoff: SightLoom → ReelForge Intelligence

This file keeps product memory when switching sessions/repos.

## SightLoom (done foundation, 0.1.6)

- Understanding / memory only — no video I/O/render in core
- VisionIndex package generations (`CURRENT` → `gen-*`) + hashes
- Subjects, TrackKey/Uid multi-source, appearances/visits, ranking
- Photo search path: host embeds → SightLoom ranks (`sightloom-host` step 1)
- Redaction intervals (provenance handles, no pixels)
- Uncertain intervals + multi-hypothesis open/accept/dismiss
- Anomalies: statistical + Isolation Forest + OCSVM + AnomalyDetector trait
- Evidence reel **handles** (not encoded media)
- Mask store + Moore contours; SLARROW1 track export

## ReelForge (expected host side)

- exact media time, MaskTimeline, RegionRedaction
- RenderGraph, ExecutionPlan, typed ops, scheduler
- encode / preview materialization

## Intelligence (this repo)

Owns the missing layer:

1. `SemanticEditPlan` — human/agent intent  
2. `AnalysisSnapshot` — host projection of VisionIndex  
3. `ResolvedEditPlan` — freeze for reproducibility  
4. `compile_resolved` — typed `RenderGraphIr` + JSON for ReelForge  
5. `approve` / `approve_and_render` — privacy Review gate  
6. `bridge_to_reelforge` — live `reelforge_render_graph::RenderGraph` + `schedule_graph`

## Do not put in Intelligence

- model weights / ONNX (SightLoom host package)
- FFmpeg encode
- VisionIndex storage format

## Technical bridges (done)

1. **Namespaced IDs** — `NamespacedId` / `sightloom://…`  
2. **Mask domains** — `MaskFidelity`, `MaskRequest`, `MaskArtifact`  
3. **PrivacyPolicy** — uncertain, missing_mask, low_confidence, track_gap  
4. **`AnalysisProvider` trait** + **`SightLoomProvider`** over `AnalysisSnapshot`  
5. **ReelForge bridge** — `RenderGraphIr` → `RenderGraph` / `ExecutionPlan` (`reelforge-render-graph` 0.2.0)

## Compiler correctness (2026-08)

Closed the nine Intelligence risks:

1. `COMPAT.md` + path pin to ReelForge `0.2.0` (`MaskAsset`, `rf.timeline.concat`)
2. Resolver: discrete appearances, pre/post-roll, visible duration, host `subject_sets`, track bindings, real event clips; FramePick/empty reels are errors
3. Bridge emits `rf.timeline.concat` (never `compose.layers`)
4. `MaskGeometry` → `MaskSample.asset`
5. CLI `resolve-bridge --mode preview|final`
6. Follow-crop computed here (`framing.rs`)
7. Approval binds SHA-256 graph/IR/plan fingerprints; bridge failure is hard
8. `serve` is JSON-RPC 2.0 MCP (`serve --legacy` for the old line protocol)
9. Package hashes are SHA-256 of payload bytes (not FNV of sizes)
10. Timed mask samples + `SLM1` / External geometry; inferred frame size; optional `RF_INTEL_APPROVAL_HMAC`
11. Emit ReelForge MaskPackage (`--write-mask-package`); graph anomalies → events; SightLoom 0.1.6 path pin

12. `rewrite_selectors` / `resolve-bridge --bindings`: host FramePick → SubjectIds (no photo I/O)
13. `RedactionKind` (`gaussian` default here, `pixelate`/`solid` for hosts): `resolve-bridge --style`, MCP `compile_and_bridge`/`bridge_graph` `style`
14. `RedactPii` (`license_plate` / `screen` / `text` / `document`): fail-closed if the freeze has no matching objects. People stay on blur_subject. No ONNX in Intelligence.

Host muxes source companion audio through `run_render_graph` (`with_audio` default). Intelligence does not emit `rf.audio.drop`.
Host MCP: stdio or `serve --http` (`POST /mcp`, loopback default, token required off-loopback).
Host ingest of Capture: session dir / CaptureProject / `capture:id` → committed video URIs only (no screen grab, no glob).
Studio (sibling, **public**): egui + Vite + mcport shim → Host HTTP. Agents use **Host MCP**, not a new repo and not LSP. Hosted = same tools + token. See Host `docs/USE-CASES.md`.

## Remaining (outside Intelligence core)

- crates.io tag publish (workflow + README benches ready; needs `CARGO_REGISTRY_TOKEN` + re-run benches on tag host)
- host `run_render_graph` / encode e2e (`reelforge graph --run` in ReelForge CLI)
- SightLoom host ONNX photo→embedding (SightLoom package, not this repo)
- ReelForge-Host orchestrator (new sibling)

## Benchmarks

`cargo bench -p reelforge-intelligence-core --bench pipeline` — resolve/compile/bridge/mask/serde.
See README table. Mask convert is batch O(n log n).

