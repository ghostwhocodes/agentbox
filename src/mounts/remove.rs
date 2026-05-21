use crate::{
    error::Result,
    shared::{
        mount_spec::MountSpec,
        types::{RelativePath, RepoId},
    },
    workspace::Workspace,
};

use super::support::{
    ensure_mount_target_can_change, map_mount_repo_error, mounts_with_removed_mount,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedMount {
    pub repo_id: RepoId,
    pub mount: MountSpec,
}

pub fn remove_mount(
    workspace: &Workspace,
    repo_id: RepoId,
    repo_path: RelativePath,
) -> Result<RemovedMount> {
    remove_mount_with_validator(workspace, repo_id.clone(), repo_path, |mount| {
        ensure_mount_target_can_change(workspace, &repo_id, mount)
    })
}

fn remove_mount_with_validator<F>(
    workspace: &Workspace,
    repo_id: RepoId,
    repo_path: RelativePath,
    validate_target: F,
) -> Result<RemovedMount>
where
    F: FnOnce(&MountSpec) -> Result<()>,
{
    let mounts = crate::mounts::repo_mounts(workspace, &repo_id).map_err(map_mount_repo_error)?;
    let (next_mounts, removed_mount) = mounts_with_removed_mount(&repo_id, &mounts, &repo_path)?;

    validate_target(&removed_mount)?;
    crate::mounts::replace_repo_mounts(workspace, &repo_id, next_mounts)
        .map_err(map_mount_repo_error)?;

    Ok(RemovedMount {
        repo_id,
        mount: removed_mount,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::shared::error::{Error, MountWorkflowError};

    use super::super::test_support::{init_repo_with_mount, rel, test_workspace};

    #[test]
    fn remove_mount_updates_manifest() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let removed =
            remove_mount_with_validator(&workspace, repo_id.clone(), rel("target"), |_| Ok(()))
                .expect("remove mount");

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert_eq!(removed.repo_id, repo_id);
        assert_eq!(removed.mount.repo, rel("target"));
        assert!(mounts.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn remove_mount_rejects_missing_repo_path() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let error =
            remove_mount_with_validator(&workspace, repo_id.clone(), rel("missing"), |_| Ok(()))
                .expect_err("missing mount should fail");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountNotFoundByRepoPath {
                repo_id: ref missing_repo_id,
                repo_path: ref missing_repo_path,
            }) if missing_repo_id == &repo_id && missing_repo_path == &rel("missing")
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn remove_mount_leaves_manifest_unchanged_when_target_cannot_change() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let error = remove_mount_with_validator(&workspace, repo_id.clone(), rel("target"), |_| {
            Err(MountWorkflowError::MountTargetStillMounted {
                repo_id: repo_id.clone(),
                repo_path: rel("target"),
                target: workspace.repo_path(&repo_id, &rel("target")),
            }
            .into())
        })
        .expect_err("active mount should block removal");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetStillMounted { .. })
        ));
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert_eq!(mounts.len(), 1);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
