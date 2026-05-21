//! Workspace administration services.
//!
//! Ownership:
//! - workspace initialization and layout repair
//! - `.gitignore` policy for workspace-owned generated state
//! - scanning workspace-owned repo directories for diagnostics
//!
//! Dependency guardrail:
//! this module owns workspace-level filesystem policy. Low-level Git, mount,
//! and template concerns stay in their own contexts.

use camino::{Utf8Path, Utf8PathBuf};
use std::{ffi::OsString, fs};

use crate::{
    error::{Error, Result, WorkspaceError},
    paths::describe_non_utf8_os_str,
    persistence::{manifest::PersistedManifest, manifest_store},
    workspace::Workspace,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChildDirScan {
    pub utf8_names: Vec<String>,
    pub non_utf8_entries: Vec<String>,
}

pub fn init_workspace(workspace: &Workspace) -> Result<Utf8PathBuf> {
    if workspace.manifest_path().exists() {
        return Err(WorkspaceError::AlreadyInitialized {
            root: workspace.root().to_path_buf(),
        }
        .into());
    }

    ensure_context_dir(workspace)?;
    ensure_repos_dir(workspace)?;
    ensure_templates_dir(workspace)?;
    manifest_store::write(workspace, &PersistedManifest::default())?;
    ensure_workspace_ignored(workspace)?;
    Ok(workspace.root().to_path_buf())
}

const REPOS_GITIGNORE_ENTRY: &str = "repos/";
const INTERNAL_GITIGNORE_ENTRY: &str = ".agentbox/";

pub(crate) fn ensure_context_dir(workspace: &Workspace) -> Result<()> {
    let path = workspace.context_dir();
    fs::create_dir_all(&path).map_err(|e| Error::io_path(&path, e))?;
    Ok(())
}

pub(crate) fn ensure_repos_dir(workspace: &Workspace) -> Result<()> {
    let path = workspace.repos_dir();
    fs::create_dir_all(&path).map_err(|e| Error::io_path(&path, e))?;
    Ok(())
}

pub(crate) fn ensure_templates_dir(workspace: &Workspace) -> Result<()> {
    let path = workspace.templates_dir();
    fs::create_dir_all(&path).map_err(|e| Error::io_path(&path, e))?;
    Ok(())
}

pub(crate) fn has_repos_ignore(workspace: &Workspace) -> Result<bool> {
    has_gitignore_entry(workspace, REPOS_GITIGNORE_ENTRY)
}

pub(crate) fn has_agentbox_ignore(workspace: &Workspace) -> Result<bool> {
    has_gitignore_entry(workspace, INTERNAL_GITIGNORE_ENTRY)
}

fn has_gitignore_entry(workspace: &Workspace, expected_entry: &str) -> Result<bool> {
    let path = workspace.gitignore_path();
    if !path.exists() {
        return Ok(false);
    }

    let contents = fs::read_to_string(&path).map_err(|e| Error::io_path(&path, e))?;
    Ok(contents
        .lines()
        .map(str::trim)
        .any(|line| line == expected_entry))
}

pub(crate) fn ensure_repos_ignored(workspace: &Workspace) -> Result<()> {
    ensure_gitignore_entries(workspace, &[REPOS_GITIGNORE_ENTRY])
}

pub(crate) fn ensure_agentbox_ignored(workspace: &Workspace) -> Result<()> {
    ensure_gitignore_entries(workspace, &[INTERNAL_GITIGNORE_ENTRY])
}

pub(crate) fn ensure_workspace_ignored(workspace: &Workspace) -> Result<()> {
    ensure_gitignore_entries(
        workspace,
        &[REPOS_GITIGNORE_ENTRY, INTERNAL_GITIGNORE_ENTRY],
    )
}

fn ensure_gitignore_entries(workspace: &Workspace, entries: &[&str]) -> Result<()> {
    let path = workspace.gitignore_path();
    if !path.exists() {
        fs::write(&path, format!("{}\n", entries.join("\n")))
            .map_err(|e| Error::io_path(&path, e))?;
        return Ok(());
    }

    let mut contents = fs::read_to_string(&path).map_err(|e| Error::io_path(&path, e))?;
    let existing_entries = contents
        .lines()
        .map(str::trim)
        .collect::<std::collections::BTreeSet<_>>();
    let missing_entries = entries
        .iter()
        .copied()
        .filter(|entry| !existing_entries.contains(entry))
        .collect::<Vec<_>>();
    if missing_entries.is_empty() {
        return Ok(());
    }

    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    for entry in missing_entries {
        contents.push_str(entry);
        contents.push('\n');
    }
    fs::write(&path, contents).map_err(|e| Error::io_path(&path, e))?;
    Ok(())
}

pub(crate) fn scan_materialized_repo_dirs(workspace: &Workspace) -> Result<ChildDirScan> {
    scan_child_dirs(&workspace.repos_dir())
}

pub(crate) fn scan_context_repo_dirs(workspace: &Workspace) -> Result<ChildDirScan> {
    scan_child_dirs(&workspace.context_dir())
}

fn scan_child_dirs(root: &Utf8Path) -> Result<ChildDirScan> {
    let mut scan = ChildDirScan::default();
    if !root.exists() {
        return Ok(scan);
    }

    let dir_label = root.file_name().unwrap_or(root.as_str()).to_string();
    for entry in fs::read_dir(root).map_err(|e| Error::io_path(root, e))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            push_child_dir_name(&mut scan, &dir_label, entry.file_name());
        }
    }

    Ok(scan)
}

