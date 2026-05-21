use camino::Utf8Path;
use std::process::Command;

use crate::{
    error::{InfraGitError, Result},
    shared::types::CloneSource,
};

pub fn clone_repo(source: &CloneSource, destination: &Utf8Path) -> Result<()> {
    let output = Command::new("git")
        .arg("clone")
        .arg(source.as_str())
        .arg(destination.as_str())
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        Err(InfraGitError::CloneFailed {
            clone_source: source.clone(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        }
        .into())
    }
}

pub fn is_git_repo(path: &Utf8Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(path.as_str())
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{shared::error::Error, test_support};

    #[test]
    fn clone_repo_returns_clone_failed_for_invalid_source() {
        let temp = test_support::TempDir::new("agentbox-registry-git-test");
        let source =
            CloneSource::new("file:///tmp/agentbox-repo-that-does-not-exist").expect("source");
        let destination = temp.path().join("clone");

        let error = clone_repo(&source, &destination).expect_err("clone should fail");

        assert!(matches!(
            error,
            Error::InfraGit(InfraGitError::CloneFailed {
                clone_source,
                stderr,
            }) if clone_source == source && !stderr.is_empty()
        ));
    }

    #[test]
    fn is_git_repo_returns_false_for_plain_directory() {
        let temp = test_support::TempDir::new("agentbox-registry-git-test");

        assert!(!is_git_repo(temp.path()));
    }
}
