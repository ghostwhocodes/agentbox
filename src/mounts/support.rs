use crate::{
    error::{Error, InfraMountError, MountWorkflowError, RepoWorkflowError, Result},
    mounts::infra as mount,
    registry::is_repo_materialized,
    shared::{
        fs_ops::{directory_tree_is_matching_subset, tree_is_directory_only_scaffolding},
        mount_spec::{MountSpec, mounts_conflict},
        types::{RelativePath, RepoId},
    },
    workspace::Workspace,
};
use std::fs;

pub(super) fn mount_exists(mounts: &[MountSpec], mount: &MountSpec) -> bool {
    mounts.iter().any(|existing| existing == mount)
}

pub(super) fn mount_conflicts(mounts: &[MountSpec], mount: &MountSpec) -> bool {
    mounts
        .iter()
        .any(|existing| mounts_conflict(existing, mount))
}

pub(super) fn map_mount_repo_error(error: Error) -> Error {
    match error {
        Error::Repo(RepoWorkflowError::RepoNotRegistered { repo_id }) => {
            MountWorkflowError::RepoNotRegistered { repo_id }.into()
        }
        other => other,
    }
}

pub(super) fn find_mount_index_by_repo_path(
    mounts: &[MountSpec],
    repo_path: &RelativePath,
) -> Option<usize> {
    mounts.iter().position(|mount| mount.repo == *repo_path)
}

pub(super) fn mounts_with_added_mount(
    repo_id: &RepoId,
    mounts: &[MountSpec],
    mount: MountSpec,
) -> Result<Vec<MountSpec>> {
    if mount_exists(mounts, &mount) {
        return Err(MountWorkflowError::MountAlreadyExists {
            repo_id: repo_id.clone(),
            context: mount.context.clone(),
            repo_path: mount.repo.clone(),
        }
        .into());
    }
    if mount_conflicts(mounts, &mount) {
        return Err(MountWorkflowError::MountConflicts {
            repo_id: repo_id.clone(),
            context: mount.context.clone(),
            repo_path: mount.repo.clone(),
        }
        .into());
    }

    let mut next_mounts = mounts.to_vec();
    next_mounts.push(mount);
    Ok(next_mounts)
}

pub(super) fn mounts_with_imported_mount(
    repo_id: &RepoId,
    mounts: &[MountSpec],
    mount: MountSpec,
) -> Result<Vec<MountSpec>> {
    if mount_exists(mounts, &mount) || mount_conflicts(mounts, &mount) {
        return Err(MountWorkflowError::MountExistsOrConflicts {
            repo_id: repo_id.clone(),
            context: mount.context.clone(),
            repo_path: mount.repo.clone(),
        }
        .into());
    }

    let mut next_mounts = mounts.to_vec();
    next_mounts.push(mount);
    Ok(next_mounts)
}

pub(super) fn mounts_with_edited_mount(
    repo_id: &RepoId,
    mounts: &[MountSpec],
    repo_path: &RelativePath,
    new_context_path: RelativePath,
) -> Result<(Vec<MountSpec>, MountSpec, MountSpec)> {
    let mount_index = find_mount_index_by_repo_path(mounts, repo_path).ok_or_else(|| {
        MountWorkflowError::MountNotFoundByRepoPath {
            repo_id: repo_id.clone(),
            repo_path: repo_path.clone(),
        }
    })?;
    let previous_mount = mounts[mount_index].clone();
    let updated_mount = MountSpec {
        context: new_context_path,
        repo: previous_mount.repo.clone(),
    };

    let conflicts = mounts
        .iter()
        .enumerate()
        .any(|(index, existing)| index != mount_index && mounts_conflict(existing, &updated_mount));
    if conflicts {
        return Err(MountWorkflowError::MountConflicts {
            repo_id: repo_id.clone(),
            context: updated_mount.context.clone(),
            repo_path: updated_mount.repo.clone(),
        }
        .into());
    }

    let mut next_mounts = mounts.to_vec();
    next_mounts[mount_index] = updated_mount.clone();
    Ok((next_mounts, previous_mount, updated_mount))
}

pub(super) fn mounts_with_removed_mount(
    repo_id: &RepoId,
    mounts: &[MountSpec],
    repo_path: &RelativePath,
) -> Result<(Vec<MountSpec>, MountSpec)> {
    let mount_index = find_mount_index_by_repo_path(mounts, repo_path).ok_or_else(|| {
        MountWorkflowError::MountNotFoundByRepoPath {
            repo_id: repo_id.clone(),
            repo_path: repo_path.clone(),
        }
    })?;
    let removed_mount = mounts[mount_index].clone();

    let mut next_mounts = mounts.to_vec();
    next_mounts.remove(mount_index);
    Ok((next_mounts, removed_mount))
}

