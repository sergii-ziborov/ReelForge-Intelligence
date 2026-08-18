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
    let mut snapshot = snapshot_from_index(&index, &generation, &content_hash, &source_hash);
    if let Some(meta) = mask_package_sidecar(root, &payload) {
        apply_mask_package_meta(&mut snapshot, &meta);
    }
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

/// Export a ReelForge MaskPackage and pin its id onto the loaded snapshot.
///
/// Also writes `{vision_index}/mask-package.json` so the next `load_package` finds it.
///
/// # Errors
///
/// I/O while writing the package.
pub fn export_and_pin_mask_package(
    loaded: &mut LoadedVisionPackage,
    dest: impl AsRef<Path>,
) -> Result<crate::MaskPackageExport, SightLoomAdapterError> {
    let package_id = loaded
        .snapshot
        .mask_package_id
        .clone()
        .unwrap_or_else(|| loaded.generation.clone());
    let exported = crate::write_mask_package(
        &loaded.index,
        dest.as_ref(),
        &package_id,
        Some(loaded.source_hash.as_str()),
        loaded.snapshot.frame_width,
        loaded.snapshot.frame_height,
    )?;
    let frame_width = loaded.snapshot.frame_width;
    let frame_height = loaded.snapshot.frame_height;
    apply_mask_package_meta(
        &mut loaded.snapshot,
        &MaskPackageMeta {
            package_id: exported.package_id.clone(),
            source_width: frame_width,
            source_height: frame_height,
        },
    );
    loaded.snapshot.mask_package_uri = Some(exported.root.to_string_lossy().into_owned());
    let pointer = serde_json::json!({
        "package_id": exported.package_id,
        "source_width": loaded.snapshot.frame_width,
        "source_height": loaded.snapshot.frame_height
    });
    let _ = fs::write(
        loaded.package_root.join("mask-package.json"),
        pointer.to_string(),
    );
    Ok(exported)
}

struct MaskPackageMeta {
    package_id: String,
    source_width: Option<u32>,
    source_height: Option<u32>,
}

fn mask_package_sidecar(root: &Path, payload: &Path) -> Option<MaskPackageMeta> {
    // Do not read SightLoom's own `manifest.json` — look for an explicit
    // ReelForge MaskPackage pointer or subdirectory.
    let candidates = [
        payload.join("mask-package.json"),
        root.join("mask-package.json"),
        payload.join("mask_package").join("manifest.json"),
        root.join("mask_package").join("manifest.json"),
    ];
    for path in candidates {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(package_id) = value
            .get("package_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        return Some(MaskPackageMeta {
            package_id: package_id.to_string(),
            source_width: value
                .get("source_width")
                .and_then(serde_json::Value::as_u64)
                .and_then(|w| u32::try_from(w).ok()),
            source_height: value
                .get("source_height")
                .and_then(serde_json::Value::as_u64)
                .and_then(|h| u32::try_from(h).ok()),
        });
    }
    None
}

fn apply_mask_package_meta(snapshot: &mut AnalysisSnapshot, meta: &MaskPackageMeta) {
    snapshot.mask_package_id = Some(meta.package_id.clone());
    if let Some(w) = meta.source_width {
        snapshot.frame_width = Some(w);
    }
    if let Some(h) = meta.source_height {
        snapshot.frame_height = Some(h);
    }
    for sample in &mut snapshot.mask_samples {
        if let Some(reelforge_intelligence_core::MaskGeometry::External { package_id, .. }) =
            &mut sample.geometry
        {
            package_id.clone_from(&meta.package_id);
        }
    }
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
        let bytes =
            fs::read(&checksums).map_err(|e| SightLoomAdapterError::Package(e.to_string()))?;
        return Ok(format!(
            "checksums:{}",
            reelforge_intelligence_core::sha256_hex(&bytes)
        ));
    }
    let mut acc = generation.as_bytes().to_vec();
    for name in [
        "entities.json",
        "tracks.cbor",
        "manifest.json",
        "gallery.json",
        "masks.bin",
    ] {
        let p = payload.join(name);
        if let Ok(bytes) = fs::read(&p) {
            acc.extend_from_slice(name.as_bytes());
            acc.extend_from_slice(&bytes);
        }
    }
    Ok(format!(
        "sha256:{}",
        reelforge_intelligence_core::sha256_hex(&acc)
    ))
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
