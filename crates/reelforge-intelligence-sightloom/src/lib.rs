//! SightLoom package adapter for ReelForge Intelligence.
//!
//! ```text
//! VisionIndex package  →  AnalysisSnapshot  →  SightLoomProvider
//! ```
//!
//! Does not re-implement understanding: only projects SightLoom evidence into
//! Intelligence freeze/resolve types.

#![allow(
    clippy::doc_markdown,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

mod convert;
mod error;
mod package;

pub use convert::{snapshot_from_index, subject_boxes_from_index};
pub use error::SightLoomAdapterError;
pub use package::{LoadedVisionPackage, load_package, provider_from_index, provider_from_package};

/// Re-export core types used with this adapter.
pub use reelforge_intelligence_core::{
    AnalysisProvider, AnalysisSnapshot, IntelligencePolicy, SemanticEditPlan, SightLoomProvider,
    resolve_plan,
};
