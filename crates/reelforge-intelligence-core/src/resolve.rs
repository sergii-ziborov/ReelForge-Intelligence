//! Intent → frozen resolution against a host analysis snapshot (SightLoom side).

use crate::edit::{FrequencyMetric, SemanticEdit, SemanticEditPlan};
use crate::error::{IntelError, Result};
use crate::ids::NamespacedId;
use crate::mask::{MaskArtifact, MaskFidelity, RegionSample};
use crate::pii::PiiKind;
use crate::policy::IntelligencePolicy;
use crate::query::EventQuery;
use crate::resolved::{
    ResolutionDecision, ResolutionWarning, ResolvedEditPlan, ResolvedEvent, ResolvedMaskAsset,
    ResolvedSubject,
};
use crate::selector::SubjectSelector;
use crate::time::{MediaRange, MediaTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Snapshot of analysis evidence the host materializes from SightLoom (or tests).
///
/// Intelligence does not open `VisionIndex` packages itself in M0 — the host
/// injects this view so resolution is pure and testable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AnalysisSnapshot {
    /// Media key.
    pub media: String,
    /// Source content hash.
    pub source_hash: String,
    /// Package generation id.
    pub vision_index_generation: String,
    /// Index content hash.
    pub vision_index_hash: String,
    /// Known subjects with appearance counts / spans.
    #[serde(default)]
    pub subjects: Vec<SubjectEvidence>,
    /// Anomaly / pattern events.
    #[serde(default)]
    pub anomalies: Vec<AnomalyEvidence>,
    /// Discrete subject appearances (not first→last envelopes).
    #[serde(default)]
    pub appearances: Vec<AppearanceEvidence>,
    /// Track → subject bindings from the VisionIndex.
    #[serde(default)]
    pub track_bindings: Vec<TrackBinding>,
    /// Host-expanded named subject sets. Never inferred from labels.
    #[serde(default)]
    pub subject_sets: BTreeMap<String, Vec<u64>>,
    /// Zone / idle / custom events (not only anomalies).
    #[serde(default)]
    pub events: Vec<EventEvidence>,
    /// Timescale for synthetic ranges when only seconds are known.
    #[serde(default = "default_timescale")]
    pub timescale: u32,
    /// Source frame width when known (follow-crop).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_width: Option<u32>,
    /// Source frame height when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_height: Option<u32>,
    /// Last-known subject boxes `(subject_id, xyxy)` for follow-crop / preview.
    #[serde(default)]
    pub subject_boxes: Vec<(u64, [f32; 4])>,
    /// Timed subject samples (boxes + optional true geometry) inside the freeze.
    #[serde(default)]
    pub mask_samples: Vec<MaskSampleEvidence>,
    /// Non-person PII objects (plates, screens, text). Host-projected.
    #[serde(default)]
    pub objects: Vec<ObjectEvidence>,
    /// ReelForge [`MaskPackage`] id (`MaskAsset::External.package_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_package_id: Option<String>,
    /// Host path / URI of the MaskPackage directory when already materialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_package_uri: Option<String>,
}

/// One timed subject observation projected from a VisionIndex track sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSampleEvidence {
    /// Subject id inside the index.
    pub subject_id: u64,
    /// Sample time ticks (snapshot timescale).
    pub ticks: i64,
    /// Axis-aligned box.
    pub box_xyxy: [f32; 4],
    /// Detector confidence.
    #[serde(default)]
    pub confidence: f32,
    /// SightLoom mask-store handle when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_ref: Option<u64>,
    /// Decoded or external true geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<crate::mask::MaskGeometry>,
}

fn default_timescale() -> u32 {
    1_000_000_000
}

/// One timed box on a PII object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectSample {
    /// Sample time ticks (snapshot timescale).
    pub ticks: i64,
    /// Axis-aligned box.
    pub box_xyxy: [f32; 4],
    /// Detector confidence.
    #[serde(default)]
    pub confidence: f32,
}

