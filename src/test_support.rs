use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};

use crate::{paths::utf8_path_from_std, workspace::Workspace};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub(crate) struct TempDir {
    path: Utf8PathBuf,
}

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        let unique = format!(
            "{label}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("current time")
                .as_nanos(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create tempdir");
        Self {
            path: utf8_path_from_std(path).expect("utf-8 tempdir"),
        }
    }

    pub(crate) fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub(crate) fn std_path(&self) -> &std::path::Path {
        self.path.as_std_path()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn test_workspace(label: &str) -> Workspace {
    let path = std::env::temp_dir().join(format!(
        "{label}-workspace-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time")
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
    ));
    fs::create_dir_all(&path).expect("create tempdir");
    Workspace::from_std_path(path).expect("utf-8 workspace path")
}

pub(crate) fn git(repo_dir: &Utf8Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .env("GIT_AUTHOR_NAME", "Agentbox Test")
        .env("GIT_AUTHOR_EMAIL", "agentbox@example.com")
        .env("GIT_COMMITTER_NAME", "Agentbox Test")
        .env("GIT_COMMITTER_EMAIL", "agentbox@example.com")
        .current_dir(repo_dir.as_std_path())
        .status()
        .expect("run git");
    assert!(
        status.success(),
        "git command failed: git {}",
        args.join(" ")
    );
}

pub(crate) fn init_git_repo(path: &Utf8Path) {
    git(path, &["init", "--initial-branch", "main"]);
}

pub(crate) fn fixture_repo(label: &str) -> (TempDir, String) {
    let repo_dir = TempDir::new(label);
    init_git_repo(repo_dir.path());
    fs::write(repo_dir.path().join("README.md"), "# fixture\n").expect("write fixture");
    git(repo_dir.path(), &["add", "README.md"]);
    git(repo_dir.path(), &["commit", "-m", "initial"]);
    let url = format!("file://{}", repo_dir.path());
    (repo_dir, url)
}
