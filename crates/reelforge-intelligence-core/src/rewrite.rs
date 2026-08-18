//! Host materialization: rewrite [`SubjectSelector::FramePick`] → [`SubjectSelector::SubjectIds`].
//!
//! Intelligence does not open photos or run re-id. The host binds each pick
//! (after SightLoom `search_photo`) and this module only rewrites the plan.

use crate::edit::{SemanticEdit, SemanticEditPlan};
use crate::error::{IntelError, Result};
use crate::selector::SubjectSelector;
use serde::{Deserialize, Serialize};

/// One host binding: this frame pick is these VisionIndex subject ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectorBinding {
    /// Media key on the [`SubjectSelector::FramePick`].
    pub media: String,
    /// Frame index on the pick.
    pub frame_index: u64,
    /// Box on the pick (`left, top, right, bottom`).
    pub box_xyxy: [f32; 4],
    /// Subject ids the host resolved (must be non-empty).
    pub ids: Vec<u64>,
}

/// Rewrite every [`SubjectSelector::FramePick`] using host bindings.
///
/// Other selectors, `media`, `policy`, and non-selector edits are left untouched.
/// An unmatched pick or a binding with empty `ids` is an error — never a silent
/// empty [`SubjectSelector::SubjectIds`].
///
/// # Errors
///
/// Unbound `FramePick`, or a matching binding with no subject ids.
pub fn rewrite_selectors(
    mut plan: SemanticEditPlan,
    bindings: &[SelectorBinding],
) -> Result<SemanticEditPlan> {
    for (edit_index, edit) in plan.edits.iter_mut().enumerate() {
        let Some(selector) = edit_selector_mut(edit) else {
            continue;
        };
        if let SubjectSelector::FramePick {
            media,
            frame_index,
            box_xyxy,
        } = selector
        {
            *selector = SubjectSelector::SubjectIds {
                ids: resolve_pick(edit_index, media, *frame_index, *box_xyxy, bindings)?,
            };
        }
    }
    Ok(plan)
}

/// Parse host bindings from JSON: a raw array or `{ "bindings": [...] }`.
///
/// # Errors
///
/// Serde / wrong shape.
pub fn bindings_from_value(value: &serde_json::Value) -> Result<Vec<SelectorBinding>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(arr) = value.as_array() {
        return serde_json::from_value(serde_json::Value::Array(arr.clone()))
            .map_err(|e| IntelError::message(format!("rewrite_selectors: bindings: {e}")));
    }
    if let Some(inner) = value.get("bindings") {
        return bindings_from_value(inner);
    }
    serde_json::from_value(value.clone())
        .map_err(|e| IntelError::message(format!("rewrite_selectors: bindings: {e}")))
}

fn edit_selector_mut(edit: &mut SemanticEdit) -> Option<&mut SubjectSelector> {
    match edit {
        SemanticEdit::BlurSubject { subject }
        | SemanticEdit::FollowSubject { subject, .. }
        | SemanticEdit::BuildSubjectReel { subject, .. } => Some(subject),
        SemanticEdit::BlurEveryoneExcept { allowed, .. } => Some(allowed),
        SemanticEdit::BuildMostFrequentSubjectReel { .. }
        | SemanticEdit::BuildAnomalyReel { .. }
        | SemanticEdit::CreateEventClips { .. }
        | SemanticEdit::RedactPii { .. } => None,
    }
}

fn resolve_pick(
    edit_index: usize,
    media: &str,
    frame_index: u64,
    box_xyxy: [f32; 4],
    bindings: &[SelectorBinding],
) -> Result<Vec<u64>> {
    let Some(hit) = bindings.iter().find(|b| {
        b.media == media && b.frame_index == frame_index && boxes_eq(b.box_xyxy, box_xyxy)
    }) else {
        return Err(IntelError::message(format!(
            "rewrite_selectors: edit {edit_index} frame_pick media={media} frame={frame_index} box={box_xyxy:?} has no host binding"
        )));
    };
    if hit.ids.is_empty() {
        return Err(IntelError::message(format!(
            "rewrite_selectors: edit {edit_index} frame_pick bound to empty subject ids"
        )));
    }
    Ok(hit.ids.clone())
}