/// Host-projected PII object (not a gallery subject).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectEvidence {
    /// Stable object id inside the snapshot (track or host key).
    pub object_id: u64,
    /// PII class.
    pub kind: PiiKind,
    /// Optional linked subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<u64>,
    /// Optional source-local track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<u32>,
    /// Source id.
    #[serde(default)]
    pub source_id: u32,
    /// First sample ticks.
    #[serde(default)]
    pub first_ticks: i64,
    /// Last sample ticks.
    #[serde(default)]
    pub last_ticks: i64,
    /// Timed boxes.
    #[serde(default)]
    pub samples: Vec<ObjectSample>,
    /// Peak confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// Subject evidence row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SubjectEvidence {
    /// Subject id.
    pub subject_id: u64,
    /// Label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Appearance / visit count for frequency ranking.
    #[serde(default)]
    pub appearance_count: u64,
    /// Distinct sources.
    #[serde(default)]
    pub source_ids: Vec<u32>,
    /// First seen ticks.
    #[serde(default)]
    pub first_ticks: i64,
    /// Last seen ticks.
    #[serde(default)]
    pub last_ticks: i64,
    /// Discrete appearances for this subject (preferred over first/last).
    #[serde(default)]
    pub appearances: Vec<AppearanceEvidence>,
    /// Sum of appearance durations. Used by [`FrequencyMetric::Duration`].
    #[serde(default)]
    pub visible_duration_ticks: i64,
    /// Confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl SubjectEvidence {
    /// Append one discrete visit and refresh visible duration.
    #[must_use]
    pub fn with_visit(mut self, start_ticks: i64, end_ticks: i64) -> Self {
        self.appearances.push(AppearanceEvidence {
            appearance_id: format!("{}-{}", self.subject_id, self.appearances.len()),
            subject_id: Some(self.subject_id),
            track_id: None,
            source_id: self.source_ids.first().copied().unwrap_or(0),
            start_ticks,
            end_ticks,
            peak_confidence: self.confidence.unwrap_or(0.0),
        });
        if self.last_ticks <= self.first_ticks {
            self.first_ticks = start_ticks;
            self.last_ticks = end_ticks;
        }
        self.visible_duration_ticks = self
            .appearances
            .iter()
            .map(AppearanceEvidence::duration_ticks)
            .sum();
        if self.appearance_count == 0 {
            self.appearance_count = self.appearances.len() as u64;
        }
        self
    }

    /// Visible duration: explicit sum, else sum of appearance rows.
    #[must_use]
    pub fn visible_duration_ticks(&self) -> i64 {
        if self.visible_duration_ticks > 0 {
            return self.visible_duration_ticks;
        }
        self.appearances
            .iter()
            .map(AppearanceEvidence::duration_ticks)
            .sum()
    }

    /// Discrete appearances, or empty when the snapshot never recorded visits.
    #[must_use]
    pub fn appearance_ranges(&self, timescale: u32) -> Vec<MediaRange> {
        let ts = timescale.max(1);
        self.appearances
            .iter()
            .filter_map(|a| a.as_range(ts))
            .collect()
    }
}

/// One continuous appearance of a subject on a source timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AppearanceEvidence {
    /// Appearance id (opaque / namespaced later).
    #[serde(default)]
    pub appearance_id: String,
    /// Subject when resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<u64>,
    /// Local track when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<u32>,
    /// Source id.
    #[serde(default)]
    pub source_id: u32,
    /// Inclusive start ticks.
    #[serde(default)]
    pub start_ticks: i64,
    /// Inclusive end ticks.
    #[serde(default)]
    pub end_ticks: i64,
    /// Peak confidence during the appearance.
    #[serde(default)]
    pub peak_confidence: f32,
}

impl AppearanceEvidence {
    /// Duration in ticks (0 if inverted).
    #[must_use]
    pub fn duration_ticks(&self) -> i64 {
        (self.end_ticks - self.start_ticks).max(0)
    }

    /// Convert to a media range when duration is positive.
    #[must_use]
    pub fn as_range(&self, timescale: u32) -> Option<MediaRange> {
        if self.end_ticks <= self.start_ticks {
            return None;
        }
        let ts = timescale.max(1);
        Some(MediaRange::new(
            MediaTime::new(self.start_ticks, ts),
            MediaTime::new(self.end_ticks, ts),
        ))
    }
}

/// Track identity binding from the VisionIndex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackBinding {
    /// Local track id.
    pub track_id: u32,
    /// Source the track belongs to.
    #[serde(default)]
    pub source_id: u32,
    /// Resolved subject.
    pub subject_id: u64,
}

/// Host / index event row (zone, idle, custom — not only anomalies).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEvidence {
    /// Event id.
    pub event_id: String,
    /// Kind tag (`zone_enter`, `idle`, …).
    #[serde(default)]
    pub kind: String,
    /// Subject when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<u64>,
    /// Start ticks.
    pub start_ticks: i64,
    /// End ticks.
    pub end_ticks: i64,
    /// Hour of day when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour_of_day: Option<u8>,
    /// Score.
    #[serde(default)]
    pub score: f32,
    /// Zone when this is a zone event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_id: Option<u16>,
}

/// Anomaly evidence row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyEvidence {
    /// Id.
    pub anomaly_id: String,
    /// Subject when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<u64>,
    /// Start ticks.
    pub start_ticks: i64,
    /// End ticks.
    pub end_ticks: i64,
    /// Hour of day 0..23 when known (for after-22:00 filters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour_of_day: Option<u8>,
    /// Kind / reason code.
    #[serde(default)]
    pub kind: String,
    /// Score.
    #[serde(default)]
    pub score: f32,
}

