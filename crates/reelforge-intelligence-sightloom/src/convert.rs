//! Convert in-memory `VisionIndex` → Intelligence `AnalysisSnapshot`.

use reelforge_intelligence_core::{
    AnalysisSnapshot, AnomalyEvidence, AppearanceEvidence, EventEvidence, MaskGeometry,
    MaskSampleEvidence, ObjectEvidence, ObjectSample, PiiKind, SubjectEvidence, TrackBinding,
};
use sightloom_analysis::{AnomalyReason, hour_of_day_ns};
use sightloom_core::{EventKind, MediaTime};
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
    let appearances = appearances_from_index(index, timescale);
    let track_bindings = track_bindings_from_index(index);
    let events = events_from_index(index, timescale);
    let subjects = subjects_from_index(index, timescale, &appearances);
    let anomalies = anomalies_from_index(index, timescale);
    let subject_boxes = subject_boxes_from_index(index);
    let (frame_width, frame_height) = infer_frame_size(index);
    let mask_package_id = generation.clone();
    let mask_samples = mask_samples_from_index(index, timescale, &mask_package_id);
    let objects = objects_from_index(index, timescale, &subjects);

    AnalysisSnapshot {
        media: index.header.name.clone(),
        source_hash,
        vision_index_generation: generation,
        vision_index_hash: content_hash,
        subjects,
        anomalies,
        appearances,
        track_bindings,
        subject_sets: BTreeMap::new(),
        events,
        timescale,
        frame_width,
        frame_height,
        subject_boxes,
        mask_samples,
        objects,
        mask_package_id: Some(mask_package_id),
        mask_package_uri: None,
    }
}

/// PII objects from labeled subjects and track `class_id` (COCO-adjacent hints only).
///
/// License plates / OCR text are **not** invented. A person-only detector
/// yields an empty list; `RedactPii` then fail-closes.
#[must_use]
pub fn objects_from_index(
    index: &VisionIndex,
    timescale: u32,
    subjects: &[SubjectEvidence],
) -> Vec<ObjectEvidence> {
    let mut by_key: BTreeMap<(u32, u32, PiiKind), ObjectEvidence> = BTreeMap::new();

    for sample in index.tracks.effective_samples() {
        let Some(kind) = sample
            .class_id
            .and_then(|c| pii_from_coco_class(c.0))
            .or_else(|| {
                sample
                    .subject_id
                    .and_then(|sid| subjects.iter().find(|s| s.subject_id == sid.0))
                    .and_then(|s| s.label.as_deref())
                    .and_then(|l| PiiKind::parse(l).ok())
            })
        else {
            continue;
        };
        let source_id = sample.source_id.0;
        let track_id = sample.track_id.0;
        let ticks = ticks_of(sample.pts, timescale);
        let e = by_key.entry((source_id, track_id, kind)).or_insert_with(|| {
            ObjectEvidence {
                object_id: u64::from(track_id)
                    .saturating_mul(4)
                    .saturating_add(pii_tag(kind)),
                kind,
                subject_id: sample.subject_id.map(|s| s.0),
                track_id: Some(track_id),
                source_id,
                first_ticks: ticks,
                last_ticks: ticks,
                samples: Vec::new(),
                confidence: None,
            }
        });
        e.first_ticks = e.first_ticks.min(ticks);
        e.last_ticks = e.last_ticks.max(ticks);
        e.confidence = Some(e.confidence.unwrap_or(0.0).max(sample.confidence));
        if e.subject_id.is_none() {
            e.subject_id = sample.subject_id.map(|s| s.0);
        }
        e.samples.push(ObjectSample {
            ticks,
            box_xyxy: [sample.left, sample.top, sample.right, sample.bottom],
            confidence: sample.confidence,
        });
    }

    by_key.into_values().collect()
}

/// COCO-80 class ids that can stand in for a screen. Plates/text have no COCO id.
fn pii_from_coco_class(class_id: u16) -> Option<PiiKind> {
    match class_id {
        62 | 63 | 67 => Some(PiiKind::Screen), // tv, laptop, cell phone
        _ => None,
    }
}

