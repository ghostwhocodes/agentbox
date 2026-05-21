mod common;

use common::{TempDir, assert_success, fixture_repo, init_workspace, run_agentbox, stdout};

#[test]
fn cli_smoke_covers_status_show_doctor_and_noop_mount_commands() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    let output = run_agentbox(workspace.path(), &["status"]);
    assert_success(&output);
    assert!(stdout(&output).contains("demo"));

    let output = run_agentbox(workspace.path(), &["show", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("repo_id: demo"));

    let output = run_agentbox(workspace.path(), &["doctor"]);
    assert_success(&output);

    let output = run_agentbox(workspace.path(), &["mount", "demo"]);
    assert_success(&output);
    assert_eq!(stdout(&output), "");

    let output = run_agentbox(workspace.path(), &["unmount", "demo"]);
    assert_success(&output);
    assert_eq!(stdout(&output), "");

    let output = run_agentbox(workspace.path(), &["dematerialize", "demo"]);
    assert_success(&output);

    let output = run_agentbox(workspace.path(), &["detach", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Detached repo `demo`"));
}
