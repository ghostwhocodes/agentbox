use std::fs;

use crate::{
    error::{Error, InfraMountError, MountWorkflowError, Result},
    mounts::{self, infra as mount, ownership},
    shared::{
        fs_ops::move_directory_with_rollback,
        mount_spec::MountSpec,
        types::{RelativePath, RepoId},
    },
    workspace::Workspace,
};

use super::support::{map_mount_repo_error, mounts_with_imported_mount};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedMount {
    pub repo_id: RepoId,
    pub mount: MountSpec,
}

#[derive(Debug, Clone)]
struct ImportRequest {
    repo_id: RepoId,
    repo_path: RelativePath,
    context_path: Option<RelativePath>,
    no_mount: bool,
}

pub fn import_mount(
    workspace: &Workspace,
    repo_id: RepoId,
    repo_path: RelativePath,
    context_path: Option<RelativePath>,
    no_mount: bool,
) -> Result<ImportedMount> {
    import_mount_with_loader(
        workspace,
        ImportRequest {
            repo_id,
            repo_path,
            context_path,
            no_mount,
        },
        mount::list_mounts,
        mount::bind_mount,
    )
}

fn import_mount_with_loader<L, F>(
    workspace: &Workspace,
    request: ImportRequest,
    mut list_mounts: L,
    bind_target: F,
) -> Result<ImportedMount>
where
    L: FnMut() -> Result<Vec<mount::MountEntry>>,
    F: FnMut(&camino::Utf8Path, &camino::Utf8Path) -> Result<()>,
{
    let mount_table = match list_mounts() {
        Ok(mount_table) => Some(mount_table),
        Err(Error::InfraMount(InfraMountError::Unsupported)) if request.no_mount => None,
        Err(error) => return Err(error),
    };
    import_mount_inner(
        workspace,
        request,
        mount_table.as_deref(),
        bind_target,
        |repo_id, mount_spec, target| {
            record_live_mount_best_effort(workspace, repo_id, mount_spec, target, &mut list_mounts);
        },
    )
}

#[cfg(test)]
fn import_mount_in_with_mount_table<F>(
    workspace: &Workspace,
    request: ImportRequest,
    mount_table: Vec<mount::MountEntry>,
    bind_target: F,
) -> Result<ImportedMount>
where
    F: FnMut(&camino::Utf8Path, &camino::Utf8Path) -> Result<()>,
{
    import_mount_inner(
        workspace,
        request,
        Some(&mount_table),
        bind_target,
        |_, _, _| {},
    )
}

fn import_mount_inner<F, R>(
    workspace: &Workspace,
    request: ImportRequest,
    mount_table: Option<&[mount::MountEntry]>,
    mut bind_target: F,
    mut record_bound_mount: R,
) -> Result<ImportedMount>
where
    F: FnMut(&camino::Utf8Path, &camino::Utf8Path) -> Result<()>,
    R: FnMut(&RepoId, &MountSpec, &camino::Utf8Path),
{
    let ImportRequest {
        repo_id,
        repo_path,
        context_path,
        no_mount,
    } = request;

    let existing_mounts = mounts::repo_mounts(workspace, &repo_id).map_err(map_mount_repo_error)?;
    crate::registry::require_materialized_repo(workspace, &repo_id)?;

    let mount_spec = MountSpec {
        context: context_path.unwrap_or_else(|| repo_path.clone()),
        repo: repo_path,
    };
    let next_mounts = mounts_with_imported_mount(&repo_id, &existing_mounts, mount_spec.clone())?;

    let source = workspace.context_path(&repo_id, &mount_spec.context);
    let target = workspace.repo_path(&repo_id, &mount_spec.repo);

    if mount_table
        .is_some_and(|mount_table| mount::find_target_mount(mount_table, &target).is_some())
    {
        return Err(MountWorkflowError::ImportTargetAlreadyMounted { target }.into());
    }

    move_directory_with_rollback(&target, &source, || {
        mounts::replace_repo_mounts(workspace, &repo_id, next_mounts.clone())
            .map_err(map_mount_repo_error)?;

        if no_mount {
            return Ok(());
        }

        fs::create_dir_all(&target).map_err(|error| Error::io_path(&target, error))?;
        if let Err(error) = bind_target(&source, &target) {
            mounts::replace_repo_mounts(workspace, &repo_id, existing_mounts.clone())
                .map_err(map_mount_repo_error)?;
            return Err(error);
        }
        record_bound_mount(&repo_id, &mount_spec, &target);

        Ok(())
    })?;

    Ok(ImportedMount {
        repo_id,
        mount: mount_spec,
    })
}

