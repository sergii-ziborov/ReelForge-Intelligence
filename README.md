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

## Crates

| Crate | Role |
| --- | --- |
| `reelforge-intelligence-core` | Intent, freeze, IDs, privacy, provider, compile, bridge |
| `reelforge-intelligence-sightloom` | Load SightLoom package → `AnalysisSnapshot` |
| `reelforge-intelligence-cli` | `reelforge-intelligence` binary (stdio MCP + resolve-bridge) |

```bash
cargo test --workspace
cargo bench -p reelforge-intelligence-core --bench pipeline
```

## Competitive landscape

There is **no crates.io package** that owns the same product slice: *semantic privacy/edit intent → frozen evidence pins → executable RenderGraph*. Nearby tools solve adjacent problems:

| Tool / crate | What it does | What it does **not** do vs Intelligence |
| --- | --- | --- |
| [OpenChatCut](https://openchatcut.com) / agent NLEs | Agent + multitrack timeline UI | No VisionIndex freeze hashes; not a Rust library contract |
| [ClipAsm](https://lib.rs/crates/clipasm) | Typed stack language for A/V graphs | No subject/privacy semantics, no analysis freeze |
| [motionloom](https://crates.io/crates/motionloom) | Motion/scene DSL for effects | Graphics-oriented, not privacy reels from tracking |
| [oximedia-*](https://crates.io/crates/oximedia-timeline) | Timeline / EDL collab | No SightLoom IDs, no privacy Review gate |
| [Remotion](https://www.remotion.dev) / [Rendiv](https://github.com/thecodacus/rendiv) | React → MP4 | Code-first frames, not vision-index resolve |
| [Shotstack](https://shotstack.io) / MoviePy / FFmpeg CLIs | Cloud or scripted edit/encode | Host executes ops; no freeze document between preview and final |
| Commercial redaction (SAR tools, Premiere blur) | Manual or CV black-box UI | Not embeddable as intent→IR→schedule library |
| [ReelForge](https://crates.io/crates/reelforge) + [SightLoom](https://crates.io/crates/sightloom) | Encode graph + vision memory | **Complement** this layer; Intelligence sits between them |

**Differentiation:** namespaced IDs (`sightloom://…`), `ResolvedEditPlan` pins (`source_hash` / generation / index hash), privacy Review → approve, and a typed bridge into `reelforge-render-graph` with `schedule_graph` smoke — without linking FFmpeg or model weights.

## Benchmarks

CPU-only path (no FFmpeg, no ONNX). Measured with Criterion on a Windows development host, release profile (`cargo bench -p reelforge-intelligence-core --bench pipeline`). Times are **median-ish midpoints** from Criterion’s `[lo mid hi]` bands — re-run on your machine for CI gates.

| Workload | N | Time (approx.) |
| --- | ---: | ---: |
| `resolve` most-frequent subject | 10 subjects | **~6 µs** |
| `resolve` most-frequent | 100 | **~6 µs** |
| `resolve` most-frequent | 1 000 | **~11 µs** |
| `resolve` most-frequent | 5 000 | **~12 µs** |
| `compile_resolved` | 100 subjects | **~18 µs** |
| `graph_from_resolved` | 100 subjects | **~15 µs** |
| `bridge_to_reelforge` + `schedule_graph` | IR only | **~170 µs** |
| `bridge_resolved` + 120 mask samples + schedule | 100 subjects | **~480 µs** |
| full `resolve → compile → bridge` | 10 / 100 / 1 000 subjects | **~200 / ~170 / ~320 µs** |
| `mask_timeline_from_regions` | 100 samples | **~6 µs** |
| `mask_timeline_from_regions` | 1 000 | **~74 µs** |
| `mask_timeline_from_regions` | 10 000 | **~800 µs** |
| IR JSON serialize / deserialize | — | **~5 µs / ~26 µs** |
| live `RenderGraph` pretty JSON | with masks | **~240 µs** |

Notes:

- Resolve is essentially free at thousands of subject rows (linear scan + max).
- Bridge + schedule dominate the handoff (~0.2–0.5 ms), still well under typical frame time.
- Mask conversion is **O(n log n)** (batch sort). An earlier per-`push` sort path was ~1.4 s at 10 k samples; batch build dropped that to **~0.8 ms**.
- These are **not** encode benchmarks. Pixel redaction / mux lives in [ReelForge](https://github.com/sergii-ziborov/ReelForge) (`cargo bench -p reelforge-fx`, `privacy_e2e`).

```bash
cargo bench -p reelforge-intelligence-core --bench pipeline
# HTML: target/criterion/report/index.html
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
| `rf.redaction.region` | fused `Redaction` (`MaskTimeline` from freeze when samples present) |
| `rf.transform.trim` | `Op` with `start` + `duration` |
| `rf.transform.crop` | `Op` when `w`/`h` set; else skip (framing → host) |
| `rf.timeline.concat` | multi-range → sequential trims + concat (not `compose.layers`) |

Also: `schedule_graph` → `ExecutionPlan`, `HostRequest::Render` (`reelforge_graph_json`, `mask_package_id`, `mask_package_uri`), MCP methods `compile_and_bridge` / `bridge_graph`.

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
cargo run -p reelforge-intelligence-cli -- serve          # JSON-RPC 2.0 MCP on stdio
cargo run -p reelforge-intelligence-cli -- serve --legacy # old line protocol
cargo run -p reelforge-intelligence-cli -- resolve-bridge \
  --package /path/to/vision_index --plan intent.json --write-graph out_graph.json \
  --bindings hits.json \
  --style gaussian
```

`--style` / MCP `style`: `gaussian` (compiler default, recoverable) | `pixelate` | `solid`. Hosts that need anonymity should pass `pixelate`.

`redact_pii` blurs plates / screens / text / documents already on the freeze. Missing evidence is an error — Intelligence does not detect PII. People stay on `blur_subject` / `blur_everyone_except`.

Photo / `frame_pick` materialization is **host search + `rewrite_selectors`**. Intelligence never opens JPEGs. Host MCP method: `rewrite_selectors`. Then encode with `reelforge graph --run`.

Binary name: `reelforge-intelligence`. No FFmpeg; host runs ReelForge after handoff.

## Masks → MaskTimeline

`bridge_resolved` / `compile_and_bridge` convert `ResolvedMaskAsset.artifact.regions`
into fused ReelForge `MaskTimeline` samples on redaction nodes (bbox proxy or denser host samples).

## Publish (crates.io)

Workflow: tag `v*` → `.github/workflows/publish.yml` (secret `CARGO_REGISTRY_TOKEN`).

**Policy:** do not publish a version whose README lacks measured benches for that tag. Numbers above are from the `pipeline` Criterion suite on the host that cut this release branch; re-run benches before tagging.

## Status

Package load → freeze → typed IR → approve → live RenderGraph + MaskTimeline + CLI/MCP + **Criterion benches in README**.

## License

MIT © Sergii Ziborov
