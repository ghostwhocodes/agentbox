//! Inspection application services.
//!
//! This layer orchestrates read-only status workflows over `Workspace`,
//! manifest state, and infrastructure probes.
//!
//! Dependency guardrail:
//! these modules may depend on `domain` persistence contracts, `infra`, and
//! `workspace`, but they should expose stable inspection-facing APIs rather
//! than leaking mount-table or filesystem probing details back to callers.

mod lifecycle;
mod load_mount_table;
mod mount_list;
mod repo_status;
mod snapshot;

pub use lifecycle::{repo_lifecycle_preflight, selected_repo_lifecycle_preflights};
pub(crate) use load_mount_table::{
    load_mount_table_for_mutating_workflow, load_mount_table_for_read_only_inspection,
};
pub use mount_list::list_mounts;
pub(crate) use repo_status::{
    evaluate_repo_mounts_for_mutating_workflow, inspect_registered_repo, observed_mount_for_target,
};
pub use repo_status::{show_repo, workspace_status};
pub(crate) use snapshot::load_workspace_snapshot;
