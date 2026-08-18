//! Write a ReelForge `MaskPackage` from a SightLoom `VisionIndex`.

use crate::convert::slm1_to_coverage;
use crate::error::SightLoomAdapterError;
use reelforge_intelligence_core::sha256_hex;
use sha2::{Digest, Sha256};
use sightloom_index::VisionIndex;
use std::fs;
use std::path::Path;

/// Result of exporting silhouettes for the ReelForge host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskPackageExport {
    /// `MaskAsset::External.package_id`.
    pub package_id: String,
    /// Destination directory (`manifest.json` + `masks/`).
    pub root: std::path::PathBuf,
    /// Number of blobs written.
    pub blob_count: usize,
}

/// Write `dest/manifest.json` + `dest/masks/{ref}.bin` from the index mask store.
///
/// # Errors
///
/// I/O while creating the package.
pub fn write_mask_package(
    index: &VisionIndex,
    dest: impl AsRef<Path>,
    package_id: impl Into<String>,
    source_hash: Option<&str>,
    frame_width: Option<u32>,
    frame_height: Option<u32>,
) -> Result<MaskPackageExport, SightLoomAdapterError> {
    let package_id = package_id.into();
    if package_id.trim().is_empty() {
        return Err(SightLoomAdapterError::Convert(
            "mask package: empty package_id".into(),
        ));
    }
    let root = dest.as_ref();
    let masks_dir = root.join("masks");
    fs::create_dir_all(&masks_dir).map_err(|e| SightLoomAdapterError::Package(e.to_string()))?;

    let mut blobs = Vec::new();
    for (handle, bytes) in index.masks.entries() {
        let Some((width, height, data)) = coverage_from_store(index, handle.0, bytes) else {
            continue;
        };
        if width == 0 || height == 0 {
            continue;
        }
        let rel = format!("masks/{}.bin", handle.0);
        let path = root.join(&rel);
        fs::write(&path, &data).map_err(|e| SightLoomAdapterError::Package(e.to_string()))?;
        let content_hash = sha256_hex(&data);
        blobs.push(serde_json::json!({
            "mask_ref": handle.0,
            "kind": "dense",
            "left": 0,
            "top": 0,
            "width": width,
            "height": height,
            "path": rel,
            "content_hash": content_hash,
        }));
    }

    blobs.sort_by_key(|b| {
        b.get("mask_ref")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    let package_hash = hash_package(
        root,
        &package_id,
        source_hash,
        frame_width,
        frame_height,
        &blobs,
    )?;

    let manifest = serde_json::json!({
        "version": 1,
        "package_id": package_id,
        "tracks": [],
        "masks": blobs,
        "package_hash": package_hash,
        "source_width": frame_width,
        "source_height": frame_height,
        "source_hash": source_hash,
    });
    fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .map_err(|e| SightLoomAdapterError::Convert(e.to_string()))?,
    )
    .map_err(|e| SightLoomAdapterError::Package(e.to_string()))?;

    Ok(MaskPackageExport {
        blob_count: blobs.len(),
        package_id,
        root: root.to_path_buf(),
    })
}

fn hash_package(
    root: &Path,
    package_id: &str,
    source_hash: Option<&str>,
    frame_width: Option<u32>,
    frame_height: Option<u32>,
    blobs: &[serde_json::Value],
) -> Result<String, SightLoomAdapterError> {
    let mut hasher = Sha256::new();
    hasher.update(b"reelforge-mask-package-v1\n");
    hasher.update(package_id.as_bytes());
    hasher.update(b"\n");
    write_opt_u32(&mut hasher, frame_width);
    write_opt_u32(&mut hasher, frame_height);
    hasher.update(normalize_hash(source_hash).as_bytes());
    hasher.update(b"\n");
    for blob in blobs {
        let rel = blob
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let bytes =
            fs::read(root.join(rel)).map_err(|e| SightLoomAdapterError::Package(e.to_string()))?;
        hasher.update(
            blob.get("mask_ref")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                .to_le_bytes(),
        );
        hasher.update(b"dense");
        hasher.update(b"\0");
        hasher.update(0_u32.to_le_bytes());
        hasher.update(0_u32.to_le_bytes());
        hasher.update(json_u32(blob, "width").to_le_bytes());
        hasher.update(json_u32(blob, "height").to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(sha256_hex(&bytes).as_bytes());
        hasher.update(b"\n");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn json_u32(value: &serde_json::Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

#[allow(clippy::cast_sign_loss)]
fn coverage_from_store(
    index: &VisionIndex,
    handle: u64,
    bytes: &[u8],
) -> Option<(u32, u32, Vec<u8>)> {
    if let Some(decoded) = slm1_to_coverage(bytes) {
        return Some(decoded);
    }
    let mut width = 0_u32;
    let mut height = 0_u32;
    for sample in index.tracks.effective_samples() {
        if sample.mask_ref != handle {
            continue;
        }
        let w = (sample.right - sample.left).abs().ceil().max(1.0) as u32;
        let h = (sample.bottom - sample.top).abs().ceil().max(1.0) as u32;
        width = width.max(w);
        height = height.max(h);
    }
    let need = (width as usize).saturating_mul(height as usize);
    if width > 0 && height > 0 && bytes.len() == need {
        return Some((width, height, bytes.to_vec()));
    }
    None
}

fn write_opt_u32(hasher: &mut Sha256, value: Option<u32>) {
    match value {
        Some(v) => {
            hasher.update([1_u8]);
            hasher.update(v.to_le_bytes());
        }
        None => hasher.update([0_u8]),
    }
}

fn normalize_hash(value: Option<&str>) -> String {
    value
        .map(|v| {
            v.trim()
                .strip_prefix("sha256:")
                .unwrap_or(v)
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default()
}