fn record_live_mount_best_effort(
    workspace: &Workspace,
    repo_id: &RepoId,
    mount_spec: &MountSpec,
    target: &camino::Utf8Path,
    list_mounts: &mut impl FnMut() -> Result<Vec<mount::MountEntry>>,
) {
    let Ok(mount_table) = list_mounts() else {
        return;
    };
    let Some(live_mount) = mount::find_target_mount(&mount_table, target) else {
        return;
    };

    ownership::record_best_effort(
        workspace,
        ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: mount_spec.context.clone(),
            repo: mount_spec.repo.clone(),
            mount_id: live_mount.mount_id,
            source_root: live_mount.preferred_source.clone(),
            filesystem_type: live_mount.filesystem_type.clone(),
        },
    );
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs};

    use camino::Utf8PathBuf;

    use super::*;
    use crate::{
        error::{Error, InfraMountError, MountWorkflowError},
        shared::types::CloneSource,
    };

    use super::super::test_support::{
        init_git_repo, init_repo, read_to_string, rel, test_workspace,
    };

    fn request(
        repo_id: RepoId,
        repo_path: RelativePath,
        context_path: Option<RelativePath>,
        no_mount: bool,
    ) -> ImportRequest {
        ImportRequest {
            repo_id,
            repo_path,
            context_path,
            no_mount,
        }
    }

    #[test]
    fn import_mount_remounts_by_default_after_move() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");

        let calls = RefCell::new(Vec::<(Utf8PathBuf, Utf8PathBuf)>::new());
        let imported = import_mount_in_with_mount_table(
            &workspace,
            request(repo_id.clone(), rel("target"), Some(rel("ctx")), false),
            vec![],
            |source, target| {
                calls
                    .borrow_mut()
                    .push((source.to_path_buf(), target.to_path_buf()));
                Ok(())
            },
        )
        .expect("import mount should succeed");

        let source = workspace.context_path(&repo_id, &rel("ctx"));
        assert_eq!(imported.repo_id, repo_id);
        assert_eq!(read_to_string(source.join("state.txt")), "state");
        assert!(target.is_dir());
        assert_eq!(calls.into_inner(), vec![(source.clone(), target.clone())]);
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            mounts::repo_mounts_in_manifest(&manifest, &imported.repo_id).expect("repo mounts");
        assert!(
            mounts
                .iter()
                .any(|mount| mount.context == rel("ctx") && mount.repo == rel("target"))
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_records_ownership_receipt_after_remount() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");
        let source = workspace.context_path(&repo_id, &rel("ctx"));
        let load_count = std::cell::Cell::new(0usize);

        let imported = import_mount_with_loader(
            &workspace,
            request(repo_id.clone(), rel("target"), Some(rel("ctx")), false),
            || {
                load_count.set(load_count.get() + 1);
                if load_count.get() == 1 {
                    Ok(Vec::new())
                } else {
                    Ok(vec![mount::MountEntry {
                        mount_id: 301,
                        preferred_source: source.clone(),
                        source_aliases: vec![source.clone()],
                        mount_point: target.clone(),
                        filesystem_type: "bind".to_string(),
                    }])
                }
            },
            |_, _| Ok(()),
        )
        .expect("import mount should succeed");

        assert_eq!(imported.repo_id, repo_id.clone());
        assert_eq!(load_count.get(), 2);
        let ownership = ownership::load(&workspace).expect("load ownership");
        assert_eq!(ownership.records().len(), 1);
        assert_eq!(ownership.records()[0].repo_id, repo_id);
        assert_eq!(ownership.records()[0].mount_id, 301);
        assert_eq!(ownership.records()[0].source_root, source);

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_rolls_back_when_bind_mount_fails() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let import_repo_path = rel("state");
        let target = workspace.repo_path(&repo_id, &import_repo_path);
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");

        let error = import_mount_in_with_mount_table(
            &workspace,
            request(
                repo_id.clone(),
                import_repo_path.clone(),
                Some(rel("ctx")),
                false,
            ),
            vec![],
            |_, _| Err(InfraMountError::Unsupported.into()),
        )
        .expect_err("bind mount failure should roll back");

        assert!(matches!(
            error,
            Error::InfraMount(crate::shared::error::InfraMountError::Unsupported)
        ));
        assert_eq!(read_to_string(target.join("state.txt")), "state");
        assert!(!workspace.context_path(&repo_id, &rel("ctx")).exists());
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts = mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert!(!mounts.iter().any(|mount| mount.context == rel("ctx")));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_rejects_duplicate_or_conflicting_mount_before_side_effects() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            crate::persistence::manifest::PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest
            .repo_mounts
            .push(crate::persistence::manifest::PersistedRepoMount {
                repo_id: repo_id.clone(),
                context: rel("ctx"),
                repo: rel("target"),
            });
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");

        let error = import_mount_in_with_mount_table(
            &workspace,
            request(repo_id.clone(), rel("target"), Some(rel("ctx")), false),
            vec![],
            |_, _| Ok(()),
        )
        .expect_err("duplicate mount should fail");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountExistsOrConflicts {
                repo_id: ref duplicate_repo_id,
                ..
            }) if duplicate_repo_id == &repo_id
        ));
        assert_eq!(read_to_string(target.join("state.txt")), "state");
        assert!(
            !workspace
                .context_path(&repo_id, &rel("ctx"))
                .join("state.txt")
                .exists()
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_rejects_occupied_target_before_move() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            crate::persistence::manifest::PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");

        let error = import_mount_in_with_mount_table(
            &workspace,
            request(repo_id.clone(), rel("target"), Some(rel("ctx")), false),
            vec![mount::MountEntry {
                mount_id: 71,
                preferred_source: Utf8PathBuf::from("/foreign/source"),
                source_aliases: vec![Utf8PathBuf::from("/foreign/source")],
                mount_point: target.clone(),
                filesystem_type: "bind".to_string(),
            }],
            |_, _| Ok(()),
        )
        .expect_err("occupied target should fail before move");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::ImportTargetAlreadyMounted {
                target: ref occupied_target,
            }) if occupied_target == &target
        ));
        assert_eq!(read_to_string(target.join("state.txt")), "state");
        assert!(!workspace.context_path(&repo_id, &rel("ctx")).exists());
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts =
            crate::mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert!(mounts.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_without_remount_checks_occupied_target_when_mount_table_is_available() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");

        let error = import_mount_with_loader(
            &workspace,
            request(repo_id.clone(), rel("target"), Some(rel("ctx")), true),
            || {
                Ok(vec![mount::MountEntry {
                    mount_id: 72,
                    preferred_source: Utf8PathBuf::from("/foreign/source"),
                    source_aliases: vec![Utf8PathBuf::from("/foreign/source")],
                    mount_point: target.clone(),
                    filesystem_type: "bind".to_string(),
                }])
            },
            |_, _| Ok(()),
        )
        .expect_err("occupied target should fail before move");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::ImportTargetAlreadyMounted {
                target: ref occupied_target,
            }) if occupied_target == &target
        ));
        assert_eq!(read_to_string(target.join("state.txt")), "state");
        assert!(
            !workspace
                .context_path(&repo_id, &rel("ctx"))
                .join("state.txt")
                .exists()
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_without_remount_proceeds_when_mount_inspection_is_unsupported() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");

        let imported = import_mount_with_loader(
            &workspace,
            request(repo_id.clone(), rel("target"), Some(rel("ctx")), true),
            || Err(InfraMountError::Unsupported.into()),
            |_, _| panic!("bind mount should not run for --no-mount"),
        )
        .expect("import mount should succeed when inspection is unsupported");

        assert_eq!(imported.repo_id, repo_id);
        assert_eq!(
            read_to_string(
                workspace
                    .context_path(&imported.repo_id, &rel("ctx"))
                    .join("state.txt")
            ),
            "state"
        );
        assert!(!target.exists());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_reports_unknown_repo_before_materialization_check() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("missing").expect("valid repo id");

        let error = import_mount(&workspace, repo_id.clone(), rel("src"), None, true)
            .expect_err("missing repo should fail");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::RepoNotRegistered {
                repo_id: ref missing_repo_id,
            }) if missing_repo_id == &repo_id
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn import_mount_fails_closed_when_mount_table_is_malformed() {
        let workspace = test_workspace();
        let repo_id = init_repo(&workspace);
        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);

        let target = workspace.repo_path(&repo_id, &rel("target"));
        fs::create_dir_all(&target).expect("create repo-side dir");
        fs::write(target.join("state.txt"), "state").expect("write file");

        let bind_calls = std::cell::Cell::new(0usize);
        let error = import_mount_with_loader(
            &workspace,
            request(repo_id.clone(), rel("target"), Some(rel("ctx")), false),
            || {
                Err(Error::malformed_mount_table_entry(
                    12,
                    "missing ` - ` separator",
                ))
            },
            |_, _| {
                bind_calls.set(bind_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("malformed mount table should block import");

        assert!(matches!(
            error,
            Error::InfraMount(
                crate::shared::error::InfraMountError::MalformedMountTableEntry { line_no: 12, .. }
            )
        ));
        assert_eq!(bind_calls.get(), 0);
        assert_eq!(read_to_string(target.join("state.txt")), "state");
        assert!(!workspace.context_path(&repo_id, &rel("ctx")).exists());
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        let mounts = mounts::repo_mounts_in_manifest(&manifest, &repo_id).expect("repo mounts");
        assert!(mounts.is_empty());

        let _ = fs::remove_dir_all(workspace.root());
    }
}
