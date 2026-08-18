//! Criterion microbenchmarks for the Intelligence contract path.
//!
//! These measure **CPU-side** freeze/compile/bridge work only — no FFmpeg,
//! no model inference, no package I/O.
//!
//! ```text
//! cargo bench -p reelforge-intelligence-core --bench pipeline
//! ```

#![allow(missing_docs, clippy::all, clippy::pedantic)]

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use reelforge_intelligence_core::{
    AnalysisSnapshot, BridgeOptions, FrequencyMetric, IntelligencePolicy, MaskArtifact,
    MaskFidelity, RegionSample, SemanticEdit, SemanticEditPlan, SubjectEvidence, bridge_resolved,
    bridge_to_reelforge, compile_resolved, graph_from_resolved, mask_timeline_from_regions,
    resolve_plan,
};
use reelforge_intelligence_core::{MediaRange, MediaTime, ResolvedMaskAsset};
use std::time::Duration;

fn subject(id: u64, appearances: u64) -> SubjectEvidence {
    SubjectEvidence {
        subject_id: id,
        label: Some(format!("s{id}")),
        appearance_count: appearances,
        source_ids: vec![1, 2],
        first_ticks: 0,
        last_ticks: 10_000_000_000,
        confidence: Some(0.9),
        ..SubjectEvidence::default()
    }
    .with_visit(0, 10_000_000_000)
}

fn snapshot(n_subjects: usize) -> AnalysisSnapshot {
    let subjects: Vec<_> = (0..n_subjects)
        .map(|i| {
            let id = i as u64 + 1;
            // Make subject N most frequent for ranking work.
            let count = if i + 1 == n_subjects {
                10_000
            } else {
                (i as u64 % 50) + 1
            };
            subject(id, count)
        })
        .collect();
    AnalysisSnapshot {
        media: "bench-cam".into(),
        source_hash: "src-bench".into(),
        vision_index_generation: "gen-bench".into(),
        vision_index_hash: "idx-bench".into(),
        timescale: 1_000_000_000,
        subjects,
        anomalies: Vec::new(),
        ..AnalysisSnapshot::default()
    }
}

fn intent_most_frequent() -> SemanticEditPlan {
    SemanticEditPlan::new("bench-cam").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
        metric: FrequencyMetric::AppearanceCount,
    })
}

fn intent_blur() -> SemanticEditPlan {
    SemanticEditPlan::new("bench-cam").with_edit(SemanticEdit::BlurSubject {
        subject: reelforge_intelligence_core::SubjectSelector::MostFrequent {
            metric: FrequencyMetric::AppearanceCount,
        },
    })
}

fn resolved_with_masks(
    n_subjects: usize,
    n_regions: usize,
) -> reelforge_intelligence_core::ResolvedEditPlan {
    let snap = snapshot(n_subjects);
    let intent = intent_blur();
    let mut resolved = resolve_plan(&intent, &snap, IntelligencePolicy::default()).unwrap();
    let regions: Vec<_> = (0..n_regions)
        .map(|i| {
            let t = i as i64 * 33_333_333; // ~30 fps
            RegionSample {
                at: MediaTime::new(t, 1_000_000_000),
                box_xyxy: [100.0 + i as f32, 80.0, 220.0 + i as f32, 300.0],
                subject: resolved.resolved_subjects.first().map(|s| s.id.clone()),
                confidence: Some(0.95),
                geometry: None,
            }
        })
        .collect();
    resolved.resolved_masks.push(ResolvedMaskAsset {
        mask_id: None,
        mask_ref: None,
        subject: resolved.resolved_subjects.first().map(|s| s.id.clone()),
        range: Some(MediaRange::new(
            MediaTime::new(0, 1_000_000_000),
            MediaTime::new(10_000_000_000, 1_000_000_000),
        )),
        fidelity: MaskFidelity::BBoxProxy,
        artifact: Some(MaskArtifact::from_regions(regions)),
    });
    resolved
}

fn bench_resolve(c: &mut Criterion) {
    let mut g = c.benchmark_group("resolve");
    g.measurement_time(Duration::from_secs(3));
    g.sample_size(50);
    let intent = intent_most_frequent();
    let policy = IntelligencePolicy::default();
    for n in [10usize, 100, 1_000, 5_000] {
        let snap = snapshot(n);
        g.bench_with_input(BenchmarkId::new("most_frequent", n), &n, |b, _| {
            b.iter(|| {
                let r = resolve_plan(
                    black_box(&intent),
                    black_box(&snap),
                    black_box(policy.clone()),
                )
                .unwrap();
                black_box(r)
            });
        });
    }
    g.finish();
}

fn bench_compile(c: &mut Criterion) {
    let mut g = c.benchmark_group("compile");
    g.measurement_time(Duration::from_secs(3));
    g.sample_size(50);
    let resolved = resolve_plan(
        &intent_most_frequent(),
        &snapshot(100),
        IntelligencePolicy::default(),
    )
    .unwrap();
    g.bench_function("compile_resolved_100subj", |b| {
        b.iter(|| {
            let report = compile_resolved(black_box(&resolved)).unwrap();
            black_box(report)
        });
    });
    g.bench_function("graph_from_resolved_100subj", |b| {
        b.iter(|| {
            let ir = graph_from_resolved(black_box(&resolved));
            black_box(ir)
        });
    });
    g.finish();
}

