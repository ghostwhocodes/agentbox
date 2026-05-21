use camino::Utf8Path;
use camino::Utf8PathBuf;

use crate::{
    error::Result,
    inspection::{self, AggregateMountState, ObservedMount, RepoStatus},
    mounts::{infra as mount, ownership},
    registry::is_repo_materialized,
    shared::{mount_spec::MountSpec, types::RepoId},
    workspace::Workspace,
};

use super::load_workspace_snapshot;

pub fn workspace_status(workspace: &Workspace) -> Result<Vec<RepoStatus>> {
    workspace_status_with_mount_loader(workspace, mount::list_mounts)
}

pub fn show_repo(workspace: &Workspace, repo_id: RepoId) -> Result<RepoStatus> {
    show_repo_with_mount_loader(workspace, repo_id, mount::list_mounts)
}

pub(crate) struct RepoMountEvaluation {
    pub materialized: bool,
    pub mount_state: AggregateMountState,
    pub mounts: Vec<inspection::MountStatus>,
    pub active_mount_targets: Vec<Utf8PathBuf>,
    pub unverified_active_mounts: Vec<inspection::UnverifiedActiveMount>,
    pub has_conflicting_mounts: bool,
}

pub(crate) fn evaluate_repo_mounts_for_mutating_workflow(
    workspace: &Workspace,
    repo_id: &RepoId,
    repo_mounts: &[MountSpec],
    mount_table: &inspection::MountTableState,
    owned_mounts: &ownership::MountOwnershipState,
) -> RepoMountEvaluation {
    evaluate_repo_mounts_with_options(
        workspace,
        repo_id,
        repo_mounts,
        mount_table,
        Some(owned_mounts),
        true,
    )
}

