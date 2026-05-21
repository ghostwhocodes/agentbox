mod common;

#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::{fs, path::Path};

use common::{
    TempDir, assert_failure, assert_success, fixture_repo, init_workspace, path, read,
    run_agentbox, stderr, stdout, write,
};

fn set_readonly(path: &Path, readonly: bool) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_readonly(readonly);
    fs::set_permissions(path, permissions).expect("set permissions");
}

#[test]
fn template_create_list_apply_and_delete() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));

    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Applied template `default`"));

    assert_eq!(read(path(&workspace, "context/demo/ai/prompt.md")), "seed");
    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(manifest.contains("[repo_templates.demo]"));
    assert!(manifest.contains("template = \"default\""));
    assert!(manifest.contains("[[repo_mounts]]"));
    assert!(manifest.contains("repo_id = \"demo\""));
    assert!(manifest.contains("context = \"ai\""));

    let output = run_agentbox(workspace.path(), &["template", "list"]);
    assert_success(&output);
    assert!(stdout(&output).contains("default"));

    let output = run_agentbox(workspace.path(), &["template", "delete", "default"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Deleted template `default`"));
}

#[test]
fn template_list_ignores_staged_delete_directories() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n",
    );

    write(
        path(&workspace, "templates/.delete-default-123/template.toml"),
        "version = 1\n",
    );

    let output = run_agentbox(workspace.path(), &["template", "list"]);
    assert_success(&output);
    assert!(stdout(&output).contains("default"));
    assert!(!stdout(&output).contains(".delete-default-123"));
}

#[test]
fn template_apply_conflict_leaves_context_unchanged() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"generated\"\nrepo = \"src\"\n",
    );
    write(
        path(&workspace, "templates/default/context/generated/prompt.md"),
        "seed",
    );

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
            "existing",
            "--repo-path",
            "src",
        ],
    ));

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("conflicts with an existing mount"));
    assert!(!Path::new(&path(&workspace, "context/demo/generated/prompt.md")).exists());

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
    assert!(!manifest.contains("context = \"generated\""));
}

#[test]
fn template_apply_rejects_overlapping_mount_conflict() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        r#"version = 1

[[mounts]]
context = "generated"
repo = "src/generated"
"#,
    );
    write(
        path(&workspace, "templates/default/context/generated/prompt.md"),
        "seed",
    );

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
            "existing",
            "--repo-path",
            "src",
        ],
    ));

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);
    assert!(
        stderr(&output).contains("overlapping mount repo paths")
            || stderr(&output).contains("conflicts with an existing mount")
    );
    assert!(!Path::new(&path(&workspace, "context/demo/generated/prompt.md")).exists());

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
    assert!(!manifest.contains("repo = \"src/generated\""));
}

#[cfg(unix)]
#[test]
fn template_apply_unsupported_entry_leaves_context_unchanged() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n",
    );
    write(
        path(&workspace, "templates/default/context/good/file.txt"),
        "seed",
    );
    fs::create_dir_all(path(&workspace, "templates/default/context/bad")).expect("create bad dir");
    symlink(
        "../good/file.txt",
        path(&workspace, "templates/default/context/bad/link.txt"),
    )
    .expect("create unsupported template symlink");

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    write(path(&workspace, "context/demo/existing.txt"), "keep");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("unsupported template entry"));
    assert_eq!(read(path(&workspace, "context/demo/existing.txt")), "keep");
    assert!(!Path::new(&path(&workspace, "context/demo/good/file.txt")).exists());
    assert!(!Path::new(&path(&workspace, "context/demo/bad")).exists());

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
}

#[test]
fn template_apply_empty_directory_collision_leaves_context_unchanged() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n",
    );
    std::fs::create_dir_all(path(&workspace, "templates/default/context/ai"))
        .expect("create empty template dir");

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    write(path(&workspace, "context/demo/ai"), "keep");
    write(path(&workspace, "context/demo/existing.txt"), "keep");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("already exists and is not a directory"));
    assert_eq!(read(path(&workspace, "context/demo/ai")), "keep");
    assert_eq!(read(path(&workspace, "context/demo/existing.txt")), "keep");

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
}

#[test]
fn template_apply_rejects_mount_that_would_hide_existing_repo_path_contents() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));
    write(path(&workspace, "repos/demo/ai/tracked.txt"), "tracked");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("already contains different files"));
    assert!(stderr(&output).contains("import-mount demo --repo-path ai --no-mount"));
    assert!(!Path::new(&path(&workspace, "context/demo/ai/prompt.md")).exists());

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
}

#[test]
fn template_apply_allows_materialized_repo_path_that_matches_subset_of_template_context() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );
    write(
        path(&workspace, "templates/default/context/ai/config.json"),
        "{\"mode\":\"expanded\"}",
    );

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));
    write(path(&workspace, "repos/demo/ai/prompt.md"), "seed");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Applied template `default`"));
    assert_eq!(read(path(&workspace, "context/demo/ai/prompt.md")), "seed");
    assert_eq!(
        read(path(&workspace, "context/demo/ai/config.json")),
        "{\"mode\":\"expanded\"}"
    );

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(manifest.contains("[repo_templates.demo]"));
    assert!(manifest.contains("template = \"default\""));
    assert!(manifest.contains("context = \"ai\""));
    assert!(manifest.contains("repo = \"ai\""));
}

