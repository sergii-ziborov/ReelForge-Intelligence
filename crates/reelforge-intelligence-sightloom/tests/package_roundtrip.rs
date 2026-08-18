//! Build a minimal `VisionIndex` package, load it, freeze most-frequent subject.

#![allow(clippy::cast_possible_truncation)]

use reelforge_intelligence_core::{
    AnalysisProvider, FrequencyMetric, IntelligencePolicy, SemanticEdit, SemanticEditPlan,
    resolve_plan,
};
use reelforge_intelligence_core::{MaskFidelity, MaskGeometry, MaskRequest};
use reelforge_intelligence_sightloom::{
    encode_slm1_rle, export_and_pin_mask_package, load_package, snapshot_from_index,
    subject_boxes_from_index,
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

fn sample_with_mask(
    subject: u64,
    source: u32,
    frame: u64,
    left: f32,
    mask_ref: u64,
) -> TrackSample {
    let mut s = sample(subject, source, frame, left);
    s.mask_ref = mask_ref;
    s
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

#[test]
fn package_projects_timed_masks_frame_size_and_slm1() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VisionIndex::new("demo-cam");
    index.add_source(SourceEntry {
        source_id: 1,
        uri: "file://a.mp4".into(),
        hash: None,
    });
    let blob = encode_slm1_rle(4, 1, &[0, 2, 2]);
    let handle = index.masks.insert(blob);
    index.push_track(sample_with_mask(7, 1, 0, 10.0, handle.0));
    index.push_track(sample_with_mask(7, 1, 1, 14.0, handle.0));
    index.push_track(sample(7, 1, 2, 18.0));

    VisionIndexPackage::save(&index, dir.path()).unwrap();
    let loaded = load_package(dir.path()).unwrap();
    assert_eq!(loaded.snapshot.mask_samples.len(), 3);
    assert_eq!(loaded.snapshot.frame_width, Some(28));
    assert_eq!(loaded.snapshot.frame_height, Some(20));
    assert!(matches!(
        loaded.snapshot.mask_samples[0].geometry,
        Some(MaskGeometry::Rle {
            width: 4,
            height: 1,
            ..
        })
    ));

    let provider = loaded.provider();
    let sid = reelforge_intelligence_core::NamespacedId::sightloom_subject(&loaded.generation, 7);
    let art = provider
        .materialize_masks(&MaskRequest::final_subjects(
            vec![sid],
            vec![reelforge_intelligence_core::MediaRange::new(
                reelforge_intelligence_core::MediaTime::new(0, 1_000_000_000),
                reelforge_intelligence_core::MediaTime::new(1_000_000_000, 1_000_000_000),
            )],
        ))
        .unwrap();
    assert_eq!(art.fidelity, MaskFidelity::TrueGeometry);
    assert!(art.carries_true_geometry());
    assert_eq!(art.regions.len(), 2);
}

#[test]
fn mask_package_sidecar_pins_package_id_and_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VisionIndex::new("demo-cam");
    index.add_source(SourceEntry {
        source_id: 1,
        uri: "file://a.mp4".into(),
        hash: None,
    });
    index.push_track(sample(3, 1, 0, 10.0));
    VisionIndexPackage::save(&index, dir.path()).unwrap();

    let sidecar = serde_json::json!({
        "package_id": "pkg-host-1",
        "source_width": 1920,
        "source_height": 1080
    });
    std::fs::write(dir.path().join("mask-package.json"), sidecar.to_string()).unwrap();

    let loaded = load_package(dir.path()).unwrap();
    assert_eq!(
        loaded.snapshot.mask_package_id.as_deref(),
        Some("pkg-host-1")
    );
    assert_eq!(loaded.snapshot.frame_width, Some(1920));
    assert_eq!(loaded.snapshot.frame_height, Some(1080));
}

#[test]
fn export_mask_package_writes_reel_forge_layout() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VisionIndex::new("demo-cam");
    index.add_source(SourceEntry {
        source_id: 1,
        uri: "file://a.mp4".into(),
        hash: None,
    });
    let blob = encode_slm1_rle(4, 1, &[0, 2, 2]);
    let handle = index.masks.insert(blob);
    index.push_track(sample_with_mask(7, 1, 0, 10.0, handle.0));
    VisionIndexPackage::save(&index, dir.path()).unwrap();

    let mut loaded = load_package(dir.path()).unwrap();
    let dest = dir.path().join("mask_package");
    let exported = export_and_pin_mask_package(&mut loaded, &dest).unwrap();
    assert_eq!(exported.blob_count, 1);
    assert!(dest.join("manifest.json").is_file());
    assert!(dest.join(format!("masks/{}.bin", handle.0)).is_file());
    let bytes = std::fs::read(dest.join(format!("masks/{}.bin", handle.0))).unwrap();
    assert_eq!(bytes.len(), 4);
    assert_eq!(bytes, vec![255, 255, 0, 0]);
    assert_eq!(
        loaded.snapshot.mask_package_id.as_deref(),
        Some(exported.package_id.as_str())
    );
    let pointer = std::fs::read_to_string(dir.path().join("mask-package.json")).unwrap();
    assert!(pointer.contains(&exported.package_id));
}

