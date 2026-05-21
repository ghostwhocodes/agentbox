mod common;

use std::fs;

use common::{TempDir, assert_success, fixture_repo, init_workspace, path, run_agentbox, stdout};
use serde_json::Value;

/// When no repo needs mount inspection (no mounts configured, not materialized),
/// status/show/doctor should succeed without reading the mount table.
#[test]
fn status_show_doctor_succeed_without_mount_inspection() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    // Attach a repo but don't add any mounts and don't materialize
    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    let output = run_agentbox(workspace.path(), &["status"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("mounted=no-mounts"));

    let output = run_agentbox(workspace.path(), &["show", "demo"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("mount_state: no-mounts"));

    let output = run_agentbox(workspace.path(), &["doctor"]);
    assert_success(&output);
}

/// When mounts exist but repo is not materialized, mount inspection is not needed.
#[test]
fn status_show_with_mounts_but_not_materialized_skips_mount_table() {
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
            "ai",
        ],
    ));

    // Not materialized — should skip mount table reading
    let output = run_agentbox(workspace.path(), &["status"]);
    assert_success(&output);
    assert!(stdout(&output).contains("mounted=unmounted"));

    let output = run_agentbox(workspace.path(), &["show", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("mount_state: unmounted"));
}

#[test]
fn status_and_show_report_repo_state() {
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
            "ai",
        ],
    ));

    let output = run_agentbox(workspace.path(), &["status"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("demo\tsource=file://"));
    assert!(output_text.contains("materialized=false"));
    assert!(output_text.contains("mounted=unmounted"));

    let output = run_agentbox(workspace.path(), &["show", "demo"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("repo_id: demo"));
    assert!(output_text.contains("mount_state: unmounted"));
    assert!(output_text.contains("context_root:"));
}

#[test]
fn status_and_show_do_not_treat_plain_repo_directory_as_materialized() {
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
            "ai",
        ],
    ));

    fs::create_dir_all(path(&workspace, "repos/demo")).expect("create plain repo dir");

    let output = run_agentbox(workspace.path(), &["status"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("materialized=false"));
    assert!(output_text.contains("mounted=unmounted"));

    let output = run_agentbox(workspace.path(), &["show", "demo"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("materialized: false"));
    assert!(output_text.contains("mount_state: unmounted"));
}

#[test]
fn status_json_reports_repo_state() {
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
            "ai",
        ],
    ));

    let output = run_agentbox(workspace.path(), &["status", "--json"]);
    assert_success(&output);

    let json: Value = serde_json::from_str(&stdout(&output)).expect("parse json");
    assert_eq!(json["repos"].as_array().expect("repos array").len(), 1);
    assert_eq!(json["repos"][0]["repo_id"], Value::from("demo"));
    assert_eq!(json["repos"][0]["materialized"], Value::Bool(false));
    assert_eq!(json["repos"][0]["mount_state"], Value::from("unmounted"));
}

#[test]
fn show_json_reports_repo_details() {
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
            "ai",
        ],
    ));

    let output = run_agentbox(workspace.path(), &["show", "demo", "--json"]);
    assert_success(&output);

    let json: Value = serde_json::from_str(&stdout(&output)).expect("parse json");
    let expected_source = path(&workspace, "context/demo/ai").display().to_string();
    let expected_target = path(&workspace, "repos/demo/ai").display().to_string();
    assert_eq!(json["repo_id"], Value::from("demo"));
    assert_eq!(json["materialized"], Value::Bool(false));
    assert_eq!(json["mount_state"], Value::from("unmounted"));
    assert_eq!(json["mounts"][0]["source"], Value::from(expected_source));
    assert_eq!(json["mounts"][0]["target"], Value::from(expected_target));
    assert_eq!(json["mounts"][0]["active"], Value::Bool(false));
    assert_eq!(json["mounts"][0]["conflicting_mount"], Value::Bool(false));
    assert_eq!(
        json["mounts"][0]["inspection_unavailable"],
        Value::Bool(false)
    );
}
