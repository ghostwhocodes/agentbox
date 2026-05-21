mod common;

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use common::{
    TempDir, assert_failure, assert_success, fixture_repo, init_workspace, path, read,
    run_agentbox, stderr, stdout,
};

#[test]
fn attach_materialize_and_dematerialize_repo() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    let output = run_agentbox(workspace.path(), &["attach", "demo", "--source", &source]);
    assert_success(&output);
    assert!(stdout(&output).contains("Attached repo `demo`"));

    let output = run_agentbox(workspace.path(), &["materialize", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Materialized `demo`"));

    assert!(path(&workspace, "repos/demo/.git").exists());

    let output = run_agentbox(workspace.path(), &["detach", "demo"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("still materialized"));

    let output = run_agentbox(workspace.path(), &["dematerialize", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Dematerialized `demo`"));

    assert!(!path(&workspace, "repos/demo").exists());
}

#[test]
fn detach_force_dematerializes_before_unregistration() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    let output = run_agentbox(workspace.path(), &["detach", "demo", "--force"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Detached repo `demo`"));

    assert!(!path(&workspace, "repos/demo").exists());
    assert!(path(&workspace, "context/demo").exists());
    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repos.demo]"));
}

#[test]
fn same_source_materializes_independently_in_two_workspaces() {
    let workspace_a = TempDir::new();
    let workspace_b = TempDir::new();
    init_workspace(workspace_a.path());
    init_workspace(workspace_b.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace_a.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace_b.path(),
        &["attach", "demo", "--source", &source],
    ));

    assert_success(&run_agentbox(workspace_a.path(), &["materialize", "demo"]));
    assert_success(&run_agentbox(workspace_b.path(), &["materialize", "demo"]));

    assert!(path(&workspace_a, "repos/demo/.git").exists());
    assert!(path(&workspace_b, "repos/demo/.git").exists());
    assert_ne!(
        path(&workspace_a, "repos/demo"),
        path(&workspace_b, "repos/demo")
    );
}

#[test]
fn attach_warns_when_reusing_preserved_context_without_restoring_mounts() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    common::write(path(&workspace, "context/demo/ai/state.txt"), "state");

    let output = run_agentbox(workspace.path(), &["attach", "demo", "--source", &source]);
    assert_success(&output);
    assert!(stdout(&output).contains("Attached repo `demo`"));
    assert!(stderr(&output).contains("existing workspace context"));
    assert!(stderr(&output).contains("no mount mappings were restored"));
}

#[test]
fn attach_warns_when_reusing_empty_preserved_context_root() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    fs::create_dir_all(path(&workspace, "context/demo")).expect("create preserved context root");

    let output = run_agentbox(workspace.path(), &["attach", "demo", "--source", &source]);
    assert_success(&output);
    assert!(stdout(&output).contains("Attached repo `demo`"));
    assert!(stderr(&output).contains("existing workspace context"));
    assert!(stderr(&output).contains("no mount mappings were restored"));
}

#[cfg(unix)]
#[test]
fn attach_succeeds_when_preserved_context_cannot_be_enumerated() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    let context_root = path(&workspace, "context/demo");
    common::write(context_root.join("ai/state.txt"), "state");

    let mut permissions = fs::metadata(&context_root)
        .expect("stat context root")
        .permissions();
    let original_mode = permissions.mode();
    permissions.set_mode(0o000);
    fs::set_permissions(&context_root, permissions).expect("make context unreadable");

    let output = run_agentbox(workspace.path(), &["attach", "demo", "--source", &source]);

    let mut permissions = fs::metadata(&context_root)
        .expect("stat unreadable context root")
        .permissions();
    permissions.set_mode(original_mode);
    fs::set_permissions(&context_root, permissions).expect("restore context permissions");

    assert_success(&output);
    assert!(stdout(&output).contains("Attached repo `demo`"));
    assert!(!stderr(&output).contains("Permission denied"));
}
