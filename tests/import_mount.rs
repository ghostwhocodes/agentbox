mod common;

use common::{
    TempDir, assert_failure, assert_success, fixture_repo, init_workspace, path, read,
    run_agentbox, stderr, stdout, write,
};

#[test]
fn import_mount_moves_repo_directory_into_context() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    write(path(&workspace, "repos/demo/.loki/state.txt"), "state");

    let output = run_agentbox(
        workspace.path(),
        &["import-mount", "demo", "--repo-path", ".loki", "--no-mount"],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("Imported `.loki`"));

    assert_eq!(
        read(path(&workspace, "context/demo/.loki/state.txt")),
        "state"
    );
    assert!(!path(&workspace, "repos/demo/.loki").exists());
}

#[test]
fn import_mount_fails_when_repo_path_missing() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    let output = run_agentbox(
        workspace.path(),
        &["import-mount", "demo", "--repo-path", ".loki", "--no-mount"],
    );
    assert_failure(&output);
    assert!(stderr(&output).contains("repo path"));
    assert!(stderr(&output).contains("does not exist for import"));
}

#[test]
fn import_mount_reports_unknown_repo_before_materialization_error() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());

    let output = run_agentbox(
        workspace.path(),
        &[
            "import-mount",
            "missing",
            "--repo-path",
            ".loki",
            "--no-mount",
        ],
    );
    assert_failure(&output);
    let output_text = stderr(&output);
    assert!(output_text.contains("repo `missing` is not registered"));
    assert!(!output_text.contains("not materialized"));
}
