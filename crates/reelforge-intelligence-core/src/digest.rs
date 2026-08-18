//! Canonical SHA-256 fingerprints for freeze / graph / approval binding.
//!
//! v1 binds SHA-256 fingerprints. When `RF_INTEL_APPROVAL_HMAC` is set, the
//! bound fingerprints are also HMAC-SHA256 signed.

use crate::error::{IntelError, Result};
use crate::render_graph::RenderGraphIr;
use crate::resolved::ResolvedEditPlan;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Env var holding the approval HMAC key (raw UTF-8 or hex).
pub const APPROVAL_HMAC_ENV: &str = "RF_INTEL_APPROVAL_HMAC";

type HmacSha256 = Hmac<Sha256>;

/// Hex SHA-256 of canonical JSON.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Sort object keys recursively so fingerprints are stable.
#[must_use]
pub fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                if let Some(v) = map.get(&k) {
                    out.insert(k, canonical_json(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

/// SHA-256 of canonical JSON text.
///
/// # Errors
///
/// Serde serialization failure.
pub fn fingerprint_value(value: &Value) -> Result<String> {
    let canon = canonical_json(value);
    let bytes = serde_json::to_vec(&canon).map_err(|e| IntelError::message(e.to_string()))?;
    Ok(sha256_hex(&bytes))
}

/// Fingerprint a resolved plan.
///
/// # Errors
///
/// Serde.
pub fn fingerprint_resolved(resolved: &ResolvedEditPlan) -> Result<String> {
    let value = serde_json::to_value(resolved).map_err(|e| IntelError::message(e.to_string()))?;
    fingerprint_value(&value)
}

/// Fingerprint typed IR.
///
/// # Errors
///
/// Serde.
pub fn fingerprint_ir(ir: &RenderGraphIr) -> Result<String> {
    let value = serde_json::to_value(ir).map_err(|e| IntelError::message(e.to_string()))?;
    fingerprint_value(&value)
}

/// Fingerprint a live graph JSON string (pretty or compact).
///
/// # Errors
///
/// JSON parse.
pub fn fingerprint_graph_json(json: &str) -> Result<String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| IntelError::message(e.to_string()))?;
    fingerprint_value(&value)
}

/// Combined freeze digest (not a signature).
///
/// # Errors
///
/// Serde / hashing.
pub fn freeze_digest(
    source_hash: &str,
    generation: &str,
    vision_index_hash: &str,
    resolved: &ResolvedEditPlan,
    graph_json: &str,
    approval_material: &str,
) -> Result<String> {
    let mut acc = String::new();
    acc.push_str(source_hash);
    acc.push('\n');
    acc.push_str(generation);
    acc.push('\n');
    acc.push_str(vision_index_hash);
    acc.push('\n');
    acc.push_str(&fingerprint_resolved(resolved)?);
    acc.push('\n');
    acc.push_str(&fingerprint_graph_json(graph_json)?);
    acc.push('\n');
    acc.push_str(approval_material);
    Ok(sha256_hex(acc.as_bytes()))
}

/// Canonical string signed / verified for an approval token.
#[must_use]
pub fn approval_material(
    graph_fingerprint: &str,
    ir_fingerprint: &str,
    resolved_fingerprint: &str,
    policy_hash: &str,
    output_uri_hash: &str,
) -> String {
    format!(
        "{graph_fingerprint}\n{ir_fingerprint}\n{resolved_fingerprint}\n{policy_hash}\n{output_uri_hash}"
    )
}

/// HMAC-SHA256 hex digest.
#[must_use]
pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key)
        .or_else(|_| HmacSha256::new_from_slice(&[0]))
        .expect("hmac accepts a non-empty key");
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

/// Sign approval material when [`APPROVAL_HMAC_ENV`] is set.
#[must_use]
pub fn maybe_sign_approval(material: &str) -> Option<String> {
    let key = approval_hmac_key()?;
    Some(hmac_sha256_hex(&key, material.as_bytes()))
}

/// Verify an approval HMAC.
#[must_use]
pub fn verify_hmac_hex(key: &[u8], message: &[u8], signature_hex: &str) -> bool {
    let Ok(sig) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return false;
    };
    mac.update(message);
    mac.verify_slice(&sig).is_ok()
}

/// If a key is configured, `signature` must be present and valid.
///
/// # Errors
///
/// Missing or invalid HMAC when the env key is set.
pub fn verify_approval_signature(signature: Option<&str>, material: &str) -> Result<()> {
    let Some(key) = approval_hmac_key() else {
        return Ok(());
    };
    let Some(sig) = signature.filter(|s| !s.is_empty()) else {
        return Err(IntelError::message(
            "approval: RF_INTEL_APPROVAL_HMAC set but token is unsigned",
        ));
    };
    if verify_hmac_hex(&key, material.as_bytes(), sig) {
        Ok(())
    } else {
        Err(IntelError::message("approval: HMAC signature mismatch"))
    }
}

fn approval_hmac_key() -> Option<Vec<u8>> {
    let raw = std::env::var(APPROVAL_HMAC_ENV).ok()?;
    if raw.is_empty() {
        return None;
    }
    if raw.len() >= 16 && raw.len().is_multiple_of(2) && raw.bytes().all(|b| b.is_ascii_hexdigit())
    {
        hex::decode(&raw).ok()
    } else {
        Some(raw.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_fingerprint() {
        let a = json!({ "b": 1, "a": 2 });
        let b = json!({ "a": 2, "b": 1 });
        assert_eq!(
            fingerprint_value(&a).unwrap(),
            fingerprint_value(&b).unwrap()
        );
    }

    #[test]
    fn hmac_roundtrip_and_tamper() {
        let key = b"test-approval-key";
        let msg = b"graph\nir\nplan\npolicy\nout";
        let sig = hmac_sha256_hex(key, msg);
        assert!(verify_hmac_hex(key, msg, &sig));
        assert!(!verify_hmac_hex(key, b"tampered", &sig));
        assert!(!verify_hmac_hex(b"other-key", msg, &sig));
    }
}
