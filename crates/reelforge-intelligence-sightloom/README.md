# reelforge-intelligence-sightloom

Loads a **SightLoom** `VisionIndex` package (or in-memory index) into
`reelforge-intelligence-core::AnalysisSnapshot` and wraps
[`SightLoomProvider`](https://docs.rs/reelforge-intelligence-core).

```text
VisionIndex package (gen-*/CURRENT)
        ↓
AnalysisSnapshot + subject bboxes
        ↓
SightLoomProvider  (AnalysisProvider)
        ↓
resolve_plan / materialize_masks
```

## Usage

```rust,no_run
use reelforge_intelligence_core::{
    resolve_plan, IntelligencePolicy, SemanticEdit, SemanticEditPlan, FrequencyMetric,
};
use reelforge_intelligence_sightloom::{
    load_package, provider_from_package, snapshot_from_index,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_package("./my-vision-index")?;
    let provider = loaded.provider();
    let pin = provider.generation();
    println!("generation={} hash={}", pin.generation, pin.content_hash);

    let intent = SemanticEditPlan::new(&loaded.snapshot.media).with_edit(
        SemanticEdit::BuildMostFrequentSubjectReel {
            metric: FrequencyMetric::AppearanceCount,
        },
    );
    let resolved = resolve_plan(&intent, &loaded.snapshot, IntelligencePolicy::default())?;
    println!("top subject: {}", resolved.resolved_subjects[0].id);
    Ok(())
}
```

## Note

Depends on crates.io `sightloom-index` **0.1.4**. For local bleeding-edge SightLoom,
override via `[patch]` in the workspace.
