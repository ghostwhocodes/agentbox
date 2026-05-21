use std::fs;

use camino::Utf8PathBuf;

use crate::{
    persistence::manifest::{PersistedRepoMount, PersistedRepoRegistration},
    shared::types::{CloneSource, RelativePath, RepoId},
    test_support,
    workspace::Workspace,
};

pub(super) fn rel(path: &str) -> RelativePath {
    RelativePath::new(path, "test path").expect("valid relative path")
}

pub(super) fn read_to_string(path: Utf8PathBuf) -> String {
    fs::read_to_string(path).expect("read file")
}

pub(super) fn test_workspace() -> Workspace {
    test_support::test_workspace("agentbox-app-mount-test")
}

pub(super) fn init_git_repo(path: &camino::Utf8Path) {
    test_support::init_git_repo(path);
}

pub(super) fn init_repo(workspace: &Workspace) -> RepoId {
    crate::workspace::init_workspace(workspace).expect("init workspace");
    let repo_id = RepoId::new("demo").expect("valid repo id");
    let mut manifest = crate::persistence::manifest_store::read(workspace).expect("load manifest");
    manifest.repos.insert(
        repo_id.clone(),
        PersistedRepoRegistration {
            source: CloneSource::new("file:///tmp/source").expect("valid source"),
        },
    );
    crate::persistence::manifest_store::write(workspace, &manifest).expect("save manifest");
    repo_id
}

pub(super) fn init_repo_with_mount(workspace: &Workspace) -> RepoId {
    let repo_id = init_repo(workspace);
    let mut manifest = crate::persistence::manifest_store::read(workspace).expect("load manifest");
    manifest.repo_mounts.push(PersistedRepoMount {
        repo_id: repo_id.clone(),
        context: rel("ctx"),
        repo: rel("target"),
    });
    crate::persistence::manifest_store::write(workspace, &manifest).expect("save manifest");
    repo_id
}

pub(super) fn materialized_repo_with_mount(workspace: &Workspace) -> RepoId {
    let repo_id = init_repo_with_mount(workspace);
    let repo_root = workspace.repo_root(&repo_id);
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_git_repo(&repo_root);
    fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx"))).expect("create context");
    repo_id
}
