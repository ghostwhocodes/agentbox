mod common;

use std::fs;

use common::{
    TempDir, assert_failure, assert_success, fixture_repo, path, run_agentbox, stderr, stdout,
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn explicit_workspace_targets_a_different_directory() {
    let outside = TempDir::new();
    let workspace = TempDir::new();
    let (_fixture, source) = fixture_repo();
    let workspace_path = workspace.path().to_string_lossy().to_string();

    let output = run_agentbox(outside.path(), &["--workspace", &workspace_path, "init"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Initialized agentbox workspace"));

    let output = run_agentbox(
        outside.path(),
        &[
            "--workspace",
            &workspace_path,
            "attach",
            "demo",
            "--source",
            &source,
        ],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("Attached repo `demo`"));

    let output = run_agentbox(
        outside.path(),
        &["--workspace", &workspace_path, "materialize", "demo"],
    );
    assert_success(&output);

    let output = run_agentbox(outside.path(), &["--workspace", &workspace_path, "status"]);
    assert_success(&output);
    assert!(stdout(&output).contains("demo"));

    assert!(path(&workspace, "agentbox.toml").exists());
    assert!(path(&workspace, "context/demo").exists());
    assert!(path(&workspace, "repos/demo/.git").exists());
    assert!(!outside.path().join("agentbox.toml").exists());
    assert!(!outside.path().join("context").exists());
    assert!(!outside.path().join("repos").exists());
}

#[test]
fn explicit_workspace_init_creates_a_new_exact_target_directory() {
    let outside = TempDir::new();
    let root = TempDir::new();
    let workspace_path = root.path().join("new-workspace");
    let workspace_path_str = workspace_path.to_string_lossy().to_string();

    let output = run_agentbox(
        outside.path(),
        &["--workspace", &workspace_path_str, "init"],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("Initialized agentbox workspace"));
    assert!(workspace_path.is_dir());
    assert!(workspace_path.join("agentbox.toml").exists());
    assert!(!outside.path().join("agentbox.toml").exists());
}

#[test]
fn explicit_workspace_uses_exact_root_without_parent_discovery() {
    let outside = TempDir::new();
    let workspace = TempDir::new();
    let workspace_path = workspace.path().to_string_lossy().to_string();

    let output = run_agentbox(outside.path(), &["--workspace", &workspace_path, "init"]);
    assert_success(&output);

    fs::create_dir_all(path(&workspace, "nested")).expect("create nested dir");
    let nested_path = path(&workspace, "nested").to_string_lossy().to_string();

    let output = run_agentbox(outside.path(), &["--workspace", &nested_path, "status"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("no agentbox workspace found"));
}

#[test]
fn omitted_workspace_discovers_parent_from_materialized_repo_directory() {
    let workspace = TempDir::new();
    let (fixture, source) = fixture_repo();
    fs::write(
        fixture.path().join("agentbox.toml"),
        "version = 1\n\n[repos]\n",
    )
    .expect("write repo-local manifest");

    assert_success(&run_agentbox(workspace.path(), &["init"]));
    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    let output = run_agentbox(path(&workspace, "repos/demo").as_path(), &["status"]);
    assert_success(&output);
    assert!(stdout(&output).contains("demo"));
}

#[test]
fn omitted_workspace_discovers_parent_from_context_directory() {
    let workspace = TempDir::new();
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(workspace.path(), &["init"]));
    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    let output = run_agentbox(
        path(&workspace, "context/demo").as_path(),
        &["show", "demo"],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("repo_id: demo"));
}

#[test]
fn omitted_workspace_discovers_workspace_with_plain_repos_parent_directory() {
    let outside = TempDir::new();
    let workspace_parent = outside.path().join("repos");
    fs::create_dir_all(&workspace_parent).expect("create plain repos parent");
    let workspace_path = workspace_parent.join("my-workspace");
    let workspace_path_str = workspace_path.to_string_lossy().to_string();

    let output = run_agentbox(
        outside.path(),
        &["--workspace", &workspace_path_str, "init"],
    );
    assert_success(&output);

    let nested = workspace_path.join("nested/deep");
    fs::create_dir_all(&nested).expect("create nested dir");

    let output = run_agentbox(&nested, &["status"]);
    assert_success(&output);
}

#[cfg(unix)]
#[test]
fn explicit_workspace_canonicalizes_symlinked_prefixes() {
    let outside = TempDir::new();
    let actual_root = TempDir::new();
    let symlink_target = path(&actual_root, "target");
    fs::create_dir_all(&symlink_target).expect("create symlink target");
    symlink(&symlink_target, path(&outside, "link")).expect("create symlink");

    let workspace_path = "./link/../workspace";
    let expected_workspace = path(&actual_root, "workspace");

    let output = run_agentbox(outside.path(), &["--workspace", workspace_path, "init"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Initialized agentbox workspace"));

    assert!(expected_workspace.join("agentbox.toml").exists());
    assert!(!path(&outside, "workspace").join("agentbox.toml").exists());
}