const fn pii_tag(kind: PiiKind) -> u64 {
    match kind {
        PiiKind::LicensePlate => 0,
        PiiKind::Screen => 1,
        PiiKind::Text => 2,
        PiiKind::Document => 3,
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

/// Timed boxes + mask geometry from every effective track sample.
#[must_use]
pub fn mask_samples_from_index(
    index: &VisionIndex,
    timescale: u32,
    generation: &str,
) -> Vec<MaskSampleEvidence> {
    let mut out = Vec::new();
    for sample in index.tracks.effective_samples() {
        let Some(sid) = sample.subject_id else {
            continue;
        };
        let mask_ref = (sample.mask_ref != 0).then_some(sample.mask_ref);
        let geometry = mask_ref.and_then(|handle| {
            decode_mask_blob(index.masks.get(sightloom_core::MaskRef(handle)).ok()).or_else(|| {
                Some(MaskGeometry::External {
                    package_id: generation.to_string(),
                    mask_ref: handle,
                })
            })
        });
        out.push(MaskSampleEvidence {
            subject_id: sid.0,
            ticks: ticks_of(sample.pts, timescale),
            box_xyxy: [sample.left, sample.top, sample.right, sample.bottom],
            confidence: sample.confidence,
            mask_ref,
            geometry,
        });
    }
    out
}

/// Encode a COCO-RLE blob in the Intelligence `SLM1` package format.
#[must_use]
pub fn encode_slm1_rle(width: u32, height: u32, counts: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(17 + counts.len() * 4);
    out.extend_from_slice(b"SLM1");
    out.push(0);
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&u32::try_from(counts.len()).unwrap_or(0).to_le_bytes());
    for c in counts {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

fn decode_mask_blob(bytes: Option<&[u8]>) -> Option<MaskGeometry> {
    let bytes = bytes?;
    if bytes.len() < 17 || !bytes.starts_with(b"SLM1") {
        return None;
    }
    let kind = bytes[4];
    let width = u32::from_le_bytes(bytes[5..9].try_into().ok()?);
    let height = u32::from_le_bytes(bytes[9..13].try_into().ok()?);
    match kind {
        0 => {
            let n = u32::from_le_bytes(bytes[13..17].try_into().ok()?) as usize;
            let mut counts = Vec::with_capacity(n);
            let mut off = 17;
            for _ in 0..n {
                if off + 4 > bytes.len() {
                    return None;
                }
                counts.push(u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?));
                off += 4;
            }
            Some(MaskGeometry::Rle {
                width,
                height,
                counts,
            })
        }
        1 => {
            let need = (width as usize).saturating_mul(height as usize);
            if bytes.len() < 13 + need {
                return None;
            }
            Some(MaskGeometry::Dense {
                width,
                height,
                data: bytes[13..13 + need].to_vec(),
            })
        }
        _ => None,
    }
}

/// Decode an SLM1 blob into ReelForge coverage bytes (`0`/`255`).
#[must_use]
pub fn slm1_to_coverage(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    match decode_mask_blob(Some(bytes))? {
        MaskGeometry::Rle {
            width,
            height,
            counts,
        } => {
            let need = (width as usize).saturating_mul(height as usize);
            let mut data = vec![0_u8; need];
            let mut off = 0_usize;
            let mut fg = false;
            for count in counts {
                let value = if fg { 255_u8 } else { 0 };
                let end = off.saturating_add(count as usize).min(need);
                data[off..end].fill(value);
                off = end;
                fg = !fg;
            }
            Some((width, height, data))
        }
        MaskGeometry::Dense {
            width,
            height,
            data,
        } => {
            let coverage: Vec<u8> = data
                .into_iter()
                .map(|b| if b == 0 { 0 } else { 255 })
                .collect();
            Some((width, height, coverage))
        }
        _ => None,
    }
}

fn infer_frame_size(index: &VisionIndex) -> (Option<u32>, Option<u32>) {
    let mut max_right = 0.0f32;
    let mut max_bottom = 0.0f32;
    let mut pixelish = false;
    for sample in index.tracks.effective_samples() {
        if sample.right > 2.0 || sample.bottom > 2.0 {
            pixelish = true;
        }
        max_right = max_right.max(sample.right);
        max_bottom = max_bottom.max(sample.bottom);
    }
    if !pixelish || max_right <= 0.0 || max_bottom <= 0.0 {
        return (None, None);
    }
    (
        Some(even_ceil_dim(max_right)),
        Some(even_ceil_dim(max_bottom)),
    )
}

#[allow(clippy::cast_sign_loss)]
fn even_ceil_dim(v: f32) -> u32 {
    let n = v.ceil().max(2.0) as u32;
    if n.is_multiple_of(2) {
        n
    } else {
        n.saturating_add(1)
    }
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

fn appearances_from_index(index: &VisionIndex, timescale: u32) -> Vec<AppearanceEvidence> {
    if !index.appearances.is_empty() {
        return index
            .appearances
            .iter()
            .map(|a| {
                let start = ticks_of(a.start, timescale);
                let end = ticks_of(a.end, timescale);
                AppearanceEvidence {
                    appearance_id: format!("{}", a.appearance_id.0),
                    subject_id: a.subject_id.map(|s| s.0),
                    track_id: a.track_id.map(|t| t.0),
                    source_id: a.source_id.0,
                    start_ticks: start,
                    end_ticks: end.max(start + 1),
                    peak_confidence: a.peak_confidence,
                }
            })
            .collect();
    }
    coalesce_track_appearances(index, timescale)
}

fn coalesce_track_appearances(index: &VisionIndex, timescale: u32) -> Vec<AppearanceEvidence> {
    let mut samples = index.tracks.effective_samples();
    samples.sort_by(|a, b| {
        a.subject_id
            .map(|s| s.0)
            .cmp(&b.subject_id.map(|s| s.0))
            .then_with(|| a.source_id.0.cmp(&b.source_id.0))
            .then_with(|| a.track_id.0.cmp(&b.track_id.0))
            .then_with(|| a.pts.as_nanos().cmp(&b.pts.as_nanos()))
    });
    let mut out = Vec::new();
    let mut run: Option<AppearanceEvidence> = None;
    let gap = i64::from(timescale); // 1s
    for sample in samples {
        let Some(sid) = sample.subject_id else {
            continue;
        };
        let t = ticks_of(sample.pts, timescale);
        match run.as_mut() {
            Some(cur)
                if cur.subject_id == Some(sid.0)
                    && cur.source_id == sample.source_id.0
                    && cur.track_id == Some(sample.track_id.0)
                    && t <= cur.end_ticks + gap =>
            {
                cur.end_ticks = t.max(cur.end_ticks + 1);
                cur.peak_confidence = cur.peak_confidence.max(sample.confidence);
            }
            _ => {
                if let Some(done) = run.take() {
                    out.push(done);
                }
                run = Some(AppearanceEvidence {
                    appearance_id: format!("coalesce-{}-{}", sid.0, out.len()),
                    subject_id: Some(sid.0),
                    track_id: Some(sample.track_id.0),
                    source_id: sample.source_id.0,
                    start_ticks: t,
                    end_ticks: t.saturating_add(1),
                    peak_confidence: sample.confidence,
                });
            }
        }
    }
    if let Some(done) = run {
        out.push(done);
    }
    out
}

fn track_bindings_from_index(index: &VisionIndex) -> Vec<TrackBinding> {
    let mut map: BTreeMap<(u32, u32), u64> = BTreeMap::new();
    for a in &index.appearances {
        if let (Some(sid), Some(tid)) = (a.subject_id, a.track_id) {
            map.insert((tid.0, a.source_id.0), sid.0);
        }
    }
    for sample in index.tracks.effective_samples() {
        if let Some(sid) = sample.subject_id {
            map.insert((sample.track_id.0, sample.source_id.0), sid.0);
        }
    }
    map.into_iter()
        .map(|((track_id, source_id), subject_id)| TrackBinding {
            track_id,
            source_id,
            subject_id,
        })
        .collect()
}

fn events_from_index(index: &VisionIndex, timescale: u32) -> Vec<EventEvidence> {
    let mut out = Vec::new();
    for z in &index.zone_stays {
        let start = ticks_of(z.start, timescale);
        let end = ticks_of(z.end, timescale);
        out.push(EventEvidence {
            event_id: format!("zone-stay-{}", z.zone_id.0),
            kind: "zone_enter".into(),
            subject_id: z.subject_id.map(|s| s.0),
            start_ticks: start,
            end_ticks: end.max(start + 1),
            hour_of_day: Some(hour_of_day_ns(z.start.as_nanos())),
            score: 1.0,
            zone_id: Some(z.zone_id.0),
        });
    }
    for ev in &index.events {
        let start = ticks_of(ev.stamp.pts, timescale);
        let kind = match ev.kind {
            EventKind::Zone => "zone_enter",
            EventKind::Dwell => "idle",
            EventKind::Occupancy => "occupancy",
            EventKind::Identity => "identity",
            EventKind::Pattern => "pattern",
            EventKind::Anomaly => "anomaly",
            EventKind::Custom => "custom",
        };
        out.push(EventEvidence {
            event_id: format!("event-{}", ev.event_id.0),
            kind: kind.into(),
            subject_id: ev.subject_id.map(|s| s.0),
            start_ticks: start,
            end_ticks: start.saturating_add(i64::from(timescale.max(1))),
            hour_of_day: Some(hour_of_day_ns(ev.stamp.pts.as_nanos())),
            score: 1.0,
            zone_id: ev.zone_id.map(|z| z.0),
        });
    }
    for a in &index.anomalies {
        let kind = reason_kind(&a.reasons);
        if !kind.contains("camera") && !kind.contains("hop") && !kind.contains("route") {
            continue;
        }
        let ticks = ticks_of(a.at, timescale);
        out.push(EventEvidence {
            event_id: format!("graph-{}", a.anomaly_id.0),
            kind,
            subject_id: a.subject_id.map(|s| s.0),
            start_ticks: ticks,
            end_ticks: ticks.saturating_add(i64::from(timescale.max(1))),
            hour_of_day: Some(hour_of_day_ns(a.at.as_nanos())),
            score: a.score,
            zone_id: None,
        });
    }
    out
}

fn subjects_from_index(
    index: &VisionIndex,
    timescale: u32,
    appearances: &[AppearanceEvidence],
) -> Vec<SubjectEvidence> {
    let mut by_subject: BTreeMap<u64, Vec<&AppearanceEvidence>> = BTreeMap::new();
    for a in appearances {
        if let Some(sid) = a.subject_id {
            by_subject.entry(sid).or_default().push(a);
        }
    }

    if !index.subjects.is_empty() {
        return index
            .subjects
            .iter()
            .map(|p| {
                let (first_ticks, last_ticks) = span_ticks(p.first_seen, p.last_seen, timescale);
                let source_ids = sources_for_subject(index, p.subject_id.0);
                let visits: Vec<AppearanceEvidence> = by_subject
                    .get(&p.subject_id.0)
                    .map(|rows| rows.iter().map(|a| (*a).clone()).collect())
                    .unwrap_or_default();
                let visible_duration_ticks =
                    visits.iter().map(AppearanceEvidence::duration_ticks).sum();
                SubjectEvidence {
                    subject_id: p.subject_id.0,
                    label: p.label.clone(),
                    appearance_count: u64::from(p.appearance_count.max(1)),
                    source_ids,
                    first_ticks,
                    last_ticks,
                    appearances: visits,
                    visible_duration_ticks,
                    confidence: None,
                }
            })
            .collect();
    }

    // Fall back: synthesize from appearances / track subject_ids.
    let mut agg: BTreeMap<u64, SubjectAgg> = BTreeMap::new();
    for a in appearances {
        let Some(sid) = a.subject_id else {
            continue;
        };
        let e = agg.entry(sid).or_default();
        e.appearance_count = e.appearance_count.saturating_add(1);
        e.sources.insert(a.source_id);
        e.peak = e.peak.max(a.peak_confidence);
        e.first = Some(e.first.map_or(a.start_ticks, |f| f.min(a.start_ticks)));
        e.last = Some(e.last.map_or(a.end_ticks, |l| l.max(a.end_ticks)));
    }
    for sample in index.tracks.effective_samples() {
        let Some(sid) = sample.subject_id else {
            continue;
        };
        let e = agg.entry(sid.0).or_default();
        e.sources.insert(sample.source_id.0);
        let t = ticks_of(sample.pts, timescale);
        e.first = Some(e.first.map_or(t, |f| f.min(t)));
        e.last = Some(e.last.map_or(t, |l| l.max(t)));
        if e.appearance_count == 0 {
            e.appearance_count = 1;
        }
        e.peak = e.peak.max(sample.confidence);
    }

    agg.into_iter()
        .map(|(id, row)| {
            let visits: Vec<AppearanceEvidence> = by_subject
                .get(&id)
                .map(|rows| rows.iter().map(|a| (*a).clone()).collect())
                .unwrap_or_default();
            let visible_duration_ticks =
                visits.iter().map(AppearanceEvidence::duration_ticks).sum();
            SubjectEvidence {
                subject_id: id,
                label: None,
                appearance_count: row.appearance_count.max(1),
                source_ids: row.sources.into_iter().collect(),
                first_ticks: row.first.unwrap_or(0),
                last_ticks: row.last.unwrap_or(0),
                appearances: visits,
                visible_duration_ticks,
                confidence: if row.peak > 0.0 { Some(row.peak) } else { None },
            }
        })
        .collect()
}

#[derive(Default)]
struct SubjectAgg {
    appearance_count: u64,
    sources: BTreeSet<u32>,
    first: Option<i64>,
    last: Option<i64>,
    peak: f32,
}

fn ticks_of(t: MediaTime, default_ts: u32) -> i64 {
    if t.timescale() == 0 {
        return t.ticks();
    }
    if t.timescale() == default_ts {
        return t.ticks();
    }
    let scaled = i128::from(t.ticks()) * i128::from(default_ts.max(1)) / i128::from(t.timescale());
    scaled.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
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
        Some(AnomalyReason::ImpossibleCrossCameraHop) => "impossible_cross_camera_hop".into(),
        Some(AnomalyReason::UnusualCameraTransition) => "unusual_camera_transition".into(),
        Some(AnomalyReason::Custom(c)) => format!("custom_{c}"),
        None => "anomaly".into(),
    }
}

fn span_ticks(first: Option<MediaTime>, last: Option<MediaTime>, default_ts: u32) -> (i64, i64) {
    let f = first.unwrap_or_else(|| MediaTime::new(0, default_ts).unwrap_or_default());
    let l = last.unwrap_or(f);
    (f.ticks(), l.ticks())
}
