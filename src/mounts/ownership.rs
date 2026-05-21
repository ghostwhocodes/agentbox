use std::fs;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{
    error::Result,
    shared::{
        fs_ops::atomic_write_text,
        mount_spec::MountSpec,
        types::{RelativePath, RepoId},
    },
    workspace::Workspace,
};

const MOUNT_OWNERSHIP_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MountOwnershipState {
    records: Vec<OwnedMountRecord>,
}

impl MountOwnershipState {
    pub(crate) fn owned_mount(
        &self,
        repo_id: &RepoId,
        mount_spec: &MountSpec,
    ) -> Option<&OwnedMountRecord> {
        self.records.iter().find(|record| {
            &record.repo_id == repo_id
                && record.context == mount_spec.context
                && record.repo == mount_spec.repo
        })
    }

    pub(crate) fn upsert(&mut self, record: OwnedMountRecord) {
        self.remove_mount(&record.repo_id, &record.mount_spec());
        self.records.push(record);
        self.records.sort_by(|left, right| {
            (
                left.repo_id.as_str(),
                left.repo.as_path().as_str(),
                left.context.as_path().as_str(),
            )
                .cmp(&(
                    right.repo_id.as_str(),
                    right.repo.as_path().as_str(),
                    right.context.as_path().as_str(),
                ))
        });
    }

    pub(crate) fn remove_mount(
        &mut self,
        repo_id: &RepoId,
        mount_spec: &MountSpec,
    ) -> Option<OwnedMountRecord> {
        let index = self.records.iter().position(|record| {
            &record.repo_id == repo_id
                && record.context == mount_spec.context
                && record.repo == mount_spec.repo
        })?;
        Some(self.records.remove(index))
    }

    pub(crate) fn remove_repo(&mut self, repo_id: &RepoId) {
        self.records.retain(|record| &record.repo_id != repo_id);
    }

    pub(crate) fn verifies_live_mount(
        &self,
        repo_id: &RepoId,
        mount_spec: &MountSpec,
        mount_id: u64,
        source_root: &Utf8Path,
        filesystem_type: &str,
    ) -> bool {
        self.owned_mount(repo_id, mount_spec)
            .is_some_and(|record| record.matches_live_mount(mount_id, source_root, filesystem_type))
    }

