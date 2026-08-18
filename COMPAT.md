# Compatibility truth

Intelligence sits between SightLoom and ReelForge. This file is the pin, not a comment in CONTEXT.md.

| Layer | This workspace | Notes |
| --- | --- | --- |
| **ReelForge Intelligence** | `0.1.0` (this repo) | Semantic intent → freeze → IR → approve → bridge |
| **SightLoom** | path `../SightLoom` @ `0.1.6` | Appearances, tracks, mask store, graph anomalies |
| **ReelForge** | path `../ReelForge` @ `0.2.0` | `MaskAsset`, `MaskPackage`, `rf.timeline.concat` execute |

## What Intelligence requires from ReelForge 0.2

- `MaskSample.asset` / `MaskSample::with_asset`
- `MaskAsset::{Dense,Rle,Polygon,External}`
- `OperationDescriptor::new` / `nary` (non-exhaustive)
- `rf.timeline.concat` as **n-ary** `concatenate_video` (not `compose.layers`)
- Host `MaskPackage` + `SightloomPackageHost` for `External` silhouettes

crates.io `reelforge-render-graph 0.1.5` is **not** this API. Workspace deps are sibling paths.

## Sibling CI

GitHub checks out this repo as `intelligence/` and siblings next to it:

```
intelligence/   ← this repo (working-directory)
ReelForge/      ← sergii-ziborov/ReelForge
SightLoom/      ← sergii-ziborov/SightLoom
```

That matches local `../ReelForge` and `../SightLoom`.

CLI: `resolve-bridge --write-mask-package DIR` writes a host-ready MaskPackage
(`manifest.json` + `masks/{ref}.bin` + SHA-256) and pins `mask_package_id`.

## Masks

Timed track samples land on `AnalysisSnapshot.mask_samples`.

True geometry:

- `SLM1` blob decodes inline to RLE/dense
- otherwise `MaskGeometry::External { package_id, mask_ref }`
- `package_id` is the ReelForge **MaskPackage** id when `mask-package.json` (or `mask_package/manifest.json`) sits next to the VisionIndex; else the VisionIndex generation
- adapter IR params include `package_id` so `SightloomPackageHost` can match
- `source_width` / `source_height` from that sidecar override inferred box size

## Hashes

SHA-256 of canonical JSON. Optional `RF_INTEL_APPROVAL_HMAC`. Package adapter hashes payload bytes.

## MCP

`serve` is JSON-RPC 2.0. `serve --legacy` is the old line protocol.
