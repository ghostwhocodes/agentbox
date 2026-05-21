use camino::Utf8PathBuf;

use crate::shared::types::RepoId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedActiveMount {
    pub existing_source: Utf8PathBuf,
    pub target: Utf8PathBuf,
}

/// Narrow lifecycle contract for repo workflows that need detach/dematerialize
/// decisions without depending on the full inspection status view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLifecyclePreflight {
    pub repo_id: RepoId,
    pub materialized: bool,
    pub has_active_mounts: bool,
    pub has_conflicting_mounts: bool,
    pub active_mount_targets: Vec<Utf8PathBuf>,
    pub unverified_active_mounts: Vec<UnverifiedActiveMount>,
}
