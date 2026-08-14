//! Convert in-memory `VisionIndex` → Intelligence `AnalysisSnapshot`.

use reelforge_intelligence_core::{AnalysisSnapshot, AnomalyEvidence, SubjectEvidence};
use sightloom_analysis::{AnomalyReason, hour_of_day_ns};
use sightloom_core::MediaTime;
use sightloom_index::VisionIndex;
use std::collections::{BTreeMap, BTreeSet};

/// Build an analysis snapshot from a loaded VisionIndex.
///
/// * `generation` — package generation directory name (`gen-00000001`) or index name.
/// * `content_hash` — checksum / digest of the generation (host or package).
/// * `source_hash` — media content hash when known (else empty string if unknown).
#[must_use]
pub fn snapshot_from_index(
    index: &VisionIndex,
    generation: impl Into<String>,
    content_hash: impl Into<String>,
    source_hash: impl Into<String>,
) -> AnalysisSnapshot {
    let generation = generation.into();
    let content_hash = content_hash.into();
    let source_hash = source_hash.into();

    let timescale = infer_timescale(index);
    let subjects = subjects_from_index(index, timescale);
    let anomalies = anomalies_from_index(index, timescale);

    AnalysisSnapshot {
        media: index.header.name.clone(),
        source_hash,
        vision_index_generation: generation,
        vision_index_hash: content_hash,
        subjects,
        anomalies,
        timescale,
    }
}

/// Last known axis-aligned box per subject from track samples (preview masks).
#[must_use]
pub fn subject_boxes_from_index(index: &VisionIndex) -> Vec<(u64, [f32; 4])> {
    let mut last: BTreeMap<u64, [f32; 4]> = BTreeMap::new();
    for sample in index.tracks.effective_samples() {
        let Some(sid) = sample.subject_id else {
            continue;
        };
        last.insert(
            sid.0,
            [sample.left, sample.top, sample.right, sample.bottom],
        );
    }
    last.into_iter().collect()
}

fn infer_timescale(index: &VisionIndex) -> u32 {
    index
        .tracks
        .samples()
        .first()
        .map(|s| s.pts.timescale())
        .or_else(|| index.appearances.first().map(|a| a.start.timescale()))
        .or_else(|| index.anomalies.first().map(|a| a.at.timescale()))
        .filter(|t| *t > 0)
        .unwrap_or(1_000_000_000)
}

fn subjects_from_index(index: &VisionIndex, timescale: u32) -> Vec<SubjectEvidence> {
    if !index.subjects.is_empty() {
        return index
            .subjects
            .iter()
            .map(|p| {
                let (first_ticks, last_ticks) = span_ticks(p.first_seen, p.last_seen, timescale);
                let source_ids = sources_for_subject(index, p.subject_id.0);
                SubjectEvidence {
                    subject_id: p.subject_id.0,
                    label: p.label.clone(),
                    appearance_count: u64::from(p.appearance_count.max(1)),
                    source_ids,
                    first_ticks,
                    last_ticks,
                    confidence: None,
                }
            })
            .collect();
    }

    // Fall back: synthesize from appearances / track subject_ids.
    let mut by_subject: BTreeMap<u64, SubjectAgg> = BTreeMap::new();
    for a in &index.appearances {
        let Some(sid) = a.subject_id else {
            continue;
        };
        let e = by_subject.entry(sid.0).or_default();
        e.appearance_count = e.appearance_count.saturating_add(1);
        e.sources.insert(a.source_id.0);
        e.peak = e.peak.max(a.peak_confidence);
        e.first = Some(min_opt_time(e.first, a.start));
        e.last = Some(max_opt_time(e.last, a.end));
    }
    for sample in index.tracks.effective_samples() {
        let Some(sid) = sample.subject_id else {
            continue;
        };
        let e = by_subject.entry(sid.0).or_default();
        e.sources.insert(sample.source_id.0);
        e.first = Some(min_opt_time(e.first, sample.pts));
        e.last = Some(max_opt_time(e.last, sample.pts));
        if e.appearance_count == 0 {
            e.appearance_count = 1;
        }
        e.peak = e.peak.max(sample.confidence);
    }

    by_subject
        .into_iter()
        .map(|(id, agg)| {
            let (first_ticks, last_ticks) = span_ticks(agg.first, agg.last, timescale);
            SubjectEvidence {
                subject_id: id,
                label: None,
                appearance_count: agg.appearance_count.max(1),
                source_ids: agg.sources.into_iter().collect(),
                first_ticks,
                last_ticks,
                confidence: if agg.peak > 0.0 { Some(agg.peak) } else { None },
            }
        })
        .collect()
}

