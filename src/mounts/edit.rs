use crate::{
    error::Result,
    shared::{
        mount_spec::MountSpec,
        types::{RelativePath, RepoId},
    },
    workspace::Workspace,
};

use super::support::{
    ensure_mount_target_can_change, map_mount_repo_error, mounts_with_edited_mount,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedMount {
    pub repo_id: RepoId,
    pub mount: MountSpec,
}

pub fn edit_mount(
    workspace: &Workspace,
    repo_id: RepoId,
    repo_path: RelativePath,
    new_context_path: RelativePath,
) -> Result<EditedMount> {
    edit_mount_with_validator(
        workspace,
        repo_id.clone(),
        repo_path,
        new_context_path,
        |mount| ensure_mount_target_can_change(workspace, &repo_id, mount),
    )
}

fn edit_mount_with_validator<F>(
    workspace: &Workspace,
    repo_id: RepoId,
    repo_path: RelativePath,
    new_context_path: RelativePath,
    validate_target: F,
) -> Result<EditedMount>
where
    F: FnOnce(&MountSpec) -> Result<()>,
{
    let mounts = crate::mounts::repo_mounts(workspace, &repo_id).map_err(map_mount_repo_error)?;
    let (next_mounts, previous_mount, updated_mount) =
        mounts_with_edited_mount(&repo_id, &mounts, &repo_path, new_context_path)?;

    validate_target(&previous_mount)?;
    crate::mounts::replace_repo_mounts(workspace, &repo_id, next_mounts)
        .map_err(map_mount_repo_error)?;

    Ok(EditedMount {
        repo_id,
        mount: updated_mount,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        error::{Error, MountWorkflowError},
        shared::types::{CloneSource, RepoId},
    };

    use super::super::test_support::{init_repo_with_mount, rel, test_workspace};
    use super::*;

    #[test]
    fn edit_mount_updates_context_path_in_manifest() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let edited = edit_mount_with_validator(
            &workspace,
            repo_id.clone(),
            rel("target"),
            rel("next"),
            |_| Ok(()),
        )
        .expect("edit mount");

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert_eq!(edited.mount.context, rel("next"));
        assert_eq!(edited.mount.repo, rel("target"));
        assert_eq!(mounts[0].context, rel("next"));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn edit_mount_rejects_missing_repo_path() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let error = edit_mount_with_validator(
            &workspace,
            repo_id.clone(),
            rel("missing"),
            rel("next"),
            |_| Ok(()),
        )
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
    fn edit_mount_rejects_context_conflicts() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            crate::persistence::manifest::PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest.repo_mounts.extend([
            crate::persistence::manifest::PersistedRepoMount {
                repo_id: repo_id.clone(),
                context: rel("ctx"),
                repo: rel("target"),
            },
            crate::persistence::manifest::PersistedRepoMount {
                repo_id: repo_id.clone(),
                context: rel("other"),
                repo: rel("other-target"),
            },
        ]);
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let error = edit_mount_with_validator(
            &workspace,
            repo_id.clone(),
            rel("target"),
            rel("other"),
            |_| Ok(()),
        )
        .expect_err("conflicting context should fail");

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
    fn edit_mount_leaves_manifest_unchanged_when_target_cannot_change() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let error = edit_mount_with_validator(
            &workspace,
            repo_id.clone(),
            rel("target"),
            rel("next"),
            |_| {
                Err(MountWorkflowError::MountTargetStillMounted {
                    repo_id: repo_id.clone(),
                    repo_path: rel("target"),
                    target: workspace.repo_path(&repo_id, &rel("target")),
                }
                .into())
            },
        )
        .expect_err("active mount should block edit");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetStillMounted { .. })
        ));
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert_eq!(mounts[0].context, rel("ctx"));

        let _ = fs::remove_dir_all(workspace.root());
    }
}
