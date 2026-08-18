//! Media time ranges shared with SightLoom / ReelForge hosts.

use serde::{Deserialize, Serialize};

/// Rational media time (ticks / timescale seconds), matching SightLoom `MediaTime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MediaTime {
    /// Tick count.
    pub ticks: i64,
    /// Ticks per second (`0` = invalid / unknown).
    pub timescale: u32,
}

impl MediaTime {
    /// New media time.
    #[must_use]
    pub const fn new(ticks: i64, timescale: u32) -> Self {
        Self { ticks, timescale }
    }

    /// True when this stamp cannot contribute a pad (zero ticks or unknown rate).
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.ticks == 0 || self.timescale == 0
    }

    /// Convert ticks into `target` timescale (lossy, saturating).
    #[must_use]
    pub fn to_timescale(self, target: u32) -> i64 {
        if self.timescale == 0 || target == 0 {
            return 0;
        }
        if self.timescale == target {
            return self.ticks;
        }
        let scaled = i128::from(self.ticks) * i128::from(target) / i128::from(self.timescale);
        i64::try_from(scaled.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap_or(i64::MAX)
    }

    /// Approximate seconds (lossy).
    #[must_use]
    pub fn as_secs_f64(self) -> f64 {
        if self.timescale == 0 {
            return 0.0;
        }
        self.ticks as f64 / f64::from(self.timescale)
    }

    /// Build from whole seconds at `timescale` (default 1e9 when `0`).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn from_secs_f64(secs: f64, timescale: u32) -> Self {
        let ts = timescale.max(1);
        if !secs.is_finite() {
            return Self::new(0, ts);
        }
        let ticks = (secs * f64::from(ts)).round() as i64;
        Self::new(ticks, ts)
    }
}

/// Half-open or inclusive media range for redaction / clips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRange {
    /// Start.
    pub start: MediaTime,
    /// End.
    pub end: MediaTime,
}

impl MediaRange {
    /// New range.
    #[must_use]
    pub const fn new(start: MediaTime, end: MediaTime) -> Self {
        Self { start, end }
    }

    /// Duration in the start timescale (0 if inverted).
    #[must_use]
    pub fn duration_ticks(self) -> i64 {
        (self.end.ticks - self.start.ticks).max(0)
    }

    /// Whether the range has a positive duration.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.duration_ticks() == 0
    }

    /// Expand by pre/post roll. Start is clamped to tick 0.
    #[must_use]
    pub fn padded(self, pre: MediaTime, post: MediaTime) -> Self {
        let ts = self.start.timescale.max(self.end.timescale).max(1);
        let pre_ticks = if pre.is_zero() {
            0
        } else {
            pre.to_timescale(ts)
        };
        let post_ticks = if post.is_zero() {
            0
        } else {
            post.to_timescale(ts)
        };
        let start = (self.start.ticks - pre_ticks).max(0);
        let end = (self.end.ticks + post_ticks).max(start);
        Self::new(MediaTime::new(start, ts), MediaTime::new(end, ts))
    }
}
