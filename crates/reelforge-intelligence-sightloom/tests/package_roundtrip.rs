//! Build a minimal `VisionIndex` package, load it, freeze most-frequent subject.

#![allow(clippy::cast_possible_truncation)]

use reelforge_intelligence_core::{
    AnalysisProvider, FrequencyMetric, IntelligencePolicy, SemanticEdit, SemanticEditPlan,
    resolve_plan,
};
use reelforge_intelligence_sightloom::{
    load_package, snapshot_from_index, subject_boxes_from_index,
};
use sightloom_analysis::{AnomalyEvent, AnomalyReason, Severity};
use sightloom_core::{AnomalyId, ClassId, MediaTime, SourceId, SubjectId, TrackId};
use sightloom_index::{
    Appearance, SourceEntry, SubjectProfile, TrackSample, VisionIndex, VisionIndexPackage,
};
use std::path::PathBuf;

fn media_time(ticks: i64) -> MediaTime {
    MediaTime::new(ticks, 1_000_000_000).unwrap()
}

fn sample(subject: u64, source: u32, frame: u64, left: f32) -> TrackSample {
    TrackSample {
        sample_id: 0,
        supersedes: None,
        revision: 1,
        idempotency_key: 0,
        source_id: SourceId(source),
        frame_index: frame,
        pts: media_time(i64::try_from(frame).unwrap() * 1_000_000_000),
        track_id: TrackId(subject as u32),
        track_uid: None,
        subject_id: Some(SubjectId(subject)),
        class_id: Some(ClassId(0)),
        left,
        top: 0.0,
        right: left + 10.0,
        bottom: 20.0,
        confidence: 0.9,
        mask_ref: 0,
    }
}

#[test]
fn package_load_resolve_most_frequent() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VisionIndex::new("demo-cam");
    index.add_source(SourceEntry {
        source_id: 1,
        uri: "file://a.mp4".into(),
        hash: None,
    });
    index.add_source(SourceEntry {
        source_id: 2,
        uri: "file://b.mp4".into(),
        hash: None,
    });

    // Alice few frames, Bob many
    for f in 0..3 {
        index.push_track(sample(1, 1, f, 1.0));
    }
    for f in 0..12 {
        index.push_track(sample(2, 2, f, 50.0));
    }

    index.subjects.push(SubjectProfile {
        subject_id: SubjectId(1),
        label: Some("alice".into()),
        appearance_count: 3,
        source_count: 1,
        total_duration_ns: 3_000_000_000,
        first_seen: Some(media_time(0)),
        last_seen: Some(media_time(2_000_000_000)),
        embedding: None,
    });
    index.subjects.push(SubjectProfile {
        subject_id: SubjectId(2),
        label: Some("bob".into()),
        appearance_count: 12,
        source_count: 1,
        total_duration_ns: 12_000_000_000,
        first_seen: Some(media_time(0)),
        last_seen: Some(media_time(11_000_000_000)),
        embedding: None,
    });

    index.appearances.push(Appearance {
        appearance_id: sightloom_core::AppearanceId(1),
        subject_id: Some(SubjectId(2)),
        track_id: Some(TrackId(2)),
        source_id: SourceId(2),
        start: media_time(0),
        end: media_time(11_000_000_000),
        class_id: None,
        peak_confidence: 0.9,
        evidence: None,
    });

    index.anomalies.push(AnomalyEvent {
        anomaly_id: AnomalyId(7),
        score: 0.95,
        severity: Severity::High,
        reasons: vec![AnomalyReason::UnusualRoute],
        evidence: Vec::new(),
        subject_id: Some(SubjectId(2)),
        source_id: Some(SourceId(2)),
        at: media_time(80_000_000_000), // late
    });

    VisionIndexPackage::save(&index, dir.path()).unwrap();

    let loaded = load_package(dir.path()).unwrap();
    assert!(!loaded.generation.is_empty());
    assert!(!loaded.content_hash.is_empty());
    assert_eq!(loaded.snapshot.subjects.len(), 2);

    let boxes = subject_boxes_from_index(&loaded.index);
    assert!(boxes.iter().any(|(id, _)| *id == 2));

    let intent = SemanticEditPlan::new(&loaded.snapshot.media).with_edit(
        SemanticEdit::BuildMostFrequentSubjectReel {
            metric: FrequencyMetric::AppearanceCount,
        },
    );
    let resolved = resolve_plan(&intent, &loaded.snapshot, IntelligencePolicy::default()).unwrap();
    assert_eq!(resolved.resolved_subjects.len(), 1);
    assert!(
        resolved.resolved_subjects[0]
            .id
            .as_uri()
            .contains("/subjects/2"),
        "got {}",
        resolved.resolved_subjects[0].id
    );

    let provider = loaded.provider();
    let pin = provider.generation();
    assert_eq!(pin.provider_id, "sightloom");
    assert_eq!(pin.generation, loaded.generation);

    // snapshot_from_index standalone
    let snap2 = snapshot_from_index(
        &loaded.index,
        &loaded.generation,
        &loaded.content_hash,
        &loaded.source_hash,
    );
    assert_eq!(snap2.subjects.len(), loaded.snapshot.subjects.len());

    let _ = PathBuf::from(dir.path());
}
