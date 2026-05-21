//! Repo registry and materialization workflows.
//!
//! Ownership in the target architecture:
//! - repo registration support and working-tree lifecycle
//!
//! Dependency guardrail:
//! this module may orchestrate shared persistence contracts, context-owned
//! adapters, and `workspace`, but it
//! should not own read-only inspection flows or mount-table interpretation.
mod attach;
mod dematerialize;
pub(crate) mod git;
mod materialize;
mod store;

use camino::Utf8PathBuf;
use std::fs;

use crate::{
    persistence::manifest::PersistedRepoRegistration,
    shared::error::{Error, Result, WorkspaceError},
    shared::types::{CloneSource, RepoId},
    workspace::Workspace,
};

pub use attach::{DetachedRepo, attach_repo, detach_repo};
pub use dematerialize::{DematerializeOutcome, dematerialize_repos};
pub use materialize::{MaterializeOutcome, materialize_repos};
#[cfg(test)]
pub(crate) use store::register_repo;
pub(crate) use store::{
    register_repo_tx, registered_repo, remove_registered_repo_record_tx, selected_registered_repos,
    selected_repo_ids,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredRepo {
    pub source: CloneSource,
}

pub(crate) fn is_repo_materialized(workspace: &Workspace, repo_id: &RepoId) -> bool {
    let repo_root = workspace.repo_root(repo_id);
    repo_root.exists() && git::is_git_repo(&repo_root)
}

pub(crate) fn ensure_repo_context_root(
    workspace: &Workspace,
    repo_id: &RepoId,
) -> Result<Utf8PathBuf> {
    let path = workspace.repo_context_root(repo_id);
    fs::create_dir_all(&path).map_err(|e| Error::io_path(&path, e))?;
    Ok(path)
}

pub(crate) fn remove_repo_root(workspace: &Workspace, repo_id: &RepoId) -> Result<()> {
    let path = workspace.repo_root(repo_id);
    fs::remove_dir_all(&path).map_err(|e| Error::io_path(&path, e))?;
    Ok(())
}

pub(crate) fn require_materialized_repo(
    workspace: &Workspace,
    repo_id: &RepoId,
) -> Result<Utf8PathBuf> {
    let repo_root = workspace.repo_root(repo_id);
    if !is_repo_materialized(workspace, repo_id) {
        return Err(WorkspaceError::RepoNotMaterialized {
            repo_id: repo_id.clone(),
        }
        .into());
    }
    Ok(repo_root)
}

#[cfg(test)]
mod repo_fs_tests {
    use super::*;
    use crate::{
        error::WorkspaceError,
        shared::types::RelativePath,
        test_support::{self, TempDir},
        workspace,
    };

    use std::fs;

    fn temp_workspace() -> (TempDir, Workspace) {
        let tempdir = TempDir::new("agentbox-repo-fs-test");
        let workspace = Workspace::new(tempdir.path().to_owned());
        (tempdir, workspace)
    }

    fn repo_id() -> RepoId {
        RepoId::new("demo").unwrap()
    }

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").unwrap()
    }

    #[test]
    fn ensure_repo_context_root_creates_repo_specific_context_directory() {
        let (_tempdir, workspace) = temp_workspace();
        let repo_id = repo_id();

        let context_root = ensure_repo_context_root(&workspace, &repo_id).unwrap();

        assert!(context_root.exists());
        assert_eq!(
            workspace::scan_context_repo_dirs(&workspace)
                .unwrap()
                .utf8_names,
            vec!["demo"]
        );
        assert_eq!(
            workspace.context_path(&repo_id, &rel("nested")),
            context_root.join("nested")
        );
    }

    #[test]
    fn require_materialized_repo_accepts_git_worktrees() {
        let (_tempdir, workspace) = temp_workspace();
        let repo_id = repo_id();
        let repo_root = workspace.repo_root(&repo_id);

        assert!(!is_repo_materialized(&workspace, &repo_id));

        fs::create_dir_all(&repo_root).unwrap();
        assert!(!is_repo_materialized(&workspace, &repo_id));
        assert!(matches!(
            require_materialized_repo(&workspace, &repo_id),
            Err(crate::shared::error::Error::Workspace(
                WorkspaceError::RepoNotMaterialized { .. }
            ))
        ));

        test_support::git(&repo_root, &["init", "--initial-branch", "main"]);
        assert!(is_repo_materialized(&workspace, &repo_id));
        assert_eq!(
            require_materialized_repo(&workspace, &repo_id).unwrap(),
            repo_root
        );
    }

    #[test]
    fn remove_repo_root_deletes_materialized_repo_tree() {
        let (_tempdir, workspace) = temp_workspace();
        workspace::init_workspace(&workspace).unwrap();
        let repo_id = repo_id();
        let repo_root = workspace.repo_root(&repo_id);

        fs::create_dir_all(repo_root.join("nested")).unwrap();
        fs::write(repo_root.join("nested/state.txt"), "state").unwrap();

        remove_repo_root(&workspace, &repo_id).unwrap();
        assert!(!repo_root.exists());
    }
}

#[cfg(test)]
mod test_support {
    use crate::{
        persistence::manifest::{PersistedRepoMount, PersistedRepoRegistration},
        shared::{
            mount_spec::MountSpec,
            types::{CloneSource, RelativePath, RepoId},
        },
        test_support,
        workspace::Workspace,
    };

    pub(super) fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    pub(super) fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-app-repo-test")
    }

    pub(super) fn init_git_repo(path: &camino::Utf8Path) {
        test_support::init_git_repo(path);
    }

    pub(super) fn insert_repo(workspace: &Workspace, repo_id: &RepoId, mounts: Vec<MountSpec>) {
        let mut manifest =
            crate::persistence::manifest_store::read(workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest
            .repo_mounts
            .extend(mounts.into_iter().map(|mount| PersistedRepoMount {
                repo_id: repo_id.clone(),
                context: mount.context,
                repo: mount.repo,
            }));
        crate::persistence::manifest_store::write(workspace, &manifest).expect("save manifest");
    }

    pub(super) fn materialized_repo_with_mount(workspace: &Workspace) -> RepoId {
        crate::workspace::init_workspace(workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo(
            workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
        );

        let repo_root = workspace.repo_root(&repo_id);
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        std::fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx")))
            .expect("create mount source");

        repo_id
    }
}

impl From<PersistedRepoRegistration> for RegisteredRepo {
    fn from(value: PersistedRepoRegistration) -> Self {
        Self {
            source: value.source,
        }
    }
}

impl From<RegisteredRepo> for PersistedRepoRegistration {
    fn from(value: RegisteredRepo) -> Self {
        Self {
            source: value.source,
        }
    }
}
