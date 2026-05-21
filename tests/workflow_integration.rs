mod common;

use agentbox::{
    error::{AttachError, Error, MountWorkflowError, RepoWorkflowError},
    inspection::{self, AggregateMountState},
    mounts, registry,
    shared::types::{CloneSource, RelativePath, RepoId, TemplateId},
    templates, workspace,
    workspace::Workspace,
};

use common::{TempDir, fixture_repo, path, write};

fn workspace_handle(dir: &TempDir) -> Workspace {
    Workspace::from_std_path(dir.path().to_path_buf()).expect("utf8 workspace path")
}

#[test]
fn workflow_layer_handles_repo_lifecycle_without_cli() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();
    let repo_id = RepoId::new("demo").expect("valid repo id");

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");

    mounts::add_mount(
        &workspace,
        repo_id.clone(),
        RelativePath::new("ai", "mount context path").expect("valid context path"),
        RelativePath::new("ai", "mount repo path").expect("valid repo path"),
    )
    .expect("add mount");

    let statuses = inspection::workspace_status(&workspace).expect("status");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].repo_id, repo_id);
    assert_eq!(statuses[0].mount_state, AggregateMountState::Unmounted);
    assert!(!statuses[0].materialized);

    let outcomes =
        registry::materialize_repos(&workspace, Some(repo_id.clone())).expect("materialize");
    assert_eq!(outcomes.len(), 1);
    assert!(path(&workspace_dir, "repos/demo/.git").exists());

    let outcomes =
        registry::dematerialize_repos(&workspace, Some(repo_id.clone())).expect("dematerialize");
    assert_eq!(outcomes.len(), 1);
    assert!(!path(&workspace_dir, "repos/demo").exists());

    let detached =
        registry::detach_repo(&workspace, repo_id.clone(), false, false).expect("detach repo");
    assert_eq!(detached.repo_id, repo_id);
    assert!(path(&workspace_dir, "context/demo").exists());
}

#[test]
fn workflow_layer_reports_typed_errors() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();
    let repo_id = RepoId::new("demo").expect("valid repo id");

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source.clone()).expect("valid source"),
        None,
    )
    .expect("attach repo");

    let error = registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect_err("duplicate attach should fail");
    assert!(matches!(
        error,
        Error::Attach(AttachError::RepoAlreadyRegistered { repo_id: ref duplicate_repo_id })
            if duplicate_repo_id == &repo_id
    ));

    mounts::add_mount(
        &workspace,
        repo_id.clone(),
        RelativePath::new("ai", "mount context path").expect("valid context path"),
        RelativePath::new("ai", "mount repo path").expect("valid repo path"),
    )
    .expect("add first mount");

    let error = match mounts::add_mount(
        &workspace,
        repo_id.clone(),
        RelativePath::new("ai", "mount context path").expect("valid context path"),
        RelativePath::new("other", "mount repo path").expect("valid repo path"),
    ) {
        Ok(_) => panic!("duplicate mount should fail"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        Error::Mount(MountWorkflowError::MountConflicts { repo_id: ref conflict_repo_id, .. })
            if conflict_repo_id == &repo_id
    ));

    let missing_repo_id = RepoId::new("missing").expect("valid repo id");
    let error = templates::apply_template_to_registered_repo(
        &workspace,
        &TemplateId::new("default").expect("valid template id"),
        &missing_repo_id,
    )
    .expect_err("template apply on missing repo should fail");
    assert!(matches!(
        error,
        Error::Repo(RepoWorkflowError::RepoNotRegistered { repo_id: ref missing })
            if missing == &missing_repo_id
    ));
}

#[test]
fn workflow_layer_persists_template_binding_and_mounts_without_cli() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();
    let repo_id = RepoId::new("demo").expect("valid repo id");
    let template_id = TemplateId::new("default").expect("valid template id");

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");

    templates::create_template(&workspace, &template_id).expect("create template");
    write(
        path(&workspace_dir, "templates/default/template.toml"),
        r#"version = 1

[[mounts]]
context = "ai"
repo = "ai"
"#,
    );

    templates::apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
        .expect("apply template");

    let manifest = agentbox::persistence::manifest_store::read(&workspace).expect("load manifest");
    assert_eq!(
        manifest
            .repo_templates
            .get(&repo_id)
            .expect("template binding")
            .template,
        template_id
    );
    assert!(
        manifest
            .repo_mounts
            .iter()
            .any(|mount| mount.repo_id == repo_id
                && mount.context.as_str() == "ai"
                && mount.repo.as_str() == "ai")
    );
}

