//! Workspace administration context.
//!
//! Ownership:
//! - workspace discovery and path derivation
//! - workspace initialization and directory repair
//! - workspace-owned directory scanning used by diagnostics

mod admin;

use camino::{Utf8Path, Utf8PathBuf};

use crate::shared::{
    error::{Result, WorkspaceError},
    paths::{resolve_relative, utf8_path_from_std},
    types::{RelativePath, RepoId, TemplateId},
};

pub use admin::init_workspace;
pub(crate) use admin::{
    ChildDirScan, ensure_agentbox_ignored, ensure_context_dir, ensure_repos_dir,
    ensure_repos_ignored, ensure_templates_dir, has_agentbox_ignore, has_repos_ignore,
    scan_context_repo_dirs, scan_materialized_repo_dirs,
};

/// Handle to an agentbox workspace rooted at a specific directory.
///
/// A workspace contains a manifest (`agentbox.toml`), a `context/` directory for
/// mount sources, a `repos/` directory for cloned repositories, and a `templates/`
/// directory for reusable mount configurations.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: Utf8PathBuf,
}

impl Workspace {
    pub fn new(root: Utf8PathBuf) -> Self {
        Self { root }
    }

    pub fn from_std_path(root: std::path::PathBuf) -> Result<Self> {
        Ok(Self::new(utf8_path_from_std(root)?))
    }

    pub fn current() -> Result<Self> {
        Self::from_std_path(std::env::current_dir()?)
    }

    pub fn discover_from(root: Utf8PathBuf) -> Option<Self> {
        let mut current = Some(root.as_path());
        while let Some(path) = current {
            let workspace = Self::new(path.to_path_buf());
            if workspace.manifest_path().exists() && !is_workspace_owned_child_root(path) {
                return Some(workspace);
            }
            current = path.parent();
        }
        None
    }

    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    #[must_use]
    pub fn manifest_path(&self) -> Utf8PathBuf {
        self.root.join("agentbox.toml")
    }

    #[must_use]
    pub fn gitignore_path(&self) -> Utf8PathBuf {
        self.root.join(".gitignore")
    }

    #[must_use]
    pub fn context_dir(&self) -> Utf8PathBuf {
        self.root.join("context")
    }

    #[must_use]
    pub fn repos_dir(&self) -> Utf8PathBuf {
        self.root.join("repos")
    }

    #[must_use]
    pub fn templates_dir(&self) -> Utf8PathBuf {
        self.root.join("templates")
    }

    #[must_use]
    pub fn internal_dir(&self) -> Utf8PathBuf {
        self.root.join(".agentbox")
    }

    #[must_use]
    pub fn internal_state_dir(&self) -> Utf8PathBuf {
        self.internal_dir().join("state")
    }

    #[must_use]
    pub fn mount_ownership_path(&self) -> Utf8PathBuf {
        self.internal_state_dir().join("mount-ownership.toml")
    }

    #[must_use]
    pub fn repo_context_root(&self, repo_id: &RepoId) -> Utf8PathBuf {
        self.context_dir().join(repo_id.as_str())
    }

    #[must_use]
    pub fn repo_root(&self, repo_id: &RepoId) -> Utf8PathBuf {
        self.repos_dir().join(repo_id.as_str())
    }

    #[must_use]
    pub fn template_root(&self, template_id: &TemplateId) -> Utf8PathBuf {
        self.templates_dir().join(template_id.as_str())
    }

    #[must_use]
    pub fn template_manifest_path(&self, template_id: &TemplateId) -> Utf8PathBuf {
        self.template_root(template_id).join("template.toml")
    }

    #[must_use]
    pub fn template_context_root(&self, template_id: &TemplateId) -> Utf8PathBuf {
        self.template_root(template_id).join("context")
    }

    #[must_use]
    pub fn context_path(&self, repo_id: &RepoId, relative: &RelativePath) -> Utf8PathBuf {
        resolve_relative(&self.repo_context_root(repo_id), relative)
    }

    #[must_use]
    pub fn repo_path(&self, repo_id: &RepoId, relative: &RelativePath) -> Utf8PathBuf {
        resolve_relative(&self.repo_root(repo_id), relative)
    }

    pub fn require_initialized(&self) -> Result<()> {
        if self.manifest_path().exists() {
            Ok(())
        } else {
            Err(WorkspaceError::NoWorkspace {
                root: self.root.clone(),
            }
            .into())
        }
    }
}

