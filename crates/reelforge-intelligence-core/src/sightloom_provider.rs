//! First analysis provider: SightLoom VisionIndex evidence (host-filled snapshot).
//!
//! This crate does not open packages or link ONNX. The host builds
//! [`crate::AnalysisSnapshot`] from SightLoom and wraps it here.

use crate::error::{IntelError, Result};
use crate::ids::NamespacedId;
use crate::mask::{MaskArtifact, MaskFidelity, MaskGeometry, MaskRequest, RegionSample};
use crate::provider::{
    AnalysisGeneration, AnalysisProvider, EventResult, SubjectQuery, SubjectResult,
};
use crate::query::EventQuery;
use crate::resolve::{AnalysisSnapshot, SubjectEvidence};
use crate::selector::SubjectSelector;
use crate::time::{MediaRange, MediaTime};

/// SightLoom-backed provider over an in-memory [`AnalysisSnapshot`].
#[derive(Debug, Clone)]
pub struct SightLoomProvider {
    /// Snapshot frozen by the host from a VisionIndex generation.
    pub snapshot: AnalysisSnapshot,
    /// Optional per-subject bbox for preview mask materialization: `(subject, [l,t,r,b])`.
    pub subject_boxes: Vec<(u64, [f32; 4])>,
}

impl SightLoomProvider {
    /// Wrap a host snapshot.
    #[must_use]
    pub fn new(snapshot: AnalysisSnapshot) -> Self {
        Self {
            snapshot,
            subject_boxes: Vec::new(),
        }
    }

    /// Attach preview bboxes (pixel or normalized — host contract).
    #[must_use]
    pub fn with_subject_boxes(mut self, boxes: Vec<(u64, [f32; 4])>) -> Self {
        self.subject_boxes = boxes;
        self
    }

    fn index_id(&self) -> &str {
        if self.snapshot.vision_index_generation.is_empty() {
            "unknown"
        } else {
            &self.snapshot.vision_index_generation
        }
    }

    fn to_subject_result(&self, s: &SubjectEvidence) -> SubjectResult {
        let ts = self.snapshot.timescale.max(1);
        SubjectResult {
            id: NamespacedId::sightloom_subject(self.index_id(), s.subject_id),
            label: s.label.clone(),
            appearance_count: s.appearance_count,
            source_ids: s.source_ids.clone(),
            span: Some(MediaRange::new(
                MediaTime::new(s.first_ticks, ts),
                MediaTime::new(s.last_ticks, ts),
            )),
            confidence: s.confidence,
        }
    }
}

impl AnalysisProvider for SightLoomProvider {
    fn generation(&self) -> AnalysisGeneration {
        AnalysisGeneration {
            provider_id: "sightloom".into(),
            generation: self.snapshot.vision_index_generation.clone(),
            content_hash: self.snapshot.vision_index_hash.clone(),
            source_hash: Some(self.snapshot.source_hash.clone()),
        }
    }

    fn query_subjects(&self, query: &SubjectQuery) -> Result<Vec<SubjectResult>> {
        let mut rows: Vec<SubjectResult> = match &query.selector {
            None => self
                .snapshot
                .subjects
                .iter()
                .map(|s| self.to_subject_result(s))
                .collect(),
            Some(SubjectSelector::SubjectIds { ids }) => self
                .snapshot
                .subjects
                .iter()
                .filter(|s| ids.contains(&s.subject_id))
                .map(|s| self.to_subject_result(s))
                .collect(),
            Some(SubjectSelector::MostFrequent { metric }) => {
                use crate::edit::FrequencyMetric;
                let best = self.snapshot.subjects.iter().max_by(|a, b| match metric {
                    FrequencyMetric::AppearanceCount => a.appearance_count.cmp(&b.appearance_count),
                    FrequencyMetric::SourceCount => a
                        .source_ids
                        .len()
                        .cmp(&b.source_ids.len())
                        .then_with(|| a.appearance_count.cmp(&b.appearance_count)),
                    FrequencyMetric::Duration => {
                        a.visible_duration_ticks().cmp(&b.visible_duration_ticks())
                    }
                });
                best.map(|s| vec![self.to_subject_result(s)])
                    .unwrap_or_default()
            }
            Some(SubjectSelector::SubjectSet { name }) => {
                let Some(ids) = self.snapshot.subject_sets.get(name) else {
                    return Err(IntelError::message(format!(
                        "SightLoomProvider: subject set '{name}' is not in snapshot.subject_sets"
                    )));
                };
                self.snapshot
                    .subjects
                    .iter()
                    .filter(|s| ids.contains(&s.subject_id))
                    .map(|s| self.to_subject_result(s))
                    .collect()
            }
            Some(SubjectSelector::FramePick { .. } | SubjectSelector::TrackIds { .. }) => {
                return Err(IntelError::message(
                    "SightLoomProvider: frame_pick/track_ids need host materialization into SubjectIds",
                ));
            }
        };

        if let Some(min_c) = query.min_confidence {
            rows.retain(|r| r.confidence.is_none_or(|c| c >= min_c));
        }
        if query.limit > 0 {
            rows.truncate(query.limit);
        }
        Ok(rows)
    }

