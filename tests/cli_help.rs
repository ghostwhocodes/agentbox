mod common;

use common::{TempDir, assert_success, run_agentbox, stdout};

#[test]
fn top_level_help_lists_required_commands() {
    let workspace = TempDir::new();
    let output = run_agentbox(workspace.path(), &["--help"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("--workspace"));
    assert!(output_text.contains("init"));
    assert!(output_text.contains("import-mount"));
    assert!(output_text.contains("remove-mount"));
    assert!(output_text.contains("edit-mount"));
    assert!(output_text.contains("list-mounts"));
    assert!(output_text.contains("template"));
    assert!(output_text.contains("doctor"));
}

#[test]
fn template_help_lists_subcommands() {
    let workspace = TempDir::new();
    let output = run_agentbox(workspace.path(), &["template", "--help"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("list"));
    assert!(output_text.contains("create"));
    assert!(output_text.contains("apply"));
}
