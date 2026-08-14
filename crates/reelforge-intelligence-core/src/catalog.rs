//! Host-filled media / scene / subject catalogs (no `SightLoom` queries here).

use serde::{Deserialize, Serialize};

/// What `inspect_media` returns. The host (Capture / `ReelForge`) fills this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaInspection {
    /// Media id / path key.
    pub media: String,
    /// Duration seconds when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    /// Pixel size when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<(u32, u32)>,
    /// Whether a video stream is present.
    #[serde(default)]
    pub has_video: bool,
    /// Whether an audio stream is present.
    #[serde(default)]
    pub has_audio: bool,
}

/// One scene / shot range (host or future analysis).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneHit {
    /// Start seconds.
    pub start: f64,
    /// End seconds.
    pub end: f64,
    /// Optional label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One subject listing (opaque id — not a `ReelForge` primitive).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectHit {
    /// Opaque subject id.
    pub id: String,
    /// Optional display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional track ids from the vision host.
    #[serde(default)]
    pub track_ids: Vec<String>,
}

/// Snapshot the host injects. Intelligence does not crawl a `VisionIndex`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HostCatalog {
    /// Media inspection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInspection>,
    /// Scenes.
    #[serde(default)]
    pub scenes: Vec<SceneHit>,
    /// Subjects.
    #[serde(default)]
    pub subjects: Vec<SubjectHit>,
}