/// Resolve intent against a frozen analysis snapshot.
///
/// # Errors
///
/// Invalid plan / empty evidence for required selectors.
pub fn resolve_plan(
    intent: &SemanticEditPlan,
    analysis: &AnalysisSnapshot,
    policy: IntelligencePolicy,
) -> Result<ResolvedEditPlan> {
    intent.validate()?;
    if analysis.source_hash.trim().is_empty() || analysis.vision_index_hash.trim().is_empty() {
        return Err(IntelError::message(
            "resolve: analysis snapshot missing hashes (cannot freeze)",
        ));
    }

    let mut resolved = ResolvedEditPlan::new(
        intent.media.clone(),
        analysis.source_hash.clone(),
        analysis.vision_index_generation.clone(),
        analysis.vision_index_hash.clone(),
    );
    resolved.policy = policy;
    resolved.intent = Some(intent.clone());
    resolved.frame_width = analysis.frame_width;
    resolved.frame_height = analysis.frame_height;
    resolved.subject_boxes.clone_from(&analysis.subject_boxes);
    resolved
        .mask_package_id
        .clone_from(&analysis.mask_package_id);
    resolved
        .mask_package_uri
        .clone_from(&analysis.mask_package_uri);
    let ts = analysis.timescale.max(1);

    let mut needs_ranges = false;

    for (edit_index, edit) in intent.edits.iter().enumerate() {
        match edit {
            SemanticEdit::BlurSubject { subject }
            | SemanticEdit::FollowSubject {
                subject,
                framing: _,
            } => {
                let subjects = select_subjects(subject, analysis, edit_index, &mut resolved)?;
                push_subject_coverage(&subjects, &mut resolved);
            }
            SemanticEdit::BuildSubjectReel {
                subject,
                pre_roll,
                post_roll,
            } => {
                needs_ranges = true;
                let subjects = select_subjects(subject, analysis, edit_index, &mut resolved)?;
                let n = push_reel_ranges(&subjects, *pre_roll, *post_roll, &mut resolved);
                resolved.decisions.push(ResolutionDecision {
                    code: "subject_reel".into(),
                    message: format!("{n} appearance ranges with pre/post-roll"),
                    edit_index: Some(edit_index),
                });
                if n == 0 {
                    return Err(IntelError::message(format!(
                        "resolve: BuildSubjectReel edit {edit_index} produced no appearance ranges"
                    )));
                }
            }
            SemanticEdit::BlurEveryoneExcept { allowed, .. } => {
                let allowed_ids = select_subjects(allowed, analysis, edit_index, &mut resolved)?;
                let allowed_set: Vec<u64> = allowed_ids
                    .iter()
                    .filter_map(|s| s.local_subject_id)
                    .collect();
                // `select_subjects` records the allowed set; invert so
                // compile/bridge redact everyone else, not the keep-list.
                resolved.resolved_subjects.retain(|s| {
                    s.local_subject_id
                        .is_some_and(|id| !allowed_set.contains(&id))
                });
                let index = index_key(analysis);
                for s in &analysis.subjects {
                    if allowed_set.contains(&s.subject_id) {
                        continue;
                    }
                    let rs = to_resolved(s, ts, index);
                    if !resolved.resolved_subjects.iter().any(|x| x.id == rs.id) {
                        resolved.resolved_subjects.push(rs);
                    }
                }
                resolved.decisions.push(ResolutionDecision {
                    code: "blur_everyone_except".into(),
                    message: format!(
                        "allowed {} subjects; others marked for blur",
                        allowed_set.len()
                    ),
                    edit_index: Some(edit_index),
                });
            }
            SemanticEdit::BuildMostFrequentSubjectReel { metric } => {
                needs_ranges = true;
                let Some(best) = most_frequent(analysis, *metric) else {
                    return Err(IntelError::message(
                        "resolve: no subjects for most-frequent reel",
                    ));
                };
                let index = index_key(analysis);
                let rs = to_resolved(best, ts, index);
                resolved.resolved_subjects.push(rs.clone());
                let n = push_reel_ranges(
                    &[rs],
                    MediaTime::default(),
                    MediaTime::default(),
                    &mut resolved,
                );
                resolved.decisions.push(ResolutionDecision {
                    code: "most_frequent".into(),
                    message: format!(
                        "subject {} appearances={} visible_ticks={}",
                        NamespacedId::sightloom_subject(index, best.subject_id).as_uri(),
                        best.appearance_count,
                        best.visible_duration_ticks()
                    ),
                    edit_index: Some(edit_index),
                });
                if n == 0 {
                    return Err(IntelError::message(
                        "resolve: most-frequent subject has no discrete appearances",
                    ));
                }
            }
            SemanticEdit::BuildAnomalyReel { query } => {
                needs_ranges = true;
                let hits: Vec<_> = analysis
                    .anomalies
                    .iter()
                    .filter(|a| filter_anomaly(a, query))
                    .collect();
                if hits.is_empty() {
                    return Err(IntelError::message(
                        "resolve: no anomalies matched query (refusing empty reel)",
                    ));
                }
                let index = index_key(analysis);
                for a in hits {
                    let range = MediaRange::new(
                        MediaTime::new(a.start_ticks, ts),
                        MediaTime::new(a.end_ticks, ts),
                    );
                    resolved.resolved_events.push(ResolvedEvent {
                        event_id: format!("sightloom://{index}/events/{}", a.anomaly_id),
                        kind: a.kind.clone(),
                        subject: a
                            .subject_id
                            .map(|s| NamespacedId::sightloom_subject(index, s)),
                        range,
                    });
                    resolved.resolved_ranges.push(range);
                }
                resolved.decisions.push(ResolutionDecision {
                    code: "anomaly_reel".into(),
                    message: format!("{} anomaly ranges frozen", resolved.resolved_events.len()),
                    edit_index: Some(edit_index),
                });
            }
            SemanticEdit::CreateEventClips {
                query,
                pad_before_secs,
                pad_after_secs,
            } => {
                needs_ranges = true;
                let hits = resolve_event_query(analysis, query);
                if hits.is_empty() {
                    resolved.warnings.push(ResolutionWarning {
                        message: format!("CreateEventClips: no events matched {query:?}"),
                        edit_index: Some(edit_index),
                    });
                    return Err(IntelError::message(
                        "resolve: CreateEventClips produced no ranges",
                    ));
                }
                let index = index_key(analysis);
                let pre = MediaTime::from_secs_f64(*pad_before_secs, ts);
                let post = MediaTime::from_secs_f64(*pad_after_secs, ts);
                for ev in hits {
                    let range = MediaRange::new(
                        MediaTime::new(ev.start_ticks, ts),
                        MediaTime::new(ev.end_ticks, ts),
                    )
                    .padded(pre, post);
                    resolved.resolved_events.push(ResolvedEvent {
                        event_id: ev.event_id.clone(),
                        kind: ev.kind.clone(),
                        subject: ev
                            .subject_id
                            .map(|s| NamespacedId::sightloom_subject(index, s)),
                        range,
                    });
                    resolved.resolved_ranges.push(range);
                }
                resolved.decisions.push(ResolutionDecision {
                    code: "event_clips".into(),
                    message: format!(
                        "{} event ranges frozen from {query:?}",
                        resolved.resolved_events.len()
                    ),
                    edit_index: Some(edit_index),
                });
            }
            SemanticEdit::RedactPii { kinds } => {
                let want = requested_pii_kinds(kinds);
                let n = resolve_redact_pii(analysis, &want, ts, &mut resolved)?;
                resolved.decisions.push(ResolutionDecision {
                    code: "redact_pii".into(),
                    message: format!("redact {n} PII objects ({want:?})"),
                    edit_index: Some(edit_index),
                });
            }
        }
    }

    if needs_ranges && resolved.resolved_ranges.is_empty() {
        return Err(IntelError::message(
            "resolve: reel/event edit produced no ranges",
        ));
    }

    resolved.validate()?;
    Ok(resolved)
}

