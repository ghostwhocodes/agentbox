// Each integration test binary includes this module but may only use a subset
// of helpers, causing false-positive dead_code warnings.
#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new() -> Self {
        let unique = format!(
            "agentbox-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("current time")
                .as_nanos(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create tempdir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn run_agentbox(dir: &Path, args: &[&str]) -> Output {
    run_agentbox_with_env(dir, args, &[])
}

pub fn run_agentbox_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agentbox"))
        .args(args)
        .envs(envs.iter().copied())
        .current_dir(dir)
        .output()
        .expect("run agentbox")
}

pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

pub fn init_workspace(dir: &Path) {
    let output = run_agentbox(dir, &["init"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Initialized agentbox workspace"));
}

pub fn fixture_repo() -> (TempDir, String) {
    let repo_dir = TempDir::new();
    git(repo_dir.path(), &["init", "--initial-branch", "main"]);
    fs::write(repo_dir.path().join("README.md"), "# fixture\n").expect("write fixture");
    git(repo_dir.path(), &["add", "README.md"]);
    git(
        repo_dir.path(),
        &[
            "-c",
            "user.name=Agentbox Test",
            "-c",
            "user.email=agentbox@example.com",
            "commit",
            "-m",
            "initial",
        ],
    );
    let url = format!("file://{}", repo_dir.path().display());
    (repo_dir, url)
}

pub fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, contents).expect("write file");
}

pub fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).expect("read file")
}

pub fn path(dir: &TempDir, relative: &str) -> PathBuf {
    dir.path().join(relative)
}

pub fn git(repo_dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .env("GIT_AUTHOR_NAME", "Agentbox Test")
        .env("GIT_AUTHOR_EMAIL", "agentbox@example.com")
        .env("GIT_COMMITTER_NAME", "Agentbox Test")
        .env("GIT_COMMITTER_EMAIL", "agentbox@example.com")
        .current_dir(repo_dir)
        .status()
        .expect("run git");
    assert!(
        status.success(),
        "git command failed: git {}",
        args.join(" ")
    );
}