#[derive(Default)]
struct SubjectAgg {
    appearance_count: u64,
    sources: BTreeSet<u32>,
    first: Option<MediaTime>,
    last: Option<MediaTime>,
    peak: f32,
}

fn sources_for_subject(index: &VisionIndex, subject_id: u64) -> Vec<u32> {
    let mut set = BTreeSet::new();
    for a in &index.appearances {
        if a.subject_id.map(|s| s.0) == Some(subject_id) {
            set.insert(a.source_id.0);
        }
    }
    for sample in index.tracks.effective_samples() {
        if sample.subject_id.map(|s| s.0) == Some(subject_id) {
            set.insert(sample.source_id.0);
        }
    }
    set.into_iter().collect()
}

fn anomalies_from_index(index: &VisionIndex, timescale: u32) -> Vec<AnomalyEvidence> {
    index
        .anomalies
        .iter()
        .map(|a| {
            let ticks = a.at.ticks();
            let ts = if a.at.timescale() > 0 {
                a.at.timescale()
            } else {
                timescale
            };
            let hour = hour_of_day_ns(a.at.as_nanos());
            AnomalyEvidence {
                anomaly_id: a.anomaly_id.0.to_string(),
                subject_id: a.subject_id.map(|s| s.0),
                start_ticks: ticks,
                end_ticks: ticks.saturating_add(i64::from(ts)), // point event → 1s-ish window
                hour_of_day: Some(hour),
                kind: reason_kind(&a.reasons),
                score: a.score,
            }
        })
        .collect()
}

fn reason_kind(reasons: &[AnomalyReason]) -> String {
    match reasons.first() {
        Some(AnomalyReason::UnusualAppearanceTime) => "unusual_appearance_time".into(),
        Some(AnomalyReason::UnusualFrequency) => "unusual_frequency".into(),
        Some(AnomalyReason::UnusualDwell) => "unusual_dwell".into(),
        Some(AnomalyReason::UnusualRoute) => "unusual_route".into(),
        Some(AnomalyReason::UnusualCoOccurrence) => "unusual_co_occurrence".into(),
        Some(AnomalyReason::MissingExpectedAppearance) => "missing_expected_appearance".into(),
        Some(AnomalyReason::SuddenBehaviourChange) => "sudden_behaviour_change".into(),
        Some(AnomalyReason::Custom(c)) => format!("custom_{c}"),
        None => "anomaly".into(),
    }
}

fn span_ticks(first: Option<MediaTime>, last: Option<MediaTime>, default_ts: u32) -> (i64, i64) {
    let f = first.unwrap_or_else(|| MediaTime::new(0, default_ts).unwrap_or_default());
    let l = last.unwrap_or(f);
    (f.ticks(), l.ticks())
}

fn min_opt_time(cur: Option<MediaTime>, t: MediaTime) -> MediaTime {
    match cur {
        None => t,
        Some(c) if t.as_nanos() < c.as_nanos() => t,
        Some(c) => c,
    }
}

fn max_opt_time(cur: Option<MediaTime>, t: MediaTime) -> MediaTime {
    match cur {
        None => t,
        Some(c) if t.as_nanos() > c.as_nanos() => t,
        Some(c) => c,
    }
}
