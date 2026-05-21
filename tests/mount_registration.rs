mod common;

use common::{
    TempDir, assert_failure, assert_success, fixture_repo, init_workspace, read, run_agentbox,
    stderr, stdout, write,
};

#[test]
fn add_mount_rejects_duplicates() {
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

    let output = run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            ".loki",
        ],
    );
    assert_failure(&output);
    assert!(stderr(&output).contains("conflicts"));
}

#[test]
fn add_mount_updates_manifest() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    let output = run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            "ai",
        ],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("Registered mount"));

    let manifest = read(workspace.path().join("agentbox.toml"));
    assert!(manifest.contains("context = \"ai\""));
    assert!(manifest.contains("repo = \"ai\""));

    write(workspace.path().join("context/demo/.keep"), "");
}
