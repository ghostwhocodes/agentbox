use crate::{
    error::Result,
    shared::{
        mount_spec::MountSpec,
        types::{RelativePath, RepoId},
    },
    workspace::Workspace,
};

use super::support::{map_mount_repo_error, mounts_with_added_mount};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedMount {
    pub repo_id: RepoId,
    pub mount: MountSpec,
}

pub fn add_mount(
    workspace: &Workspace,
    repo_id: RepoId,
    context_path: RelativePath,
    repo_path: RelativePath,
) -> Result<AddedMount> {
    let mount = MountSpec {
        context: context_path,
        repo: repo_path,
    };
    let mounts = crate::mounts::repo_mounts(workspace, &repo_id).map_err(map_mount_repo_error)?;
    let next_mounts = mounts_with_added_mount(&repo_id, &mounts, mount.clone())?;
    crate::mounts::replace_repo_mounts(workspace, &repo_id, next_mounts)
        .map_err(map_mount_repo_error)?;
    Ok(AddedMount { repo_id, mount })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::shared::error::{Error, MountWorkflowError};

    use super::super::test_support::{init_repo, rel, test_workspace};

    #[test]
    fn add_mount_registers_mount_in_manifest() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);

        let added = add_mount(&workspace, repo_id.clone(), rel("ctx"), rel("target"))
            .expect("mount should be added");

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert_eq!(added.repo_id, repo_id);
        assert_eq!(added.mount.context, rel("ctx"));
        assert_eq!(added.mount.repo, rel("target"));
        assert!(
            mounts
                .iter()
                .any(|mount| mount.context == rel("ctx") && mount.repo == rel("target"))
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn add_mount_rejects_conflicting_mount() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        add_mount(&workspace, repo_id.clone(), rel("ctx"), rel("target")).expect("first mount");

        let error = add_mount(&workspace, repo_id.clone(), rel("ctx"), rel("other"))
            .expect_err("conflicting mount should fail");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountConflicts {
                repo_id: ref conflict_repo_id,
                ..
            }) if conflict_repo_id == &repo_id
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn add_mount_rejects_overlapping_repo_path() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        add_mount(&workspace, repo_id.clone(), rel("ctx"), rel("target")).expect("first mount");

        let error = add_mount(
            &workspace,
            repo_id.clone(),
            rel("nested"),
            rel("target/sub"),
        )
        .expect_err("overlapping mount should fail");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountConflicts {
                repo_id: ref conflict_repo_id,
                ..
            }) if conflict_repo_id == &repo_id
        ));

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert_eq!(mounts.len(), 1);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
