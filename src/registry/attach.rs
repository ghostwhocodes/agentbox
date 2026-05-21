use camino::Utf8PathBuf;

use crate::{
    error::{AttachError, Error, RepoWorkflowError, Result},
    inspection::{RepoLifecyclePreflight, repo_lifecycle_preflight},
    mounts::{infra as mount, ownership},
    persistence::composite,
    shared::types::{CloneSource, RepoId, TemplateId},
    templates::{self, AppliedTemplate},
    workspace::Workspace,
};

use super::{RegisteredRepo, registered_repo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedRepo {
    pub repo_id: RepoId,
    pub context_root: Utf8PathBuf,
}

pub fn attach_repo(
    workspace: &Workspace,
    repo_id: RepoId,
    source: CloneSource,
    template_id: Option<TemplateId>,
) -> Result<RepoId> {
    let repo = RegisteredRepo { source };
    let has_template = template_id.is_some();
    let prepared_template = if let Some(template_id) = &template_id {
        Some(templates::prepare_template_application(
            workspace,
            &repo_id,
            &[],
            template_id,
        )?)
    } else {
        None
    };
    let repo_mounts = prepared_template
        .as_ref()
        .map(|prepared| prepared.updated_mounts.clone())
        .unwrap_or_default();
    let result = composite::persist_attached_repo(
        workspace,
        repo_id.clone(),
        repo,
        repo_mounts,
        template_id.map(|template_id| AppliedTemplate { template_id }),
    );
    if let Err(error) = result {
        if let Some(prepared_template) = prepared_template {
            prepared_template.rollback()?;
        }
        return Err(map_attach_error(error));
    }
    if !has_template {
        if let Err(error) = super::ensure_repo_context_root(workspace, &repo_id) {
            composite::remove_registered_repo(workspace, &repo_id).map_err(map_attach_error)?;
            return Err(error);
        }
    }
    Ok(repo_id)
}

pub fn detach_repo(
    workspace: &Workspace,
    repo_id: RepoId,
    force: bool,
    unsafe_unmount: bool,
) -> Result<DetachedRepo> {
    detach_repo_with_preflight_loader(
        workspace,
        repo_id,
        force,
        unsafe_unmount,
        repo_lifecycle_preflight,
        mount::unmount,
    )
}

fn detach_repo_with_preflight_loader<P, F>(
    workspace: &Workspace,
    repo_id: RepoId,
    force: bool,
    unsafe_unmount: bool,
    load_preflight: P,
    unmount_target: F,
) -> Result<DetachedRepo>
where
    P: FnOnce(&Workspace, RepoId) -> Result<RepoLifecyclePreflight>,
    F: FnMut(&camino::Utf8Path) -> Result<()>,
{
    let preflight = load_preflight(workspace, repo_id.clone()).map_err(map_attach_error)?;
    detach_repo_in_with_state(
        workspace,
        repo_id,
        force,
        unsafe_unmount,
        preflight,
        unmount_target,
    )
}

fn detach_repo_in_with_state<F>(
    workspace: &Workspace,
    repo_id: RepoId,
    force: bool,
    unsafe_unmount: bool,
    preflight: RepoLifecyclePreflight,
    mut unmount_target: F,
) -> Result<DetachedRepo>
where
    F: FnMut(&camino::Utf8Path) -> Result<()>,
{
    registered_repo(workspace, &repo_id).map_err(map_attach_error)?;
    if preflight.has_conflicting_mounts {
        return Err(AttachError::RepoHasConflictingMounts {
            repo_id: repo_id.clone(),
        }
        .into());
    }
    if !force {
        if preflight.materialized {
            return Err(AttachError::RepoStillMaterialized {
                repo_id: repo_id.clone(),
            }
            .into());
        }
        if preflight.has_active_mounts {
            return Err(AttachError::RepoHasActiveMounts {
                repo_id: repo_id.clone(),
            }
            .into());
        }
    } else if preflight.materialized || preflight.has_active_mounts {
        if !unsafe_unmount {
            if let Some(mount) = preflight.unverified_active_mounts.first() {
                return Err(
                    crate::shared::error::MountWorkflowError::UnmountTargetOwnershipUnverified {
                        existing_source: mount.existing_source.clone(),
                        target: mount.target.clone(),
                    }
                    .into(),
                );
            }
        }
        for target in &preflight.active_mount_targets {
            unmount_target(target)?;
        }
        ownership::remove_repo_best_effort(workspace, &repo_id);
        if workspace.repo_root(&repo_id).exists() {
            super::remove_repo_root(workspace, &repo_id)?;
        }
    }

    composite::remove_registered_repo(workspace, &repo_id).map_err(map_attach_error)?;
    ownership::remove_repo_best_effort(workspace, &repo_id);
    let context_root = workspace.repo_context_root(&repo_id);
    Ok(DetachedRepo {
        repo_id,
        context_root,
    })
}

fn map_attach_error(error: Error) -> Error {
    match error {
        Error::Repo(RepoWorkflowError::RepoAlreadyRegistered { repo_id }) => {
            AttachError::RepoAlreadyRegistered { repo_id }.into()
        }
        Error::Repo(RepoWorkflowError::RepoNotRegistered { repo_id }) => {
            AttachError::RepoNotRegistered { repo_id }.into()
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs, path::Path};

    use super::*;
    use crate::{
        error::Error,
        registry::RegisteredRepo,
        shared::{mount_spec::MountSpec, types::RelativePath},
        templates::{create_template, infra::save_template_manifest},
        test_support,
    };

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-app-attach-test")
    }

    fn set_readonly(path: &Path, readonly: bool) {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(path, permissions).expect("set permissions");
    }

    fn init_git_repo(path: &camino::Utf8Path) {
        test_support::init_git_repo(path);
    }

    #[test]
    fn attach_with_template_persists_template_binding() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        save_template_manifest(
            &workspace,
            &template_id,
            &crate::templates::domain::TemplateManifest {
                version: crate::templates::domain::TEMPLATE_MANIFEST_VERSION,
                mounts: vec![MountSpec {
                    context: rel("ai"),
                    repo: rel("ai"),
                }],
            },
        )
        .expect("save template manifest");

        attach_repo(
            &workspace,
            repo_id.clone(),
            CloneSource::new("file:///tmp/source").expect("valid source"),
            Some(template_id.clone()),
        )
        .expect("attach repo");

        let applied_template = crate::templates::store::applied_template(&workspace, &repo_id)
            .expect("read applied template");
        assert_eq!(
            applied_template.expect("template binding").template_id,
            template_id
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn detach_without_force_fails_when_repo_is_still_materialized() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
        )
        .expect("replace mounts");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        let error = detach_repo_in_with_state(
            &workspace,
            repo_id.clone(),
            false,
            false,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: true,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![target],
                unverified_active_mounts: Vec::new(),
            },
            |_| Ok(()),
        )
        .expect_err("materialized repo should block detach");

        assert!(matches!(
            error,
            Error::Attach(crate::shared::error::AttachError::RepoStillMaterialized {
                repo_id: ref active_repo_id,
            }) if active_repo_id == &repo_id
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn detach_without_force_fails_when_repo_has_conflicting_mounts() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
        )
        .expect("replace mounts");

        let unmount_calls = Cell::new(0usize);
        let error = detach_repo_in_with_state(
            &workspace,
            repo_id.clone(),
            false,
            false,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: false,
                has_active_mounts: false,
                has_conflicting_mounts: true,
                active_mount_targets: vec![],
                unverified_active_mounts: Vec::new(),
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("conflicting mount should block detach");

        assert!(matches!(
            error,
            Error::Attach(crate::shared::error::AttachError::RepoHasConflictingMounts {
                repo_id: ref conflicted_repo_id,
            }) if conflicted_repo_id == &repo_id
        ));
        assert_eq!(unmount_calls.get(), 0);
        assert!(
            crate::persistence::manifest_store::read(&workspace)
                .unwrap()
                .repos
                .contains_key(&repo_id)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn detach_force_unmounts_active_mounts_and_removes_repo_root() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
        )
        .expect("replace mounts");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        let unmount_calls = Cell::new(0usize);

        let detached = detach_repo_in_with_state(
            &workspace,
            repo_id.clone(),
            true,
            false,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: true,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![target.clone()],
                unverified_active_mounts: Vec::new(),
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect("force detach should succeed");

        assert_eq!(detached.repo_id, repo_id);
        assert_eq!(unmount_calls.get(), 1);
        assert!(!workspace.repo_root(&detached.repo_id).exists());
        assert!(
            !crate::persistence::manifest_store::read(&workspace)
                .unwrap()
                .repos
                .contains_key(&detached.repo_id)
        );
        assert!(
            !crate::persistence::manifest_store::read(&workspace)
                .unwrap()
                .repo_templates
                .contains_key(&detached.repo_id)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }
    #[test]
    fn detach_force_unmounts_active_mounts_for_non_materialized_repo_root() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
        )
        .expect("replace mounts");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");

        let target = workspace.repo_path(&repo_id, &rel("target"));
        let unmount_calls = Cell::new(0usize);

        let detached = detach_repo_in_with_state(
            &workspace,
            repo_id.clone(),
            true,
            false,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: false,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![target.clone()],
                unverified_active_mounts: Vec::new(),
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect("force detach should still clean up broken repo roots");

        assert_eq!(detached.repo_id, repo_id);
        assert_eq!(unmount_calls.get(), 1);
        assert!(!workspace.repo_root(&detached.repo_id).exists());
        assert!(
            !crate::persistence::manifest_store::read(&workspace)
                .unwrap()
                .repos
                .contains_key(&detached.repo_id)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn detach_force_blocks_unverified_active_mount_without_unsafe_override() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let unmount_calls = Cell::new(0usize);

        let error = detach_repo_in_with_state(
            &workspace,
            repo_id.clone(),
            true,
            false,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: false,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![target.clone()],
                unverified_active_mounts: vec![crate::inspection::UnverifiedActiveMount {
                    existing_source: Utf8PathBuf::from("/workspace/context/demo/ctx"),
                    target: target.clone(),
                }],
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("unverified mount should block force detach");

        assert!(matches!(
            error,
            Error::Mount(crate::shared::error::MountWorkflowError::UnmountTargetOwnershipUnverified {
                target: ref unverified_target,
                ..
            }) if unverified_target == &target
        ));
        assert_eq!(unmount_calls.get(), 0);
        assert!(workspace.repo_root(&repo_id).exists());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn detach_force_allows_unverified_active_mount_with_unsafe_override() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        let target = workspace.repo_path(&repo_id, &rel("target"));
        let unmount_calls = Cell::new(0usize);

        let detached = detach_repo_in_with_state(
            &workspace,
            repo_id.clone(),
            true,
            true,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: false,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![target.clone()],
                unverified_active_mounts: vec![crate::inspection::UnverifiedActiveMount {
                    existing_source: Utf8PathBuf::from("/workspace/context/demo/ctx"),
                    target,
                }],
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect("unsafe force detach should proceed");

        assert_eq!(detached.repo_id, repo_id);
        assert_eq!(unmount_calls.get(), 1);
        assert!(!workspace.repo_root(&detached.repo_id).exists());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn detach_force_fails_when_repo_has_conflicting_mounts() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
        )
        .expect("replace mounts");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let unmount_calls = Cell::new(0usize);
        let error = detach_repo_in_with_state(
            &workspace,
            repo_id.clone(),
            true,
            false,
            RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: true,
                has_active_mounts: true,
                has_conflicting_mounts: true,
                active_mount_targets: vec![],
                unverified_active_mounts: Vec::new(),
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("conflicting mount should block force detach");

        assert!(matches!(
            error,
            Error::Attach(crate::shared::error::AttachError::RepoHasConflictingMounts {
                repo_id: ref conflicted_repo_id,
            }) if conflicted_repo_id == &repo_id
        ));
        assert_eq!(unmount_calls.get(), 0);
        assert!(workspace.repo_root(&repo_id).exists());
        assert!(
            crate::persistence::manifest_store::read(&workspace)
                .unwrap()
                .repos
                .contains_key(&repo_id)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn attach_without_template_does_not_leave_orphan_context_root_when_manifest_write_fails() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");

        set_readonly(workspace.root().as_std_path(), true);
        let error = attach_repo(
            &workspace,
            repo_id.clone(),
            CloneSource::new("file:///tmp/source").expect("valid source"),
            None,
        )
        .expect_err("manifest write should fail");
        set_readonly(workspace.root().as_std_path(), false);

        assert!(matches!(
            error,
            Error::IoPath { ref source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert!(!workspace.repo_context_root(&repo_id).exists());
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repos.contains_key(&repo_id));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn attach_with_template_rolls_back_context_copy_when_manifest_write_fails() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        save_template_manifest(
            &workspace,
            &template_id,
            &crate::templates::domain::TemplateManifest {
                version: crate::templates::domain::TEMPLATE_MANIFEST_VERSION,
                mounts: vec![MountSpec {
                    context: rel("ai"),
                    repo: rel("ai"),
                }],
            },
        )
        .expect("save template manifest");
        let prompt_path = workspace
            .template_context_root(&template_id)
            .join("ai/prompt.md");
        fs::create_dir_all(prompt_path.parent().expect("prompt parent"))
            .expect("create prompt dir");
        fs::write(&prompt_path, "seed").expect("write template file");

        set_readonly(workspace.root().as_std_path(), true);
        let error = attach_repo(
            &workspace,
            repo_id.clone(),
            CloneSource::new("file:///tmp/source").expect("valid source"),
            Some(template_id),
        )
        .expect_err("manifest write should fail");
        set_readonly(workspace.root().as_std_path(), false);

        assert!(matches!(
            error,
            Error::IoPath { ref source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert!(!workspace.repo_context_root(&repo_id).exists());
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repos.contains_key(&repo_id));
        assert!(!manifest.repo_templates.contains_key(&repo_id));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn detach_force_fails_closed_when_mount_inspection_is_unavailable() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
        )
        .expect("replace mounts");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let unmount_calls = Cell::new(0usize);
        let error = detach_repo_with_preflight_loader(
            &workspace,
            repo_id.clone(),
            true,
            false,
            |_, _| {
                Err(Error::io_path(
                    "/proc/self/mountinfo",
                    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
                ))
            },
            |_| {
                unmount_calls.set(unmount_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("mount inspection failure should block force detach");

        assert!(
            matches!(error, Error::IoPath { ref path, .. } if path == &camino::Utf8PathBuf::from("/proc/self/mountinfo"))
        );
        assert_eq!(unmount_calls.get(), 0);
        assert!(workspace.repo_root(&repo_id).exists());
        assert!(
            crate::persistence::manifest_store::read(&workspace)
                .unwrap()
                .repos
                .contains_key(&repo_id)
        );

        let _ = fs::remove_dir_all(workspace.root());
    }
}
