//! Intent → frozen resolution against a host analysis snapshot (SightLoom side).

use crate::edit::{FrequencyMetric, SemanticEdit, SemanticEditPlan};
use crate::error::{IntelError, Result};
use crate::policy::IntelligencePolicy;
use crate::resolved::{
    ResolutionDecision, ResolutionWarning, ResolvedEditPlan, ResolvedEvent, ResolvedSubject,
};
use crate::selector::SubjectSelector;
use crate::time::{MediaRange, MediaTime};
use serde::{Deserialize, Serialize};

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
    /// Timescale for synthetic ranges when only seconds are known.
    #[serde(default = "default_timescale")]
    pub timescale: u32,
}

fn default_timescale() -> u32 {
    1_000_000_000
}

/// Subject evidence row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
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
    let ts = analysis.timescale.max(1);

    for (edit_index, edit) in intent.edits.iter().enumerate() {
        match edit {
            SemanticEdit::BlurSubject { subject }
            | SemanticEdit::FollowSubject {
                subject,
                framing: _,
            }
            | SemanticEdit::BuildSubjectReel {
                subject,
                pre_roll: _,
                post_roll: _,
            } => {
                let subjects = select_subjects(subject, analysis, edit_index, &mut resolved)?;
                push_subject_ranges(&subjects, ts, &mut resolved);
            }
            SemanticEdit::BlurEveryoneExcept { allowed, .. } => {
                let allowed_ids = select_subjects(allowed, analysis, edit_index, &mut resolved)?;
                let allowed_set: Vec<u64> = allowed_ids.iter().map(|s| s.subject_id).collect();
                for s in &analysis.subjects {
                    if !allowed_set.contains(&s.subject_id) {
                        resolved.resolved_subjects.push(to_resolved(s, ts));
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
                let Some(best) = most_frequent(analysis, *metric) else {
                    resolved.warnings.push(ResolutionWarning {
                        message: "no subjects for most-frequent reel".into(),
                        edit_index: Some(edit_index),
                    });
                    continue;
                };
                resolved.resolved_subjects.push(to_resolved(best, ts));
                push_subject_ranges(&[to_resolved(best, ts)], ts, &mut resolved);
                resolved.decisions.push(ResolutionDecision {
                    code: "most_frequent".into(),
                    message: format!(
                        "subject {} appearances={}",
                        best.subject_id, best.appearance_count
                    ),
                    edit_index: Some(edit_index),
                });
            }
            SemanticEdit::BuildAnomalyReel { query } => {
                let hits: Vec<_> = analysis
                    .anomalies
                    .iter()
                    .filter(|a| filter_anomaly(a, query))
                    .collect();
                if hits.is_empty() {
                    resolved.warnings.push(ResolutionWarning {
                        message: "no anomalies matched query".into(),
                        edit_index: Some(edit_index),
                    });
                }
                for a in hits {
                    resolved.resolved_events.push(ResolvedEvent {
                        event_id: a.anomaly_id.clone(),
                        kind: a.kind.clone(),
                        subject_id: a.subject_id,
                        range: MediaRange::new(
                            MediaTime::new(a.start_ticks, ts),
                            MediaTime::new(a.end_ticks, ts),
                        ),
                    });
                    resolved.resolved_ranges.push(MediaRange::new(
                        MediaTime::new(a.start_ticks, ts),
                        MediaTime::new(a.end_ticks, ts),
                    ));
                }
                resolved.decisions.push(ResolutionDecision {
                    code: "anomaly_reel".into(),
                    message: format!("{} anomaly ranges frozen", resolved.resolved_events.len()),
                    edit_index: Some(edit_index),
                });
            }
            SemanticEdit::CreateEventClips { query, .. } => {
                // Event clips: host must expand EventQuery; mark decision only.
                resolved.decisions.push(ResolutionDecision {
                    code: "event_clips".into(),
                    message: format!("event query needs host materialization: {query:?}"),
                    edit_index: Some(edit_index),
                });
                resolved.warnings.push(ResolutionWarning {
                    message: "CreateEventClips: ranges not expanded without host event table"
                        .into(),
                    edit_index: Some(edit_index),
                });
            }
        }
    }

    resolved.validate()?;
    Ok(resolved)
}

fn to_resolved(s: &SubjectEvidence, ts: u32) -> ResolvedSubject {
    ResolvedSubject {
        subject_id: s.subject_id,
        label: s.label.clone(),
        source_ids: s.source_ids.clone(),
        span: Some(MediaRange::new(
            MediaTime::new(s.first_ticks, ts),
            MediaTime::new(s.last_ticks, ts),
        )),
        confidence: s.confidence,
    }
}

fn push_subject_ranges(subjects: &[ResolvedSubject], _ts: u32, out: &mut ResolvedEditPlan) {
    for s in subjects {
        if let Some(span) = s.span {
            out.resolved_ranges.push(span);
        }
    }
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
        FrequencyMetric::Duration => (a.last_ticks - a.first_ticks)
            .cmp(&(b.last_ticks - b.first_ticks))
            .then_with(|| a.appearance_count.cmp(&b.appearance_count)),
    })
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
    let list = match selector {
        SubjectSelector::SubjectIds { ids } => analysis
            .subjects
            .iter()
            .filter(|s| ids.contains(&s.subject_id))
            .map(|s| to_resolved(s, ts))
            .collect::<Vec<_>>(),
        SubjectSelector::SubjectSet { name } => {
            // Host policy maps sets; without mapping, warn and match labels.
            let lower = name.to_lowercase();
            let hits: Vec<_> = analysis
                .subjects
                .iter()
                .filter(|s| {
                    s.label
                        .as_ref()
                        .is_some_and(|l| l.to_lowercase().contains(&lower))
                })
                .map(|s| to_resolved(s, ts))
                .collect();
            if hits.is_empty() {
                resolved.warnings.push(ResolutionWarning {
                    message: format!(
                        "subject set '{name}' not resolved — host should expand sets before freeze"
                    ),
                    edit_index: Some(edit_index),
                });
            }
            hits
        }
        SubjectSelector::TrackIds { ids } => {
            resolved.warnings.push(ResolutionWarning {
                message: format!(
                    "track_ids {ids:?} require host track→subject map; no subjects frozen"
                ),
                edit_index: Some(edit_index),
            });
            Vec::new()
        }
        SubjectSelector::FramePick { .. } => {
            resolved.warnings.push(ResolutionWarning {
                message: "frame_pick requires host SightLoom seed materialization".into(),
                edit_index: Some(edit_index),
            });
            Vec::new()
        }
        SubjectSelector::MostFrequent { metric } => most_frequent(analysis, *metric)
            .map(|s| vec![to_resolved(s, ts)])
            .unwrap_or_default(),
    };

    for s in &list {
        if !resolved
            .resolved_subjects
            .iter()
            .any(|x| x.subject_id == s.subject_id)
        {
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

    fn snap() -> AnalysisSnapshot {
        AnalysisSnapshot {
            media: "cam1".into(),
            source_hash: "src-abc".into(),
            vision_index_generation: "gen-00000001".into(),
            vision_index_hash: "idx-xyz".into(),
            timescale: 1_000_000_000,
            subjects: vec![
                SubjectEvidence {
                    subject_id: 1,
                    label: Some("alice".into()),
                    appearance_count: 3,
                    source_ids: vec![1],
                    first_ticks: 0,
                    last_ticks: 30_000_000_000,
                    confidence: Some(0.9),
                },
                SubjectEvidence {
                    subject_id: 2,
                    label: Some("bob".into()),
                    appearance_count: 10,
                    source_ids: vec![1, 2],
                    first_ticks: 0,
                    last_ticks: 60_000_000_000,
                    confidence: Some(0.8),
                },
            ],
            anomalies: vec![AnomalyEvidence {
                anomaly_id: "a1".into(),
                subject_id: Some(2),
                start_ticks: 80_000_000_000,
                end_ticks: 90_000_000_000,
                hour_of_day: Some(23),
                kind: "unusual_route".into(),
                score: 0.9,
            }],
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
        assert_eq!(resolved.resolved_subjects[0].subject_id, 2);
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
}
