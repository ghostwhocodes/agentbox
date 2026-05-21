use camino::Utf8PathBuf;

use crate::{
    error::{MountWorkflowError, Result},
    mounts::{self, infra as mount, ownership},
    shared::types::RepoId,
    workspace::Workspace,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmountOutcome {
    Unmounted { target: Utf8PathBuf },
    AlreadyUnmounted { target: Utf8PathBuf },
}

pub fn unmount_repos(
    workspace: &Workspace,
    requested: Option<RepoId>,
    unsafe_unmount: bool,
) -> Result<Vec<UnmountOutcome>> {
    unmount_repos_with_loader(
        workspace,
        requested,
        unsafe_unmount,
        mount::list_mounts,
        mount::unmount,
    )
}

fn unmount_repos_with_loader<L, F>(
    workspace: &Workspace,
    requested: Option<RepoId>,
    unsafe_unmount: bool,
    list_mounts: L,
    unmount_target: F,
) -> Result<Vec<UnmountOutcome>>
where
    L: FnOnce() -> Result<Vec<mount::MountEntry>>,
    F: FnMut(&camino::Utf8Path) -> Result<()>,
{
    let selected_mounts = mounts::selected_repo_mounts(workspace, requested)?;
    let mount_table = list_mounts()?;
    let owned_mounts = ownership::load_advisory(workspace);
    unmount_repos_with_mount_table(
        workspace,
        selected_mounts,
        mount_table,
        owned_mounts,
        unsafe_unmount,
        unmount_target,
    )
}

fn unmount_repos_with_mount_table<F>(
    workspace: &Workspace,
    selected_mounts: Vec<(RepoId, Vec<crate::shared::mount_spec::MountSpec>)>,
    mount_table: Vec<mount::MountEntry>,
    owned_mounts: ownership::MountOwnershipState,
    unsafe_unmount: bool,
    mut unmount_target: F,
) -> Result<Vec<UnmountOutcome>>
where
    F: FnMut(&camino::Utf8Path) -> Result<()>,
{
    let mut outcomes = Vec::new();

    for (repo_id, repo_mounts) in selected_mounts {
        for mount_spec in &repo_mounts {
            let source = workspace.context_path(&repo_id, &mount_spec.context);
            let target = workspace.repo_path(&repo_id, &mount_spec.repo);
            if let Some(existing) = mount::find_target_mount(&mount_table, &target) {
                let ownership_verified = owned_mounts.verifies_live_mount(
                    &repo_id,
                    mount_spec,
                    existing.mount_id,
                    &existing.preferred_source,
                    &existing.filesystem_type,
                );
                if !existing.matches_source(&source) && !ownership_verified {
                    return Err(MountWorkflowError::UnmountTargetConflicts {
                        expected_source: source,
                        existing_source: existing.preferred_source.clone(),
                        target,
                    }
                    .into());
                }
                if !ownership_verified && !unsafe_unmount {
                    return Err(MountWorkflowError::UnmountTargetOwnershipUnverified {
                        existing_source: existing.preferred_source.clone(),
                        target,
                    }
                    .into());
                }
                unmount_target(&target)?;
                ownership::remove_mount_best_effort(workspace, &repo_id, mount_spec);
                outcomes.push(UnmountOutcome::Unmounted { target });
            } else {
                ownership::remove_mount_best_effort(workspace, &repo_id, mount_spec);
                outcomes.push(UnmountOutcome::AlreadyUnmounted { target });
            }
        }
    }

    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs};

    use camino::Utf8PathBuf;

    use super::*;
    use crate::{
        mounts,
        shared::{
            error::{Error, MountWorkflowError},
            mount_spec::MountSpec,
        },
    };

    use super::super::test_support::{init_repo_with_mount, rel, test_workspace};

    fn selected_mounts(workspace: &Workspace, repo_id: &RepoId) -> Vec<(RepoId, Vec<MountSpec>)> {
        mounts::selected_repo_mounts(workspace, Some(repo_id.clone())).expect("selected mounts")
    }

    fn live_mount(
        mount_id: u64,
        source: Utf8PathBuf,
        target: Utf8PathBuf,
        filesystem_type: &str,
    ) -> mount::MountEntry {
        mount::MountEntry {
            mount_id,
            preferred_source: source.clone(),
            source_aliases: vec![source],
            mount_point: target,
            filesystem_type: filesystem_type.to_string(),
        }
    }

    fn owned_mount_state(
        repo_id: &RepoId,
        context: &str,
        repo: &str,
        mount_id: u64,
        source_root: Utf8PathBuf,
        filesystem_type: &str,
    ) -> ownership::MountOwnershipState {
        let mut state = ownership::MountOwnershipState::default();
        state.upsert(ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: rel(context),
            repo: rel(repo),
            mount_id,
            source_root,
            filesystem_type: filesystem_type.to_string(),
        });
        state
    }

    #[test]
    fn unmount_fails_when_target_is_mounted_from_foreign_source() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let target = workspace.repo_path(&repo_id, &rel("target"));

        let error = unmount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                21,
                Utf8PathBuf::from("/foreign/source"),
                target.clone(),
                "bind",
            )],
            ownership::MountOwnershipState::default(),
            false,
            |_| unreachable!("foreign mounts should not be unmounted"),
        )
        .expect_err("foreign mount should block unmount");

        assert!(
            matches!(error, Error::Mount(MountWorkflowError::UnmountTargetConflicts {
            expected_source: _,
            existing_source: _,
            target: ref conflicted_target,
        }) if conflicted_target == &target)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_succeeds_when_target_is_mounted_from_exact_expected_source() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let mount_id = 22;
        let unmount_calls = Cell::new(0usize);

        let outcomes = unmount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                mount_id,
                expected_source.clone(),
                target.clone(),
                "bind",
            )],
            owned_mount_state(
                &repo_id,
                "ctx",
                "target",
                mount_id,
                expected_source.clone(),
                "bind",
            ),
            false,
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect("expected mount should unmount cleanly");

        assert_eq!(outcomes, vec![UnmountOutcome::Unmounted { target }]);
        assert_eq!(unmount_calls.get(), 1);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_loader_removes_receipt_after_successful_unmount() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        ownership::save(
            &workspace,
            &owned_mount_state(
                &repo_id,
                "ctx",
                "target",
                222,
                expected_source.clone(),
                "bind",
            ),
        )
        .expect("save ownership");

        let outcomes = unmount_repos_with_loader(
            &workspace,
            Some(repo_id),
            false,
            || {
                Ok(vec![live_mount(
                    222,
                    expected_source,
                    target.clone(),
                    "bind",
                )])
            },
            |_| Ok(()),
        )
        .expect("owned live mount should unmount");

        assert_eq!(outcomes, vec![UnmountOutcome::Unmounted { target }]);
        assert!(
            ownership::load(&workspace)
                .expect("load ownership")
                .records()
                .is_empty()
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_refuses_alias_only_source_match_without_ownership_receipt() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let unmount_calls = Cell::new(0usize);

        let error = unmount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                23,
                Utf8PathBuf::from("/@workspace/context/demo/ctx"),
                target.clone(),
                "btrfs",
            )],
            ownership::MountOwnershipState::default(),
            false,
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("alias-only source should block unmount");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::UnmountTargetConflicts {
                target: ref conflicted_target,
                ref existing_source,
                ..
            }) if conflicted_target == &target
                && existing_source == &Utf8PathBuf::from("/@workspace/context/demo/ctx")
        ));
        assert_eq!(unmount_calls.get(), 0);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_accepts_alias_only_source_match_when_ownership_receipt_verifies_live_mount() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let unmount_calls = Cell::new(0usize);
        let mount_id = 23;
        let alias_source = Utf8PathBuf::from("/@workspace/context/demo/ctx");

        let outcomes = unmount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                mount_id,
                alias_source.clone(),
                target.clone(),
                "btrfs",
            )],
            owned_mount_state(&repo_id, "ctx", "target", mount_id, alias_source, "btrfs"),
            false,
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect("owned alias-only mount should unmount cleanly");

        assert_eq!(outcomes, vec![UnmountOutcome::Unmounted { target }]);
        assert_eq!(unmount_calls.get(), 1);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_reports_already_unmounted_when_target_has_no_mount_entry() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let target = workspace.repo_path(&repo_id, &rel("target"));

        let outcomes = unmount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![],
            ownership::MountOwnershipState::default(),
            false,
            |_| unreachable!("missing mount entries should not unmount"),
        )
        .expect("missing mount entry should be reported");

        assert_eq!(outcomes, vec![UnmountOutcome::AlreadyUnmounted { target }]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_loader_removes_receipt_when_target_is_already_unmounted() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        ownership::save(
            &workspace,
            &owned_mount_state(&repo_id, "ctx", "target", 333, expected_source, "bind"),
        )
        .expect("save ownership");

        let outcomes = unmount_repos_with_loader(
            &workspace,
            Some(repo_id),
            false,
            || Ok(Vec::new()),
            |_| unreachable!("missing mount entries should not unmount"),
        )
        .expect("already-unmounted target should be reported");

        assert_eq!(outcomes, vec![UnmountOutcome::AlreadyUnmounted { target }]);
        assert!(
            ownership::load(&workspace)
                .expect("load ownership")
                .records()
                .is_empty()
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_fails_closed_when_mount_table_is_malformed() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let error = unmount_repos_with_loader(
            &workspace,
            Some(repo_id),
            false,
            || {
                Err(Error::malformed_mount_table_entry(
                    12,
                    "missing ` - ` separator",
                ))
            },
            |_| unreachable!("malformed mount tables should fail closed"),
        )
        .expect_err("malformed mount table should block unmount");

        assert!(matches!(
            error,
            Error::InfraMount(
                crate::shared::error::InfraMountError::MalformedMountTableEntry { line_no: 12, .. }
            )
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_propagates_unmount_target_errors() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));

        let error = unmount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                25,
                expected_source.clone(),
                target.clone(),
                "bind",
            )],
            owned_mount_state(&repo_id, "ctx", "target", 25, expected_source, "bind"),
            false,
            |_| Err(crate::shared::error::InfraMountError::Unsupported.into()),
        )
        .expect_err("unmount failures should propagate");

        assert!(matches!(error, Error::InfraMount(_)));
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn public_unmount_reports_already_unmounted_when_no_mount_is_active() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let outcomes =
            unmount_repos(&workspace, Some(repo_id), false).expect("public wrapper should succeed");

        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0],
            UnmountOutcome::AlreadyUnmounted { .. }
        ));
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_loader_refuses_matching_live_mount_without_receipt() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        crate::test_support::init_git_repo(&repo_root);
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let unmount_calls = Cell::new(0usize);

        let error = unmount_repos_with_loader(
            &workspace,
            Some(repo_id.clone()),
            false,
            || {
                Ok(vec![live_mount(
                    32,
                    expected_source.clone(),
                    target.clone(),
                    "bind",
                )])
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("matching live mount without receipt should not unmount");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::UnmountTargetOwnershipUnverified {
                target: ref unverified_target,
                ..
            }) if unverified_target == &target
        ));
        assert_eq!(unmount_calls.get(), 0);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn unmount_loader_accepts_matching_live_mount_with_unsafe_override() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let unmount_calls = Cell::new(0usize);

        let outcomes = unmount_repos_with_loader(
            &workspace,
            Some(repo_id.clone()),
            true,
            || {
                Ok(vec![live_mount(
                    33,
                    expected_source.clone(),
                    target.clone(),
                    "bind",
                )])
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect("matching live mount should unmount even without .git");

        assert_eq!(outcomes, vec![UnmountOutcome::Unmounted { target }]);
        assert_eq!(unmount_calls.get(), 1);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
