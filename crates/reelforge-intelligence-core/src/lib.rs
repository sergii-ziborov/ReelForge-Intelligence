//! Semantic Intelligence for the **ReelForge** portfolio.
//!
//! # Product split
//!
//! | Product | Owns |
//! |---------|------|
//! | **SightLoom** | VisionIndex, subjects, tracks, masks, anomalies, reels **handles** |
//! | **ReelForge Intelligence** (this crate) | Semantic intent, freeze, compile to RenderGraph IR |
//! | **ReelForge** | MaskTimeline, RegionRedaction, RenderGraph execution, encode |
//! | **Capture** | CaptureProject, ingest |
//!
//! # Critical pipeline
//!
//! ```text
//! Human / Agent request
//!   → SemanticEditPlan          (intent)
//!   → AnalysisProvider          (SightLoomProvider first)
//!   → ResolvedEditPlan          (frozen, namespaced IDs)
//!   → compile_resolved          (final RenderGraph JSON stub)
//!   → host preview / approve
//!   → ReelForge execute
//! ```
//!
//! Compiling raw intent without freeze is **preview-only** (`final_graph: false`).

#![allow(
    clippy::doc_markdown,
    clippy::cast_precision_loss,
    clippy::large_enum_variant,
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    clippy::collapsible_if,
    clippy::needless_lifetimes
)]

mod catalog;
mod compile;
mod edit;
mod error;
mod ids;
mod mask;
mod mcp;
mod ops;
mod policy;
mod provider;
mod query;
mod resolve;
mod resolved;
mod selector;
mod service;
mod sightloom_provider;
mod time;

pub use catalog::{HostCatalog, MediaInspection, SceneHit, SubjectHit};
pub use compile::AnalysisProvider as AnalysisProviderDescriptor;
pub use compile::{CompileReport, CompileWarning, compile_resolved};
pub use edit::{
    AnomalyQuery, FramingPolicy, FrequencyMetric, SEMANTIC_EDIT_PLAN_VERSION, SemanticEdit,
    SemanticEditPlan,
};
pub use error::{IntelError, Result};
pub use ids::{AnalysisNamespace, EntityKind, NamespacedId};
pub use mask::{MaskArtifact, MaskFidelity, MaskGeometry, MaskRequest, RegionSample};
pub use mcp::{dispatch, service_with_catalog};
pub use ops::{IntelOperation, edit_op_id, operations, schemas};
pub use policy::{
    GapAction, IntelligencePolicy, LowConfidenceAction, MissingMaskAction, PrivacyPolicy,
    UncertainAction, UncertaintyPolicy,
};
pub use provider::{
    AnalysisGeneration, AnalysisProvider, AnalysisProviderInfo, EventResult, SubjectQuery,
    SubjectResult,
};
pub use query::EventQuery;
pub use resolve::{AnalysisSnapshot, AnomalyEvidence, SubjectEvidence, resolve_plan};
pub use resolved::{
    RESOLVED_EDIT_PLAN_VERSION, ResolutionDecision, ResolutionWarning, ResolvedEditPlan,
    ResolvedEvent, ResolvedMaskAsset, ResolvedSubject, whole_media_range,
};
pub use selector::SubjectSelector;
pub use service::{HostRequest, IntelligenceService};
pub use sightloom_provider::SightLoomProvider;
pub use time::{MediaRange, MediaTime};