fn requested_pii_kinds(kinds: &[PiiKind]) -> Vec<PiiKind> {
    if kinds.is_empty() {
        PiiKind::ALL.to_vec()
    } else {
        kinds.to_vec()
    }
}

fn resolve_redact_pii(
    analysis: &AnalysisSnapshot,
    want: &[PiiKind],
    ts: u32,
    resolved: &mut ResolvedEditPlan,
) -> Result<usize> {
    let index = index_key(analysis);
    let mut hits = 0_usize;

    for obj in &analysis.objects {
        if !want.contains(&obj.kind) {
            continue;
        }
        push_pii_object(obj, ts, index, resolved);
        hits += 1;
    }

    for s in &analysis.subjects {
        let Some(label) = s.label.as_deref() else {
            continue;
        };
        let Ok(kind) = PiiKind::parse(label) else {
            continue;
        };
        if !want.contains(&kind) {
            continue;
        }
        if resolved.resolved_subjects.iter().any(|r| {
            r.local_subject_id == Some(s.subject_id)
        }) {
            continue;
        }
        let rs = to_resolved(s, ts, index);
        push_subject_coverage(std::slice::from_ref(&rs), resolved);
        resolved.resolved_subjects.push(rs);
        hits += 1;
    }

    if hits == 0 {
        let names: Vec<_> = want.iter().map(PiiKind::as_str).collect();
        return Err(IntelError::message(format!(
            "resolve: no PII evidence for [{}] (refusing empty redaction; host must project detections)",
            names.join(",")
        )));
    }
    Ok(hits)
}

fn push_pii_object(
    obj: &ObjectEvidence,
    ts: u32,
    index: &str,
    resolved: &mut ResolvedEditPlan,
) {
    let id = NamespacedId::sightloom_object(index, obj.object_id);
    let span = if obj.last_ticks > obj.first_ticks {
        Some(MediaRange::new(
            MediaTime::new(obj.first_ticks, ts),
            MediaTime::new(obj.last_ticks, ts),
        ))
    } else {
        obj.samples.first().map(|s| {
            MediaRange::new(MediaTime::new(s.ticks, ts), MediaTime::new(s.ticks, ts))
        })
    };
    if let Some(span) = span {
        resolved.resolved_ranges.push(span);
    }
    if !resolved.resolved_subjects.iter().any(|s| s.id == id) {
        resolved.resolved_subjects.push(ResolvedSubject {
            id: id.clone(),
            local_subject_id: obj.subject_id,
            label: Some(obj.kind.as_str().to_string()),
            source_ids: vec![obj.source_id],
            source_uris: vec![NamespacedId::sightloom_source(index, obj.source_id)],
            span,
            appearances: span.into_iter().collect(),
            visible_duration_ticks: obj.last_ticks.saturating_sub(obj.first_ticks),
            confidence: obj.confidence,
        });
    }
    if !obj.samples.is_empty() {
        let regions: Vec<RegionSample> = obj
            .samples
            .iter()
            .map(|s| RegionSample {
                at: MediaTime::new(s.ticks, ts),
                box_xyxy: s.box_xyxy,
                subject: Some(id.clone()),
                confidence: Some(s.confidence),
                geometry: None,
            })
            .collect();
        resolved.resolved_masks.push(ResolvedMaskAsset {
            mask_id: None,
            mask_ref: None,
            subject: Some(id),
            range: span,
            fidelity: MaskFidelity::BBoxProxy,
            artifact: Some(MaskArtifact::from_regions(regions)),
        });
    }
}

fn index_key(analysis: &AnalysisSnapshot) -> &str {
    if analysis.vision_index_generation.is_empty() {
        "unknown"
    } else {
        &analysis.vision_index_generation
    }
}

