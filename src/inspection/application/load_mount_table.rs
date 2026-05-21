use crate::{
    error::{Error, Result},
    inspection::{MountTableState, ObservedMount},
    mounts::infra as mount,
    shared::types::RepoId,
    workspace::Workspace,
};
use std::{fs, io::ErrorKind};

pub(crate) fn load_mount_table_for_read_only_inspection(
    workspace: &Workspace,
    repos: &[(RepoId, usize)],
    list_mounts: impl FnOnce() -> Result<Vec<mount::MountEntry>>,
) -> MountTableState {
    if !needs_read_only_mount_inspection(workspace, repos) {
        return MountTableState::Available(Vec::new());
    }

    match list_mounts() {
        Ok(entries) => MountTableState::Available(observed_mounts(entries)),
        Err(error) => MountTableState::Unavailable(error.to_string()),
    }
}

pub(crate) fn load_mount_table_for_mutating_workflow(
    workspace: &Workspace,
    repos: &[(RepoId, usize)],
    list_mounts: impl FnOnce() -> Result<Vec<mount::MountEntry>>,
) -> Result<Vec<ObservedMount>> {
    if !needs_mutating_mount_inspection(workspace, repos)? {
        return Ok(Vec::new());
    }

    Ok(observed_mounts(list_mounts()?))
}

pub(crate) fn observed_mounts(
    entries: impl IntoIterator<Item = mount::MountEntry>,
) -> Vec<ObservedMount> {
    entries
        .into_iter()
        .map(|entry| ObservedMount {
            mount_id: entry.mount_id,
            preferred_source: entry.preferred_source,
            source_aliases: entry.source_aliases,
            target: entry.mount_point,
            filesystem_type: entry.filesystem_type,
        })
        .collect()
}

pub(crate) fn read_only_repo_needs_mount_inspection(
    workspace: &Workspace,
    repo_id: &RepoId,
    mount_count: usize,
) -> bool {
    mount_count > 0 && repo_root_requires_mount_inspection(workspace, repo_id).unwrap_or(true)
}

fn needs_read_only_mount_inspection(workspace: &Workspace, repos: &[(RepoId, usize)]) -> bool {
    repos.iter().any(|(repo_id, mount_count)| {
        read_only_repo_needs_mount_inspection(workspace, repo_id, *mount_count)
    })
}

