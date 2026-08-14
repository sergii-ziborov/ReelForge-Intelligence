//! Load VisionIndex packages and produce providers.

use crate::convert::{snapshot_from_index, subject_boxes_from_index};
use crate::error::SightLoomAdapterError;
use reelforge_intelligence_core::{AnalysisSnapshot, SightLoomProvider};
use sightloom_index::{VisionIndex, VisionIndexPackage};
use std::fs;
use std::path::{Path, PathBuf};

/// Loaded package ready for Intelligence resolve.
#[derive(Debug, Clone)]
pub struct LoadedVisionPackage {
    /// In-memory index.
    pub index: VisionIndex,
    /// Package root path.
    pub package_root: PathBuf,
    /// Active generation name (`gen-00000001` or `legacy`).
    pub generation: String,
    /// Content hash (from checksums or computed digest).
    pub content_hash: String,
    /// Media/source hash when available (header provenance or empty).
    pub source_hash: String,
    /// Snapshot for resolve_plan.
    pub snapshot: AnalysisSnapshot,
    /// Preview bboxes per subject.
    pub subject_boxes: Vec<(u64, [f32; 4])>,
}

impl LoadedVisionPackage {
    /// Build a [`SightLoomProvider`] from this load.
    #[must_use]
    pub fn provider(&self) -> SightLoomProvider {
        SightLoomProvider::new(self.snapshot.clone()).with_subject_boxes(self.subject_boxes.clone())
    }
}

/// Load a SightLoom VisionIndex package directory (`CURRENT` + `gen-*` or legacy).
///
/// # Errors
///
/// Package missing / corrupt.
pub fn load_package(dir: impl AsRef<Path>) -> Result<LoadedVisionPackage, SightLoomAdapterError> {
    let root = dir.as_ref();
    let index = VisionIndexPackage::load(root)?;
    let generation =
        VisionIndexPackage::current_generation(root).unwrap_or_else(|| "legacy".to_string());
    let payload = VisionIndexPackage::active_payload_dir(root);
    let content_hash = content_hash_for_payload(&payload, &generation)?;
    let mut source_hash = source_hash_from_index(&index);
    if source_hash.trim().is_empty() {
        // Freeze still needs a pin; fall back to media name digest.
        source_hash = format!("media:{}", fnv1a64(index.header.name.as_bytes()));
    }
    let snapshot = snapshot_from_index(&index, &generation, &content_hash, &source_hash);
    let subject_boxes = subject_boxes_from_index(&index);
    Ok(LoadedVisionPackage {
        index,
        package_root: root.to_path_buf(),
        generation,
        content_hash,
        source_hash,
        snapshot,
        subject_boxes,
    })
}

/// Convert an already-loaded index (e.g. from IndexSession) without disk package.
#[must_use]
pub fn provider_from_index(
    index: &VisionIndex,
    generation: impl Into<String>,
    content_hash: impl Into<String>,
    source_hash: impl Into<String>,
) -> SightLoomProvider {
    let generation = generation.into();
    let content_hash = content_hash.into();
    let source_hash = source_hash.into();
    let snapshot = snapshot_from_index(index, &generation, &content_hash, &source_hash);
    let boxes = subject_boxes_from_index(index);
    SightLoomProvider::new(snapshot).with_subject_boxes(boxes)
}

/// Convenience: load package and return provider only.
///
/// # Errors
///
/// Package load failures.
pub fn provider_from_package(
    dir: impl AsRef<Path>,
) -> Result<SightLoomProvider, SightLoomAdapterError> {
    Ok(load_package(dir)?.provider())
}

fn source_hash_from_index(index: &VisionIndex) -> String {
    // Prefer first source entry hash if present.
    for s in &index.header.sources {
        if let Some(h) = &s.hash {
            // SourceHash display — Debug/string
            return format!("{h:?}");
        }
    }
    if let Some(p) = &index.header.provenance {
        return format!("provenance:{p:?}");
    }
    String::new()
}

fn content_hash_for_payload(
    payload: &Path,
    generation: &str,
) -> Result<String, SightLoomAdapterError> {
    let checksums = payload.join("checksums.json");
    if checksums.is_file() {
        let text = fs::read_to_string(&checksums)
            .map_err(|e| SightLoomAdapterError::Package(e.to_string()))?;
        // Prefer fnv aggregate or whole-file hash of checksums document.
        return Ok(format!("checksums:{}", fnv1a64(text.as_bytes())));
    }
    // Fallback: stable digest of generation name + entity file sizes.
    let mut acc = generation.as_bytes().to_vec();
    for name in [
        "entities.json",
        "tracks.cbor",
        "manifest.json",
        "gallery.json",
    ] {
        let p = payload.join(name);
        if let Ok(meta) = fs::metadata(&p) {
            acc.extend_from_slice(name.as_bytes());
            acc.extend_from_slice(&meta.len().to_le_bytes());
        }
    }
    Ok(format!("digest:{:016x}", fnv1a64(&acc)))
}

fn fnv1a64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for b in data {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}
