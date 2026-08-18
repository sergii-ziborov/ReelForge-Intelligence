//! Oracle host: two known people on a real composed mp4 → VisionIndex package.
//!
//! This is **not** ONNX detection. It writes the document SightLoom would emit
//! after a host mapped photo→subject and boxed both people.

use reelforge_intelligence_sightloom::encode_slm1_rle;
use sightloom_core::{AppearanceId, ClassId, MediaTime, SourceId, SubjectId, TrackId};
use sightloom_index::{
    Appearance, SourceEntry, SubjectProfile, TrackSample, VisionIndex, VisionIndexPackage,
};
use std::env;
use std::path::PathBuf;

const W: u32 = 1280;
const H: u32 = 720;
const FPS: u32 = 10;
const FRAMES: u64 = 30;
const ALICE: (f32, f32, f32, f32) = (80.0, 40.0, 560.0, 680.0);
const BOB: (f32, f32, f32, f32) = (720.0, 40.0, 1200.0, 680.0);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/real-video-e2e/vision_index"));
    let media = env::args().nth(2).unwrap_or_else(|| {
        std::fs::canonicalize("target/real-video-e2e/scene.mp4")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "target/real-video-e2e/scene.mp4".into())
    });

    let mut index = VisionIndex::new(media.clone());
    index.add_source(SourceEntry {
        source_id: 1,
        uri: format!("file://{media}"),
        hash: None,
    });

    let alice_mask = index.masks.insert(rect_slm1(ALICE));
    let bob_mask = index.masks.insert(rect_slm1(BOB));

    for frame in 0..FRAMES {
        index.push_track(sample(1, frame, ALICE, alice_mask.0));
        index.push_track(sample(2, frame, BOB, bob_mask.0));
    }

    let last = MediaTime::new(i64::try_from(FRAMES - 1).unwrap(), FPS).unwrap();
    let start = MediaTime::new(0, FPS).unwrap();
    index.subjects.push(profile(1, "alice", last));
    index.subjects.push(profile(2, "bob", last));
    index.appearances.push(appearance(1, 1, start, last));
    index.appearances.push(appearance(2, 2, start, last));

    std::fs::create_dir_all(&out)?;
    VisionIndexPackage::save(&index, &out)?;
    println!("wrote VisionIndex package → {}", out.display());
    println!("media={media}");
    println!("subjects=alice:1 bob:2 frames={FRAMES} fps={FPS}");
    Ok(())
}

fn rect_slm1(box_xyxy: (f32, f32, f32, f32)) -> Vec<u8> {
    let (left, top, right, bottom) = box_xyxy;
    let l = left.max(0.0) as u32;
    let t = top.max(0.0) as u32;
    let r = right.min(W as f32) as u32;
    let b = bottom.min(H as f32) as u32;
    encode_slm1_rle(W, H, &rect_rle(W, H, l, t, r, b))
}

fn rect_rle(width: u32, height: u32, left: u32, top: u32, right: u32, bottom: u32) -> Vec<u32> {
    let mut counts = Vec::new();
    let mut run_fg = false;
    let mut run_len = 0_u32;
    for y in 0..height {
        for x in 0..width {
            let fg = x >= left && x < right && y >= top && y < bottom;
            if fg == run_fg {
                run_len += 1;
            } else {
                counts.push(run_len);
                run_fg = fg;
                run_len = 1;
            }
        }
    }
    counts.push(run_len);
    counts
}

fn sample(subject: u64, frame: u64, box_xyxy: (f32, f32, f32, f32), mask_ref: u64) -> TrackSample {
    let (left, top, right, bottom) = box_xyxy;
    TrackSample {
        sample_id: 0,
        supersedes: None,
        revision: 1,
        idempotency_key: 0,
        source_id: SourceId(1),
        frame_index: frame,
        pts: MediaTime::new(i64::try_from(frame).unwrap(), FPS).unwrap(),
        track_id: TrackId(u32::try_from(subject).unwrap()),
        track_uid: None,
        subject_id: Some(SubjectId(subject)),
        class_id: Some(ClassId(0)),
        left,
        top,
        right,
        bottom,
        confidence: 0.94,
        mask_ref,
    }
}

fn profile(id: u64, label: &str, last: MediaTime) -> SubjectProfile {
    SubjectProfile {
        subject_id: SubjectId(id),
        label: Some(label.into()),
        appearance_count: 1,
        source_count: 1,
        total_duration_ns: 3_000_000_000,
        first_seen: Some(MediaTime::new(0, FPS).unwrap()),
        last_seen: Some(last),
        embedding: None,
    }
}

fn appearance(id: u64, track: u32, start: MediaTime, end: MediaTime) -> Appearance {
    Appearance {
        appearance_id: AppearanceId(id),
        subject_id: Some(SubjectId(id)),
        track_id: Some(TrackId(track)),
        source_id: SourceId(1),
        start,
        end,
        class_id: Some(ClassId(0)),
        peak_confidence: 0.94,
        evidence: None,
    }
}
