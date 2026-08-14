//! Namespaced entity IDs so ReelForge never collides packages or providers.
//!
//! SightLoom uses `SubjectId(u64)` inside one VisionIndex. ReelForge uses
//! opaque strings. Intelligence bridges with URIs:
//!
//! ```text
//! sightloom://<vision-index-id>/subjects/184
//! sightloom://<vision-index-id>/sources/2/tracks/91
//! ```

use crate::error::{IntelError, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Provider namespace for analysis sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisNamespace {
    /// SightLoom VisionIndex.
    #[default]
    SightLoom,
    /// Capture event stream.
    Capture,
    /// Transcript / ASR.
    Transcript,
    /// OCR.
    Ocr,
    /// Audio activity.
    AudioActivity,
    /// Host custom.
    Custom,
}

impl AnalysisNamespace {
    /// URI scheme prefix (`sightloom`, `capture`, …).
    #[must_use]
    pub const fn scheme(self) -> &'static str {
        match self {
            Self::SightLoom => "sightloom",
            Self::Capture => "capture",
            Self::Transcript => "transcript",
            Self::Ocr => "ocr",
            Self::AudioActivity => "audio",
            Self::Custom => "custom",
        }
    }

    /// Parse scheme string.
    #[must_use]
    pub fn from_scheme(s: &str) -> Option<Self> {
        match s {
            "sightloom" => Some(Self::SightLoom),
            "capture" => Some(Self::Capture),
            "transcript" => Some(Self::Transcript),
            "ocr" => Some(Self::Ocr),
            "audio" => Some(Self::AudioActivity),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// Kind of namespaced entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// Subject / identity.
    Subject,
    /// Local track within a source.
    Track,
    /// Source / camera.
    Source,
    /// Mask handle.
    Mask,
    /// Event / anomaly.
    Event,
}

impl EntityKind {
    /// Path segment (`subjects`, `tracks`, …).
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Subject => "subjects",
            Self::Track => "tracks",
            Self::Source => "sources",
            Self::Mask => "masks",
            Self::Event => "events",
        }
    }

    fn from_path(s: &str) -> Option<Self> {
        match s {
            "subjects" => Some(Self::Subject),
            "tracks" => Some(Self::Track),
            "sources" => Some(Self::Source),
            "masks" => Some(Self::Mask),
            "events" => Some(Self::Event),
            _ => None,
        }
    }
}

/// Opaque, globally unique entity id for Intelligence → ReelForge.
///
/// Display form: `{scheme}://{index_id}/subjects/{n}` or
/// `{scheme}://{index_id}/sources/{src}/tracks/{local}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NamespacedId {
    /// Analysis provider namespace.
    pub namespace: AnalysisNamespace,
    /// VisionIndex package id / analysis generation key (never empty for SightLoom).
    pub index_id: String,
    /// Entity kind.
    pub kind: EntityKind,
    /// Primary numeric id (subject, track local, source, mask, event).
    pub id: u64,
    /// Optional parent source for tracks: `sources/{source}/tracks/{id}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<u32>,
}

impl NamespacedId {
    /// SightLoom subject: `sightloom://{index}/subjects/{id}`.
    #[must_use]
    pub fn sightloom_subject(index_id: impl Into<String>, subject_id: u64) -> Self {
        Self {
            namespace: AnalysisNamespace::SightLoom,
            index_id: index_id.into(),
            kind: EntityKind::Subject,
            id: subject_id,
            source_id: None,
        }
    }

    /// SightLoom track: `sightloom://{index}/sources/{src}/tracks/{local}`.
    #[must_use]
    pub fn sightloom_track(index_id: impl Into<String>, source_id: u32, track_id: u32) -> Self {
        Self {
            namespace: AnalysisNamespace::SightLoom,
            index_id: index_id.into(),
            kind: EntityKind::Track,
            id: u64::from(track_id),
            source_id: Some(source_id),
        }
    }