#[test]
fn template_apply_allows_materialized_repo_path_with_only_empty_directory_scaffolding() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));
    fs::create_dir_all(path(&workspace, "repos/demo/ai/subdir")).expect("create scaffold dir");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_success(&output);
    assert_eq!(read(path(&workspace, "context/demo/ai/prompt.md")), "seed");
}

#[test]
fn template_apply_allows_mount_only_template_over_scaffold_only_repo_target() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));
    fs::create_dir_all(path(&workspace, "repos/demo/ai/subdir")).expect("create scaffold dir");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Applied template `default`"));
    assert!(!path(&workspace, "context/demo/ai").exists());

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(manifest.contains("[repo_templates.demo]"));
    assert!(manifest.contains("template = \"default\""));
    assert!(manifest.contains("context = \"ai\""));
    assert!(manifest.contains("repo = \"ai\""));
}

#[test]
fn template_apply_rejects_mount_only_template_when_repo_target_has_files() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));
    write(path(&workspace, "repos/demo/ai/tracked.txt"), "tracked");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("already contains different files"));
    assert!(!Path::new(&path(&workspace, "context/demo/ai")).exists());

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
}

#[test]
fn template_apply_rejects_duplicate_mount_when_materialized_repo_path_differs() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );

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
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));
    write(path(&workspace, "repos/demo/ai/tracked.txt"), "tracked");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);
    assert!(
        stderr(&output).contains("already contains different files"),
        "expected duplicate mount to fail the materialized target safety check"
    );
    assert!(
        !path(&workspace, "context/demo/ai/prompt.md").exists(),
        "failed duplicate-mount apply should roll back copied template context"
    );

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
    assert!(!manifest.contains("template = \"default\""));
    assert!(manifest.contains("context = \"ai\""));
    assert!(manifest.contains("repo = \"ai\""));
}

#[test]
fn template_apply_rolls_back_context_when_target_inspection_fails() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));
    write(path(&workspace, "repos/demo/ai"), "not a directory");

    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    assert_failure(&output);

    let err = stderr(&output);
    assert!(err.contains("Not a directory") || err.contains("not a directory"));
    assert!(!Path::new(&path(&workspace, "context/demo/ai/prompt.md")).exists());

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
}

#[test]
fn attach_failure_does_not_create_orphan_context_root() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());

    let output = run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", "not-a-valid-clone-source"],
    );
    assert_failure(&output);
    assert!(stderr(&output).contains("invalid clone source"));
    assert!(!Path::new(&path(&workspace, "context/demo")).exists());
}

#[test]
fn attach_manifest_write_failure_rolls_back_empty_context_root() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    set_readonly(workspace.path(), true);
    let output = run_agentbox(workspace.path(), &["attach", "demo", "--source", &source]);
    set_readonly(workspace.path(), false);

    assert_failure(&output);
    assert!(stderr(&output).contains("Permission denied"));
    assert!(!Path::new(&path(&workspace, "context/demo")).exists());
    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repos.demo]"));
}

#[test]
fn template_apply_manifest_write_failure_rolls_back_context_and_manifest() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );
    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));

    set_readonly(workspace.path(), true);
    let output = run_agentbox(workspace.path(), &["template", "apply", "default", "demo"]);
    set_readonly(workspace.path(), false);

    assert_failure(&output);
    assert!(stderr(&output).contains("Permission denied"));
    assert!(!Path::new(&path(&workspace, "context/demo/ai/prompt.md")).exists());
    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
    assert!(!manifest.contains("context = \"ai\""));
}

#[test]
fn attach_with_template_manifest_write_failure_rolls_back_context_and_manifest() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );

    set_readonly(workspace.path(), true);
    let output = run_agentbox(
        workspace.path(),
        &[
            "attach",
            "demo",
            "--source",
            &source,
            "--template",
            "default",
        ],
    );
    set_readonly(workspace.path(), false);

    assert_failure(&output);
    assert!(stderr(&output).contains("Permission denied"));
    assert!(!Path::new(&path(&workspace, "context/demo")).exists());
    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repos.demo]"));
    assert!(!manifest.contains("[repo_templates.demo]"));
}

#[test]
fn template_delete_clears_bookkeeping_but_preserves_repo_owned_state() {
    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "create", "default"],
    ));
    write(
        path(&workspace, "templates/default/template.toml"),
        "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\n",
    );
    write(
        path(&workspace, "templates/default/context/ai/prompt.md"),
        "seed",
    );
    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &["template", "apply", "default", "demo"],
    ));

    let output = run_agentbox(workspace.path(), &["template", "delete", "default"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Deleted template `default`"));
    assert_eq!(read(path(&workspace, "context/demo/ai/prompt.md")), "seed");

    let manifest = read(path(&workspace, "agentbox.toml"));
    assert!(!manifest.contains("[repo_templates.demo]"));
    assert!(manifest.contains("[[repo_mounts]]"));
    assert!(manifest.contains("context = \"ai\""));
    assert!(!Path::new(&path(&workspace, "templates/default")).exists());
}