fn boxes_eq(left: [f32; 4], right: [f32; 4]) -> bool {
    left.iter()
        .zip(right)
        .all(|(a, b)| (*a - b).abs() <= 1.0e-3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::SemanticEdit;
    use crate::policy::UncertaintyPolicy;

    fn pick(media: &str, frame: u64) -> SubjectSelector {
        SubjectSelector::FramePick {
            media: media.into(),
            frame_index: frame,
            box_xyxy: [80.0, 40.0, 560.0, 680.0],
        }
    }

    fn bind(media: &str, frame: u64, ids: Vec<u64>) -> SelectorBinding {
        SelectorBinding {
            media: media.into(),
            frame_index: frame,
            box_xyxy: [80.0, 40.0, 560.0, 680.0],
            ids,
        }
    }

    #[test]
    fn rewrites_frame_pick_to_subject_ids() {
        let plan = SemanticEditPlan::new("scene.mp4")
            .with_edit(SemanticEdit::BlurEveryoneExcept {
                allowed: pick("scene.mp4", 0),
                uncertain_identity: Some(UncertaintyPolicy::Blur),
            })
            .with_edit(SemanticEdit::FollowSubject {
                subject: pick("scene.mp4", 0),
                framing: crate::edit::FramingPolicy::Tight,
            });
        let out = rewrite_selectors(plan.clone(), &[bind("scene.mp4", 0, vec![1])]).unwrap();
        assert_eq!(out.media, plan.media);
        assert_eq!(out.policy, plan.policy);
        assert_eq!(
            out.edits[0],
            SemanticEdit::BlurEveryoneExcept {
                allowed: SubjectSelector::SubjectIds { ids: vec![1] },
                uncertain_identity: Some(UncertaintyPolicy::Blur),
            }
        );
        match &out.edits[1] {
            SemanticEdit::FollowSubject { subject, .. } => {
                assert_eq!(subject, &SubjectSelector::SubjectIds { ids: vec![1] });
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_frame_pick_is_an_error() {
        let plan = SemanticEditPlan::new("cam").with_edit(SemanticEdit::BlurSubject {
            subject: pick("cam", 3),
        });
        let err = rewrite_selectors(plan, &[bind("cam", 0, vec![1])]).unwrap_err();
        assert!(err.to_string().contains("no host binding"));
    }

    #[test]
    fn empty_binding_ids_are_an_error() {
        let plan = SemanticEditPlan::new("cam").with_edit(SemanticEdit::BlurSubject {
            subject: pick("cam", 0),
        });
        let err = rewrite_selectors(plan, &[bind("cam", 0, vec![])]).unwrap_err();
        assert!(err.to_string().contains("empty subject ids"));
    }

    #[test]
    fn non_pick_selectors_pass_through() {
        let plan = SemanticEditPlan::new("cam").with_edit(SemanticEdit::BlurSubject {
            subject: SubjectSelector::MostFrequent {
                metric: crate::edit::FrequencyMetric::Duration,
            },
        });
        let out = rewrite_selectors(plan.clone(), &[]).unwrap();
        assert_eq!(out, plan);
    }

    #[test]
    fn bindings_json_accepts_array_or_wrapper() {
        let raw = serde_json::json!([{
            "media": "cam",
            "frame_index": 1,
            "box_xyxy": [0.0, 0.0, 1.0, 1.0],
            "ids": [7, 8]
        }]);
        let wrapped = serde_json::json!({ "bindings": raw });
        assert_eq!(
            bindings_from_value(&raw).unwrap(),
            bindings_from_value(&wrapped).unwrap()
        );
        assert_eq!(bindings_from_value(&raw).unwrap()[0].ids, vec![7, 8]);
    }
}