pub(super) fn ensure_mount_target_can_change(
    workspace: &Workspace,
    repo_id: &RepoId,
    mount_spec: &MountSpec,
) -> Result<()> {
    ensure_mount_target_can_change_with_loader(workspace, repo_id, mount_spec, mount::list_mounts)
}

pub(super) fn prepare_mount_paths(
    source: &camino::Utf8Path,
    target: &camino::Utf8Path,
) -> Result<()> {
    fs::create_dir_all(source).map_err(|e| Error::io_path(source, e))?;
    fs::create_dir_all(target).map_err(|e| Error::io_path(target, e))?;
    Ok(())
}

pub(super) fn ensure_mount_target_matches_source_or_is_empty(
    repo_id: &RepoId,
    mount: &MountSpec,
    source: &camino::Utf8Path,
    target: &camino::Utf8Path,
) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }

    if source.exists() && directory_tree_is_matching_subset(target, source)? {
        return Ok(());
    }

    if !source.exists() && target.is_dir() && tree_is_directory_only_scaffolding(target)? {
        return Ok(());
    }

    if target.is_dir()
        && fs::read_dir(target)
            .map_err(|e| Error::io_path(target, e))?
            .next()
            .is_none()
    {
        return Ok(());
    }

    Err(MountWorkflowError::MountTargetWouldHideDifferentFiles {
        repo_id: repo_id.clone(),
        repo_path: mount.repo.clone(),
        mount_source: source.to_path_buf(),
        target: target.to_path_buf(),
    }
    .into())
}

fn ensure_mount_target_can_change_with_loader<F>(
    workspace: &Workspace,
    repo_id: &RepoId,
    mount_spec: &MountSpec,
    list_mounts: F,
) -> Result<()>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    if !is_repo_materialized(workspace, repo_id) {
        return Ok(());
    }

    let target = workspace.repo_path(repo_id, &mount_spec.repo);
    let expected_source = workspace.context_path(repo_id, &mount_spec.context);
    let mount_table = match list_mounts() {
        Ok(mount_table) => mount_table,
        Err(Error::InfraMount(InfraMountError::Unsupported)) => return Ok(()),
        Err(error) => return Err(error),
    };
    ensure_mount_target_can_change_in(repo_id, mount_spec, target, expected_source, &mount_table)
}

