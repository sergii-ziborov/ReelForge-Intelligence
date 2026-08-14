//! Event / vision queries against a `VisionIndex` (host provides results).

use serde::{Deserialize, Serialize};

/// Query for events or subjects in a `VisionIndex`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventQuery {
    /// All zone enter events for a zone id.
    ZoneEnters {
        /// Zone id.
        zone_id: u16,
    },
    /// Idle / low-activity ranges (Capture activity stream).
    IdleRanges {
        /// Minimum idle duration seconds.
        min_seconds: f64,
    },
    /// Custom host query string.
    Custom {
        /// Opaque query.
        expr: String,
    },
}
