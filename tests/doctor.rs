mod common;

use common::{
    TempDir, assert_failure, assert_success, fixture_repo, init_workspace, path, run_agentbox,
    stderr, stdout,
};
use serde_json::Value;
use std::fs;

#[test]
fn doctor_reports_and_fixes_structural_issues() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    fs::remove_dir_all(path(&workspace, "repos")).expect("remove repos dir");
    fs::remove_dir_all(path(&workspace, "templates")).expect("remove templates dir");
    fs::write(path(&workspace, ".gitignore"), "").expect("clear gitignore");

    let output = run_agentbox(workspace.path(), &["doctor"]);
    assert_failure(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("missing `repos/` directory"));
    assert!(output_text.contains("does not ignore `repos/`"));
    assert!(output_text.contains("does not ignore `.agentbox/`"));

    let output = run_agentbox(workspace.path(), &["doctor", "--fix"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("created missing `repos/` directory"));
    assert!(output_text.contains("added missing `repos/` entry"));
    assert!(output_text.contains("added missing `.agentbox/` entry"));

    assert!(path(&workspace, "repos").exists());
    assert!(path(&workspace, "templates").exists());
    assert!(
        fs::read_to_string(path(&workspace, ".gitignore"))
            .expect("read gitignore")
            .contains("repos/")
    );
    assert!(
        fs::read_to_string(path(&workspace, ".gitignore"))
            .expect("read gitignore")
            .contains(".agentbox/")
    );
}

#[test]
fn doctor_reports_orphans_and_invalid_materialized_repos() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    fs::create_dir_all(path(&workspace, "repos/demo")).expect("create invalid repo root");
    fs::create_dir_all(path(&workspace, "repos/orphan")).expect("create orphan repo root");
    fs::create_dir_all(path(&workspace, "context/orphan")).expect("create orphan context root");

    let output = run_agentbox(workspace.path(), &["doctor"]);
    assert_failure(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("materialized repo `demo` exists at"));
    assert!(output_text.contains("orphan materialized repo found at `repos/orphan`"));
    assert!(output_text.contains("orphan context root found at `context/orphan`"));
}

#[test]
fn doctor_json_reports_findings_and_preserves_failure_exit() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    fs::remove_dir_all(path(&workspace, "repos")).expect("remove repos dir");

    let output = run_agentbox(workspace.path(), &["doctor", "--json"]);
    assert_failure(&output);

    let json: Value = serde_json::from_str(&stdout(&output)).expect("parse json");
    assert_eq!(json["ok"], Value::Bool(false));
    assert!(json["error_count"].as_u64().expect("error count") >= 1);
    assert!(
        json["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .any(|finding| {
                finding["severity"] == "error"
                    && finding["message"]
                        .as_str()
                        .expect("finding message")
                        .contains("missing `repos/` directory")
            })
    );
    assert!(stderr(&output).contains("doctor found"));
}

#[test]
fn doctor_warns_and_fixes_stale_template_bindings() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    std::fs::write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    )
    .expect("write template manifest");
    fs::create_dir_all(path(&workspace, "templates/default/context/ai"))
        .expect("create template dir");
    std::fs::write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    )
    .expect("write template file");
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "attach",
            "demo",
            "--source",
            &source,
            "--template",
            "default",
        ],
    ));
    fs::remove_dir_all(path(&workspace, "templates/default")).expect("remove template");

    let output = run_agentbox(workspace.path(), &["doctor"]);
    assert_success(&output);
    assert!(stdout(&output).contains("references missing template `default`"));

    let output = run_agentbox(workspace.path(), &["doctor", "--fix"]);
    assert_success(&output);
    assert!(stdout(&output).contains("removed stale template binding"));

    let manifest = fs::read_to_string(path(&workspace, "agentbox.toml")).expect("read manifest");
    assert!(!manifest.contains("[repo_templates.demo]"));
}
