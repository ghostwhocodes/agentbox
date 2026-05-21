//! Inspection read models and read-only policy helpers.
//!
//! Ownership:
//! - repo and mount status view types
//! - projection logic that derives status from already-fetched runtime inputs
//! - helper rules for interpreting observed mount state
//!
//! Dependency guardrail:
//! read models should stay focused on inspection semantics. Runtime data
//! gathering belongs in `inspection::application` or lower adapters.

mod repo_lifecycle;
mod status_view;

pub use repo_lifecycle::{RepoLifecyclePreflight, UnverifiedActiveMount};
pub use status_view::{
    AggregateMountState, MountStatus, MountTableState, ObservedMount, RegisteredMountStatus,
    RepoStatus,
};

pub(crate) use status_view::aggregate_mount_state;