fn ensure_mount_target_can_change_in(
    repo_id: &RepoId,
    mount_spec: &MountSpec,
    target: camino::Utf8PathBuf,
    expected_source: camino::Utf8PathBuf,
    mount_table: &[mount::MountEntry],
) -> Result<()> {
    if let Some(existing) = mount::find_target_mount(mount_table, &target) {
        if existing.matches_source(&expected_source) {
            return Err(MountWorkflowError::MountTargetStillMounted {
                repo_id: repo_id.clone(),
                repo_path: mount_spec.repo.clone(),
                target,
            }
            .into());
        }

        return Err(MountWorkflowError::MountTargetConflictedForUpdate {
            repo_id: repo_id.clone(),
            repo_path: mount_spec.repo.clone(),
            expected_source,
            existing_source: existing.preferred_source.clone(),
            target,
        }
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::{error::Error, mount_spec::MountSpec};

    use super::super::test_support::{
        init_git_repo, materialized_repo_with_mount, rel, test_workspace,
    };

    #[test]
    fn mounts_with_added_mount_rejects_duplicate_mount() {
        let repo_id = RepoId::new("demo").unwrap();
        let mount = MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: RelativePath::new("target", "repo").unwrap(),
        };
        let existing_mount = mount.clone();
        let error = mounts_with_added_mount(&repo_id, std::slice::from_ref(&existing_mount), mount)
            .expect_err("duplicate mount");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountAlreadyExists { .. })
        ));
    }

    #[test]
    fn find_mount_index_matches_repo_path() {
        let mounts = vec![MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: RelativePath::new("target", "repo").unwrap(),
        }];

        assert_eq!(
            find_mount_index_by_repo_path(&mounts, &RelativePath::new("target", "repo").unwrap()),
            Some(0)
        );
    }

    #[test]
    fn mounts_with_removed_mount_returns_removed_mount() {
        let repo_id = RepoId::new("demo").unwrap();
        let repo_path = RelativePath::new("target", "repo").unwrap();
        let mounts = vec![MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: repo_path.clone(),
        }];

        let (next_mounts, removed) =
            mounts_with_removed_mount(&repo_id, &mounts, &repo_path).expect("remove mount");

        assert!(next_mounts.is_empty());
        assert_eq!(removed.repo, repo_path);
    }

    #[test]
    fn ensure_mount_target_can_change_rejects_expected_active_mount() {
        let repo_id = RepoId::new("demo").unwrap();
        let mount_spec = MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: RelativePath::new("target", "repo").unwrap(),
        };
        let target = camino::Utf8PathBuf::from("/workspace/repos/demo/target");
        let expected_source = camino::Utf8PathBuf::from("/workspace/context/demo/ctx");

        let error = ensure_mount_target_can_change_in(
            &repo_id,
            &mount_spec,
            target.clone(),
            expected_source.clone(),
            &[mount::MountEntry {
                mount_id: 31,
                preferred_source: expected_source.clone(),
                source_aliases: vec![expected_source.clone()],
                mount_point: target.clone(),
                filesystem_type: "bind".to_string(),
            }],
        )
        .expect_err("active mount should block mutation");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetStillMounted {
                repo_id: ref active_repo_id,
                target: ref active_target,
                ..
            }) if active_repo_id == &repo_id && active_target == &target
        ));
    }

    #[test]
    fn ensure_mount_target_can_change_rejects_foreign_mount() {
        let repo_id = RepoId::new("demo").unwrap();
        let mount_spec = MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: RelativePath::new("target", "repo").unwrap(),
        };
        let target = camino::Utf8PathBuf::from("/workspace/repos/demo/target");
        let expected_source = camino::Utf8PathBuf::from("/workspace/context/demo/ctx");

        let error = ensure_mount_target_can_change_in(
            &repo_id,
            &mount_spec,
            target.clone(),
            expected_source.clone(),
            &[mount::MountEntry {
                mount_id: 32,
                preferred_source: camino::Utf8PathBuf::from("/foreign/source"),
                source_aliases: vec![camino::Utf8PathBuf::from("/foreign/source")],
                mount_point: target.clone(),
                filesystem_type: "bind".to_string(),
            }],
        )
        .expect_err("foreign mount should block mutation");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetConflictedForUpdate {
                repo_id: ref conflict_repo_id,
                target: ref conflict_target,
                ..
            }) if conflict_repo_id == &repo_id && conflict_target == &target
        ));
    }

    #[test]
    fn ensure_mount_target_can_change_skips_unsupported_mount_table_inspection() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);
        let mount_spec = MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: RelativePath::new("target", "repo").unwrap(),
        };

        ensure_mount_target_can_change_with_loader(&workspace, &repo_id, &mount_spec, || {
            Err(InfraMountError::Unsupported.into())
        })
        .expect("unsupported mount inspection should not block manifest-only mutation");
    }

    #[test]
    fn ensure_mount_target_can_change_fails_closed_when_mount_table_is_malformed() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);
        let mount_spec = MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: RelativePath::new("target", "repo").unwrap(),
        };

        let error =
            ensure_mount_target_can_change_with_loader(&workspace, &repo_id, &mount_spec, || {
                Err(Error::malformed_mount_table_entry(7, "missing separator"))
            })
            .expect_err("malformed mount inspection should block mutation");

        assert!(matches!(
            error,
            Error::InfraMount(InfraMountError::MalformedMountTableEntry { line_no: 7, .. })
        ));
    }

    #[test]
    fn ensure_mount_target_can_change_treats_matching_mount_as_active() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);
        let mount_spec = MountSpec {
            context: RelativePath::new("ctx", "context").unwrap(),
            repo: RelativePath::new("target", "repo").unwrap(),
        };
        let expected_source = workspace.context_path(&repo_id, &mount_spec.context);

        let error =
            ensure_mount_target_can_change_with_loader(&workspace, &repo_id, &mount_spec, || {
                Ok(vec![mount::MountEntry {
                    mount_id: 61,
                    preferred_source: expected_source.clone(),
                    source_aliases: vec![expected_source.clone()],
                    mount_point: workspace.repo_path(&repo_id, &mount_spec.repo),
                    filesystem_type: "bind".to_string(),
                }])
            })
            .expect_err("matching mount should block mutation as already active");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetStillMounted { .. })
        ));
    }

    #[test]
    fn prepare_mount_paths_create_workspace_roots_and_are_scannable() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").unwrap();
        let mount = MountSpec {
            context: rel("ai"),
            repo: rel("tooling/ai"),
        };
        let source = workspace.context_path(&repo_id, &mount.context);
        let target = workspace.repo_path(&repo_id, &mount.repo);

        prepare_mount_paths(&source, &target).unwrap();

        assert!(source.is_dir());
        assert!(target.is_dir());

        let repo_root = workspace.repo_root(&repo_id);
        init_git_repo(&repo_root);

        assert_eq!(
            crate::workspace::scan_context_repo_dirs(&workspace)
                .unwrap()
                .utf8_names,
            vec!["demo"]
        );
        assert_eq!(
            crate::workspace::scan_materialized_repo_dirs(&workspace)
                .unwrap()
                .utf8_names,
            vec!["demo"]
        );
    }
}
