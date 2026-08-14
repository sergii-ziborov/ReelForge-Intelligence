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

## Status

Public repo bootstrap. Core contracts + resolve + final compile stub. Host adapters that open real VisionIndex packages land next.

## License

MIT © Sergii Ziborov
