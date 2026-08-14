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
                        (a.last_ticks - a.first_ticks).cmp(&(b.last_ticks - b.first_ticks))
                    }
                });
                best.map(|s| vec![self.to_subject_result(s)])
                    .unwrap_or_default()
            }
            Some(SubjectSelector::SubjectSet { name }) => {
                let lower = name.to_lowercase();
                self.snapshot
                    .subjects
                    .iter()
                    .filter(|s| {
                        s.label
                            .as_ref()
                            .is_some_and(|l| l.to_lowercase().contains(&lower))
                    })
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
                // Treat as anomaly kind filter.
                for a in &self.snapshot.anomalies {
                    if a.kind.to_lowercase().contains(&expr.to_lowercase()) {
                        out.push(event_from_anomaly(a, self.index_id(), ts));
                    }
                }
            }
            EventQuery::ZoneEnters { .. } | EventQuery::IdleRanges { .. } => {
                // Snapshot may not carry zone/idle tables yet — return empty with honesty.
            }
        }
        // Always include anomalies as events for BuildAnomalyReel path.
        if out.is_empty() {
            for a in &self.snapshot.anomalies {
                out.push(event_from_anomaly(a, self.index_id(), ts));
            }
        }
        Ok(out)
    }

    fn materialize_masks(&self, request: &MaskRequest) -> Result<MaskArtifact> {
        let mut regions = Vec::new();
        let mut notes = Vec::new();
        let at = request.ranges.first().map(|r| r.start).unwrap_or_default();

        for sid in &request.subjects {
            if sid.kind != crate::ids::EntityKind::Subject {
                notes.push(format!("skip non-subject id {}", sid.as_uri()));
                continue;
            }
            let bbox = self
                .subject_boxes
                .iter()
                .find(|(id, _)| *id == sid.id)
                .map(|(_, b)| *b);
            match (request.fidelity, bbox) {
                (MaskFidelity::BBoxProxy | MaskFidelity::TrueGeometry, Some(box_xyxy)) => {
                    regions.push(RegionSample {
                        at,
                        box_xyxy,
                        subject: Some(sid.clone()),
                        confidence: None,
                    });
                    if request.fidelity == MaskFidelity::TrueGeometry {
                        notes.push(format!(
                            "subject {} true geometry not in snapshot — bbox proxy",
                            sid.as_uri()
                        ));
                    }
                }
                (_, None) => {
                    notes.push(format!(
                        "missing mask/box for {} — privacy missing_mask policy applies",
                        sid.as_uri()
                    ));
                }
            }
        }

        let geometry = if request.fidelity == MaskFidelity::TrueGeometry {
            regions.first().map(|r| MaskGeometry::BBox {
                box_xyxy: r.box_xyxy,
            })
        } else {
            None
        };

        Ok(MaskArtifact {
            fidelity: if geometry.is_some() && request.fidelity == MaskFidelity::TrueGeometry {
                // Honest: still bbox until host supplies RLE/dense.
                MaskFidelity::BBoxProxy
            } else {
                request.fidelity
            },
            regions,
            geometry,
            mask_ids: request.mask_ids.clone(),
            notes,
        })
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
            subjects: vec![SubjectEvidence {
                subject_id: 184,
                label: Some("alice".into()),
                appearance_count: 5,
                source_ids: vec![2, 5],
                first_ticks: 0,
                last_ticks: 10,
                confidence: Some(0.9),
            }],
            anomalies: vec![AnomalyEvidence {
                anomaly_id: "9".into(),
                subject_id: Some(184),
                start_ticks: 1,
                end_ticks: 2,
                hour_of_day: Some(23),
                kind: "route".into(),
                score: 0.8,
            }],
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
}
