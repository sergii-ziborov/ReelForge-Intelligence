//! Convert Intelligence mask artifacts into ReelForge [`MaskTimeline`].

use crate::mask::{MaskArtifact, MaskGeometry, RegionSample};
use crate::resolved::{ResolvedEditPlan, ResolvedMaskAsset};
use reelforge_core::MediaTime as RfMediaTime;
use reelforge_render_graph::{MaskAsset, MaskAssetRef, MaskSample, MaskTimeline};

/// Build a fused [`MaskTimeline`] from regional samples.
///
/// Samples are collected then sorted once (O(n log n)) instead of per-`push`
/// insertion on the ReelForge timeline.
#[must_use]
pub fn mask_timeline_from_regions(regions: &[RegionSample]) -> MaskTimeline {
    let mut samples = Vec::with_capacity(regions.len());
    for region in regions {
        if let Some(sample) = region_to_sample(region) {
            samples.push(sample);
        }
    }
    sort_samples(&mut samples);
    MaskTimeline {
        samples,
        ..MaskTimeline::new()
    }
}

/// Fuse all materialized artifacts on a frozen plan.
#[must_use]
pub fn mask_timeline_from_resolved(resolved: &ResolvedEditPlan) -> MaskTimeline {
    mask_timeline_from_assets(&resolved.resolved_masks)
}

/// Fuse resolved mask assets (skips empty artifacts).
#[must_use]
pub fn mask_timeline_from_assets(assets: &[ResolvedMaskAsset]) -> MaskTimeline {
    let mut samples = Vec::new();
    for asset in assets {
        if let Some(artifact) = &asset.artifact {
            for region in &artifact.regions {
                if let Some(sample) =
                    region_to_sample_with_geometry(region, artifact.geometry.as_ref())
                {
                    samples.push(sample);
                }
            }
        }
    }
    sort_samples(&mut samples);
    MaskTimeline {
        samples,
        ..MaskTimeline::new()
    }
}

/// Append one artifact's regions onto a timeline (re-sorts by time).
pub fn append_artifact(timeline: &mut MaskTimeline, artifact: &MaskArtifact) {
    for region in &artifact.regions {
        if let Some(sample) = region_to_sample_with_geometry(region, artifact.geometry.as_ref()) {
            timeline.samples.push(sample);
        }
    }
    sort_samples(&mut timeline.samples);
}

fn sort_samples(samples: &mut [MaskSample]) {
    samples.sort_by(|a, b| {
        a.t.as_secs()
            .partial_cmp(&b.t.as_secs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Convert a single region sample to a ReelForge mask sample.
#[must_use]
pub fn region_to_sample(region: &RegionSample) -> Option<MaskSample> {
    region_to_sample_with_geometry(region, region.geometry.as_ref())
}

/// Convert a region, attaching artifact-level geometry when the region has none.
#[must_use]
pub fn region_to_sample_with_geometry(
    region: &RegionSample,
    fallback: Option<&MaskGeometry>,
) -> Option<MaskSample> {
    let timescale = region.at.timescale.max(1);
    let t = RfMediaTime::new(region.at.ticks, timescale).ok()?;
    let [left, top, right, bottom] = region.box_xyxy;
    let conf = region.confidence.unwrap_or(1.0);
    let mut sample = MaskSample::from_box(t, left, top, right, bottom, conf);
    if let Some(id) = region.subject.as_ref() {
        sample = sample.with_subject(reelforge_render_graph::SubjectId::new(id.as_uri()));
    }
    let geometry = region.geometry.as_ref().or(fallback);
    if let Some(asset) = geometry.and_then(geometry_to_asset) {
        sample = sample.with_asset(MaskAssetRef::inline(asset));
    }
    Some(sample)
}

/// Map Intelligence [`MaskGeometry`] onto a ReelForge [`MaskAsset`].
#[must_use]
pub fn geometry_to_asset(geometry: &MaskGeometry) -> Option<MaskAsset> {
    match geometry {
        MaskGeometry::Rle {
            width,
            height,
            counts,
        } => {
            // COCO-style alternating background/foreground, starting with background.
            let mut runs = Vec::with_capacity(counts.len());
            let mut value = 0_u8;
            for &count in counts {
                runs.push((count, value));
                value = 1_u8.saturating_sub(value);
            }
            Some(MaskAsset::Rle {
                width: *width,
                height: *height,
                runs,
            })
        }
        MaskGeometry::Dense {
            width,
            height,
            data,
        } => Some(MaskAsset::Dense {
            width: *width,
            height: *height,
            data: data.clone(),
        }),
        MaskGeometry::Polygon { rings } => {
            let ring = rings.first()?;
            if ring.len() < 3 {
                return None;
            }
            Some(MaskAsset::Polygon {
                points: ring.iter().map(|p| (p[0], p[1])).collect(),
            })
        }
        MaskGeometry::BBox { .. } => None,
        MaskGeometry::External {
            package_id,
            mask_ref,
        } => Some(MaskAsset::External {
            package_id: package_id.clone(),
            mask_ref: *mask_ref,
        }),
    }
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
            geometry: None,
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
                    geometry: None,
                },
                RegionSample {
                    at: MediaTime::new(15, 30),
                    box_xyxy: [5.0, 5.0, 45.0, 45.0],
                    subject: None,
                    confidence: None,
                    geometry: None,
                },
            ])),
        });
        let tl = mask_timeline_from_resolved(&resolved);
        assert_eq!(tl.samples.len(), 2);
        assert!(timeline_has_samples(&tl));
    }

    #[test]
    fn rle_geometry_survives_as_mask_asset() {
        let region = RegionSample {
            at: MediaTime::new(0, 30),
            box_xyxy: [0.0, 0.0, 3.0, 1.0],
            subject: None,
            confidence: Some(1.0),
            geometry: Some(crate::mask::MaskGeometry::Rle {
                width: 3,
                height: 1,
                counts: vec![0, 1, 2],
            }),
        };
        let sample = region_to_sample(&region).expect("sample");
        let asset = sample.asset.expect("true geometry asset");
        match asset.asset {
            reelforge_render_graph::MaskAsset::Rle {
                width,
                height,
                runs,
            } => {
                assert_eq!(width, 3);
                assert_eq!(height, 1);
                assert_eq!(runs, vec![(0, 0), (1, 1), (2, 0)]);
            }
            other => panic!("expected RLE, got {other:?}"),
        }
    }
}
