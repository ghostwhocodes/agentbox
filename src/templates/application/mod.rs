//! Template application services.
//!
//! This layer orchestrates template workflows over workspace state,
//! manifest persistence, and template filesystem adapters.
//!
//! Dependency guardrail:
//! callers should consume these focused workflows rather than reaching into
//! template storage or mutating repo template state directly.

mod apply;
mod create;
mod delete;
mod list;

pub use apply::apply_template_to_registered_repo;
pub(crate) use apply::prepare_template_application;
pub use create::create_template;
pub use delete::delete_template;
pub use list::list_templates;

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::{fs, path::Path};

    use camino::Utf8PathBuf;

    use super::*;
    use crate::{
        error::{Error, TemplateError},
        mounts::infra::MountEntry,
        shared::{
            mount_spec::MountSpec,
            types::{CloneSource, RelativePath, RepoId, TemplateId},
        },
        templates::infra::{load_template_manifest, save_template_manifest},
        test_support, workspace,
        workspace::Workspace,
    };

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-template-test")
    }

    fn set_readonly(path: &Path, readonly: bool) {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(path, permissions).expect("set permissions");
    }

    #[test]
    fn create_template_rejects_existing_template() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        let error =
            create_template(&workspace, &template_id).expect_err("duplicate template should fail");

        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateAlreadyExists {
                template_id: ref duplicate_id,
            }) if duplicate_id == &template_id
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn delete_template_rejects_missing_template() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("missing").expect("valid template id");

        let error =
            delete_template(&workspace, &template_id).expect_err("missing template should fail");
        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMissing {
                template_id: ref missing_id,
            }) if missing_id == &template_id
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn delete_template_removes_unbound_template() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        let prompt_path = workspace
            .template_context_root(&template_id)
            .join("ai/prompt.md");
        fs::create_dir_all(prompt_path.parent().expect("prompt parent"))
            .expect("create prompt dir");
        fs::write(&prompt_path, "seed").expect("write template file");

        delete_template(&workspace, &template_id).expect("delete template");

        assert!(!workspace.template_root(&template_id).exists());
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(manifest.repo_templates.is_empty());
        assert!(manifest.repo_mounts.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rejects_missing_template() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("missing").expect("valid template id");

        let error = prepare_template_application(&workspace, &repo_id, &[], &template_id)
            .expect_err("missing template should fail");
        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMissing {
                template_id: ref missing_id,
            }) if missing_id == &template_id
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rejects_mount_conflicts() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        let manifest = crate::templates::domain::TemplateManifest {
            version: crate::templates::domain::TEMPLATE_MANIFEST_VERSION,
            mounts: vec![MountSpec {
                context: rel("generated"),
                repo: rel("src"),
            }],
        };
        save_template_manifest(&workspace, &template_id, &manifest)
            .expect("save template manifest");

        let existing_mounts = vec![MountSpec {
            context: rel("existing"),
            repo: rel("src"),
        }];

        let error =
            prepare_template_application(&workspace, &repo_id, &existing_mounts, &template_id)
                .expect_err("conflicting template should fail");
        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMountConflict {
                template_id: ref conflict_template_id,
                repo_id: ref conflict_repo_id,
                ..
            }) if conflict_template_id == &template_id && conflict_repo_id == &repo_id
        ));
        assert_eq!(existing_mounts.len(), 1);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_allows_materialized_target_that_matches_subset_of_copied_context() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let target = workspace.repo_path(&repo_id, &rel("ai"));
        fs::create_dir_all(&target).expect("create target");
        fs::write(target.join("prompt.md"), "seed").expect("write shared target file");

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
        let config_path = workspace
            .template_context_root(&template_id)
            .join("ai/config.json");
        fs::create_dir_all(prompt_path.parent().expect("prompt parent"))
            .expect("create prompt dir");
        fs::write(&prompt_path, "seed").expect("write template prompt");
        fs::write(&config_path, "{\"mode\":\"expanded\"}").expect("write template config");

        apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
            .expect("matching subset target should be allowed");

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert_eq!(
            manifest
                .repo_templates
                .get(&repo_id)
                .expect("applied template")
                .template,
            template_id
        );
        let mounts = crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("mounts");
        assert_eq!(
            mounts,
            vec![MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            }]
        );
        assert_eq!(
            fs::read_to_string(
                workspace
                    .context_path(&repo_id, &rel("ai"))
                    .join("prompt.md")
            )
            .expect("read copied prompt"),
            "seed"
        );
        assert_eq!(
            fs::read_to_string(
                workspace
                    .context_path(&repo_id, &rel("ai"))
                    .join("config.json")
            )
            .expect("read copied config"),
            "{\"mode\":\"expanded\"}"
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_allows_mount_only_template_over_scaffold_only_repo_target() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        fs::create_dir_all(workspace.repo_path(&repo_id, &rel("ai/subdir")))
            .expect("create repo scaffolding");

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

        apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
            .expect("mount-only template should allow scaffold-only target");

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert_eq!(
            manifest
                .repo_templates
                .get(&repo_id)
                .expect("applied template")
                .template,
            template_id
        );
        let mounts = crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("mounts");
        assert_eq!(
            mounts,
            vec![MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            }]
        );
        assert!(
            !workspace.context_path(&repo_id, &rel("ai")).exists(),
            "mount-only template should not create a missing context tree"
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rejects_mount_only_template_when_repo_target_has_files() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let target = workspace.repo_path(&repo_id, &rel("ai"));
        fs::create_dir_all(&target).expect("create repo target");
        fs::write(target.join("tracked.txt"), "tracked").expect("write tracked file");

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

        let error = apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
            .expect_err("file-bearing target should still be rejected");

        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMountWouldHideDifferentFiles {
                template_id: ref conflict_template_id,
                repo_id: ref conflict_repo_id,
                repo_path: ref conflict_repo_path,
                target: ref conflict_target,
            }) if conflict_template_id == &template_id
                && conflict_repo_id == &repo_id
                && conflict_repo_path == &rel("ai")
                && conflict_target == &target
        ));
        assert!(
            !workspace.context_path(&repo_id, &rel("ai")).exists(),
            "failed mount-only template inspection should not create context"
        );
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rolls_back_context_copy_when_target_inspection_fails() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        fs::write(workspace.repo_path(&repo_id, &rel("ai")), "not a directory")
            .expect("write blocking file");

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

        let error = apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
            .expect_err("target inspection failure should roll back");

        assert!(matches!(
            error,
            Error::IoPath {
                path,
                ref source,
            } if path == workspace.repo_path(&repo_id, &rel("ai"))
                && source.kind() == std::io::ErrorKind::NotADirectory
        ));
        assert!(!workspace.context_path(&repo_id, &rel("ai")).exists());

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));
        assert!(
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id)
                .expect("repo mounts")
                .is_empty()
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rolls_back_context_copy_when_manifest_write_fails() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: crate::shared::types::CloneSource::new("file:///tmp/source")
                    .expect("valid source"),
            },
        )
        .expect("register repo");
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
        let error = apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
            .expect_err("manifest write should fail");
        set_readonly(workspace.root().as_std_path(), false);

        assert!(matches!(
            error,
            Error::IoPath { ref source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert!(!workspace.context_path(&repo_id, &rel("ai")).exists());
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));
        assert!(
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id)
                .expect("repo mounts")
                .is_empty()
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rejects_duplicate_mount_when_materialized_target_differs() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::add_mount(&workspace, repo_id.clone(), rel("ai"), rel("ai"))
            .expect("add existing mount");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let target = workspace.repo_path(&repo_id, &rel("ai"));
        fs::create_dir_all(&target).expect("create target");
        fs::write(target.join("tracked.txt"), "tracked").expect("write tracked file");

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

        let error = apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
            .expect_err("duplicate mount should fail when target differs");

        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMountWouldHideDifferentFiles {
                template_id: ref conflict_template_id,
                repo_id: ref conflict_repo_id,
                repo_path: ref conflict_repo_path,
                target: ref conflict_target,
            }) if conflict_template_id == &template_id
                && conflict_repo_id == &repo_id
                && conflict_repo_path == &rel("ai")
                && conflict_target == &target
        ));
        assert!(
            !workspace.context_path(&repo_id, &rel("ai")).exists(),
            "failed duplicate-mount inspection should roll back copied context"
        );
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));
        let mounts = crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("mounts");
        assert_eq!(
            mounts,
            vec![MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            }]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rejects_target_that_is_mounted_from_foreign_source() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::add_mount(&workspace, repo_id.clone(), rel("ai"), rel("ai"))
            .expect("add existing mount");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let target = workspace.repo_path(&repo_id, &rel("ai"));
        fs::create_dir_all(&target).expect("create target");
        fs::write(target.join("tracked.txt"), "hidden repo-owned file")
            .expect("write hidden target file");

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
        let context_root = workspace.context_path(&repo_id, &rel("ai"));
        let template_prompt = workspace
            .template_context_root(&template_id)
            .join("ai/prompt.md");
        fs::create_dir_all(template_prompt.parent().expect("template prompt parent"))
            .expect("create template prompt dir");
        fs::write(&template_prompt, "seed").expect("write template file");

        let error = super::apply::prepare_template_application_with_mount_table_loader(
            &workspace,
            &repo_id,
            &[MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            }],
            &template_id,
            || {
                Ok(vec![MountEntry {
                    mount_id: 81,
                    preferred_source: Utf8PathBuf::from("/foreign/source"),
                    source_aliases: vec![Utf8PathBuf::from("/foreign/source")],
                    mount_point: target.clone(),
                    filesystem_type: "bind".to_string(),
                }])
            },
        )
        .expect_err("mounted target should be rejected");

        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMountTargetStillMounted {
                template_id: ref conflict_template_id,
                repo_id: ref conflict_repo_id,
                repo_path: ref conflict_repo_path,
                target: ref conflict_target,
            }) if conflict_template_id == &template_id
                && conflict_repo_id == &repo_id
                && conflict_repo_path == &rel("ai")
                && conflict_target == &target
        ));
        assert_eq!(
            fs::read_to_string(target.join("tracked.txt")).expect("read hidden target file"),
            "hidden repo-owned file"
        );
        assert!(
            !context_root.join("prompt.md").exists(),
            "failed inspection should roll back copied context additions"
        );
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_template_rejects_target_already_mounted_from_expected_source() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::add_mount(&workspace, repo_id.clone(), rel("ai"), rel("ai"))
            .expect("add existing mount");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let target = workspace.repo_path(&repo_id, &rel("ai"));
        fs::create_dir_all(&target).expect("create target");
        fs::write(target.join("tracked.txt"), "hidden repo-owned file")
            .expect("write hidden target file");

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
        let context_root = workspace.context_path(&repo_id, &rel("ai"));
        let template_prompt = workspace
            .template_context_root(&template_id)
            .join("ai/prompt.md");
        fs::create_dir_all(template_prompt.parent().expect("template prompt parent"))
            .expect("create template prompt dir");
        fs::write(&template_prompt, "seed").expect("write template file");

        let error = super::apply::prepare_template_application_with_mount_table_loader(
            &workspace,
            &repo_id,
            &[MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            }],
            &template_id,
            || {
                Ok(vec![MountEntry {
                    mount_id: 82,
                    preferred_source: context_root.clone(),
                    source_aliases: vec![context_root.clone()],
                    mount_point: target.clone(),
                    filesystem_type: "bind".to_string(),
                }])
            },
        )
        .expect_err("same-source mounted target should be rejected");

        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMountTargetStillMounted {
                template_id: ref conflict_template_id,
                repo_id: ref conflict_repo_id,
                repo_path: ref conflict_repo_path,
                target: ref conflict_target,
            }) if conflict_template_id == &template_id
                && conflict_repo_id == &repo_id
                && conflict_repo_path == &rel("ai")
                && conflict_target == &target
        ));
        assert_eq!(
            fs::read_to_string(target.join("tracked.txt")).expect("read hidden target file"),
            "hidden repo-owned file"
        );
        assert!(
            !context_root.join("prompt.md").exists(),
            "failed inspection should roll back copied template additions"
        );
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[cfg(unix)]
    #[test]
    fn apply_template_allows_duplicate_mount_when_materialized_target_matches_symlink_tree() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        crate::mounts::add_mount(&workspace, repo_id.clone(), rel("ai"), rel("ai"))
            .expect("add existing mount");

        let source = workspace.context_path(&repo_id, &rel("ai"));
        fs::create_dir_all(&source).expect("create source");
        symlink("shared-target", source.join("link")).expect("create source symlink");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        test_support::init_git_repo(&repo_root);
        let target = workspace.repo_path(&repo_id, &rel("ai"));
        fs::create_dir_all(&target).expect("create target");
        symlink("shared-target", target.join("link")).expect("create target symlink");

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

        apply_template_to_registered_repo(&workspace, &template_id, &repo_id)
            .expect("matching symlink tree should be allowed");

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert_eq!(
            manifest
                .repo_templates
                .get(&repo_id)
                .expect("applied template")
                .template,
            template_id
        );
        let mounts = crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("mounts");
        assert_eq!(
            mounts,
            vec![MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            }]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn delete_template_clears_bookkeeping_but_preserves_repo_mounts() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: crate::shared::types::CloneSource::new("file:///tmp/source")
                    .expect("valid source"),
            },
        )
        .expect("register repo");
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

        apply_template_to_registered_repo(&workspace, &template_id, &repo_id).expect("apply");
        delete_template(&workspace, &template_id).expect("delete template");

        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));
        let mounts = crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("mounts");
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].context, rel("ai"));
        assert_eq!(
            fs::read_to_string(
                workspace
                    .context_path(&repo_id, &rel("ai"))
                    .join("prompt.md")
            )
            .expect("read copied file"),
            "seed"
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn delete_template_rolls_back_when_clearing_bindings_fails() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        let prompt_path = workspace
            .template_context_root(&template_id)
            .join("ai/prompt.md");
        fs::create_dir_all(prompt_path.parent().expect("prompt parent"))
            .expect("create prompt dir");
        fs::write(&prompt_path, "seed").expect("write template file");
        fs::write(
            workspace.manifest_path(),
            "version = 1\n\n[repo_templates.demo]\ntemplate = \"default\"\n",
        )
        .expect("write invalid manifest");

        let error =
            delete_template(&workspace, &template_id).expect_err("invalid manifest should fail");

        assert!(matches!(
            error,
            Error::Validation(crate::error::ValidationError::RepoTemplateUnknownRepo { .. })
        ));
        assert!(workspace.template_root(&template_id).exists());
        assert_eq!(
            fs::read_to_string(
                workspace
                    .template_context_root(&template_id)
                    .join("ai/prompt.md")
            )
            .expect("read restored template file"),
            "seed"
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn delete_template_restores_bookkeeping_when_final_removal_fails() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let template_id = TemplateId::new("default").expect("valid template id");

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            crate::registry::RegisteredRepo {
                source: crate::shared::types::CloneSource::new("file:///tmp/source")
                    .expect("valid source"),
            },
        )
        .expect("register repo");
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

        apply_template_to_registered_repo(&workspace, &template_id, &repo_id).expect("apply");

        let error = super::delete::delete_template_with_cleanup(&workspace, &template_id, |_| {
            Err(std::io::Error::other("boom").into())
        })
        .expect_err("delete should fail");

        assert!(error.to_string().contains("boom"));
        assert!(workspace.template_root(&template_id).exists());
        assert_eq!(
            fs::read_to_string(
                workspace
                    .template_context_root(&template_id)
                    .join("ai/prompt.md")
            )
            .expect("read restored template file"),
            "seed"
        );
        assert_eq!(
            crate::templates::store::applied_template(&workspace, &repo_id)
                .expect("read applied template")
                .expect("template binding")
                .template_id,
            template_id
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn delete_template_restores_unbound_template_when_final_removal_fails() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        let prompt_path = workspace
            .template_context_root(&template_id)
            .join("ai/prompt.md");
        fs::create_dir_all(prompt_path.parent().expect("prompt parent"))
            .expect("create prompt dir");
        fs::write(&prompt_path, "seed").expect("write template file");

        let error = super::delete::delete_template_with_cleanup(&workspace, &template_id, |_| {
            Err(std::io::Error::other("boom").into())
        })
        .expect_err("delete should fail");

        assert!(error.to_string().contains("boom"));
        assert!(workspace.template_root(&template_id).exists());
        assert_eq!(
            fs::read_to_string(
                workspace
                    .template_context_root(&template_id)
                    .join("ai/prompt.md")
            )
            .expect("read restored template file"),
            "seed"
        );
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(manifest.repo_templates.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn template_manifest_store_roundtrip_and_list() {
        let workspace = test_workspace();
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("base").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        let manifest = crate::templates::domain::TemplateManifest {
            version: crate::templates::domain::TEMPLATE_MANIFEST_VERSION,
            mounts: vec![MountSpec {
                context: rel("ctx"),
                repo: rel("repo"),
            }],
        };
        save_template_manifest(&workspace, &template_id, &manifest)
            .expect("save template manifest");

        let loaded =
            load_template_manifest(&workspace, &template_id).expect("load template manifest");
        assert_eq!(loaded.mounts.len(), 1);
        assert_eq!(list_templates(&workspace).unwrap(), vec![template_id]);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn list_templates_filters_non_templates_and_sorts_results() {
        let workspace = test_workspace();
        workspace::ensure_templates_dir(&workspace).expect("create templates dir");

        for name in ["zulu", "alpha"] {
            let template_id = TemplateId::new(name).expect("valid template id");
            crate::templates::infra::create_template_layout(&workspace, &template_id)
                .expect("create template layout");
            save_template_manifest(
                &workspace,
                &template_id,
                &crate::templates::domain::TemplateManifest::default(),
            )
            .expect("save template manifest");
        }

        fs::create_dir_all(workspace.templates_dir().join("ignored")).expect("create ignored dir");
        fs::write(
            workspace.templates_dir().join("README.md"),
            "not a template",
        )
        .expect("write ignored file");

        let templates = list_templates(&workspace)
            .expect("list templates")
            .into_iter()
            .map(|template_id| template_id.to_string())
            .collect::<Vec<_>>();

        assert_eq!(templates, vec!["alpha", "zulu"]);

        let _ = fs::remove_dir_all(workspace.root());
    }
}