    /// SightLoom source: `sightloom://{index}/sources/{id}`.
    #[must_use]
    pub fn sightloom_source(index_id: impl Into<String>, source_id: u32) -> Self {
        Self {
            namespace: AnalysisNamespace::SightLoom,
            index_id: index_id.into(),
            kind: EntityKind::Source,
            id: u64::from(source_id),
            source_id: None,
        }
    }

    /// SightLoom mask: `sightloom://{index}/masks/{handle}`.
    #[must_use]
    pub fn sightloom_mask(index_id: impl Into<String>, mask_ref: u64) -> Self {
        Self {
            namespace: AnalysisNamespace::SightLoom,
            index_id: index_id.into(),
            kind: EntityKind::Mask,
            id: mask_ref,
            source_id: None,
        }
    }

    /// Canonical URI string (ReelForge `SubjectId` domain).
    #[must_use]
    pub fn as_uri(&self) -> String {
        let scheme = self.namespace.scheme();
        match (self.kind, self.source_id) {
            (EntityKind::Track, Some(src)) => {
                format!(
                    "{scheme}://{}/sources/{src}/tracks/{}",
                    self.index_id, self.id
                )
            }
            (kind, _) => {
                format!("{scheme}://{}/{}/{}", self.index_id, kind.path(), self.id)
            }
        }
    }

    /// Parse a namespaced URI.
    ///
    /// # Errors
    ///
    /// Malformed URI.
    pub fn parse(uri: &str) -> Result<Self> {
        let (scheme, rest) = uri
            .split_once("://")
            .ok_or_else(|| IntelError::message(format!("id missing :// : {uri}")))?;
        let namespace = AnalysisNamespace::from_scheme(scheme)
            .ok_or_else(|| IntelError::message(format!("unknown id scheme: {scheme}")))?;
        let mut parts = rest.split('/');
        let index_id = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| IntelError::message("id missing index id"))?
            .to_string();
        let seg = parts
            .next()
            .ok_or_else(|| IntelError::message("id missing entity path"))?;
        if seg == "sources" {
            let src: u32 = parts
                .next()
                .ok_or_else(|| IntelError::message("id missing source number"))?
                .parse()
                .map_err(|_| IntelError::message("bad source id"))?;
            match parts.next() {
                None => {
                    return Ok(Self {
                        namespace,
                        index_id,
                        kind: EntityKind::Source,
                        id: u64::from(src),
                        source_id: None,
                    });
                }
                Some("tracks") => {
                    let track: u32 = parts
                        .next()
                        .ok_or_else(|| IntelError::message("id missing track number"))?
                        .parse()
                        .map_err(|_| IntelError::message("bad track id"))?;
                    return Ok(Self {
                        namespace,
                        index_id,
                        kind: EntityKind::Track,
                        id: u64::from(track),
                        source_id: Some(src),
                    });
                }
                Some(other) => {
                    return Err(IntelError::message(format!(
                        "unexpected path after sources: {other}"
                    )));
                }
            }
        }
        let kind = EntityKind::from_path(seg)
            .ok_or_else(|| IntelError::message(format!("unknown entity path: {seg}")))?;
        let id: u64 = parts
            .next()
            .ok_or_else(|| IntelError::message("id missing numeric id"))?
            .parse()
            .map_err(|_| IntelError::message("bad numeric id"))?;
        Ok(Self {
            namespace,
            index_id,
            kind,
            id,
            source_id: None,
        })
    }
}

impl fmt::Display for NamespacedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_uri())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_uri_roundtrip() {
        let id = NamespacedId::sightloom_subject("gen-00000001", 184);
        assert_eq!(id.as_uri(), "sightloom://gen-00000001/subjects/184");
        let back = NamespacedId::parse(&id.as_uri()).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn track_uri_roundtrip() {
        let id = NamespacedId::sightloom_track("pkg-a", 2, 91);
        assert_eq!(id.as_uri(), "sightloom://pkg-a/sources/2/tracks/91");
        let back = NamespacedId::parse(&id.as_uri()).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn two_packages_do_not_collide() {
        let a = NamespacedId::sightloom_subject("pkg-a", 184).as_uri();
        let b = NamespacedId::sightloom_subject("pkg-b", 184).as_uri();
        assert_ne!(a, b);
    }
}
