use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use std::fmt;

use crate::shared::types::{CloneSource, RelativePath, RepoId};

/// Summary of a repo's mount activity across all its mount specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AggregateMountState {
    NoMounts,
    Unmounted,
    Mounted,
    Partial,
    Conflicted,
    /// Mount inspection was needed but the mount table could not be read.
    Unknown,
}

impl fmt::Display for AggregateMountState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::NoMounts => "no-mounts",
            Self::Unmounted => "unmounted",
            Self::Mounted => "mounted",
            Self::Partial => "partial",
            Self::Conflicted => "conflicted",
            Self::Unknown => "unknown",
        };
        f.write_str(label)
    }
}

/// Runtime status of a single mount spec (active, conflicting, etc.).
#[derive(Debug, Clone, Serialize)]
pub struct MountStatus {
    pub source: Utf8PathBuf,
    pub target: Utf8PathBuf,
    pub active: bool,
    pub conflicting_mount: bool,
    /// `true` when the mount table could not be read and mount state is unknown.
    pub inspection_unavailable: bool,
}

/// Complete runtime status snapshot for a registered repository.
#[derive(Debug, Clone, Serialize)]
pub struct RepoStatus {
    pub repo_id: RepoId,
    pub source: CloneSource,
    pub context_root: Utf8PathBuf,
    pub repo_root: Utf8PathBuf,
    pub materialized: bool,
    pub mount_state: AggregateMountState,
    pub mounts: Vec<MountStatus>,
}

/// Flattened runtime status for a single registered mount rule.
#[derive(Debug, Clone, Serialize)]
pub struct RegisteredMountStatus {
    pub repo_id: RepoId,
    pub repo_path: RelativePath,
    pub context_path: RelativePath,
    pub source: Utf8PathBuf,
    pub target: Utf8PathBuf,
    pub materialized: bool,
    pub active: bool,
    pub conflicting_mount: bool,
    pub inspection_unavailable: bool,
}

/// Observed mount-table row normalized for inspection workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedMount {
    pub mount_id: u64,
    /// The source path we prefer to show for this observed mount.
    pub preferred_source: Utf8PathBuf,
    /// Every source path that should be treated as equivalent for matching.
    pub source_aliases: Vec<Utf8PathBuf>,
    pub target: Utf8PathBuf,
    pub filesystem_type: String,
}

impl ObservedMount {
    pub fn matches_source(&self, source: &Utf8Path) -> bool {
        self.preferred_source == source
            || self
                .source_aliases
                .iter()
                .any(|candidate| candidate == source)
    }
}

/// The mount table to use for inspection, or an indication that it could not be loaded.
pub enum MountTableState {
    Available(Vec<ObservedMount>),
    Unavailable(String),
}

pub(crate) fn aggregate_mount_state(
    mount_count: usize,
    active_count: usize,
    conflicting_count: usize,
    inspection_unavailable: bool,
) -> AggregateMountState {
    if inspection_unavailable {
        AggregateMountState::Unknown
    } else if mount_count == 0 {
        AggregateMountState::NoMounts
    } else if conflicting_count > 0 {
        AggregateMountState::Conflicted
    } else if active_count == 0 {
        AggregateMountState::Unmounted
    } else if active_count == mount_count {
        AggregateMountState::Mounted
    } else {
        AggregateMountState::Partial
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_mount_state_reports_no_mounts() {
        assert_eq!(
            aggregate_mount_state(0, 0, 0, false),
            AggregateMountState::NoMounts
        );
    }

    #[test]
    fn aggregate_mount_state_reports_unmounted() {
        assert_eq!(
            aggregate_mount_state(2, 0, 0, false),
            AggregateMountState::Unmounted
        );
    }

    #[test]
    fn aggregate_mount_state_reports_unknown_when_inspection_unavailable() {
        assert_eq!(
            aggregate_mount_state(1, 0, 0, true),
            AggregateMountState::Unknown
        );
    }

    #[test]
    fn aggregate_mount_state_reports_mounted_partial_and_conflicted() {
        assert_eq!(
            aggregate_mount_state(2, 2, 0, false),
            AggregateMountState::Mounted
        );
        assert_eq!(
            aggregate_mount_state(2, 1, 0, false),
            AggregateMountState::Partial
        );
        assert_eq!(
            aggregate_mount_state(2, 1, 1, false),
            AggregateMountState::Conflicted
        );
    }

    #[test]
    fn aggregate_mount_state_display_covers_all_labels() {
        assert_eq!(AggregateMountState::NoMounts.to_string(), "no-mounts");
        assert_eq!(AggregateMountState::Unmounted.to_string(), "unmounted");
        assert_eq!(AggregateMountState::Mounted.to_string(), "mounted");
        assert_eq!(AggregateMountState::Partial.to_string(), "partial");
        assert_eq!(AggregateMountState::Conflicted.to_string(), "conflicted");
        assert_eq!(AggregateMountState::Unknown.to_string(), "unknown");
    }

    #[test]
    fn observed_mount_matches_preferred_source_and_rejects_unlisted_aliases() {
        let mount = ObservedMount {
            mount_id: 42,
            preferred_source: Utf8PathBuf::from("/workspace/context/demo/ctx"),
            source_aliases: vec![Utf8PathBuf::from("/workspace/context/demo/ctx")],
            target: Utf8PathBuf::from("/workspace/repos/demo/ctx"),
            filesystem_type: "bind".to_string(),
        };

        assert!(mount.matches_source(Utf8Path::new("/workspace/context/demo/ctx")));
        assert!(!mount.matches_source(Utf8Path::new("/mnt/root/workspace/context/demo/ctx")));
        assert!(!mount.matches_source(Utf8Path::new("/workspace/context/demo/other")));
    }
}
