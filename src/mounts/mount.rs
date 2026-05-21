use camino::Utf8PathBuf;

use crate::{
    error::{MountWorkflowError, Result},
    mounts::{self, infra as mount, ownership},
    shared::types::{RelativePath, RepoId},
    workspace::Workspace,
};

use crate::registry as repo;

use super::support::{ensure_mount_target_matches_source_or_is_empty, prepare_mount_paths};

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparedMount {
    Bind {
        repo_id: RepoId,
        context: RelativePath,
        repo: RelativePath,
        source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
    AlreadyMounted {
        source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountOutcome {
    Mounted {
        source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
    AlreadyMounted {
        source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
}

pub fn mount_repos(workspace: &Workspace, requested: Option<RepoId>) -> Result<Vec<MountOutcome>> {
    mount_repos_with_loader(workspace, requested, mount::list_mounts, mount::bind_mount)
}

fn mount_repos_with_loader<L, F>(
    workspace: &Workspace,
    requested: Option<RepoId>,
    mut list_mounts: L,
    bind_target: F,
) -> Result<Vec<MountOutcome>>
where
    L: FnMut() -> Result<Vec<mount::MountEntry>>,
    F: FnMut(&camino::Utf8Path, &camino::Utf8Path) -> Result<()>,
{
    let selected_mounts = mounts::selected_repo_mounts(workspace, requested)?;
    let owned_mounts = ownership::load_advisory(workspace);

    // Mount is a state-changing command; fail hard if mount table is unreadable.
    let mount_table = list_mounts()?;
    mount_repos_with_mount_table_and_recorder(
        workspace,
        selected_mounts,
        mount_table,
        &owned_mounts,
        bind_target,
        |repo_id, context, repo, target| {
            record_live_mount_best_effort(
                workspace,
                repo_id,
                context,
                repo,
                target,
                &mut list_mounts,
            );
        },
    )
}

#[cfg(test)]
fn mount_repos_with_mount_table<F>(
    workspace: &Workspace,
    selected_mounts: Vec<(RepoId, Vec<crate::shared::mount_spec::MountSpec>)>,
    mount_table: Vec<mount::MountEntry>,
    owned_mounts: &ownership::MountOwnershipState,
    bind_target: F,
) -> Result<Vec<MountOutcome>>
where
    F: FnMut(&camino::Utf8Path, &camino::Utf8Path) -> Result<()>,
{
    mount_repos_with_mount_table_and_recorder(
        workspace,
        selected_mounts,
        mount_table,
        owned_mounts,
        bind_target,
        |_, _, _, _| {},
    )
}

fn mount_repos_with_mount_table_and_recorder<F, R>(
    workspace: &Workspace,
    selected_mounts: Vec<(RepoId, Vec<crate::shared::mount_spec::MountSpec>)>,
    mount_table: Vec<mount::MountEntry>,
    owned_mounts: &ownership::MountOwnershipState,
    mut bind_target: F,
    mut record_bound_mount: R,
) -> Result<Vec<MountOutcome>>
where
    F: FnMut(&camino::Utf8Path, &camino::Utf8Path) -> Result<()>,
    R: FnMut(&RepoId, &RelativePath, &RelativePath, &camino::Utf8Path),
{
    let mut outcomes = Vec::new();
    let mut prepared_mounts = Vec::new();

    for (repo_id, repo_mounts) in selected_mounts {
        repo::require_materialized_repo(workspace, &repo_id)?;

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
                if existing.matches_source(&source) || ownership_verified {
                    prepared_mounts.push(PreparedMount::AlreadyMounted { source, target });
                    continue;
                }

                return Err(MountWorkflowError::MountTargetAlreadyMounted {
                    mount_source: source,
                    target,
                    existing_source: existing.preferred_source.clone(),
                }
                .into());
            }

            ensure_mount_target_matches_source_or_is_empty(&repo_id, mount_spec, &source, &target)?;
            prepared_mounts.push(PreparedMount::Bind {
                repo_id: repo_id.clone(),
                context: mount_spec.context.clone(),
                repo: mount_spec.repo.clone(),
                source,
                target,
            });
        }
    }

    for prepared_mount in prepared_mounts {
        match prepared_mount {
            PreparedMount::Bind {
                repo_id,
                context,
                repo,
                source,
                target,
            } => {
                prepare_mount_paths(&source, &target)?;
                bind_target(&source, &target)?;
                record_bound_mount(&repo_id, &context, &repo, &target);
                outcomes.push(MountOutcome::Mounted { source, target });
            }
            PreparedMount::AlreadyMounted { source, target } => {
                outcomes.push(MountOutcome::AlreadyMounted { source, target });
            }
        }
    }

    Ok(outcomes)
}

fn record_live_mount_best_effort(
    workspace: &Workspace,
    repo_id: &RepoId,
    context: &RelativePath,
    repo: &RelativePath,
    target: &camino::Utf8Path,
    list_mounts: &mut impl FnMut() -> Result<Vec<mount::MountEntry>>,
) {
    let Ok(mount_table) = list_mounts() else {
        return;
    };
    let Some(live_mount) = mount::find_target_mount(&mount_table, target) else {
        return;
    };

    ownership::record_best_effort(
        workspace,
        ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: context.clone(),
            repo: repo.clone(),
            mount_id: live_mount.mount_id,
            source_root: live_mount.preferred_source.clone(),
            filesystem_type: live_mount.filesystem_type.clone(),
        },
    );
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::{
        cell::{Cell, RefCell},
        fs,
    };

    use camino::Utf8PathBuf;

    use super::*;
    use crate::{
        persistence::manifest::PersistedRepoMount,
        shared::error::{Error, MountWorkflowError},
        shared::mount_spec::MountSpec,
    };

    use super::super::test_support::{init_git_repo, init_repo_with_mount, rel, test_workspace};

    fn add_mount(workspace: &Workspace, repo_id: &RepoId, context: &str, target: &str) {
        let mut manifest =
            crate::persistence::manifest_store::read(workspace).expect("load manifest");
        manifest.repo_mounts.push(PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: rel(context),
            repo: rel(target),
        });
        crate::persistence::manifest_store::write(workspace, &manifest).expect("save manifest");
    }

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
    fn mount_succeeds_when_target_is_free() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let recorded = RefCell::new(Vec::<(Utf8PathBuf, Utf8PathBuf)>::new());
        let outcomes = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |source, target| {
                recorded
                    .borrow_mut()
                    .push((source.to_path_buf(), target.to_path_buf()));
                Ok(())
            },
        )
        .expect("mount should succeed");

        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let expected_target = workspace.repo_path(&repo_id, &rel("target"));
        assert_eq!(
            outcomes,
            vec![MountOutcome::Mounted {
                source: expected_source.clone(),
                target: expected_target.clone(),
            }]
        );
        assert_eq!(
            recorded.into_inner(),
            vec![(expected_source, expected_target)]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_loader_records_ownership_receipt_after_bind_mount() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let load_count = Cell::new(0usize);

        let outcomes = mount_repos_with_loader(
            &workspace,
            Some(repo_id.clone()),
            || {
                load_count.set(load_count.get() + 1);
                if load_count.get() == 1 {
                    Ok(Vec::new())
                } else {
                    Ok(vec![live_mount(
                        201,
                        source.clone(),
                        target.clone(),
                        "bind",
                    )])
                }
            },
            |_, _| Ok(()),
        )
        .expect("mount should succeed");

        assert_eq!(
            outcomes,
            vec![MountOutcome::Mounted {
                source: source.clone(),
                target,
            }]
        );
        assert_eq!(load_count.get(), 2);
        let ownership = ownership::load(&workspace).expect("load ownership");
        assert_eq!(ownership.records().len(), 1);
        assert_eq!(ownership.records()[0].repo_id, repo_id);
        assert_eq!(ownership.records()[0].mount_id, 201);
        assert_eq!(ownership.records()[0].source_root, source);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_loader_does_not_rollback_when_receipt_capture_fails() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let load_count = Cell::new(0usize);
        let bind_calls = Cell::new(0usize);

        let outcomes = mount_repos_with_loader(
            &workspace,
            Some(repo_id),
            || {
                load_count.set(load_count.get() + 1);
                if load_count.get() == 1 {
                    Ok(Vec::new())
                } else {
                    Err(Error::io_path(
                        "/proc/self/mountinfo",
                        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
                    ))
                }
            },
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect("receipt capture failure should not roll back mount");

        assert_eq!(outcomes.len(), 1);
        assert_eq!(bind_calls.get(), 1);
        assert!(!workspace.mount_ownership_path().exists());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_reports_already_mounted_for_exact_expected_source() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let mount_id = 11;
        let bind_calls = Cell::new(0usize);
        let outcomes = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(mount_id, source.clone(), target.clone(), "bind")],
            &ownership::MountOwnershipState::default(),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect("already mounted should be reported cleanly");

        assert_eq!(bind_calls.get(), 0);
        assert_eq!(
            outcomes,
            vec![MountOutcome::AlreadyMounted { source, target }]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_loader_treats_matching_live_mount_as_already_mounted() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let bind_calls = Cell::new(0usize);

        let error = mount_repos_with_loader(
            &workspace,
            Some(repo_id.clone()),
            || Ok(vec![live_mount(31, source.clone(), target.clone(), "bind")]),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect("matching live mount should be reported cleanly");

        assert_eq!(bind_calls.get(), 0);
        assert_eq!(error, vec![MountOutcome::AlreadyMounted { source, target }]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_rejects_alias_only_source_match_as_conflict_without_ownership_receipt() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let bind_calls = Cell::new(0usize);
        let error = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                12,
                Utf8PathBuf::from("/@workspace/context/demo/ctx"),
                target.clone(),
                "btrfs",
            )],
            &ownership::MountOwnershipState::default(),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("alias-only source should block mount as a conflict");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetAlreadyMounted {
                ref mount_source,
                target: ref occupied_target,
                ref existing_source,
            }) if mount_source == &source
                && occupied_target == &target
                && existing_source == &Utf8PathBuf::from("/@workspace/context/demo/ctx")
        ));
        assert_eq!(bind_calls.get(), 0);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_accepts_alias_only_source_match_when_ownership_receipt_verifies_live_mount() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let bind_calls = Cell::new(0usize);
        let mount_id = 12;
        let alias_source = Utf8PathBuf::from("/@workspace/context/demo/ctx");

        let outcomes = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                mount_id,
                alias_source.clone(),
                target.clone(),
                "btrfs",
            )],
            &owned_mount_state(&repo_id, "ctx", "target", mount_id, alias_source, "btrfs"),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect("owned alias-only mount should be treated as already mounted");

        assert_eq!(bind_calls.get(), 0);
        assert_eq!(
            outcomes,
            vec![MountOutcome::AlreadyMounted { source, target }]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_fails_when_target_is_already_mounted_from_foreign_source() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let bind_calls = Cell::new(0usize);
        let error = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![live_mount(
                13,
                Utf8PathBuf::from("/foreign/source"),
                target.clone(),
                "bind",
            )],
            &ownership::MountOwnershipState::default(),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("foreign mount should block mount");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetAlreadyMounted {
                ref mount_source,
                target: ref occupied_target,
                ref existing_source,
            }) if mount_source == &source
                && occupied_target == &target
                && existing_source == &Utf8PathBuf::from("/foreign/source")
        ));
        assert_eq!(bind_calls.get(), 0);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_fails_when_target_contains_different_files() {
        let workspace = test_workspace();
        let expected_repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&expected_repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&expected_repo_id, &rel("ctx"));
        let target = workspace.repo_path(&expected_repo_id, &rel("target"));
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&target).expect("create target");
        fs::write(source.join("state.txt"), "workspace").expect("write source");
        fs::write(target.join("state.txt"), "repo").expect("write target");

        let bind_calls = Cell::new(0usize);
        let error = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &expected_repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("divergent target should block mount");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetWouldHideDifferentFiles {
                ref repo_id,
                ref repo_path,
                ref mount_source,
                ref target,
            }) if repo_id == &expected_repo_id
                && repo_path.as_str() == "target"
                && mount_source == &workspace.context_path(&expected_repo_id, &rel("ctx"))
                && target == &workspace.repo_path(&expected_repo_id, &rel("target"))
        ));
        assert_eq!(bind_calls.get(), 0);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_fails_without_creating_context_dir_when_safety_check_rejects_target() {
        let workspace = test_workspace();
        let expected_repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&expected_repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&expected_repo_id, &rel("ctx"));
        let target = workspace.repo_path(&expected_repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create target");
        fs::write(target.join("state.txt"), "repo").expect("write target");

        let bind_calls = Cell::new(0usize);
        let error = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &expected_repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("divergent target should block mount");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetWouldHideDifferentFiles {
                ref repo_id,
                ref repo_path,
                ref mount_source,
                ref target,
            }) if repo_id == &expected_repo_id
                && repo_path.as_str() == "target"
                && mount_source == &source
                && target == &workspace.repo_path(&expected_repo_id, &rel("target"))
        ));
        assert_eq!(bind_calls.get(), 0);
        assert!(
            !source.exists(),
            "failed preflight should not create context dirs"
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_allows_target_with_only_empty_directory_scaffolding() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(target.join("subdir/empty")).expect("create target scaffold");
        fs::write(source.join("prompt.md"), "seed").expect("write source");

        let recorded = RefCell::new(Vec::<(Utf8PathBuf, Utf8PathBuf)>::new());
        let outcomes = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |source, target| {
                recorded
                    .borrow_mut()
                    .push((source.to_path_buf(), target.to_path_buf()));
                Ok(())
            },
        )
        .expect("empty directory scaffolding should not block mount");

        assert_eq!(
            outcomes,
            vec![MountOutcome::Mounted {
                source: source.clone(),
                target: target.clone(),
            }]
        );
        assert_eq!(recorded.into_inner(), vec![(source, target)]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_allows_scaffold_only_target_when_source_dir_is_missing() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(target.join("subdir/empty")).expect("create target scaffold");
        assert!(!source.exists(), "source should start missing");

        let recorded = RefCell::new(Vec::<(Utf8PathBuf, Utf8PathBuf)>::new());
        let outcomes = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |source, target| {
                recorded
                    .borrow_mut()
                    .push((source.to_path_buf(), target.to_path_buf()));
                Ok(())
            },
        )
        .expect("empty directory scaffolding should be allowed without a source tree");

        assert_eq!(
            outcomes,
            vec![MountOutcome::Mounted {
                source: source.clone(),
                target: target.clone(),
            }]
        );
        assert_eq!(recorded.into_inner(), vec![(source, target)]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[cfg(unix)]
    #[test]
    fn mount_fails_when_target_file_executable_bit_differs() {
        let workspace = test_workspace();
        let expected_repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&expected_repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&expected_repo_id, &rel("ctx"));
        let target = workspace.repo_path(&expected_repo_id, &rel("target"));
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&target).expect("create target");
        fs::write(source.join("script.sh"), "#!/bin/sh\necho hi\n").expect("write source");
        fs::write(target.join("script.sh"), "#!/bin/sh\necho hi\n").expect("write target");

        let mut source_permissions = fs::metadata(source.join("script.sh"))
            .expect("source metadata")
            .permissions();
        source_permissions.set_mode(0o755);
        fs::set_permissions(source.join("script.sh"), source_permissions).expect("set source mode");

        let mut target_permissions = fs::metadata(target.join("script.sh"))
            .expect("target metadata")
            .permissions();
        target_permissions.set_mode(0o644);
        fs::set_permissions(target.join("script.sh"), target_permissions).expect("set target mode");

        let bind_calls = Cell::new(0usize);
        let error = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &expected_repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("executable-bit mismatch should block mount");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetWouldHideDifferentFiles {
                ref repo_id,
                ref repo_path,
                ref mount_source,
                ref target,
            }) if repo_id == &expected_repo_id
                && repo_path.as_str() == "target"
                && mount_source == &workspace.context_path(&expected_repo_id, &rel("ctx"))
                && target == &workspace.repo_path(&expected_repo_id, &rel("target"))
        ));
        assert_eq!(bind_calls.get(), 0);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[cfg(unix)]
    #[test]
    fn mount_succeeds_when_target_matches_source_symlink_tree() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&target).expect("create target");
        symlink("shared-target", source.join("link")).expect("create source symlink");
        symlink("shared-target", target.join("link")).expect("create target symlink");

        let recorded = RefCell::new(Vec::<(Utf8PathBuf, Utf8PathBuf)>::new());
        let outcomes = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |source, target| {
                recorded
                    .borrow_mut()
                    .push((source.to_path_buf(), target.to_path_buf()));
                Ok(())
            },
        )
        .expect("matching symlink trees should allow mount");

        assert_eq!(recorded.borrow().len(), 1);
        assert_eq!(outcomes, vec![MountOutcome::Mounted { source, target }]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_succeeds_when_target_is_matching_subset_of_source_tree() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&target).expect("create target");
        fs::write(source.join("prompt.md"), "seed").expect("write shared source file");
        fs::write(target.join("prompt.md"), "seed").expect("write shared target file");
        fs::write(source.join("config.json"), "{\"mode\":\"expanded\"}")
            .expect("write source-only file");

        let recorded = RefCell::new(Vec::<(Utf8PathBuf, Utf8PathBuf)>::new());
        let outcomes = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |source, target| {
                recorded
                    .borrow_mut()
                    .push((source.to_path_buf(), target.to_path_buf()));
                Ok(())
            },
        )
        .expect("matching subset target should be allowed");

        assert_eq!(
            outcomes,
            vec![MountOutcome::Mounted {
                source: source.clone(),
                target: target.clone(),
            }]
        );
        assert_eq!(recorded.borrow().as_slice(), &[(source, target)]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_preflights_all_mounts_before_binding_any_targets() {
        let workspace = test_workspace();
        let expected_repo_id = init_repo_with_mount(&workspace);
        add_mount(&workspace, &expected_repo_id, "ctx-two", "target-two");
        let repo_root = workspace.repo_root(&expected_repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let first_source = workspace.context_path(&expected_repo_id, &rel("ctx"));
        let first_target = workspace.repo_path(&expected_repo_id, &rel("target"));
        let second_source = workspace.context_path(&expected_repo_id, &rel("ctx-two"));
        let second_target = workspace.repo_path(&expected_repo_id, &rel("target-two"));

        fs::create_dir_all(&second_target).expect("create second target");
        fs::write(second_target.join("state.txt"), "repo").expect("write second target");

        let recorded = RefCell::new(Vec::<(Utf8PathBuf, Utf8PathBuf)>::new());
        let error = mount_repos_with_mount_table(
            &workspace,
            selected_mounts(&workspace, &expected_repo_id),
            vec![],
            &ownership::MountOwnershipState::default(),
            |source, target| {
                recorded
                    .borrow_mut()
                    .push((source.to_path_buf(), target.to_path_buf()));
                Ok(())
            },
        )
        .expect_err("later divergent target should block all mounts");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountTargetWouldHideDifferentFiles {
                ref repo_id,
                ref repo_path,
                ref mount_source,
                ref target,
            }) if repo_id == &expected_repo_id
                && repo_path.as_str() == "target-two"
                && mount_source == &second_source
                && target == &second_target
        ));
        assert!(
            recorded.borrow().is_empty(),
            "preflight failure should prevent earlier binds"
        );
        assert!(
            !first_source.exists(),
            "preflight should not create first source"
        );
        assert!(
            !first_target.exists(),
            "preflight should not create first target"
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_fails_closed_when_mount_table_is_malformed() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let bind_calls = Cell::new(0usize);
        let error = mount_repos_with_loader(
            &workspace,
            Some(repo_id),
            || {
                Err(Error::malformed_mount_table_entry(
                    12,
                    "missing ` - ` separator",
                ))
            },
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("malformed mount table should block mount");

        assert!(matches!(
            error,
            Error::InfraMount(
                crate::shared::error::InfraMountError::MalformedMountTableEntry { line_no: 12, .. }
            )
        ));
        assert_eq!(bind_calls.get(), 0);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