fn bench_bridge(c: &mut Criterion) {
    let mut g = c.benchmark_group("bridge");
    g.measurement_time(Duration::from_secs(4));
    g.sample_size(40);
    let opts = BridgeOptions {
        output_uri: Some("out.mp4".into()),
        schedule: true,
        ..BridgeOptions::default()
    };
    let resolved = resolved_with_masks(100, 120);
    let ir = graph_from_resolved(&resolved);

    g.bench_function("bridge_ir_schedule", |b| {
        b.iter(|| {
            let r = bridge_to_reelforge(black_box(&ir), black_box(&opts)).unwrap();
            black_box(r)
        });
    });
    g.bench_function("bridge_resolved_120masks_schedule", |b| {
        b.iter(|| {
            let r = bridge_resolved(black_box(&resolved), black_box(&opts)).unwrap();
            black_box(r)
        });
    });
    let no_sched = BridgeOptions {
        schedule: false,
        ..opts.clone()
    };
    g.bench_function("bridge_resolved_no_schedule", |b| {
        b.iter(|| {
            let r = bridge_resolved(black_box(&resolved), black_box(&no_sched)).unwrap();
            black_box(r)
        });
    });
    g.finish();
}

fn bench_pipeline(c: &mut Criterion) {
    let mut g = c.benchmark_group("pipeline");
    g.measurement_time(Duration::from_secs(4));
    g.sample_size(40);
    let intent = intent_blur();
    let policy = IntelligencePolicy::default();
    let opts = BridgeOptions {
        output_uri: Some("out.mp4".into()),
        schedule: true,
        ..BridgeOptions::default()
    };
    for n in [10usize, 100, 1_000] {
        let snap = snapshot(n);
        g.bench_with_input(BenchmarkId::new("resolve_compile_bridge", n), &n, |b, _| {
            b.iter(|| {
                let mut resolved =
                    resolve_plan(black_box(&intent), black_box(&snap), policy.clone()).unwrap();
                // Attach a few regions like a host would after materialize_masks.
                resolved.resolved_masks.push(ResolvedMaskAsset {
                    mask_id: None,
                    mask_ref: None,
                    subject: resolved.resolved_subjects.first().map(|s| s.id.clone()),
                    range: None,
                    fidelity: MaskFidelity::BBoxProxy,
                    artifact: Some(MaskArtifact::from_regions(vec![RegionSample {
                        at: MediaTime::new(0, 1_000_000_000),
                        box_xyxy: [10.0, 20.0, 110.0, 220.0],
                        subject: None,
                        confidence: Some(1.0),
                        geometry: None,
                    }])),
                });
                let report = compile_resolved(&resolved).unwrap();
                let bridged = bridge_resolved(&resolved, &opts).unwrap();
                black_box((report, bridged))
            });
        });
    }
    g.finish();
}

fn bench_mask_timeline(c: &mut Criterion) {
    let mut g = c.benchmark_group("mask_timeline");
    g.measurement_time(Duration::from_secs(2));
    g.sample_size(40);
    for n in [100usize, 1_000, 10_000] {
        let regions: Vec<_> = (0..n)
            .map(|i| RegionSample {
                at: MediaTime::new(i as i64 * 1_000_000, 1_000_000_000),
                box_xyxy: [i as f32, 0.0, i as f32 + 40.0, 40.0],
                subject: None,
                confidence: Some(0.8),
                geometry: None,
            })
            .collect();
        g.bench_with_input(BenchmarkId::new("from_regions", n), &n, |b, _| {
            b.iter(|| {
                let tl = mask_timeline_from_regions(black_box(&regions));
                black_box(tl)
            });
        });
    }
    g.finish();
}

fn bench_serde(c: &mut Criterion) {
    let mut g = c.benchmark_group("serde");
    g.measurement_time(Duration::from_secs(2));
    g.sample_size(50);
    let resolved = resolved_with_masks(100, 60);
    let report = compile_resolved(&resolved).unwrap();
    let ir = report.render_graph.as_ref().unwrap();
    let ir_json = ir.to_json().unwrap();
    g.bench_function("render_graph_ir_to_json", |b| {
        b.iter(|| black_box(ir.to_json().unwrap()));
    });
    g.bench_function("render_graph_ir_from_json", |b| {
        b.iter(|| {
            let v: reelforge_intelligence_core::RenderGraphIr =
                serde_json::from_str(black_box(&ir_json)).unwrap();
            black_box(v)
        });
    });
    let bridged = bridge_resolved(
        &resolved,
        &BridgeOptions {
            schedule: true,
            ..BridgeOptions::default()
        },
    )
    .unwrap();
    g.bench_function("reelforge_graph_to_json", |b| {
        b.iter(|| black_box(bridged.graph.to_json_pretty().unwrap()));
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_resolve,
    bench_compile,
    bench_bridge,
    bench_pipeline,
    bench_mask_timeline,
    bench_serde
);
criterion_main!(benches);