fn evaluate_repo_mounts_with_options(
    workspace: &Workspace,
    repo_id: &RepoId,
    repo_mounts: &[MountSpec],
    mount_table: &inspection::MountTableState,
    owned_mounts: Option<&ownership::MountOwnershipState>,
    inspect_when_not_materialized: bool,
) -> RepoMountEvaluation {
    let materialized = is_repo_materialized(workspace, repo_id);
    let should_inspect_live_mounts = materialized || inspect_when_not_materialized;
    let inspection_unavailable = should_inspect_live_mounts
        && !repo_mounts.is_empty()
        && matches!(mount_table, inspection::MountTableState::Unavailable(_));
    let mut mounts = Vec::with_capacity(repo_mounts.len());
    let mut active_count = 0usize;
    let mut conflicting_count = 0usize;
    let mut active_mount_targets = Vec::new();
    let mut unverified_active_mounts = Vec::new();

    for mount_spec in repo_mounts {
        let source = workspace.context_path(repo_id, &mount_spec.context);
        let target = workspace.repo_path(repo_id, &mount_spec.repo);

        if inspection_unavailable {
            mounts.push(inspection::MountStatus {
                source,
                target,
                active: false,
                conflicting_mount: false,
                inspection_unavailable: true,
            });
            continue;
        }

        let target_mount = if should_inspect_live_mounts {
            observed_mount_for_target(mount_table, &target)
        } else {
            None
        };
        let active = target_mount.is_some_and(|entry| {
            entry.matches_source(&source)
                || owned_mounts.is_some_and(|owned_mounts| {
                    owned_mounts.verifies_live_mount(
                        repo_id,
                        mount_spec,
                        entry.mount_id,
                        &entry.preferred_source,
                        &entry.filesystem_type,
                    )
                })
        });
        let conflicting_mount = target_mount.is_some() && !active;
        if active {
            active_count += 1;
            active_mount_targets.push(target.clone());
            if let Some(entry) = target_mount {
                let ownership_unverified = owned_mounts.is_some_and(|owned_mounts| {
                    !owned_mounts.verifies_live_mount(
                        repo_id,
                        mount_spec,
                        entry.mount_id,
                        &entry.preferred_source,
                        &entry.filesystem_type,
                    )
                });
                if ownership_unverified {
                    unverified_active_mounts.push(inspection::UnverifiedActiveMount {
                        existing_source: entry.preferred_source.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
        if conflicting_mount {
            conflicting_count += 1;
        }

        mounts.push(inspection::MountStatus {
            source,
            target,
            active,
            conflicting_mount,
            inspection_unavailable: false,
        });
    }

    RepoMountEvaluation {
        materialized,
        mount_state: crate::inspection::domain::aggregate_mount_state(
            repo_mounts.len(),
            active_count,
            conflicting_count,
            inspection_unavailable,
        ),
        mounts,
        active_mount_targets,
        unverified_active_mounts,
        has_conflicting_mounts: conflicting_count > 0,
    }
}

pub(crate) fn inspect_registered_repo(
    workspace: &Workspace,
    repo_id: &RepoId,
    source: &crate::shared::types::CloneSource,
    repo_mounts: &[MountSpec],
    mount_table: &inspection::MountTableState,
    owned_mounts: &ownership::MountOwnershipState,
    inspect_non_materialized_live_mounts: bool,
) -> Result<RepoStatus> {
    let context_root = workspace.repo_context_root(repo_id);
    let repo_root = workspace.repo_root(repo_id);
    let evaluation = evaluate_repo_mounts_with_options(
        workspace,
        repo_id,
        repo_mounts,
        mount_table,
        Some(owned_mounts),
        inspect_non_materialized_live_mounts,
    );

    Ok(RepoStatus {
        repo_id: repo_id.clone(),
        source: source.clone(),
        context_root,
        repo_root,
        materialized: evaluation.materialized,
        mount_state: evaluation.mount_state,
        mounts: evaluation.mounts,
    })
}

pub(crate) fn observed_mount_for_target<'a>(
    mount_table: &'a inspection::MountTableState,
    target: &Utf8Path,
) -> Option<&'a ObservedMount> {
    match mount_table {
        inspection::MountTableState::Available(entries) => {
            entries.iter().find(|entry| entry.target == target)
        }
        inspection::MountTableState::Unavailable(_) => None,
    }
}

fn workspace_status_with_mount_loader<F>(
    workspace: &Workspace,
    list_mounts: F,
) -> Result<Vec<RepoStatus>>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    let snapshot = load_workspace_snapshot(workspace, None, list_mounts)?;
    let mount_table = snapshot.mount_table;
    let owned_mounts = ownership::load_advisory(workspace);

    snapshot
        .repos
        .into_iter()
        .map(|repo| {
            inspect_registered_repo(
                workspace,
                &repo.repo_id,
                &repo.source,
                &repo.repo_mounts,
                &mount_table,
                &owned_mounts,
                repo.inspect_non_materialized_live_mounts,
            )
        })
        .collect()
}

fn show_repo_with_mount_loader<F>(
    workspace: &Workspace,
    repo_id: RepoId,
    list_mounts: F,
) -> Result<RepoStatus>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    let mut snapshot = load_workspace_snapshot(workspace, Some(repo_id), list_mounts)?;
    let repo = snapshot
        .repos
        .pop()
        .expect("single selected repo snapshot should exist");
    let mount_table = snapshot.mount_table;
    let owned_mounts = ownership::load_advisory(workspace);
    inspect_registered_repo(
        workspace,
        &repo.repo_id,
        &repo.source,
        &repo.repo_mounts,
        &mount_table,
        &owned_mounts,
        repo.inspect_non_materialized_live_mounts,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        error::Error,
        inspection::AggregateMountState,
        mounts::ownership,
        persistence::manifest::{PersistedRepoMount, PersistedRepoRegistration},
        shared::types::{CloneSource, RelativePath},
        test_support,
    };

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-inspection-repo-status-test")
    }

    fn materialized_repo_with_mount(workspace: &Workspace) -> RepoId {
        crate::workspace::init_workspace(workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");

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

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx")))
            .expect("create mount source");

        repo_id
    }

    #[test]
    fn workspace_status_treats_alias_only_mount_sources_as_conflicts_without_ownership_receipt() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let alias_source = Utf8PathBuf::from("/mnt/root/workspace/context/demo/ctx");
        let target = workspace.repo_path(&repo_id, &rel("target"));

        let statuses = workspace_status_with_mount_loader(&workspace, || {
            Ok(vec![mount::MountEntry {
                mount_id: 42,
                preferred_source: alias_source.clone(),
                source_aliases: vec![alias_source],
                mount_point: target.clone(),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].repo_id, repo_id);
        assert_eq!(statuses[0].mount_state, AggregateMountState::Conflicted);
        assert!(statuses[0].mounts.iter().all(|mount| {
            mount.source == expected_source
                && mount.target == target
                && !mount.active
                && mount.conflicting_mount
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn workspace_status_treats_owned_alias_only_mount_sources_as_active() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let alias_source = Utf8PathBuf::from("/mnt/root/workspace/context/demo/ctx");
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let mount_id = 42;

        ownership::save(&workspace, &{
            let mut state = ownership::MountOwnershipState::default();
            state.upsert(ownership::OwnedMountRecord {
                repo_id: repo_id.clone(),
                context: rel("ctx"),
                repo: rel("target"),
                mount_id,
                source_root: alias_source.clone(),
                filesystem_type: "bind".to_string(),
            });
            state
        })
        .expect("save ownership");

        let statuses = workspace_status_with_mount_loader(&workspace, || {
            Ok(vec![mount::MountEntry {
                mount_id,
                preferred_source: alias_source,
                source_aliases: vec![Utf8PathBuf::from("/mnt/root/workspace/context/demo/ctx")],
                mount_point: target.clone(),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].repo_id, repo_id);
        assert_eq!(statuses[0].mount_state, AggregateMountState::Mounted);
        assert!(statuses[0].mounts.iter().all(|mount| {
            mount.source == expected_source
                && mount.target == target
                && mount.active
                && !mount.conflicting_mount
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn workspace_status_degrades_to_unknown_when_mount_table_is_unavailable() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);

        let statuses = workspace_status_with_mount_loader(&workspace, || {
            Err(Error::io_path(
                "/proc/self/mountinfo",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            ))
        })
        .expect("status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].repo_id, repo_id);
        assert_eq!(statuses[0].mount_state, AggregateMountState::Unknown);
        assert!(
            statuses[0]
                .mounts
                .iter()
                .all(|mount| mount.inspection_unavailable
                    && !mount.active
                    && !mount.conflicting_mount)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn workspace_status_treats_matching_live_mount_as_active() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));

        let statuses = workspace_status_with_mount_loader(&workspace, || {
            Ok(vec![mount::MountEntry {
                mount_id: 43,
                preferred_source: expected_source.clone(),
                source_aliases: vec![expected_source.clone()],
                mount_point: target.clone(),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].repo_id, repo_id.clone());
        assert_eq!(statuses[0].mount_state, AggregateMountState::Mounted);
        assert!(statuses[0].mounts.iter().all(|mount| {
            mount.source == expected_source
                && mount.target == target
                && mount.active
                && !mount.conflicting_mount
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn workspace_status_treats_matching_live_mount_as_active_for_plain_repo_root() {
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
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        fs::create_dir_all(workspace.repo_root(&repo_id)).expect("create plain repo root");
        fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx")))
            .expect("create mount source");
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        let target = workspace.repo_path(&repo_id, &rel("target"));

        let statuses = workspace_status_with_mount_loader(&workspace, || {
            Ok(vec![mount::MountEntry {
                mount_id: 44,
                preferred_source: expected_source.clone(),
                source_aliases: vec![expected_source.clone()],
                mount_point: target.clone(),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].repo_id, repo_id);
        assert!(!statuses[0].materialized);
        assert_eq!(statuses[0].mount_state, AggregateMountState::Mounted);
        assert!(statuses[0].mounts.iter().all(|mount| {
            mount.source == expected_source
                && mount.target == target
                && mount.active
                && !mount.conflicting_mount
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn workspace_status_skips_mount_table_loading_when_inspection_is_unnecessary() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let statuses = workspace_status_with_mount_loader(&workspace, || {
            panic!("mount table should not be loaded when inspection is unnecessary")
        })
        .expect("status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].repo_id, repo_id);
        assert_eq!(statuses[0].mount_state, AggregateMountState::NoMounts);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn show_repo_degrades_to_unknown_when_mount_table_is_unavailable() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);

        let status = show_repo_with_mount_loader(&workspace, repo_id.clone(), || {
            Err(Error::io_path(
                "/proc/self/mountinfo",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            ))
        })
        .expect("show repo");

        assert_eq!(status.repo_id, repo_id);
        assert_eq!(status.mount_state, AggregateMountState::Unknown);
        assert!(
            status
                .mounts
                .iter()
                .all(|mount| mount.inspection_unavailable
                    && !mount.active
                    && !mount.conflicting_mount)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn show_repo_degrades_to_unknown_for_plain_repo_root_when_mount_table_is_unavailable() {
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
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        fs::create_dir_all(workspace.repo_root(&repo_id)).expect("create plain repo root");

        let status = show_repo_with_mount_loader(&workspace, repo_id.clone(), || {
            Err(Error::io_path(
                "/proc/self/mountinfo",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            ))
        })
        .expect("show repo");

        assert_eq!(status.repo_id, repo_id);
        assert!(!status.materialized);
        assert_eq!(status.mount_state, AggregateMountState::Unknown);
        assert!(
            status
                .mounts
                .iter()
                .all(|mount| mount.inspection_unavailable
                    && !mount.active
                    && !mount.conflicting_mount)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn workspace_status_degrades_to_unknown_when_mount_table_is_malformed() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);

        let statuses = workspace_status_with_mount_loader(&workspace, || {
            Err(Error::malformed_mount_table_entry(
                12,
                "missing ` - ` separator",
            ))
        })
        .expect("status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].repo_id, repo_id);
        assert_eq!(statuses[0].mount_state, AggregateMountState::Unknown);
        assert!(
            statuses[0]
                .mounts
                .iter()
                .all(|mount| mount.inspection_unavailable
                    && !mount.active
                    && !mount.conflicting_mount)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn show_repo_skips_mount_table_loading_when_inspection_is_unnecessary() {
        let workspace = test_workspace();
        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let status = show_repo_with_mount_loader(&workspace, repo_id.clone(), || {
            panic!("mount table should not be loaded when inspection is unnecessary")
        })
        .expect("show repo");

        assert_eq!(status.repo_id, repo_id);
        assert_eq!(status.mount_state, AggregateMountState::NoMounts);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