fn is_workspace_owned_child_root(path: &Utf8Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(parent_name) = parent.file_name() else {
        return false;
    };
    if !matches!(parent_name, "context" | "repos" | "templates" | ".agentbox") {
        return false;
    }

    parent
        .parent()
        .is_some_and(|workspace_root| workspace_root.join("agentbox.toml").exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::{
        error::WorkspaceError,
        persistence::{manifest::PersistedRepoRegistration, manifest_store},
        shared::{types::CloneSource, types::TemplateId},
        test_support::TempDir,
        workspace,
    };

    fn temp_workspace() -> (TempDir, Workspace) {
        let tempdir = TempDir::new("agentbox-workspace-test");
        let workspace = Workspace::new(tempdir.path().to_owned());
        (tempdir, workspace)
    }

    #[test]
    fn require_initialized_reports_missing_manifest() {
        let (_tempdir, workspace) = temp_workspace();

        assert!(matches!(
            workspace.require_initialized(),
            Err(crate::shared::error::Error::Workspace(
                WorkspaceError::NoWorkspace { .. }
            ))
        ));
    }

    #[test]
    fn path_helpers_derive_expected_workspace_locations() {
        let (_tempdir, workspace) = temp_workspace();
        let repo_id = RepoId::new("demo").unwrap();
        let template_id = TemplateId::new("base").unwrap();
        let relative = RelativePath::new("nested/path", "test path").unwrap();

        assert_eq!(
            workspace.repo_context_root(&repo_id),
            workspace.context_dir().join("demo")
        );
        assert_eq!(
            workspace.repo_root(&repo_id),
            workspace.repos_dir().join("demo")
        );
        assert_eq!(
            workspace.template_root(&template_id),
            workspace.templates_dir().join("base")
        );
        assert_eq!(
            workspace.template_manifest_path(&template_id),
            workspace.templates_dir().join("base/template.toml")
        );
        assert_eq!(
            workspace.template_context_root(&template_id),
            workspace.templates_dir().join("base/context")
        );
        assert_eq!(
            workspace.context_path(&repo_id, &relative),
            workspace.context_dir().join("demo/nested/path")
        );
        assert_eq!(
            workspace.repo_path(&repo_id, &relative),
            workspace.repos_dir().join("demo/nested/path")
        );
    }

    #[test]
    fn manifest_store_round_trips_manifest_through_workspace_path() {
        let (_tempdir, workspace) = temp_workspace();
        workspace::init_workspace(&workspace).unwrap();

        let mut manifest = manifest_store::read(&workspace).unwrap();
        let repo_id = RepoId::new("demo").unwrap();
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").unwrap(),
            },
        );
        manifest_store::write(&workspace, &manifest).unwrap();

        let reloaded = manifest_store::read(&workspace).unwrap();
        assert!(reloaded.repos.contains_key(&repo_id));
        assert!(
            fs::read_to_string(workspace.manifest_path())
                .unwrap()
                .contains("[repos.demo]")
        );
    }

    #[test]
    fn discover_from_walks_up_to_parent_manifest() {
        let (_tempdir, workspace) = temp_workspace();
        workspace::init_workspace(&workspace).unwrap();
        let nested = workspace.root().join("repos/demo/deep");
        fs::create_dir_all(&nested).unwrap();

        let discovered = Workspace::discover_from(nested).expect("discover workspace");
        assert_eq!(discovered.root(), workspace.root());
    }

    #[test]
    fn discover_from_ignores_manifest_in_workspace_owned_repo_root() {
        let (_tempdir, workspace) = temp_workspace();
        workspace::init_workspace(&workspace).unwrap();
        let repo_root = workspace.root().join("repos/demo");
        let nested = repo_root.join("deep");
        fs::create_dir_all(&nested).unwrap();
        fs::write(repo_root.join("agentbox.toml"), "version = 1\n\n[repos]\n").unwrap();

        let discovered = Workspace::discover_from(nested).expect("discover workspace");
        assert_eq!(discovered.root(), workspace.root());
    }

    #[test]
    fn discover_from_allows_workspace_under_plain_repos_directory() {
        let tempdir = TempDir::new("agentbox-workspace-test");
        let workspace_root = tempdir.path().join("repos/plain-workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let workspace = Workspace::new(workspace_root);
        workspace::init_workspace(&workspace).unwrap();
        let nested = workspace.root().join("nested/deep");
        fs::create_dir_all(&nested).unwrap();

        let discovered = Workspace::discover_from(nested).expect("discover workspace");
        assert_eq!(discovered.root(), workspace.root());
    }

    #[test]
    fn discover_from_returns_none_when_no_manifest_exists() {
        let (_tempdir, workspace) = temp_workspace();
        let nested = workspace.root().join("repos/demo");
        fs::create_dir_all(&nested).unwrap();

        assert!(Workspace::discover_from(nested).is_none());
    }
}
