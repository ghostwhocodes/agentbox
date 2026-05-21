use crate::{
    Result,
    inspection::{self, MountTableState},
    mounts::{self, infra as mount},
    registry,
    shared::{mount_spec::MountSpec, types::RepoId},
    workspace::Workspace,
};

use super::load_mount_table::read_only_repo_needs_mount_inspection;

#[derive(Debug, Clone)]
pub(crate) struct InspectionRepoSnapshot {
    pub repo_id: RepoId,
    pub source: crate::shared::types::CloneSource,
    pub repo_mounts: Vec<MountSpec>,
    pub inspect_non_materialized_live_mounts: bool,
}

pub(crate) struct InspectionWorkspaceSnapshot {
    pub repos: Vec<InspectionRepoSnapshot>,
    pub mount_table: MountTableState,
}

pub(crate) fn load_workspace_snapshot<F>(
    workspace: &Workspace,
    requested: Option<RepoId>,
    list_mounts: F,
) -> Result<InspectionWorkspaceSnapshot>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    let repos = registry::selected_registered_repos(workspace, requested)?;
    let repo_ids: Vec<_> = repos.iter().map(|(repo_id, _)| repo_id.clone()).collect();
    let mut repo_mounts_by_repo = mounts::repo_mounts_by_repo(workspace, &repo_ids)?;
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
    let mount_table = inspection::load_mount_table_for_read_only_inspection(
        workspace,
        &repos_for_mount_table,
        list_mounts,
    );

    Ok(InspectionWorkspaceSnapshot {
        repos: repos
            .into_iter()
            .map(|(repo_id, repo)| InspectionRepoSnapshot {
                repo_id: repo_id.clone(),
                source: repo.source,
                repo_mounts: repo_mounts_by_repo.remove(&repo_id).unwrap_or_default(),
                inspect_non_materialized_live_mounts: read_only_repo_needs_mount_inspection(
                    workspace,
                    &repo_id,
                    selected_mounts
                        .iter()
                        .find(|(candidate_repo_id, _)| candidate_repo_id == &repo_id)
                        .map_or(0, |(_, repo_mounts)| repo_mounts.len()),
                ),
            })
            .collect(),
        mount_table,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs};

    use super::*;
    use crate::{
        persistence::manifest::{PersistedRepoMount, PersistedRepoRegistration},
        shared::types::{CloneSource, RelativePath},
        test_support,
    };

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-inspection-snapshot-test")
    }

    #[test]
    fn snapshot_skips_mount_table_loading_when_no_selected_repo_needs_inspection() {
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
        let called = Cell::new(false);

        let snapshot = load_workspace_snapshot(&workspace, Some(repo_id), || {
            called.set(true);
            Ok(Vec::new())
        })
        .expect("snapshot");

        assert!(!called.get());
        assert_eq!(snapshot.repos.len(), 1);
        assert!(
            matches!(snapshot.mount_table, MountTableState::Available(entries) if entries.is_empty())
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn snapshot_loads_mount_table_when_selected_repo_needs_inspection() {
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

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);

        let called = Cell::new(false);
        let snapshot = load_workspace_snapshot(&workspace, Some(repo_id.clone()), || {
            called.set(true);
            Ok(vec![mount::MountEntry {
                mount_id: 42,
                preferred_source: workspace.context_path(&repo_id, &rel("ctx")),
                source_aliases: vec![workspace.context_path(&repo_id, &rel("ctx"))],
                mount_point: workspace.repo_path(&repo_id, &rel("target")),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("snapshot");

        assert!(called.get());
        assert_eq!(snapshot.repos.len(), 1);
        assert!(
            matches!(snapshot.mount_table, MountTableState::Available(entries) if entries.len() == 1)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn snapshot_preserves_selected_repo_order() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        for repo_name in ["zulu", "alpha"] {
            let repo_id = RepoId::new(repo_name).expect("valid repo id");
            manifest.repos.insert(
                repo_id.clone(),
                PersistedRepoRegistration {
                    source: CloneSource::new("file:///tmp/source").expect("valid source"),
                },
            );
        }
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let snapshot =
            load_workspace_snapshot(&workspace, None, || panic!("mount table should not load"))
                .expect("snapshot");
        let ordered_ids: Vec<_> = snapshot
            .repos
            .iter()
            .map(|repo| repo.repo_id.as_str().to_string())
            .collect();

        assert_eq!(ordered_ids, vec!["alpha", "zulu"]);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
