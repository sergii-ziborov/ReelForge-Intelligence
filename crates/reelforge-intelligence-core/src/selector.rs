//! Subject selection for semantic privacy / follow ops.

use crate::edit::FrequencyMetric;
use serde::{Deserialize, Serialize};

/// How a subject is designated before re-id / tracking materialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubjectSelector {
    /// Click / pick on a single frame (M3 vertical slice).
    FramePick {
        /// Media asset id in the host project.
        media: String,
        /// Frame index or stamp (host-defined).
        frame_index: u64,
        /// Normalized or pixel box: left, top, right, bottom.
        box_xyxy: [f32; 4],
    },
    /// Named set (e.g. `family`) resolved by Capture / host policy.
    SubjectSet {
        /// Set name.
        name: String,
    },
    /// Explicit subject ids from `VisionIndex`.
    SubjectIds {
        /// Subject ids.
        ids: Vec<u64>,
    },
    /// Track ids from `VisionIndex`.
    TrackIds {
        /// Track ids.
        ids: Vec<u32>,
    },
    /// Resolve to the most frequent subject under a metric.
    MostFrequent {
        /// Frequency metric.
        #[serde(default)]
        metric: FrequencyMetric,
    },
}