#[test]
fn workflow_layer_detach_cleans_template_binding_and_mounts_without_cli() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();
    let repo_id = RepoId::new("demo").expect("valid repo id");
    let template_id = TemplateId::new("default").expect("valid template id");

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");

    templates::create_template(&workspace, &template_id).expect("create template");
    write(
        path(&workspace_dir, "templates/default/template.toml"),
        r#"version = 1

[[mounts]]
context = "ai"
repo = "ai"
"#,
    );

    templates::apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
        .expect("apply template");

    registry::detach_repo(&workspace, repo_id.clone(), false, false).expect("detach repo");

    let manifest = agentbox::persistence::manifest_store::read(&workspace).expect("load manifest");
    assert!(!manifest.repos.contains_key(&repo_id));
    assert!(!manifest.repo_templates.contains_key(&repo_id));
    assert!(
        !manifest
            .repo_mounts
            .iter()
            .any(|mount| mount.repo_id == repo_id)
    );
}

#[test]
fn doctor_workflow_fixes_workspace_without_cli() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        RepoId::new("demo").expect("valid repo id"),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");

    std::fs::remove_dir_all(path(&workspace_dir, "repos")).expect("remove repos dir");
    std::fs::remove_dir_all(path(&workspace_dir, "templates")).expect("remove templates dir");
    write(path(&workspace_dir, ".gitignore"), "");

    let report = inspection::run_doctor(&workspace, false).expect("doctor report");
    assert!(report.error_count() >= 2);

    let report = inspection::run_doctor(&workspace, true).expect("doctor fix");
    assert_eq!(report.error_count(), 0);
    assert!(path(&workspace_dir, "repos").exists());
    assert!(path(&workspace_dir, "templates").exists());
}

#[test]
fn status_succeeds_when_no_mount_inspection_needed() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();
    let repo_id = RepoId::new("demo").expect("valid repo id");

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");

    // No mounts, not materialized — status should work without mount table
    let statuses = inspection::workspace_status(&workspace).expect("status");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].mount_state, AggregateMountState::NoMounts);
}

#[test]
fn show_succeeds_when_no_mount_inspection_needed() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();
    let repo_id = RepoId::new("demo").expect("valid repo id");

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");

    let status = inspection::show_repo(&workspace, repo_id.clone()).expect("show repo");
    assert_eq!(status.repo_id, repo_id);
    assert_eq!(status.mount_state, AggregateMountState::NoMounts);
}

#[test]
fn inspection_context_exposes_read_only_status_and_mount_views() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();
    let repo_id = RepoId::new("demo").expect("valid repo id");

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        repo_id.clone(),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");
    mounts::add_mount(
        &workspace,
        repo_id.clone(),
        RelativePath::new("ai", "mount context path").expect("valid context path"),
        RelativePath::new("ai", "mount repo path").expect("valid repo path"),
    )
    .expect("add mount");

    let statuses = inspection::workspace_status(&workspace).expect("inspection workspace status");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].repo_id, repo_id);
    assert_eq!(statuses[0].mount_state, AggregateMountState::Unmounted);

    let status = inspection::show_repo(&workspace, repo_id.clone()).expect("inspection show repo");
    assert_eq!(status.repo_id, repo_id);
    assert_eq!(status.mount_state, AggregateMountState::Unmounted);

    let mounts =
        inspection::list_mounts(&workspace, Some(repo_id.clone())).expect("inspection mount list");
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].repo_id, repo_id);
    assert!(!mounts[0].materialized);
    assert!(!mounts[0].active);
    assert!(!mounts[0].inspection_unavailable);
}

#[test]
fn doctor_succeeds_when_no_mount_inspection_needed() {
    let workspace_dir = TempDir::new();
    let workspace = workspace_handle(&workspace_dir);
    let (_fixture, source) = fixture_repo();

    workspace::init_workspace(&workspace).expect("init workspace");
    registry::attach_repo(
        &workspace,
        RepoId::new("demo").expect("valid repo id"),
        CloneSource::new(source).expect("valid source"),
        None,
    )
    .expect("attach repo");

    // No mounts — doctor should succeed without reading mount table
    let report = inspection::run_doctor(&workspace, false).expect("doctor");
    assert_eq!(report.error_count(), 0);
}
