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
//!   → query SightLoom snapshot  (host fills AnalysisSnapshot)
//!   → ResolvedEditPlan          (frozen evidence)
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
mod mcp;
mod ops;
mod policy;
mod query;
mod resolve;
mod resolved;
mod selector;
mod service;
mod time;

pub use catalog::{HostCatalog, MediaInspection, SceneHit, SubjectHit};
pub use compile::{AnalysisProvider, CompileReport, CompileWarning, compile_resolved};
pub use edit::{
    AnomalyQuery, FramingPolicy, FrequencyMetric, SEMANTIC_EDIT_PLAN_VERSION, SemanticEdit,
    SemanticEditPlan,
};
pub use error::{IntelError, Result};
pub use mcp::{dispatch, service_with_catalog};
pub use ops::{IntelOperation, edit_op_id, operations, schemas};
pub use policy::{IntelligencePolicy, PrivacyPolicy, UncertaintyPolicy};
pub use query::EventQuery;
pub use resolve::{AnalysisSnapshot, AnomalyEvidence, SubjectEvidence, resolve_plan};
pub use resolved::{
    RESOLVED_EDIT_PLAN_VERSION, ResolutionDecision, ResolutionWarning, ResolvedEditPlan,
    ResolvedEvent, ResolvedMaskAsset, ResolvedSubject, whole_media_range,
};
pub use selector::SubjectSelector;
pub use service::{HostRequest, IntelligenceService};
pub use time::{MediaRange, MediaTime};