fn needs_mutating_mount_inspection(
    workspace: &Workspace,
    repos: &[(RepoId, usize)],
) -> Result<bool> {
    for (repo_id, mount_count) in repos {
        if *mount_count == 0 {
            continue;
        }

        if repo_root_requires_mount_inspection(workspace, repo_id)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn repo_root_requires_mount_inspection(workspace: &Workspace, repo_id: &RepoId) -> Result<bool> {
    let repo_root = workspace.repo_root(repo_id);
    match fs::symlink_metadata(&repo_root) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io_path(&repo_root, error)),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::{cell::Cell, fs};

    use camino::Utf8PathBuf;

    use super::*;
    use crate::test_support;

    #[test]
    fn skips_mount_table_loading_when_no_repo_needs_inspection() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let called = Cell::new(false);

        let mount_table =
            load_mount_table_for_read_only_inspection(&workspace, &[(repo_id, 1)], || {
                called.set(true);
                Ok(Vec::new())
            });

        assert!(!called.get());
        assert!(matches!(mount_table, MountTableState::Available(entries) if entries.is_empty()));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn returns_available_mount_entries_when_loading_succeeds() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let expected = ObservedMount {
            mount_id: 42,
            preferred_source: Utf8PathBuf::from("/workspace/context/demo/ctx"),
            source_aliases: vec![Utf8PathBuf::from("/workspace/context/demo/ctx")],
            target: Utf8PathBuf::from("/workspace/repos/demo/target"),
            filesystem_type: "bind".to_string(),
        };
        let loaded = mount::MountEntry {
            mount_id: 42,
            preferred_source: Utf8PathBuf::from("/workspace/context/demo/ctx"),
            source_aliases: vec![Utf8PathBuf::from("/workspace/context/demo/ctx")],
            mount_point: Utf8PathBuf::from("/workspace/repos/demo/target"),
            filesystem_type: "bind".to_string(),
        };

        let mount_table =
            load_mount_table_for_read_only_inspection(&workspace, &[(repo_id, 1)], || {
                Ok(vec![loaded])
            });

        assert!(
            matches!(mount_table, MountTableState::Available(entries) if entries == vec![expected])
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn returns_unavailable_when_mount_table_loading_fails() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);

        let mount_table =
            load_mount_table_for_read_only_inspection(&workspace, &[(repo_id, 1)], || {
                Err(std::io::Error::other("boom").into())
            });

        assert!(
            matches!(mount_table, MountTableState::Unavailable(message) if message.contains("boom"))
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn read_only_inspection_loads_mount_table_for_non_materialized_repo_root_with_mounts() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let repo_root = workspace.repo_root(&repo_id);
        let called = Cell::new(false);

        fs::create_dir_all(&repo_root).expect("create plain repo root");

        let mount_table =
            load_mount_table_for_read_only_inspection(&workspace, &[(repo_id, 1)], || {
                called.set(true);
                Ok(Vec::new())
            });

        assert!(called.get());
        assert!(matches!(mount_table, MountTableState::Available(entries) if entries.is_empty()));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[cfg(unix)]
    #[test]
    fn read_only_inspection_loads_mount_table_for_broken_repo_root_symlink_with_mounts() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let repo_root = workspace.repo_root(&repo_id);
        let called = Cell::new(false);

        fs::create_dir_all(repo_root.parent().expect("repo root parent"))
            .expect("create repos dir");
        symlink("/path/that/does/not/exist", &repo_root).expect("create broken repo-root symlink");

        let mount_table =
            load_mount_table_for_read_only_inspection(&workspace, &[(repo_id, 1)], || {
                called.set(true);
                Ok(Vec::new())
            });

        assert!(called.get());
        assert!(matches!(mount_table, MountTableState::Available(entries) if entries.is_empty()));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mutating_workflow_propagates_mount_table_loading_failure() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);

        let error = load_mount_table_for_mutating_workflow(&workspace, &[(repo_id, 1)], || {
            Err(std::io::Error::other("boom").into())
        })
        .expect_err("mutating workflow should fail closed");

        assert!(error.to_string().contains("boom"));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mutating_workflow_skips_mount_table_for_never_materialized_repo_with_mounts() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let called = Cell::new(false);
        let entries = load_mount_table_for_mutating_workflow(&workspace, &[(repo_id, 1)], || {
            called.set(true);
            Ok(Vec::new())
        })
        .expect("never-materialized repos should skip mount inspection");

        assert!(!called.get());
        assert!(entries.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mutating_workflow_loads_mount_table_for_non_materialized_repo_root_with_mounts() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let repo_root = workspace.repo_root(&repo_id);
        let called = Cell::new(false);
        let expected = ObservedMount {
            mount_id: 42,
            preferred_source: Utf8PathBuf::from("/workspace/context/demo/ctx"),
            source_aliases: vec![Utf8PathBuf::from("/workspace/context/demo/ctx")],
            target: Utf8PathBuf::from("/workspace/repos/demo/target"),
            filesystem_type: "bind".to_string(),
        };

        fs::create_dir_all(&repo_root).expect("create plain repo root");

        let entries = load_mount_table_for_mutating_workflow(&workspace, &[(repo_id, 1)], || {
            called.set(true);
            Ok(vec![mount::MountEntry {
                mount_id: expected.mount_id,
                preferred_source: expected.preferred_source.clone(),
                source_aliases: expected.source_aliases.clone(),
                mount_point: expected.target.clone(),
                filesystem_type: expected.filesystem_type.clone(),
            }])
        })
        .expect("broken repo roots should still load mount inspection");

        assert!(called.get());
        assert_eq!(entries, vec![expected]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[cfg(unix)]
    #[test]
    fn mutating_workflow_loads_mount_table_for_broken_repo_root_symlink_with_mounts() {
        let workspace = test_support::test_workspace("agentbox-inspection-load-mount-table-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let repo_root = workspace.repo_root(&repo_id);
        let called = Cell::new(false);

        fs::create_dir_all(repo_root.parent().expect("repo root parent"))
            .expect("create repos dir");
        symlink("/path/that/does/not/exist", &repo_root).expect("create broken repo-root symlink");

        let entries = load_mount_table_for_mutating_workflow(&workspace, &[(repo_id, 1)], || {
            called.set(true);
            Ok(Vec::new())
        })
        .expect("broken symlink should still trigger mount inspection");

        assert!(called.get());
        assert!(entries.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }
}
