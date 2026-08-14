//! Convert Intelligence mask artifacts into ReelForge [`MaskTimeline`].

use crate::mask::{MaskArtifact, RegionSample};
use crate::resolved::{ResolvedEditPlan, ResolvedMaskAsset};
use reelforge_core::MediaTime as RfMediaTime;
use reelforge_render_graph::{MaskSample, MaskTimeline};

/// Build a fused [`MaskTimeline`] from regional samples.
#[must_use]
pub fn mask_timeline_from_regions(regions: &[RegionSample]) -> MaskTimeline {
    let mut timeline = MaskTimeline::new();
    for region in regions {
        if let Some(sample) = region_to_sample(region) {
            timeline.push(sample);
        }
    }
    timeline
}

/// Fuse all materialized artifacts on a frozen plan.
#[must_use]
pub fn mask_timeline_from_resolved(resolved: &ResolvedEditPlan) -> MaskTimeline {
    mask_timeline_from_assets(&resolved.resolved_masks)
}

/// Fuse resolved mask assets (skips empty artifacts).
#[must_use]
pub fn mask_timeline_from_assets(assets: &[ResolvedMaskAsset]) -> MaskTimeline {
    let mut timeline = MaskTimeline::new();
    for asset in assets {
        if let Some(artifact) = &asset.artifact {
            append_artifact(&mut timeline, artifact);
        }
    }
    timeline
}

/// Append one artifact's regions onto a timeline.
pub fn append_artifact(timeline: &mut MaskTimeline, artifact: &MaskArtifact) {
    for region in &artifact.regions {
        if let Some(sample) = region_to_sample(region) {
            timeline.push(sample);
        }
    }
}

/// Convert a single region sample to a ReelForge mask sample.
#[must_use]
pub fn region_to_sample(region: &RegionSample) -> Option<MaskSample> {
    let timescale = region.at.timescale.max(1);
    let t = RfMediaTime::new(region.at.ticks, timescale).ok()?;
    let [left, top, right, bottom] = region.box_xyxy;
    let conf = region.confidence.unwrap_or(1.0);
    Some(MaskSample::from_box(t, left, top, right, bottom, conf))
}

/// Whether the timeline has any samples (ready for fused redaction).
#[must_use]
pub fn timeline_has_samples(timeline: &MaskTimeline) -> bool {
    !timeline.samples.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::NamespacedId;
    use crate::mask::MaskFidelity;
    use crate::time::{MediaRange, MediaTime};

    #[test]
    fn regions_become_box_samples() {
        let regions = vec![RegionSample {
            at: MediaTime::new(1_000_000, 1_000_000),
            box_xyxy: [10.0, 20.0, 110.0, 220.0],
            subject: Some(NamespacedId::sightloom_subject("g1", 7)),
            confidence: Some(0.9),
        }];
        let tl = mask_timeline_from_regions(&regions);
        assert_eq!(tl.samples.len(), 1);
        let s = &tl.samples[0];
        assert_eq!(s.left, Some(10.0));
        assert_eq!(s.top, Some(20.0));
        assert_eq!(s.right, Some(110.0));
        assert_eq!(s.bottom, Some(220.0));
        assert!((s.cx - 60.0).abs() < 0.01);
        assert!((s.conf - 0.9).abs() < 0.01);
    }

    #[test]
    fn resolved_masks_fuse() {
        let mut resolved = ResolvedEditPlan::new("m", "s", "g", "h");
        resolved.resolved_masks.push(ResolvedMaskAsset {
            mask_id: None,
            mask_ref: None,
            subject: None,
            range: Some(MediaRange::new(
                MediaTime::new(0, 30),
                MediaTime::new(30, 30),
            )),
            fidelity: MaskFidelity::BBoxProxy,
            artifact: Some(MaskArtifact::from_regions(vec![
                RegionSample {
                    at: MediaTime::new(0, 30),
                    box_xyxy: [0.0, 0.0, 40.0, 40.0],
                    subject: None,
                    confidence: None,
                },
                RegionSample {
                    at: MediaTime::new(15, 30),
                    box_xyxy: [5.0, 5.0, 45.0, 45.0],
                    subject: None,
                    confidence: None,
                },
            ])),
        });
        let tl = mask_timeline_from_resolved(&resolved);
        assert_eq!(tl.samples.len(), 2);
        assert!(timeline_has_samples(&tl));
    }
}