#[test]
fn graph_anomalies_become_events() {
    let mut index = VisionIndex::new("demo-cam");
    index.anomalies.push(AnomalyEvent {
        anomaly_id: AnomalyId(42),
        score: 0.99,
        severity: Severity::High,
        reasons: vec![AnomalyReason::ImpossibleCrossCameraHop],
        evidence: Vec::new(),
        subject_id: Some(SubjectId(3)),
        source_id: Some(SourceId(1)),
        at: media_time(5_000_000_000),
    });
    let snap = snapshot_from_index(&index, "gen-1", "h", "s");
    assert!(
        snap.events
            .iter()
            .any(|e| e.kind == "impossible_cross_camera_hop" && e.subject_id == Some(3))
    );
    assert!(
        snap.anomalies
            .iter()
            .any(|a| a.kind == "impossible_cross_camera_hop")
    );
}

#[test]
fn resolve_reel_hands_host_mask_package_and_concat() {
    use reelforge_intelligence_core::{
        BridgeOptions, FrequencyMetric, HostRequest, IntelligenceService, SemanticEdit,
        SemanticEditPlan, compile_and_bridge, op_id,
    };

    let dir = tempfile::tempdir().unwrap();
    let mut index = VisionIndex::new("demo-cam");
    index.add_source(SourceEntry {
        source_id: 1,
        uri: "file://a.mp4".into(),
        hash: None,
    });
    let blob = encode_slm1_rle(4, 1, &[0, 2, 2]);
    let handle = index.masks.insert(blob);
    for f in 0..4 {
        index.push_track(sample_with_mask(2, 1, f, 50.0, handle.0));
    }
    index.subjects.push(SubjectProfile {
        subject_id: SubjectId(2),
        label: Some("bob".into()),
        appearance_count: 4,
        source_count: 1,
        total_duration_ns: 4_000_000_000,
        first_seen: Some(media_time(0)),
        last_seen: Some(media_time(3_000_000_000)),
        embedding: None,
    });
    index.appearances.push(Appearance {
        appearance_id: sightloom_core::AppearanceId(1),
        subject_id: Some(SubjectId(2)),
        track_id: Some(TrackId(2)),
        source_id: SourceId(1),
        start: media_time(0),
        end: media_time(1_000_000_000),
        class_id: None,
        peak_confidence: 0.9,
        evidence: None,
    });
    index.appearances.push(Appearance {
        appearance_id: sightloom_core::AppearanceId(2),
        subject_id: Some(SubjectId(2)),
        track_id: Some(TrackId(2)),
        source_id: SourceId(1),
        start: media_time(2_000_000_000),
        end: media_time(3_000_000_000),
        class_id: None,
        peak_confidence: 0.9,
        evidence: None,
    });
    VisionIndexPackage::save(&index, dir.path()).unwrap();

    let mut loaded = load_package(dir.path()).unwrap();
    let dest = dir.path().join("mask_package");
    export_and_pin_mask_package(&mut loaded, &dest).unwrap();

    let intent = SemanticEditPlan::new(&loaded.snapshot.media).with_edit(
        SemanticEdit::BuildMostFrequentSubjectReel {
            metric: FrequencyMetric::AppearanceCount,
        },
    );
    let svc = IntelligenceService::new();
    let resolved = svc.resolve_plan(&intent, &loaded.snapshot).unwrap();
    assert_eq!(resolved.resolved_ranges.len(), 2);
    assert_eq!(
        resolved.mask_package_id.as_deref(),
        loaded.snapshot.mask_package_id.as_deref()
    );
    assert!(
        resolved
            .mask_package_uri
            .as_ref()
            .is_some_and(|u| u.contains("mask_package"))
    );

    let (report, bridged) = compile_and_bridge(&resolved, &BridgeOptions::default()).unwrap();
    assert_eq!(report.mask_package_id, resolved.mask_package_id);
    assert!(bridged.graph_json.contains("rf.timeline.concat"));
    assert!(
        bridged.graph_json.contains("package_id")
            || report
                .render_graph_json
                .as_ref()
                .is_some_and(|j| j.contains("package_id"))
    );

    let HostRequest::Render {
        mask_package_id,
        mask_package_uri,
        reelforge_graph_json,
        ..
    } = svc.render_resolved(&resolved).unwrap()
    else {
        panic!("expected render request");
    };
    assert_eq!(mask_package_id, resolved.mask_package_id);
    assert!(mask_package_uri.is_some());
    assert!(reelforge_graph_json.as_ref().is_some_and(|j| {
        j.contains(op_id::TIMELINE_CONCAT)
            || j.contains("rf.timeline.concat")
            || j.contains("rf.transform.trim")
    }));
}
