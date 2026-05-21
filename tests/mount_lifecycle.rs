mod common;

use common::{
    TempDir, assert_failure, assert_success, fixture_repo, init_workspace, path, read,
    run_agentbox, stderr, stdout, write,
};
use serde_json::Value;

#[test]
fn remove_mount_updates_manifest_and_preserves_context_data() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            ".loki",
        ],
    ));
    write(path(&workspace, "context/demo/ai/state.txt"), "state");

    let output = run_agentbox(
        workspace.path(),
        &["remove-mount", "demo", "--repo-path", ".loki"],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("Removed mount for repo path `.loki`"));

    let manifest = read(workspace.path().join("agentbox.toml"));
    assert!(!manifest.contains("repo = \".loki\""));
    assert_eq!(read(path(&workspace, "context/demo/ai/state.txt")), "state");
}

#[test]
fn edit_mount_updates_manifest_by_repo_path_only() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            ".loki",
        ],
    ));
    write(path(&workspace, "context/demo/ai/state.txt"), "state");

    let output = run_agentbox(
        workspace.path(),
        &[
            "edit-mount",
            "demo",
            "--repo-path",
            ".loki",
            "--new-context-path",
            "assistant",
        ],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("Updated mount for repo path `.loki`"));
    assert!(stdout(&output).contains("to context `assistant`"));

    let manifest = read(workspace.path().join("agentbox.toml"));
    assert!(manifest.contains("context = \"assistant\""));
    assert!(manifest.contains("repo = \".loki\""));
    assert_eq!(read(path(&workspace, "context/demo/ai/state.txt")), "state");
    assert!(!path(&workspace, "context/demo/assistant").exists());
}

#[test]
fn edit_mount_rejects_unknown_repo_path() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            ".loki",
        ],
    ));

    let output = run_agentbox(
        workspace.path(),
        &[
            "edit-mount",
            "demo",
            "--repo-path",
            ".missing",
            "--new-context-path",
            "assistant",
        ],
    );
    assert_failure(&output);
    assert!(stderr(&output).contains("has no registered mount for repo path `.missing`"));
}

#[test]
fn list_mounts_reports_all_repos_and_optional_filter() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "other", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            ".loki",
        ],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "other",
            "--context-path",
            "docs",
            "--repo-path",
            "docs",
        ],
    ));

    let output = run_agentbox(workspace.path(), &["list-mounts"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("demo\trepo_path=.loki\tcontext_path=ai"));
    assert!(output_text.contains("other\trepo_path=docs\tcontext_path=docs"));

    let output = run_agentbox(workspace.path(), &["list-mounts", "demo"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("demo\trepo_path=.loki\tcontext_path=ai"));
    assert!(!output_text.contains("other\trepo_path=docs"));
}

#[test]
fn list_mounts_json_reports_runtime_fields() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            ".loki",
        ],
    ));

    let output = run_agentbox(workspace.path(), &["list-mounts", "--json"]);
    assert_success(&output);

    let json: Value = serde_json::from_str(&stdout(&output)).expect("parse json");
    assert_eq!(json["mounts"].as_array().expect("mounts array").len(), 1);
    assert_eq!(json["mounts"][0]["repo_id"], Value::from("demo"));
    assert_eq!(json["mounts"][0]["repo_path"], Value::from(".loki"));
    assert_eq!(json["mounts"][0]["context_path"], Value::from("ai"));
    assert_eq!(json["mounts"][0]["materialized"], Value::Bool(false));
    assert_eq!(json["mounts"][0]["active"], Value::Bool(false));
    assert_eq!(json["mounts"][0]["conflicting_mount"], Value::Bool(false));
    assert_eq!(
        json["mounts"][0]["inspection_unavailable"],
        Value::Bool(false)
    );
}

#[test]
fn mount_failure_from_safety_check_does_not_create_missing_context_dir() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            ".loki",
        ],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    write(path(&workspace, "repos/demo/.loki/state.txt"), "repo");
    assert!(!path(&workspace, "context/demo/ai").exists());

    let output = run_agentbox(workspace.path(), &["mount", "demo"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("already contains different files"));
    assert!(!path(&workspace, "context/demo/ai").exists());
}
