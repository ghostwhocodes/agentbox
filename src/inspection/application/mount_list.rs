use crate::{
    error::Result,
    inspection::{self, RegisteredMountStatus},
    mounts::{infra as mount, ownership},
    shared::types::RepoId,
    workspace::Workspace,
};

use super::load_workspace_snapshot;

pub fn list_mounts(
    workspace: &Workspace,
    requested: Option<RepoId>,
) -> Result<Vec<RegisteredMountStatus>> {
    list_mounts_with_loader(workspace, requested, mount::list_mounts)
}

fn list_mounts_with_loader<F>(
    workspace: &Workspace,
    requested: Option<RepoId>,
    list_mounts: F,
) -> Result<Vec<RegisteredMountStatus>>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    let snapshot = load_workspace_snapshot(workspace, requested, list_mounts)?;
    let mut rows = Vec::new();
    let mount_table = snapshot.mount_table;
    let owned_mounts = ownership::load_advisory(workspace);

    for repo in snapshot.repos {
        let repo_status = inspection::inspect_registered_repo(
            workspace,
            &repo.repo_id,
            &repo.source,
            &repo.repo_mounts,
            &mount_table,
            &owned_mounts,
            repo.inspect_non_materialized_live_mounts,
        )?;

        for (mount_spec, mount_status) in repo.repo_mounts.iter().zip(repo_status.mounts.iter()) {
            rows.push(RegisteredMountStatus {
                repo_id: repo.repo_id.clone(),
                repo_path: mount_spec.repo.clone(),
                context_path: mount_spec.context.clone(),
                source: mount_status.source.clone(),
                target: mount_status.target.clone(),
                materialized: repo_status.materialized,
                active: mount_status.active,
                conflicting_mount: mount_status.conflicting_mount,
                inspection_unavailable: mount_status.inspection_unavailable,
            });
        }
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        error::Error,
        shared::types::{CloneSource, RelativePath},
        test_support,
    };

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-inspection-mount-list-test")
    }

    fn init_repo_with_mount(workspace: &Workspace) -> RepoId {
        crate::workspace::init_workspace(workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");

        let mut manifest =
            crate::persistence::manifest_store::read(workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            crate::persistence::manifest::PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest
            .repo_mounts
            .push(crate::persistence::manifest::PersistedRepoMount {
                repo_id: repo_id.clone(),
                context: rel("ctx"),
                repo: rel("target"),
            });
        crate::persistence::manifest_store::write(workspace, &manifest).expect("save manifest");

        repo_id
    }

    fn materialized_repo_with_mount(workspace: &Workspace) -> RepoId {
        let repo_id = init_repo_with_mount(workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx"))).expect("create context");
        repo_id
    }

    #[test]
    fn list_mounts_skips_mount_table_loading_when_inspection_is_unnecessary() {
        let workspace = test_workspace();
        let repo_id = init_repo_with_mount(&workspace);

        let mounts = list_mounts_with_loader(&workspace, Some(repo_id), || {
            panic!("mount table should not be loaded")
        })
        .expect("list mounts");

        assert_eq!(mounts.len(), 1);
        assert!(!mounts[0].materialized);
        assert!(!mounts[0].inspection_unavailable);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn list_mounts_reports_inspection_unavailable_when_mount_table_cannot_be_read() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);

        let mounts = list_mounts_with_loader(&workspace, Some(repo_id), || {
            Err(Error::io_path(
                "/proc/self/mountinfo",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            ))
        })
        .expect("list mounts");

        assert_eq!(mounts.len(), 1);
        assert!(mounts[0].inspection_unavailable);
        assert!(!mounts[0].active);
        assert!(!mounts[0].conflicting_mount);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
