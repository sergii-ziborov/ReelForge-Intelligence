//! Catalog of Intelligence operations (intent, not `ReelForge` render ops).

use crate::edit::SemanticEdit;
use serde::{Deserialize, Serialize};

/// One semantic operation the contract exposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelOperation {
    /// Stable id (`blur_subject`, …).
    pub id: &'static str,
    /// Human summary.
    pub summary: &'static str,
}

/// Built-in semantic edits (v1 small set).
#[must_use]
pub fn operations() -> &'static [IntelOperation] {
    &[
        IntelOperation {
            id: "blur_subject",
            summary: "Blur one selected subject for the whole media",
        },
        IntelOperation {
            id: "blur_everyone_except",
            summary: "Blur everyone except an allowed subject or set",
        },
        IntelOperation {
            id: "follow_subject",
            summary: "Keep framing on a subject (follow / smart crop)",
        },
        IntelOperation {
            id: "build_subject_reel",
            summary: "Assemble clips of a subject's appearances",
        },
        IntelOperation {
            id: "build_most_frequent_subject_reel",
            summary: "Find the most frequent subject and build their reel",
        },
        IntelOperation {
            id: "build_anomaly_reel",
            summary: "Build a reel from anomaly ranges (e.g. after 22:00)",
        },
        IntelOperation {
            id: "create_event_clips",
            summary: "Cut clips around events matching a query",
        },
    ]
}

/// JSON Schema-ish maps keyed by operation id (for MCP `schemas()`).
#[must_use]
pub fn schemas() -> serde_json::Value {
    serde_json::json!({
        "blur_subject": {
            "type": "object",
            "required": ["subject"],
            "properties": { "subject": { "type": "object" } }
        },
        "blur_everyone_except": {
            "type": "object",
            "required": ["allowed"],
            "properties": {
                "allowed": { "type": "object" },
                "uncertain_identity": { "type": "string", "enum": ["blur", "allow", "review"] }
            }
        },
        "follow_subject": {
            "type": "object",
            "required": ["subject"],
            "properties": {
                "subject": { "type": "object" },
                "framing": { "type": "string", "enum": ["tight", "medium", "wide"] }
            }
        },
        "build_subject_reel": {
            "type": "object",
            "required": ["subject"],
            "properties": {
                "subject": { "type": "object" },
                "pre_roll": { "type": "object" },
                "post_roll": { "type": "object" }
            }
        },
        "build_most_frequent_subject_reel": {
            "type": "object",
            "properties": {
                "metric": { "type": "string", "enum": ["appearance_count", "source_count", "duration"] }
            }
        },
        "build_anomaly_reel": {
            "type": "object",
            "required": ["query"],
            "properties": { "query": { "type": "object" } }
        },
        "create_event_clips": {
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "object" },
                "pad_before_secs": { "type": "number" },
                "pad_after_secs": { "type": "number" }
            }
        }
    })
}

/// Discriminator string for an edit.
#[must_use]
pub fn edit_op_id(edit: &SemanticEdit) -> &'static str {
    match edit {
        SemanticEdit::BlurSubject { .. } => "blur_subject",
        SemanticEdit::BlurEveryoneExcept { .. } => "blur_everyone_except",
        SemanticEdit::FollowSubject { .. } => "follow_subject",
        SemanticEdit::BuildSubjectReel { .. } => "build_subject_reel",
        SemanticEdit::BuildMostFrequentSubjectReel { .. } => "build_most_frequent_subject_reel",
        SemanticEdit::BuildAnomalyReel { .. } => "build_anomaly_reel",
        SemanticEdit::CreateEventClips { .. } => "create_event_clips",
    }
}
