//! Runtime inspection and diagnostics context.
//!
//! Ownership:
//! - read-only repo status projection
//! - mount-table inspection orchestration
//! - shared inspection APIs consumed by status/show/list-mounts/doctor
//!
//! Dependency guardrail:
//! this context may depend on shared persistence contracts, context-owned
//! adapters, and `workspace`, but the read models it exposes should remain
//! stable for CLI and JSON output consumers.

mod application;
mod doctor;
mod domain;

pub use application::{
    list_mounts, repo_lifecycle_preflight, selected_repo_lifecycle_preflights, show_repo,
    workspace_status,
};
pub use doctor::{DoctorReport, Finding, Severity, run_doctor};
pub use domain::{
    AggregateMountState, MountStatus, MountTableState, ObservedMount, RegisteredMountStatus,
    RepoLifecyclePreflight, RepoStatus, UnverifiedActiveMount,
};

pub(crate) use application::evaluate_repo_mounts_for_mutating_workflow;
pub(crate) use application::inspect_registered_repo;
pub(crate) use application::load_mount_table_for_read_only_inspection;
pub(crate) use application::load_workspace_snapshot;
pub(crate) use application::observed_mount_for_target;
