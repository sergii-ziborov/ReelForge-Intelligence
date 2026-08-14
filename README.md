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
compile_resolved          ← final RenderGraph JSON (ReelForge-shaped)
        ↓
validate / preview / approve
        ↓
ReelForge execute
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

## Status

Package load → freeze → typed graph → approve gate. Next: map `RenderGraphIr` into live `reelforge::RenderGraph` / ExecutionPlan.

## License

MIT © Sergii Ziborov
