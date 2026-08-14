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

    /// Approximate seconds (lossy).
    #[must_use]
    pub fn as_secs_f64(self) -> f64 {
        if self.timescale == 0 {
            return 0.0;
        }
        self.ticks as f64 / f64::from(self.timescale)
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
}
