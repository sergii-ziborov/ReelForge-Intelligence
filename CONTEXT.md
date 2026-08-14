# Context handoff: SightLoom → ReelForge Intelligence

This file keeps product memory when switching sessions/repos.

## SightLoom (done foundation, crates.io 0.1.4)

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
5. **ReelForge bridge** — `RenderGraphIr` → `RenderGraph` / `ExecutionPlan` (`reelforge-render-graph` 0.1.5)

## Next milestones

1. ~~Fill `AnalysisSnapshot` from real SightLoom package~~ **done**  
2. ~~Stronger RenderGraph schema~~ **done** (`RenderGraphIr` + op ids)  
3. ~~Approve workflow~~ **done** (`ApprovalRecord`, `approve_and_render`)  
4. ~~Map `RenderGraphIr` → live `reelforge::RenderGraph` / schedule~~ **done**  
5. MCP host binary packaging  
6. Optional: denser mask materialization into `MaskTimeline` before redaction