fn to_resolved(s: &SubjectEvidence, ts: u32, index_id: &str) -> ResolvedSubject {
    let source_uris: Vec<NamespacedId> = s
        .source_ids
        .iter()
        .map(|src| NamespacedId::sightloom_source(index_id, *src))
        .collect();
    let appearances = s.appearance_ranges(ts);
    let span = if s.last_ticks > s.first_ticks {
        Some(MediaRange::new(
            MediaTime::new(s.first_ticks, ts),
            MediaTime::new(s.last_ticks, ts),
        ))
    } else {
        appearances.first().copied()
    };
    ResolvedSubject {
        id: NamespacedId::sightloom_subject(index_id, s.subject_id),
        local_subject_id: Some(s.subject_id),
        label: s.label.clone(),
        source_ids: s.source_ids.clone(),
        source_uris,
        span,
        appearances,
        visible_duration_ticks: s.visible_duration_ticks(),
        confidence: s.confidence,
    }
}

/// Coverage ranges for blur/follow: discrete appearances, else nothing (no invented span).
fn push_subject_coverage(subjects: &[ResolvedSubject], out: &mut ResolvedEditPlan) {
    for s in subjects {
        if s.appearances.is_empty() {
            if let Some(span) = s.span {
                out.resolved_ranges.push(span);
            }
        } else {
            out.resolved_ranges.extend(s.appearances.iter().copied());
        }
    }
}

/// Reel ranges: one padded range per discrete appearance. Envelope is not a reel.
fn push_reel_ranges(
    subjects: &[ResolvedSubject],
    pre_roll: MediaTime,
    post_roll: MediaTime,
    out: &mut ResolvedEditPlan,
) -> usize {
    let mut n = 0;
    for s in subjects {
        for appearance in &s.appearances {
            let range = appearance.padded(pre_roll, post_roll);
            if !range.is_empty() {
                out.resolved_ranges.push(range);
                n += 1;
            }
        }
    }
    n
}

fn most_frequent<'a>(
    analysis: &'a AnalysisSnapshot,
    metric: FrequencyMetric,
) -> Option<&'a SubjectEvidence> {
    analysis.subjects.iter().max_by(|a, b| match metric {
        FrequencyMetric::AppearanceCount => a.appearance_count.cmp(&b.appearance_count),
        FrequencyMetric::SourceCount => a
            .source_ids
            .len()
            .cmp(&b.source_ids.len())
            .then_with(|| a.appearance_count.cmp(&b.appearance_count)),
        FrequencyMetric::Duration => a
            .visible_duration_ticks()
            .cmp(&b.visible_duration_ticks())
            .then_with(|| a.appearance_count.cmp(&b.appearance_count)),
    })
}

fn resolve_event_query(analysis: &AnalysisSnapshot, query: &EventQuery) -> Vec<EventEvidence> {
    match query {
        EventQuery::ZoneEnters { zone_id } => analysis
            .events
            .iter()
            .filter(|e| e.zone_id == Some(*zone_id) || e.kind.to_lowercase().contains("zone"))
            .filter(|e| e.zone_id.is_none_or(|z| z == *zone_id))
            .cloned()
            .collect(),
        EventQuery::IdleRanges { min_seconds } => {
            let ts = analysis.timescale.max(1);
            let min_ticks = MediaTime::from_secs_f64(*min_seconds, ts).ticks;
            analysis
                .events
                .iter()
                .filter(|e| {
                    e.kind.to_lowercase().contains("idle")
                        && (e.end_ticks - e.start_ticks) >= min_ticks
                })
                .cloned()
                .collect()
        }
        EventQuery::Custom { expr } => {
            let needle = expr.to_lowercase();
            let mut hits: Vec<EventEvidence> = analysis
                .events
                .iter()
                .filter(|e| {
                    e.kind.to_lowercase().contains(&needle)
                        || e.event_id.to_lowercase().contains(&needle)
                })
                .cloned()
                .collect();
            for a in &analysis.anomalies {
                if a.kind.to_lowercase().contains(&needle) {
                    hits.push(EventEvidence {
                        event_id: format!("anomaly:{}", a.anomaly_id),
                        kind: a.kind.clone(),
                        subject_id: a.subject_id,
                        start_ticks: a.start_ticks,
                        end_ticks: a.end_ticks,
                        hour_of_day: a.hour_of_day,
                        score: a.score,
                        zone_id: None,
                    });
                }
            }
            hits
        }
    }
}

fn filter_anomaly(a: &AnomalyEvidence, query: &crate::edit::AnomalyQuery) -> bool {
    if let Some(min_h) = query.min_hour_inclusive {
        if a.hour_of_day.is_some_and(|h| h < min_h) {
            return false;
        }
    }
    if let Some(max_h) = query.max_hour_exclusive {
        if a.hour_of_day.is_some_and(|h| h >= max_h) {
            return false;
        }
    }
    if let Some(min_score) = query.min_score {
        if a.score < min_score {
            return false;
        }
    }
    if let Some(kind) = &query.kind_contains {
        if !a.kind.to_lowercase().contains(&kind.to_lowercase()) {
            return false;
        }
    }
    true
}

