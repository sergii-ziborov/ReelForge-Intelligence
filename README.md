# ReelForge Intelligence

**Semantic bridge** between [SightLoom](https://github.com/sergii-ziborov/SightLoom) (understand / remember) and **ReelForge** (trim / mask / blur / encode).

```text
Human / Agent request
        ↓
SemanticEditPlan          ← intent (mutable, not reproducible alone)
        ↓
query SightLoom snapshot  ← host fills AnalysisSnapshot from VisionIndex
        ↓
ResolvedEditPlan          ← frozen subjects / events / masks / ranges / hashes
        ↓
compile_resolved          ← final RenderGraphIr + JSON
        ↓
approve (privacy Review)  ← optional operator gate
        ↓
bridge_to_reelforge       ← live reelforge_render_graph::RenderGraph + schedule
        ↓
ReelForge execute         ← host run_render_graph / encode
```

## Why two documents?

A request like *“find the most frequent person”* can change if VisionIndex updates between preview and final render.

| Document | Role |
| --- | --- |
| `SemanticEditPlan` | Intent only |
| `ResolvedEditPlan` | Frozen evidence + `source_hash` + `vision_index_generation` + `vision_index_hash` |

**Only `ResolvedEditPlan` compiles to a final RenderGraph** (`final_graph: true`).

## Product split

| Product | Owns |
| --- | --- |
| SightLoom | VisionIndex, subjects, tracks, masks, anomalies, ranking, evidence reel **handles** |
| **Intelligence (this repo)** | Semantic ops, freeze, compile IR |
| ReelForge | MaskTimeline, RegionRedaction, RenderGraph, scheduler, encode |
| Capture | CaptureProject |

## Semantic ops (v1 small set)

- `blur_subject`
- `blur_everyone_except`
- `follow_subject` (+ framing)
- `build_subject_reel`
- `build_most_frequent_subject_reel`
- `build_anomaly_reel` (e.g. after 22:00)
- `create_event_clips`

Low-level ops stay in ReelForge.

## Crate

```toml
reelforge-intelligence-core = "0.1"
```

```bash
cargo test -p reelforge-intelligence-core
```

## SightLoom context (do not re-implement)

Intelligence **queries** what SightLoom already provides: subject queries, appearances, visits, ranking, redaction intervals, uncertain intervals, patterns, anomalies, evidence reel handles, masks, photo search ranks.

It does **not** store VisionIndex or decode video.

## Technical bridges (v0.1)

| Problem | Solution |
| --- | --- |
| SightLoom `SubjectId(u64)` vs ReelForge `String` | **Namespaced IDs**: `sightloom://{gen}/subjects/184`, `…/sources/2/tracks/91` |
| Mask handles vs region samples | **MaskFidelity**: preview = bbox proxy; final = true RLE/dense/polygon request |
| Privacy edge cases | **PrivacyPolicy**: uncertain / missing_mask / low_confidence / track_gap |
| Multi-source analysis | **`AnalysisProvider` trait**; first impl **`SightLoomProvider`** |

## Crates

| Crate | Role |
| --- | --- |
| `reelforge-intelligence-core` | Intent, freeze, IDs, privacy, provider trait, compile |
| `reelforge-intelligence-sightloom` | Load SightLoom package → `AnalysisSnapshot` + `SightLoomProvider` |

```bash
cargo test -p reelforge-intelligence-sightloom
```

## Typed RenderGraph + approve

`compile_resolved` produces [`RenderGraphIr`](crates/reelforge-intelligence-core/src/render_graph.rs):

- nodes: `source` → `rf.adapter.sightloom` → semantic ops (`rf.redaction.region`, `rf.transform.concat`, …) → `output`
- pins: `vision_index_generation` / `vision_index_hash` / `source_hash`
- **approval gate** when privacy is `review` (`approve_and_render`)

Preview intent compile stays `final_graph: false`.

## Bridge to live ReelForge

`bridge_to_reelforge` / `compile_and_bridge` (dep: [`reelforge-render-graph`](https://crates.io/crates/reelforge-render-graph) 0.1.5):

| Intelligence IR | ReelForge |
| --- | --- |
| `rf.adapter.sightloom` | `Op` (registry extended if missing on crate tag) |
| `rf.redaction.region` | fused `Redaction` (empty `MaskTimeline`; host fills) |
| `rf.transform.trim` | `Op` with `start` + `duration` |
| `rf.transform.crop` | `Op` when `w`/`h` set; else skip (framing → host) |
| `rf.transform.concat` | multi-range → trims + `rf.compose.layers` |

Also: `schedule_graph` → `ExecutionPlan`, `HostRequest::Render.reelforge_graph_json`, MCP methods `compile_and_bridge` / `bridge_graph`.

```rust
use reelforge_intelligence_core::{
    bridge_to_reelforge, compile_resolved, BridgeOptions,
};

let report = compile_resolved(&resolved)?;
let ir = report.render_graph.as_ref().unwrap();
let live = bridge_to_reelforge(ir, &BridgeOptions {
    output_uri: Some("out.mp4".into()),
    require_approval: true,
    ..Default::default()
})?;
// host: run_render_graph(&live.graph)
```

No FFmpeg in Intelligence — only graph structure + schedule smoke.

## CLI / MCP host

```bash
cargo run -p reelforge-intelligence-cli -- methods
cargo run -p reelforge-intelligence-cli -- serve          # line JSON on stdio
cargo run -p reelforge-intelligence-cli -- resolve-bridge \
  --package /path/to/vision_index --plan intent.json --write-graph out_graph.json
```

Binary name: `reelforge-intelligence`. No FFmpeg; host runs ReelForge after handoff.

## Masks → MaskTimeline

`bridge_resolved` / `compile_and_bridge` convert `ResolvedMaskAsset.artifact.regions`
into fused ReelForge `MaskTimeline` samples on redaction nodes (bbox proxy or denser host samples).

## Publish

CI on `main`. Tag `v*` publishes crates.io when secret `CARGO_REGISTRY_TOKEN` is set.

## Status

Package load → freeze → typed IR → approve → live RenderGraph + **MaskTimeline** + **CLI/MCP host**.

## License

MIT © Sergii Ziborov