    #[cfg(test)]
    pub(crate) fn records(&self) -> &[OwnedMountRecord] {
        &self.records
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMountRecord {
    pub(crate) repo_id: RepoId,
    pub(crate) context: RelativePath,
    pub(crate) repo: RelativePath,
    pub(crate) mount_id: u64,
    pub(crate) source_root: camino::Utf8PathBuf,
    pub(crate) filesystem_type: String,
}

impl OwnedMountRecord {
    pub(crate) fn mount_spec(&self) -> MountSpec {
        MountSpec {
            context: self.context.clone(),
            repo: self.repo.clone(),
        }
    }

    pub(crate) fn matches_live_mount(
        &self,
        mount_id: u64,
        source_root: &Utf8Path,
        filesystem_type: &str,
    ) -> bool {
        self.mount_id == mount_id
            && self.source_root == source_root
            && self.filesystem_type == filesystem_type
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedMountOwnershipState {
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mounts: Vec<PersistedOwnedMountRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedOwnedMountRecord {
    repo_id: RepoId,
    context: RelativePath,
    repo: RelativePath,
    mount_id: u64,
    source_root: camino::Utf8PathBuf,
    filesystem_type: String,
}

impl Default for PersistedMountOwnershipState {
    fn default() -> Self {
        Self {
            version: MOUNT_OWNERSHIP_STATE_VERSION,
            mounts: Vec::new(),
        }
    }
}

impl TryFrom<PersistedMountOwnershipState> for MountOwnershipState {
    type Error = crate::Error;

    fn try_from(value: PersistedMountOwnershipState) -> Result<Self> {
        Ok(Self {
            records: value.mounts.into_iter().map(Into::into).collect(),
        })
    }
}

impl From<MountOwnershipState> for PersistedMountOwnershipState {
    fn from(value: MountOwnershipState) -> Self {
        Self {
            version: MOUNT_OWNERSHIP_STATE_VERSION,
            mounts: value.records.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<PersistedOwnedMountRecord> for OwnedMountRecord {
    fn from(value: PersistedOwnedMountRecord) -> Self {
        Self {
            repo_id: value.repo_id,
            context: value.context,
            repo: value.repo,
            mount_id: value.mount_id,
            source_root: value.source_root,
            filesystem_type: value.filesystem_type,
        }
    }
}

impl From<OwnedMountRecord> for PersistedOwnedMountRecord {
    fn from(value: OwnedMountRecord) -> Self {
        Self {
            repo_id: value.repo_id,
            context: value.context,
            repo: value.repo,
            mount_id: value.mount_id,
            source_root: value.source_root,
            filesystem_type: value.filesystem_type,
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn load(workspace: &Workspace) -> Result<MountOwnershipState> {
    Ok(load_if_present(workspace)?.unwrap_or_default())
}

fn load_if_present(workspace: &Workspace) -> Result<Option<MountOwnershipState>> {
    let path = workspace.mount_ownership_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path).map_err(|error| crate::Error::io_path(&path, error))?;
    let persisted = toml::from_str::<PersistedMountOwnershipState>(&raw).map_err(|error| {
        crate::Error::io_path(
            &path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to parse mount ownership state: {error}"),
            ),
        )
    })?;
    if persisted.version != MOUNT_OWNERSHIP_STATE_VERSION {
        return Err(crate::Error::io_path(
            &path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "unsupported mount ownership state version {}; expected {}",
                    persisted.version, MOUNT_OWNERSHIP_STATE_VERSION
                ),
            ),
        ));
    }
    persisted.try_into().map(Some)
}

pub(crate) fn load_advisory(workspace: &Workspace) -> MountOwnershipState {
    load(workspace).unwrap_or_default()
}

pub(crate) fn save(workspace: &Workspace, state: &MountOwnershipState) -> Result<()> {
    let path = workspace.mount_ownership_path();
    let persisted: PersistedMountOwnershipState = state.clone().into();
    let contents = toml::to_string_pretty(&persisted)?;
    atomic_write_text(&path, &format!("{contents}\n"))
}

pub(crate) fn record_best_effort(workspace: &Workspace, record: OwnedMountRecord) {
    let mut state = load_advisory(workspace);
    state.upsert(record);
    let _ = save(workspace, &state);
}

pub(crate) fn remove_mount_best_effort(
    workspace: &Workspace,
    repo_id: &RepoId,
    mount_spec: &MountSpec,
) {
    let Ok(mut state) = load(workspace) else {
        return;
    };
    state.remove_mount(repo_id, mount_spec);
    let _ = save(workspace, &state);
}

pub(crate) fn remove_repo_best_effort(workspace: &Workspace, repo_id: &RepoId) {
    let Ok(mut state) = load(workspace) else {
        return;
    };
    state.remove_repo(repo_id);
    let _ = save(workspace, &state);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::test_support;

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-mount-ownership-test")
    }

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn owned_record() -> OwnedMountRecord {
        OwnedMountRecord {
            repo_id: RepoId::new("demo").expect("valid repo id"),
            context: rel("ctx"),
            repo: rel("target"),
            mount_id: 42,
            source_root: camino::Utf8PathBuf::from("/workspace/context/demo/ctx"),
            filesystem_type: "bind".to_string(),
        }
    }

    #[test]
    fn load_returns_empty_state_when_receipt_is_missing() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let state = load(&workspace).expect("load ownership");

        assert!(state.records().is_empty());
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn save_and_load_round_trip() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let mut state = MountOwnershipState::default();
        let record = owned_record();
        state.upsert(record.clone());

        save(&workspace, &state).expect("save ownership");
        let loaded = load(&workspace).expect("load ownership");

        assert_eq!(loaded.records(), &[record]);
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn load_advisory_ignores_corrupt_receipts() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        fs::create_dir_all(workspace.internal_state_dir()).expect("create state dir");
        fs::write(workspace.mount_ownership_path(), "not valid toml").expect("write receipt");

        let state = load_advisory(&workspace);

        assert!(state.records().is_empty());
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn state_upsert_replaces_existing_mount_spec_record() {
        let mut state = MountOwnershipState::default();
        let original = owned_record();
        let replacement = OwnedMountRecord {
            mount_id: 84,
            ..original.clone()
        };

        state.upsert(original);
        state.upsert(replacement.clone());

        assert_eq!(state.records(), &[replacement]);
    }

    #[test]
    fn remove_repo_drops_all_repo_records() {
        let mut state = MountOwnershipState::default();
        state.upsert(owned_record());
        state.upsert(OwnedMountRecord {
            repo_id: RepoId::new("other").expect("valid repo id"),
            context: rel("ctx"),
            repo: rel("target"),
            mount_id: 43,
            source_root: camino::Utf8PathBuf::from("/workspace/context/other/ctx"),
            filesystem_type: "bind".to_string(),
        });

        state.remove_repo(&RepoId::new("demo").expect("valid repo id"));

        assert_eq!(state.records().len(), 1);
        assert_eq!(state.records()[0].repo_id.as_str(), "other");
    }
}