    fn query_events(&self, query: &EventQuery) -> Result<Vec<EventResult>> {
        let ts = self.snapshot.timescale.max(1);
        let mut out = Vec::new();
        match query {
            EventQuery::Custom { expr } => {
                let needle = expr.to_lowercase();
                for e in &self.snapshot.events {
                    if e.kind.to_lowercase().contains(&needle)
                        || e.event_id.to_lowercase().contains(&needle)
                    {
                        out.push(event_from_row(e, self.index_id(), ts));
                    }
                }
                for a in &self.snapshot.anomalies {
                    if a.kind.to_lowercase().contains(&needle) {
                        out.push(event_from_anomaly(a, self.index_id(), ts));
                    }
                }
            }
            EventQuery::ZoneEnters { zone_id } => {
                for e in &self.snapshot.events {
                    if e.zone_id == Some(*zone_id) {
                        out.push(event_from_row(e, self.index_id(), ts));
                    }
                }
            }
            EventQuery::IdleRanges { min_seconds } => {
                let min_ticks = crate::time::MediaTime::from_secs_f64(*min_seconds, ts).ticks;
                for e in &self.snapshot.events {
                    if e.kind.to_lowercase().contains("idle")
                        && (e.end_ticks - e.start_ticks) >= min_ticks
                    {
                        out.push(event_from_row(e, self.index_id(), ts));
                    }
                }
            }
        }
        Ok(out)
    }

    fn materialize_masks(&self, request: &MaskRequest) -> Result<MaskArtifact> {
        let mut regions = Vec::new();
        let mut notes = Vec::new();
        let ts = self.snapshot.timescale.max(1);
        let wanted: Vec<&NamespacedId> = request
            .subjects
            .iter()
            .filter(|sid| {
                if sid.kind == crate::ids::EntityKind::Subject {
                    true
                } else {
                    notes.push(format!("skip non-subject id {}", sid.as_uri()));
                    false
                }
            })
            .collect();

        for sid in &wanted {
            let timed: Vec<_> = self
                .snapshot
                .mask_samples
                .iter()
                .filter(|s| s.subject_id == sid.id && in_requested_ranges(s.ticks, &request.ranges))
                .collect();
            if timed.is_empty() {
                let bbox = self
                    .subject_boxes
                    .iter()
                    .find(|(id, _)| *id == sid.id)
                    .map(|(_, b)| *b)
                    .or_else(|| {
                        self.snapshot
                            .subject_boxes
                            .iter()
                            .find(|(id, _)| *id == sid.id)
                            .map(|(_, b)| *b)
                    });
                let Some(box_xyxy) = bbox else {
                    notes.push(format!(
                        "missing mask/box for {} — privacy missing_mask policy applies",
                        sid.as_uri()
                    ));
                    continue;
                };
                let at = request.ranges.first().map(|r| r.start).unwrap_or_default();
                regions.push(RegionSample {
                    at,
                    box_xyxy,
                    subject: Some((*sid).clone()),
                    confidence: None,
                    geometry: None,
                });
                if request.fidelity == MaskFidelity::TrueGeometry {
                    notes.push(format!(
                        "subject {} has no timed geometry — bbox proxy",
                        sid.as_uri()
                    ));
                }
                continue;
            }
            for sample in timed {
                let geometry = match request.fidelity {
                    MaskFidelity::TrueGeometry => sample.geometry.clone(),
                    MaskFidelity::BBoxProxy => None,
                };
                if request.fidelity == MaskFidelity::TrueGeometry && geometry.is_none() {
                    notes.push(format!(
                        "subject {} t={} true geometry missing — bbox at this sample",
                        sid.as_uri(),
                        sample.ticks
                    ));
                }
                regions.push(RegionSample {
                    at: MediaTime::new(sample.ticks, ts),
                    box_xyxy: sample.box_xyxy,
                    subject: Some((*sid).clone()),
                    confidence: (sample.confidence > 0.0).then_some(sample.confidence),
                    geometry,
                });
            }
        }

        let has_true = regions.iter().any(|r| {
            r.geometry
                .as_ref()
                .is_some_and(|g| !matches!(g, MaskGeometry::BBox { .. }))
        });
        let fidelity = if request.fidelity == MaskFidelity::TrueGeometry && has_true {
            MaskFidelity::TrueGeometry
        } else {
            MaskFidelity::BBoxProxy
        };

        Ok(MaskArtifact {
            fidelity,
            regions,
            geometry: None,
            mask_ids: request.mask_ids.clone(),
            notes,
        })
    }
}

fn in_requested_ranges(ticks: i64, ranges: &[MediaRange]) -> bool {
    if ranges.is_empty() {
        return true;
    }
    ranges
        .iter()
        .any(|r| ticks >= r.start.ticks && ticks <= r.end.ticks)
}