fn push_child_dir_name(scan: &mut ChildDirScan, dir_label: &str, file_name: OsString) {
    match file_name.into_string() {
        Ok(name) => scan.utf8_names.push(name),
        Err(name) => scan
            .non_utf8_entries
            .push(format!("{dir_label}/<{}>", describe_non_utf8_os_str(&name))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;
    #[cfg(unix)]
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    fn temp_workspace() -> (TempDir, Workspace) {
        let tempdir = TempDir::new("agentbox-workspace-admin-test");
        let workspace = Workspace::new(tempdir.path().to_owned());
        (tempdir, workspace)
    }

    #[test]
    fn init_workspace_creates_layout_manifest_and_gitignore() {
        let (_tempdir, workspace) = temp_workspace();

        assert!(matches!(
            workspace.require_initialized(),
            Err(crate::shared::error::Error::Workspace(
                WorkspaceError::NoWorkspace { .. }
            ))
        ));

        init_workspace(&workspace).unwrap();

        assert!(workspace.context_dir().is_dir());
        assert!(workspace.repos_dir().is_dir());
        assert!(workspace.templates_dir().is_dir());
        assert!(workspace.manifest_path().is_file());
        assert_eq!(
            fs::read_to_string(workspace.gitignore_path()).unwrap(),
            "repos/\n.agentbox/\n"
        );
        assert!(has_repos_ignore(&workspace).unwrap());
        assert!(has_agentbox_ignore(&workspace).unwrap());
        assert!(manifest_store::read(&workspace).unwrap().repos.is_empty());

        assert!(matches!(
            init_workspace(&workspace),
            Err(crate::shared::error::Error::Workspace(
                WorkspaceError::AlreadyInitialized { .. }
            ))
        ));
    }

    #[test]
    fn ensure_repos_ignored_appends_once_to_existing_gitignore() {
        let (_tempdir, workspace) = temp_workspace();

        fs::write(workspace.gitignore_path(), "target").unwrap();

        ensure_repos_ignored(&workspace).unwrap();
        ensure_repos_ignored(&workspace).unwrap();

        assert_eq!(
            fs::read_to_string(workspace.gitignore_path()).unwrap(),
            "target\nrepos/\n"
        );
    }

    #[test]
    fn ensure_agentbox_ignored_appends_once_to_existing_gitignore() {
        let (_tempdir, workspace) = temp_workspace();

        fs::write(workspace.gitignore_path(), "target").unwrap();

        ensure_agentbox_ignored(&workspace).unwrap();
        ensure_agentbox_ignored(&workspace).unwrap();

        assert_eq!(
            fs::read_to_string(workspace.gitignore_path()).unwrap(),
            "target\n.agentbox/\n"
        );
    }

    #[test]
    fn has_repos_ignore_returns_false_when_gitignore_is_missing() {
        let (_tempdir, workspace) = temp_workspace();

        assert!(!has_repos_ignore(&workspace).unwrap());
        assert!(!has_agentbox_ignore(&workspace).unwrap());
    }

    #[test]
    fn ensure_repos_ignored_leaves_existing_entry_unchanged() {
        let (_tempdir, workspace) = temp_workspace();
        fs::write(workspace.gitignore_path(), "repos/\n").unwrap();

        ensure_repos_ignored(&workspace).unwrap();

        assert_eq!(
            fs::read_to_string(workspace.gitignore_path()).unwrap(),
            "repos/\n"
        );
    }

    #[test]
    fn scan_context_repo_dirs_lists_utf8_directories_and_ignores_files() {
        let (_tempdir, workspace) = temp_workspace();

        ensure_context_dir(&workspace).unwrap();
        fs::create_dir_all(workspace.context_dir().join("demo")).unwrap();
        fs::write(workspace.context_dir().join("note.txt"), "ignore").unwrap();

        let scan = scan_context_repo_dirs(&workspace).unwrap();

        assert_eq!(scan.utf8_names, vec!["demo".to_string()]);
        assert!(scan.non_utf8_entries.is_empty());
    }

    #[test]
    fn ensure_workspace_dirs_propagate_filesystem_errors() {
        let (_tempdir, workspace) = temp_workspace();
        fs::write(workspace.context_dir(), "not a dir").unwrap();
        fs::write(workspace.repos_dir(), "not a dir").unwrap();
        fs::write(workspace.templates_dir(), "not a dir").unwrap();

        assert!(matches!(
            ensure_context_dir(&workspace),
            Err(Error::IoPath { .. })
        ));
        assert!(matches!(
            ensure_repos_dir(&workspace),
            Err(Error::IoPath { .. })
        ));
        assert!(matches!(
            ensure_templates_dir(&workspace),
            Err(Error::IoPath { .. })
        ));
    }

    #[test]
    fn gitignore_helpers_propagate_read_errors() {
        let (_tempdir, workspace) = temp_workspace();
        fs::create_dir_all(workspace.gitignore_path()).unwrap();

        assert!(matches!(
            has_repos_ignore(&workspace),
            Err(Error::IoPath { .. })
        ));
        assert!(matches!(
            ensure_repos_ignored(&workspace),
            Err(Error::IoPath { .. })
        ));
    }

    #[test]
    fn scan_materialized_repo_dirs_propagates_read_dir_errors() {
        let (_tempdir, workspace) = temp_workspace();
        fs::write(workspace.repos_dir(), "not a dir").unwrap();

        assert!(matches!(
            scan_materialized_repo_dirs(&workspace),
            Err(Error::IoPath { .. })
        ));
    }

    #[test]
    fn scan_materialized_repo_dirs_is_empty_when_root_is_missing() {
        let (_tempdir, workspace) = temp_workspace();

        let scan = scan_materialized_repo_dirs(&workspace).unwrap();

        assert!(scan.utf8_names.is_empty());
        assert!(scan.non_utf8_entries.is_empty());
    }

    #[test]
    fn push_child_dir_name_records_utf8_names() {
        let mut scan = ChildDirScan::default();

        push_child_dir_name(&mut scan, "repos", OsString::from("demo"));

        assert_eq!(scan.utf8_names, vec!["demo".to_string()]);
        assert!(scan.non_utf8_entries.is_empty());
    }

    #[test]
    fn scan_materialized_repo_dirs_lists_utf8_directories() {
        let (_tempdir, workspace) = temp_workspace();

        ensure_repos_dir(&workspace).unwrap();
        fs::create_dir_all(workspace.repos_dir().join("demo")).unwrap();

        let scan = scan_materialized_repo_dirs(&workspace).unwrap();
        assert_eq!(scan.utf8_names, vec!["demo".to_string()]);
        assert!(scan.non_utf8_entries.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn push_child_dir_name_records_non_utf8_entries() {
        let mut scan = ChildDirScan::default();
        let non_utf8 = OsString::from_vec(vec![0x66, 0x6f, 0x80]);

        push_child_dir_name(&mut scan, "repos", non_utf8);

        assert!(scan.utf8_names.is_empty());
        assert_eq!(scan.non_utf8_entries.len(), 1);
        assert!(scan.non_utf8_entries[0].starts_with("repos/<"));
    }
}
