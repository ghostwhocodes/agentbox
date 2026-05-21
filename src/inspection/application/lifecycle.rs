use crate::{
    Result,
    inspection::{self, RepoLifecyclePreflight},
    mounts::infra as mount,
    mounts::ownership,
    registry,
    shared::types::RepoId,
    workspace::Workspace,
};

use super::load_mount_table_for_mutating_workflow;

pub fn repo_lifecycle_preflight(
    workspace: &Workspace,
    repo_id: RepoId,
) -> Result<RepoLifecyclePreflight> {
    repo_lifecycle_preflight_with_mount_loader(workspace, repo_id, mount::list_mounts)
}

pub fn selected_repo_lifecycle_preflights(
    workspace: &Workspace,
    requested: Option<RepoId>,
) -> Result<Vec<RepoLifecyclePreflight>> {
    selected_repo_lifecycle_preflights_with_mount_loader(workspace, requested, mount::list_mounts)
}

fn selected_repo_lifecycle_preflights_with_mount_loader<F>(
    workspace: &Workspace,
    requested: Option<RepoId>,
    list_mounts: F,
) -> Result<Vec<RepoLifecyclePreflight>>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    let repos = registry::selected_registered_repos(workspace, requested)?;
    let repo_ids: Vec<_> = repos.iter().map(|(repo_id, _)| repo_id.clone()).collect();
    let mut repo_mounts_by_repo = crate::mounts::repo_mounts_by_repo(workspace, &repo_ids)?;
    let selected_mounts = repo_ids
        .iter()
        .map(|repo_id| {
            (
                repo_id.clone(),
                repo_mounts_by_repo
                    .get(repo_id)
                    .cloned()
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let repos_for_mount_table = selected_mounts
        .iter()
        .map(|(repo_id, repo_mounts)| (repo_id.clone(), repo_mounts.len()))
        .collect::<Vec<_>>();
    let mount_table = inspection::MountTableState::Available(
        load_mount_table_for_mutating_workflow(workspace, &repos_for_mount_table, list_mounts)?,
    );
    let owned_mounts = ownership::load_advisory(workspace);

    repos
        .into_iter()
        .map(|(repo_id, _repo)| {
            let repo_mounts = repo_mounts_by_repo.remove(&repo_id).unwrap_or_default();
            Ok(inspect_repo_lifecycle_preflight(
                workspace,
                &repo_id,
                &repo_mounts,
                &mount_table,
                &owned_mounts,
            ))
        })
        .collect()
}

fn repo_lifecycle_preflight_with_mount_loader<F>(
    workspace: &Workspace,
    repo_id: RepoId,
    list_mounts: F,
) -> Result<RepoLifecyclePreflight>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    let mut preflights = selected_repo_lifecycle_preflights_with_mount_loader(
        workspace,
        Some(repo_id),
        list_mounts,
    )?;
    Ok(preflights.remove(0))
}

fn inspect_repo_lifecycle_preflight(
    workspace: &Workspace,
    repo_id: &RepoId,
    repo_mounts: &[crate::shared::mount_spec::MountSpec],
    mount_table: &inspection::MountTableState,
    owned_mounts: &ownership::MountOwnershipState,
) -> RepoLifecyclePreflight {
    let evaluation = inspection::evaluate_repo_mounts_for_mutating_workflow(
        workspace,
        repo_id,
        repo_mounts,
        mount_table,
        owned_mounts,
    );

    RepoLifecyclePreflight {
        repo_id: repo_id.clone(),
        materialized: evaluation.materialized,
        has_active_mounts: !evaluation.active_mount_targets.is_empty(),
        has_conflicting_mounts: evaluation.has_conflicting_mounts,
        active_mount_targets: evaluation.active_mount_targets,
        unverified_active_mounts: evaluation.unverified_active_mounts,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs};

    use camino::Utf8PathBuf;

    use super::*;
    use crate::{
        error::Error,
        persistence::manifest::{PersistedRepoMount, PersistedRepoRegistration},
        shared::types::{CloneSource, RelativePath},
        test_support,
    };

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-inspection-lifecycle-test")
    }

    fn register_repo_with_mount(workspace: &Workspace, repo_id: &RepoId) {
        crate::workspace::init_workspace(workspace).expect("init workspace");
        let mut manifest =
            crate::persistence::manifest_store::read(workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest.repo_mounts.push(PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
        });
        crate::persistence::manifest_store::write(workspace, &manifest).expect("save manifest");
    }

    #[test]
    fn lifecycle_preflight_skips_mount_table_when_repo_root_is_missing() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let called = Cell::new(false);
        register_repo_with_mount(&workspace, &repo_id);

        let preflights = selected_repo_lifecycle_preflights_with_mount_loader(
            &workspace,
            Some(repo_id.clone()),
            || {
                called.set(true);
                Ok(Vec::new())
            },
        )
        .expect("preflights");

        assert!(!called.get());
        assert_eq!(
            preflights,
            vec![RepoLifecyclePreflight {
                repo_id,
                materialized: false,
                has_active_mounts: false,
                has_conflicting_mounts: false,
                active_mount_targets: Vec::new(),
                unverified_active_mounts: Vec::new(),
            }]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_reports_active_mounts_for_non_materialized_repo_root() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        register_repo_with_mount(&workspace, &repo_id);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        let active_source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let preflight =
            repo_lifecycle_preflight_with_mount_loader(&workspace, repo_id.clone(), || {
                Ok(vec![mount::MountEntry {
                    mount_id: 54,
                    preferred_source: active_source.clone(),
                    source_aliases: vec![active_source.clone()],
                    mount_point: target.clone(),
                    filesystem_type: "bind".to_string(),
                }])
            })
            .expect("preflight");

        assert_eq!(
            preflight,
            RepoLifecyclePreflight {
                repo_id,
                materialized: false,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![target.clone()],
                unverified_active_mounts: vec![inspection::UnverifiedActiveMount {
                    existing_source: active_source,
                    target,
                }],
            }
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_treats_owned_alias_only_mount_as_active_without_conflict() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        register_repo_with_mount(&workspace, &repo_id);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        let alias_source = Utf8PathBuf::from("/@workspace/context/demo/ctx");
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let mount_id = 55;

        let mut ownership_state = ownership::MountOwnershipState::default();
        ownership_state.upsert(ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
            mount_id,
            source_root: alias_source.clone(),
            filesystem_type: "btrfs".to_string(),
        });
        ownership::save(&workspace, &ownership_state).expect("save ownership");

        let preflight =
            repo_lifecycle_preflight_with_mount_loader(&workspace, repo_id.clone(), || {
                Ok(vec![mount::MountEntry {
                    mount_id,
                    preferred_source: alias_source,
                    source_aliases: vec![Utf8PathBuf::from("/@workspace/context/demo/ctx")],
                    mount_point: target.clone(),
                    filesystem_type: "btrfs".to_string(),
                }])
            })
            .expect("preflight");

        assert_eq!(
            preflight,
            RepoLifecyclePreflight {
                repo_id,
                materialized: false,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![target],
                unverified_active_mounts: Vec::new(),
            }
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_loads_mount_table_for_plain_repo_root() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let called = Cell::new(false);
        register_repo_with_mount(&workspace, &repo_id);
        fs::create_dir_all(workspace.repo_root(&repo_id)).expect("create plain repo root");

        let preflights = selected_repo_lifecycle_preflights_with_mount_loader(
            &workspace,
            Some(repo_id.clone()),
            || {
                called.set(true);
                Ok(Vec::new())
            },
        )
        .expect("preflights");

        assert!(called.get());
        assert_eq!(
            preflights,
            vec![RepoLifecyclePreflight {
                repo_id,
                materialized: false,
                has_active_mounts: false,
                has_conflicting_mounts: false,
                active_mount_targets: Vec::new(),
                unverified_active_mounts: Vec::new(),
            }]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_fails_closed_when_repo_root_cannot_be_statted() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        register_repo_with_mount(&workspace, &repo_id);
        let repos_dir = workspace.repos_dir();

        fs::remove_dir_all(&repos_dir).expect("remove repos dir");
        fs::write(&repos_dir, "not a directory").expect("replace repos dir with file");

        let error =
            selected_repo_lifecycle_preflights_with_mount_loader(&workspace, Some(repo_id), || {
                Ok(Vec::new())
            })
            .expect_err("repo-root stat failures must not skip mount inspection");

        assert!(error.to_string().contains("repos/demo"));

        let _ = fs::remove_file(&repos_dir);
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_reports_active_and_conflicting_mounts() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest.repo_mounts.push(PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
        });
        manifest.repo_mounts.push(PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: rel("other"),
            repo: rel("other"),
        });
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let active_source = workspace.context_path(&repo_id, &rel("ctx"));

        let preflight =
            repo_lifecycle_preflight_with_mount_loader(&workspace, repo_id.clone(), || {
                Ok(vec![
                    mount::MountEntry {
                        mount_id: 51,
                        preferred_source: active_source.clone(),
                        source_aliases: vec![active_source.clone()],
                        mount_point: workspace.repo_path(&repo_id, &rel("target")),
                        filesystem_type: "bind".to_string(),
                    },
                    mount::MountEntry {
                        mount_id: 52,
                        preferred_source: Utf8PathBuf::from("/foreign/source"),
                        source_aliases: vec![Utf8PathBuf::from("/foreign/source")],
                        mount_point: workspace.repo_path(&repo_id, &rel("other")),
                        filesystem_type: "bind".to_string(),
                    },
                ])
            })
            .expect("preflight");

        assert_eq!(
            preflight,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: true,
                has_active_mounts: true,
                has_conflicting_mounts: true,
                active_mount_targets: vec![workspace.repo_path(&repo_id, &rel("target"))],
                unverified_active_mounts: vec![inspection::UnverifiedActiveMount {
                    existing_source: active_source,
                    target: workspace.repo_path(&repo_id, &rel("target")),
                }],
            }
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_degrades_when_mount_table_is_unavailable() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        register_repo_with_mount(&workspace, &repo_id);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);

        let error = repo_lifecycle_preflight_with_mount_loader(&workspace, repo_id.clone(), || {
            Err(Error::io_path(
                "/proc/self/mountinfo",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            ))
        })
        .expect_err("mount inspection failure should block mutating preflight");

        assert!(
            matches!(error, Error::IoPath { ref path, .. } if path == &camino::Utf8PathBuf::from("/proc/self/mountinfo"))
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_treats_matching_live_mount_as_active() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        register_repo_with_mount(&workspace, &repo_id);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let active_source = workspace.context_path(&repo_id, &rel("ctx"));

        let preflight =
            repo_lifecycle_preflight_with_mount_loader(&workspace, repo_id.clone(), || {
                Ok(vec![mount::MountEntry {
                    mount_id: 53,
                    preferred_source: active_source.clone(),
                    source_aliases: vec![active_source.clone()],
                    mount_point: workspace.repo_path(&repo_id, &rel("target")),
                    filesystem_type: "bind".to_string(),
                }])
            })
            .expect("preflight");

        assert_eq!(
            preflight,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: true,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![workspace.repo_path(&repo_id, &rel("target"))],
                unverified_active_mounts: vec![inspection::UnverifiedActiveMount {
                    existing_source: active_source,
                    target: workspace.repo_path(&repo_id, &rel("target")),
                }],
            }
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_verifies_active_mount_with_matching_receipt() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        register_repo_with_mount(&workspace, &repo_id);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let active_source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let mut ownership_state = ownership::MountOwnershipState::default();
        ownership_state.upsert(ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
            mount_id: 55,
            source_root: active_source.clone(),
            filesystem_type: "bind".to_string(),
        });
        ownership::save(&workspace, &ownership_state).expect("save ownership");

        let preflight =
            repo_lifecycle_preflight_with_mount_loader(&workspace, repo_id.clone(), || {
                Ok(vec![mount::MountEntry {
                    mount_id: 55,
                    preferred_source: active_source,
                    source_aliases: vec![workspace.context_path(&repo_id, &rel("ctx"))],
                    mount_point: target.clone(),
                    filesystem_type: "bind".to_string(),
                }])
            })
            .expect("preflight");

        assert_eq!(preflight.active_mount_targets, vec![target]);
        assert!(preflight.unverified_active_mounts.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn lifecycle_preflight_fails_when_mount_table_is_malformed() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        register_repo_with_mount(&workspace, &repo_id);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);

        let error = repo_lifecycle_preflight_with_mount_loader(&workspace, repo_id, || {
            Err(Error::malformed_mount_table_entry(
                12,
                "missing ` - ` separator",
            ))
        })
        .expect_err("malformed mount table should block mutating preflight");

        assert!(matches!(
            error,
            Error::InfraMount(
                crate::shared::error::InfraMountError::MalformedMountTableEntry { line_no: 12, .. }
            )
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }
}