fn select_subjects(
    selector: &SubjectSelector,
    analysis: &AnalysisSnapshot,
    edit_index: usize,
    resolved: &mut ResolvedEditPlan,
) -> Result<Vec<ResolvedSubject>> {
    let ts = analysis.timescale.max(1);
    let index = index_key(analysis);
    let list = match selector {
        SubjectSelector::SubjectIds { ids } => {
            let hits: Vec<_> = analysis
                .subjects
                .iter()
                .filter(|s| ids.contains(&s.subject_id))
                .map(|s| to_resolved(s, ts, index))
                .collect();
            if hits.is_empty() {
                return Err(IntelError::message(format!(
                    "resolve: SubjectIds {ids:?} matched no snapshot subjects"
                )));
            }
            hits
        }
        SubjectSelector::SubjectSet { name } => {
            let Some(ids) = analysis.subject_sets.get(name) else {
                return Err(IntelError::message(format!(
                    "resolve: subject set '{name}' is not in snapshot.subject_sets (host must expand)"
                )));
            };
            let hits: Vec<_> = analysis
                .subjects
                .iter()
                .filter(|s| ids.contains(&s.subject_id))
                .map(|s| to_resolved(s, ts, index))
                .collect();
            if hits.is_empty() {
                return Err(IntelError::message(format!(
                    "resolve: subject set '{name}' mapped to {} ids but none match snapshot subjects",
                    ids.len()
                )));
            }
            hits
        }
        SubjectSelector::TrackIds { ids } => {
            let mut hits = Vec::new();
            let mut missing = Vec::new();
            for tid in ids {
                let Some(binding) = analysis.track_bindings.iter().find(|b| b.track_id == *tid)
                else {
                    missing.push(*tid);
                    continue;
                };
                if let Some(s) = analysis
                    .subjects
                    .iter()
                    .find(|s| s.subject_id == binding.subject_id)
                {
                    let rs = to_resolved(s, ts, index);
                    if !hits.iter().any(|h: &ResolvedSubject| h.id == rs.id) {
                        hits.push(rs);
                    }
                } else {
                    missing.push(*tid);
                }
            }
            if !missing.is_empty() {
                return Err(IntelError::message(format!(
                    "resolve: track_ids {missing:?} have no track_bindings→subject map"
                )));
            }
            if hits.is_empty() {
                return Err(IntelError::message(
                    "resolve: track_ids matched no subjects",
                ));
            }
            hits
        }
        SubjectSelector::FramePick { .. } => {
            return Err(IntelError::message(
                "resolve: frame_pick cannot be frozen from a snapshot — host must rewrite to SubjectIds",
            ));
        }
        SubjectSelector::MostFrequent { metric } => {
            let Some(best) = most_frequent(analysis, *metric) else {
                return Err(IntelError::message(
                    "resolve: MostFrequent matched no snapshot subjects",
                ));
            };
            vec![to_resolved(best, ts, index)]
        }
    };

    for s in &list {
        if !resolved.resolved_subjects.iter().any(|x| x.id == s.id) {
            resolved.resolved_subjects.push(s.clone());
        }
    }
    resolved.decisions.push(ResolutionDecision {
        code: "select_subjects".into(),
        message: format!("selected {} subjects", list.len()),
        edit_index: Some(edit_index),
    });
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::SemanticEdit;
    use crate::policy::IntelligencePolicy;
    use crate::query::EventQuery;

    fn snap() -> AnalysisSnapshot {
        AnalysisSnapshot {
            media: "cam1".into(),
            source_hash: "src-abc".into(),
            vision_index_generation: "gen-00000001".into(),
            vision_index_hash: "idx-xyz".into(),
            timescale: 1_000_000_000,
            subjects: vec![
                subject_row(
                    1,
                    "alice",
                    3,
                    vec![1],
                    0,
                    30_000_000_000,
                    0.9,
                    &[(0, 1_000_000_000), (10_000_000_000, 11_000_000_000)],
                ),
                subject_row(
                    2,
                    "bob",
                    10,
                    vec![1, 2],
                    0,
                    60_000_000_000,
                    0.8,
                    &[(0, 8_000_000_000)],
                ),
            ],
            appearances: Vec::new(),
            track_bindings: vec![TrackBinding {
                track_id: 91,
                source_id: 1,
                subject_id: 2,
            }],
            subject_sets: BTreeMap::from([("vip".into(), vec![2])]),
            events: vec![EventEvidence {
                event_id: "z1".into(),
                kind: "zone_enter".into(),
                subject_id: Some(2),
                start_ticks: 4_000_000_000,
                end_ticks: 5_000_000_000,
                hour_of_day: Some(10),
                score: 1.0,
                zone_id: Some(7),
            }],
            anomalies: vec![AnomalyEvidence {
                anomaly_id: "a1".into(),
                subject_id: Some(2),
                start_ticks: 80_000_000_000,
                end_ticks: 90_000_000_000,
                hour_of_day: Some(23),
                kind: "unusual_route".into(),
                score: 0.9,
            }],
            ..AnalysisSnapshot::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn subject_row(
        id: u64,
        label: &str,
        count: u64,
        sources: Vec<u32>,
        first: i64,
        last: i64,
        conf: f32,
        visits: &[(i64, i64)],
    ) -> SubjectEvidence {
        let appearances: Vec<AppearanceEvidence> = visits
            .iter()
            .enumerate()
            .map(|(i, (s, e))| AppearanceEvidence {
                appearance_id: format!("{id}-{i}"),
                subject_id: Some(id),
                track_id: None,
                source_id: sources.first().copied().unwrap_or(0),
                start_ticks: *s,
                end_ticks: *e,
                peak_confidence: conf,
            })
            .collect();
        let visible_duration_ticks = appearances
            .iter()
            .map(AppearanceEvidence::duration_ticks)
            .sum();
        SubjectEvidence {
            subject_id: id,
            label: Some(label.into()),
            appearance_count: count,
            source_ids: sources,
            first_ticks: first,
            last_ticks: last,
            appearances,
            visible_duration_ticks,
            confidence: Some(conf),
        }
    }

    #[test]
    fn freezes_most_frequent_subject() {
        let intent =
            SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
                metric: FrequencyMetric::AppearanceCount,
            });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_subjects.len(), 1);
        assert_eq!(
            resolved.resolved_subjects[0].id.as_uri(),
            "sightloom://gen-00000001/subjects/2"
        );
        assert_eq!(resolved.vision_index_hash, "idx-xyz");
    }

    #[test]
    fn anomaly_after_22() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildAnomalyReel {
            query: crate::edit::AnomalyQuery {
                min_hour_inclusive: Some(22),
                max_hour_exclusive: None,
                min_score: Some(0.5),
                kind_contains: None,
            },
        });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_events.len(), 1);
    }

    #[test]
    fn subject_reel_uses_discrete_appearances_and_pads() {
        let ts = 1_000_000_000;
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildSubjectReel {
            subject: SubjectSelector::SubjectIds { ids: vec![1] },
            pre_roll: MediaTime::new(500_000_000, ts),
            post_roll: MediaTime::new(500_000_000, ts),
        });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_ranges.len(), 2);
        assert_eq!(resolved.resolved_ranges[0].start.ticks, 0);
        assert_eq!(resolved.resolved_ranges[0].end.ticks, 1_500_000_000);
        assert_eq!(resolved.resolved_ranges[1].start.ticks, 9_500_000_000);
        assert_eq!(resolved.resolved_ranges[1].end.ticks, 11_500_000_000);
        let envelope = resolved.resolved_subjects[0].span.unwrap();
        assert_eq!(envelope.start.ticks, 0);
        assert_eq!(envelope.end.ticks, 30_000_000_000);
        assert!(resolved.resolved_ranges[0].end.ticks < envelope.end.ticks);
    }

    #[test]
    fn duration_metric_uses_visible_time_not_envelope() {
        let intent =
            SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
                metric: FrequencyMetric::Duration,
            });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        // Bob envelope is longer (60s) but Alice visible is 2s vs Bob 8s → Bob still wins.
        // Rebuild snap with Alice envelope huge / visible tiny vs Bob.
        assert_eq!(resolved.resolved_subjects[0].local_subject_id, Some(2));
        assert_eq!(
            resolved.resolved_subjects[0].visible_duration_ticks,
            8_000_000_000
        );
    }

    #[test]
    fn duration_prefers_longer_visible_over_wider_envelope() {
        let mut analysis = snap();
        analysis.subjects[0].appearances = vec![AppearanceEvidence {
            appearance_id: "a-vis".into(),
            subject_id: Some(1),
            track_id: None,
            source_id: 1,
            start_ticks: 0,
            end_ticks: 2_000_000_000,
            peak_confidence: 0.9,
        }];
        analysis.subjects[0].visible_duration_ticks = 2_000_000_000;
        analysis.subjects[0].first_ticks = 0;
        analysis.subjects[0].last_ticks = 60_000_000_000;
        analysis.subjects[1].appearances = vec![AppearanceEvidence {
            appearance_id: "b-vis".into(),
            subject_id: Some(2),
            track_id: None,
            source_id: 1,
            start_ticks: 0,
            end_ticks: 8_000_000_000,
            peak_confidence: 0.8,
        }];
        analysis.subjects[1].visible_duration_ticks = 8_000_000_000;
        analysis.subjects[1].first_ticks = 0;
        analysis.subjects[1].last_ticks = 10_000_000_000;
        let intent =
            SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BuildMostFrequentSubjectReel {
                metric: FrequencyMetric::Duration,
            });
        let resolved = resolve_plan(&intent, &analysis, IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_subjects[0].local_subject_id, Some(2));
    }

    #[test]
    fn subject_set_requires_host_mapping() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: SubjectSelector::SubjectSet {
                name: "family".into(),
            },
        });
        assert!(resolve_plan(&intent, &snap(), IntelligencePolicy::default()).is_err());
        let ok = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: SubjectSelector::SubjectSet { name: "vip".into() },
        });
        let resolved = resolve_plan(&ok, &snap(), IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_subjects[0].local_subject_id, Some(2));
    }

    #[test]
    fn track_ids_resolve_via_bindings() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: SubjectSelector::TrackIds { ids: vec![91] },
        });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_subjects[0].local_subject_id, Some(2));
        let missing = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: SubjectSelector::TrackIds { ids: vec![404] },
        });
        assert!(resolve_plan(&missing, &snap(), IntelligencePolicy::default()).is_err());
    }

    #[test]
    fn rewritten_frame_pick_resolves_blur_everyone_except() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurEveryoneExcept {
            allowed: SubjectSelector::FramePick {
                media: "cam1".into(),
                frame_index: 12,
                box_xyxy: [0.0, 0.0, 10.0, 10.0],
            },
            uncertain_identity: None,
        });
        let rewritten = crate::rewrite_selectors(
            intent,
            &[crate::SelectorBinding {
                media: "cam1".into(),
                frame_index: 12,
                box_xyxy: [0.0, 0.0, 10.0, 10.0],
                ids: vec![1],
            }],
        )
        .unwrap();
        let resolved = resolve_plan(&rewritten, &snap(), IntelligencePolicy::default()).unwrap();
        let ids: Vec<u64> = resolved
            .resolved_subjects
            .iter()
            .filter_map(|s| s.local_subject_id)
            .collect();
        assert_eq!(
            ids,
            vec![2],
            "allowed alice must not be in the redaction set"
        );
    }

    #[test]
    fn blur_everyone_except_redacts_others_not_allowed() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurEveryoneExcept {
            allowed: SubjectSelector::SubjectIds { ids: vec![1] },
            uncertain_identity: None,
        });
        let resolved = resolve_plan(&intent, &snap(), IntelligencePolicy::default()).unwrap();
        let ids: Vec<u64> = resolved
            .resolved_subjects
            .iter()
            .filter_map(|s| s.local_subject_id)
            .collect();
        assert_eq!(
            ids,
            vec![2],
            "allowed alice must not be in the redaction set"
        );
        assert!(
            resolved
                .decisions
                .iter()
                .any(|d| d.code == "blur_everyone_except")
        );
    }

    #[test]
    fn frame_pick_is_not_silently_empty() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::BlurSubject {
            subject: SubjectSelector::FramePick {
                media: "cam1".into(),
                frame_index: 12,
                box_xyxy: [0.0, 0.0, 10.0, 10.0],
            },
        });
        assert!(resolve_plan(&intent, &snap(), IntelligencePolicy::default()).is_err());
    }

    #[test]
    fn event_clips_expand_custom_and_zone() {
        let custom = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::CreateEventClips {
            query: EventQuery::Custom {
                expr: "unusual_route".into(),
            },
            pad_before_secs: 0.5,
            pad_after_secs: 0.5,
        });
        let resolved = resolve_plan(&custom, &snap(), IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_ranges.len(), 1);
        assert_eq!(resolved.resolved_ranges[0].start.ticks, 79_500_000_000);
        assert_eq!(resolved.resolved_ranges[0].end.ticks, 90_500_000_000);

        let zone = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::CreateEventClips {
            query: EventQuery::ZoneEnters { zone_id: 7 },
            pad_before_secs: 0.0,
            pad_after_secs: 0.0,
        });
        let z = resolve_plan(&zone, &snap(), IntelligencePolicy::default()).unwrap();
        assert_eq!(z.resolved_events.len(), 1);
        assert_eq!(z.resolved_events[0].kind, "zone_enter");
    }

    #[test]
    fn empty_event_query_is_an_error() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::CreateEventClips {
            query: EventQuery::Custom {
                expr: "does-not-exist".into(),
            },
            pad_before_secs: 1.0,
            pad_after_secs: 1.0,
        });
        assert!(resolve_plan(&intent, &snap(), IntelligencePolicy::default()).is_err());
    }

    #[test]
    fn redact_pii_without_evidence_is_an_error() {
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::RedactPii {
            kinds: vec![PiiKind::LicensePlate],
        });
        let err = resolve_plan(&intent, &snap(), IntelligencePolicy::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("no PII evidence"), "{err}");
    }

    #[test]
    fn redact_pii_freezes_object_boxes() {
        let mut analysis = snap();
        analysis.objects.push(ObjectEvidence {
            object_id: 9,
            kind: PiiKind::LicensePlate,
            subject_id: None,
            track_id: Some(3),
            source_id: 1,
            first_ticks: 0,
            last_ticks: 2_000_000_000,
            samples: vec![ObjectSample {
                ticks: 0,
                box_xyxy: [10.0, 20.0, 80.0, 50.0],
                confidence: 0.91,
            }],
            confidence: Some(0.91),
        });
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::RedactPii {
            kinds: vec![PiiKind::LicensePlate],
        });
        let resolved = resolve_plan(&intent, &analysis, IntelligencePolicy::default()).unwrap();
        assert_eq!(resolved.resolved_subjects.len(), 1);
        assert_eq!(
            resolved.resolved_subjects[0].label.as_deref(),
            Some("license_plate")
        );
        assert_eq!(resolved.resolved_masks.len(), 1);
        assert!(
            resolved
                .resolved_subjects
                .iter()
                .any(|s| s.id.kind == crate::ids::EntityKind::Object)
        );
    }

    #[test]
    fn redact_pii_reads_subject_label() {
        let mut analysis = snap();
        analysis.subjects.push(subject_row(
            77,
            "screen",
            1,
            vec![1],
            0,
            1_000_000_000,
            0.8,
            &[(0, 1_000_000_000)],
        ));
        let intent = SemanticEditPlan::new("cam1").with_edit(SemanticEdit::RedactPii {
            kinds: vec![PiiKind::Screen],
        });
        let resolved = resolve_plan(&intent, &analysis, IntelligencePolicy::default()).unwrap();
        assert!(
            resolved
                .resolved_subjects
                .iter()
                .any(|s| s.local_subject_id == Some(77))
        );
    }
}