fn event_from_row(e: &crate::resolve::EventEvidence, index_id: &str, ts: u32) -> EventResult {
    EventResult {
        event_id: e.event_id.clone(),
        kind: e.kind.clone(),
        subject: e
            .subject_id
            .map(|s| NamespacedId::sightloom_subject(index_id, s)),
        range: MediaRange::new(
            MediaTime::new(e.start_ticks, ts),
            MediaTime::new(e.end_ticks, ts),
        ),
        hour_of_day: e.hour_of_day,
        score: e.score,
    }
}

fn event_from_anomaly(a: &crate::resolve::AnomalyEvidence, index_id: &str, ts: u32) -> EventResult {
    EventResult {
        event_id: format!("sightloom://{index_id}/events/{}", a.anomaly_id),
        kind: a.kind.clone(),
        subject: a
            .subject_id
            .map(|s| NamespacedId::sightloom_subject(index_id, s)),
        range: MediaRange::new(
            MediaTime::new(a.start_ticks, ts),
            MediaTime::new(a.end_ticks, ts),
        ),
        hour_of_day: a.hour_of_day,
        score: a.score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::FrequencyMetric;
    use crate::mask::MaskFidelity;
    use crate::resolve::{AnomalyEvidence, SubjectEvidence};
    use crate::time::MediaTime;

    fn snap() -> AnalysisSnapshot {
        AnalysisSnapshot {
            media: "cam1".into(),
            source_hash: "src".into(),
            vision_index_generation: "gen-1".into(),
            vision_index_hash: "idx".into(),
            timescale: 1_000_000_000,
            subjects: vec![
                SubjectEvidence {
                    subject_id: 184,
                    label: Some("alice".into()),
                    appearance_count: 5,
                    source_ids: vec![2, 5],
                    first_ticks: 0,
                    last_ticks: 10,
                    confidence: Some(0.9),
                    ..SubjectEvidence::default()
                }
                .with_visit(0, 10),
            ],
            anomalies: vec![AnomalyEvidence {
                anomaly_id: "9".into(),
                subject_id: Some(184),
                start_ticks: 1,
                end_ticks: 2,
                hour_of_day: Some(23),
                kind: "route".into(),
                score: 0.8,
            }],
            ..AnalysisSnapshot::default()
        }
    }

    #[test]
    fn namespaced_subject_from_provider() {
        let p = SightLoomProvider::new(snap());
        let rows = p
            .query_subjects(&SubjectQuery {
                selector: Some(SubjectSelector::MostFrequent {
                    metric: FrequencyMetric::AppearanceCount,
                }),
                limit: 1,
                min_confidence: None,
            })
            .unwrap();
        assert_eq!(rows[0].id.as_uri(), "sightloom://gen-1/subjects/184");
    }

    #[test]
    fn mask_preview_bbox() {
        let p =
            SightLoomProvider::new(snap()).with_subject_boxes(vec![(184, [1.0, 2.0, 3.0, 4.0])]);
        let sub = NamespacedId::sightloom_subject("gen-1", 184);
        let art = p
            .materialize_masks(&MaskRequest::preview_subjects(
                vec![sub],
                MediaRange::new(MediaTime::new(0, 1), MediaTime::new(1, 1)),
            ))
            .unwrap();
        assert_eq!(art.fidelity, MaskFidelity::BBoxProxy);
        assert!((art.regions[0].box_xyxy[0] - 1.0).abs() < 1e-6);
        assert!((art.regions[0].box_xyxy[3] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn timed_samples_and_true_geometry() {
        let mut snap = snap();
        snap.mask_samples = vec![
            crate::resolve::MaskSampleEvidence {
                subject_id: 184,
                ticks: 0,
                box_xyxy: [10.0, 10.0, 20.0, 20.0],
                confidence: 0.8,
                mask_ref: Some(3),
                geometry: Some(MaskGeometry::Rle {
                    width: 4,
                    height: 1,
                    counts: vec![0, 2, 2],
                }),
            },
            crate::resolve::MaskSampleEvidence {
                subject_id: 184,
                ticks: 5,
                box_xyxy: [12.0, 10.0, 22.0, 20.0],
                confidence: 0.7,
                mask_ref: Some(4),
                geometry: Some(MaskGeometry::External {
                    package_id: "gen-1".into(),
                    mask_ref: 4,
                }),
            },
            crate::resolve::MaskSampleEvidence {
                subject_id: 184,
                ticks: 99,
                box_xyxy: [0.0, 0.0, 1.0, 1.0],
                confidence: 0.1,
                mask_ref: None,
                geometry: None,
            },
        ];
        let p = SightLoomProvider::new(snap);
        let sub = NamespacedId::sightloom_subject("gen-1", 184);
        let art = p
            .materialize_masks(&MaskRequest::final_subjects(
                vec![sub],
                vec![MediaRange::new(
                    MediaTime::new(0, 1_000_000_000),
                    MediaTime::new(5, 1_000_000_000),
                )],
            ))
            .unwrap();
        assert_eq!(art.regions.len(), 2);
        assert_eq!(art.fidelity, MaskFidelity::TrueGeometry);
        assert!(art.carries_true_geometry());
        assert!(matches!(
            art.regions[0].geometry,
            Some(MaskGeometry::Rle { .. })
        ));
    }
}
