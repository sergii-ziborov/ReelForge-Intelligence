//! Follow-subject crop: margins, aspect fit, clamp, EMA, pan-speed cap.

use crate::edit::FramingPolicy;
use crate::mask::RegionSample;
use serde::{Deserialize, Serialize};

/// Input for a follow-crop computation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSize {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

impl FrameSize {
    /// Construct a frame size (zero is invalid).
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self { width, height })
        }
    }
}

/// Computed crop rectangle (even integer pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropRect {
    /// Left.
    pub x: u32,
    /// Top.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl CropRect {
    /// JSON params for `rf.transform.crop`.
    #[must_use]
    pub fn to_params(self) -> serde_json::Value {
        serde_json::json!({ "x": self.x, "y": self.y, "w": self.w, "h": self.h })
    }
}

/// Follow-crop knobs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramingOptions {
    /// EMA alpha for box center (0..1).
    pub alpha: f32,
    /// Max pan as a fraction of frame width per second.
    pub max_pan_frac_per_sec: f32,
    /// Output aspect `w/h`. `None` keeps source aspect.
    pub output_aspect: Option<f32>,
}

impl Default for FramingOptions {
    fn default() -> Self {
        Self {
            alpha: 0.35,
            max_pan_frac_per_sec: 0.4,
            output_aspect: None,
        }
    }
}

fn margin_scale(policy: FramingPolicy) -> f32 {
    match policy {
        FramingPolicy::Tight => 1.10,
        FramingPolicy::Medium => 1.25,
        FramingPolicy::Wide => 1.40,
    }
}

/// Compute a single crop from subject boxes. Errors when geometry or frame is missing.
///
/// # Errors
///
/// No boxes or invalid frame size.
#[allow(clippy::similar_names, clippy::cast_possible_truncation)]
pub fn compute_follow_crop(
    boxes: &[RegionSample],
    policy: FramingPolicy,
    frame: FrameSize,
    opts: FramingOptions,
) -> Result<CropRect, String> {
    if boxes.is_empty() {
        return Err("follow-crop: no subject boxes".into());
    }
    let scale = margin_scale(policy);
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut bw = 0.0f32;
    let mut bh = 0.0f32;
    let mut prev_t: Option<f64> = None;
    let fw = frame.width as f32;
    let fh = frame.height as f32;
    let max_pan = opts.max_pan_frac_per_sec * fw;

    for (i, sample) in boxes.iter().enumerate() {
        let [left, top, right, bottom] = sample.box_xyxy;
        let sample_cx = (left + right) * 0.5;
        let sample_cy = (top + bottom) * 0.5;
        let sample_w = (right - left).abs() * scale;
        let sample_h = (bottom - top).abs() * scale;
        if i == 0 {
            cx = sample_cx;
            cy = sample_cy;
            bw = sample_w;
            bh = sample_h;
            prev_t = Some(sample.at.as_secs_f64());
            continue;
        }
        cx = opts.alpha * sample_cx + (1.0 - opts.alpha) * cx;
        cy = opts.alpha * sample_cy + (1.0 - opts.alpha) * cy;
        bw = opts.alpha * sample_w + (1.0 - opts.alpha) * bw;
        bh = opts.alpha * sample_h + (1.0 - opts.alpha) * bh;
        if let Some(pt) = prev_t {
            let dt = (sample.at.as_secs_f64() - pt).max(1e-3);
            #[allow(clippy::cast_possible_truncation)]
            let cap = max_pan * dt as f32;
            let dx = sample_cx - cx;
            if dx.abs() > cap {
                cx += cap.copysign(dx);
            }
        }
        prev_t = Some(sample.at.as_secs_f64());
    }

    let aspect = opts.output_aspect.unwrap_or(fw / fh);
    if bw < 1.0 {
        bw = 1.0;
    }
    if bh < 1.0 {
        bh = 1.0;
    }
    if bw / bh > aspect {
        bh = bw / aspect;
    } else {
        bw = bh * aspect;
    }
    bw = bw.min(fw);
    bh = bh.min(fh);

    let mut x = cx - bw * 0.5;
    let mut y = cy - bh * 0.5;
    x = x.clamp(0.0, fw - bw);
    y = y.clamp(0.0, fh - bh);

    let xi = even_floor(x, frame.width);
    let yi = even_floor(y, frame.height);
    let mut wi = even_ceil(bw, frame.width.saturating_sub(xi));
    let mut hi = even_ceil(bh, frame.height.saturating_sub(yi));
    wi = wi.max(2);
    hi = hi.max(2);
    Ok(CropRect {
        x: xi,
        y: yi,
        w: wi,
        h: hi,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn even_floor(v: f32, max: u32) -> u32 {
    let n = v.floor().max(0.0) as u32;
    let n = n.min(max.saturating_sub(2));
    n & !1
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn even_ceil(v: f32, max: u32) -> u32 {
    let n = v.ceil().max(2.0) as u32;
    let n = n.min(max.max(2));
    if n.is_multiple_of(2) {
        n
    } else {
        n.saturating_sub(1).max(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::MediaTime;

    fn box_at(l: f32, t: f32, r: f32, b: f32) -> RegionSample {
        RegionSample {
            at: MediaTime::new(0, 1_000_000_000),
            box_xyxy: [l, t, r, b],
            subject: None,
            confidence: Some(1.0),
            geometry: None,
        }
    }

    #[test]
    fn tight_crop_contains_box() {
        let frame = FrameSize::new(1920, 1080).unwrap();
        let crop = compute_follow_crop(
            &[box_at(100.0, 100.0, 200.0, 200.0)],
            FramingPolicy::Tight,
            frame,
            FramingOptions::default(),
        )
        .unwrap();
        assert!(crop.w >= 100 && crop.h >= 100);
        assert_eq!(crop.w % 2, 0);
        assert_eq!(crop.h % 2, 0);
        assert!(crop.x <= 100);
        assert!(crop.x + crop.w >= 200);
        assert!(crop.y + crop.h <= 1080);
    }

    #[test]
    fn wide_is_larger_than_tight() {
        let frame = FrameSize::new(1920, 1080).unwrap();
        let boxes = [box_at(400.0, 300.0, 500.0, 400.0)];
        let tight = compute_follow_crop(
            &boxes,
            FramingPolicy::Tight,
            frame,
            FramingOptions::default(),
        )
        .unwrap();
        let wide = compute_follow_crop(
            &boxes,
            FramingPolicy::Wide,
            frame,
            FramingOptions::default(),
        )
        .unwrap();
        assert!(wide.w >= tight.w);
        assert!(wide.h >= tight.h);
    }

    #[test]
    fn missing_boxes_error() {
        let frame = FrameSize::new(1920, 1080).unwrap();
        assert!(
            compute_follow_crop(&[], FramingPolicy::Tight, frame, FramingOptions::default())
                .is_err()
        );
    }
}
