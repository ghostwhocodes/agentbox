use crate::{
    error::{Result, TemplateError},
    mounts,
    persistence::composite,
    shared::{
        fs_ops::{
            TreeCopyRollback, directory_tree_is_matching_subset,
            tree_is_directory_only_scaffolding, tree_is_empty,
        },
        mount_spec::MountSpec,
        types::{RepoId, TemplateId},
    },
    templates::{AppliedTemplate, domain, infra},
    workspace::Workspace,
};

type MountTable = Vec<mounts::infra::MountEntry>;

#[derive(Debug)]
pub(crate) struct PreparedTemplateApplication {
    pub(crate) updated_mounts: Vec<MountSpec>,
    copied_context: TreeCopyRollback,
}

impl PreparedTemplateApplication {
    pub(crate) fn rollback(self) -> Result<()> {
        self.copied_context.rollback()
    }
}

pub fn apply_template_to_registered_repo(
    workspace: &Workspace,
    template_id: &TemplateId,
    repo_id: &RepoId,
) -> Result<()> {
    crate::registry::registered_repo(workspace, repo_id)?;
    let existing_mounts = mounts::repo_mounts(workspace, repo_id)?;
    let prepared = prepare_template_application(workspace, repo_id, &existing_mounts, template_id)?;
    let result = composite::apply_template_to_repo(
        workspace,
        repo_id,
        prepared.updated_mounts.clone(),
        AppliedTemplate {
            template_id: template_id.clone(),
        },
    );
    if let Err(error) = result {
        prepared.rollback()?;
        return Err(error);
    }
    Ok(())
}

pub(crate) fn prepare_template_application(
    workspace: &Workspace,
    repo_id: &RepoId,
    existing_mounts: &[MountSpec],
    template_id: &TemplateId,
) -> Result<PreparedTemplateApplication> {
    prepare_template_application_with_mount_table_loader(
        workspace,
        repo_id,
        existing_mounts,
        template_id,
        mounts::infra::list_mounts,
    )
}

pub(super) fn prepare_template_application_with_mount_table_loader<F>(
    workspace: &Workspace,
    repo_id: &RepoId,
    existing_mounts: &[MountSpec],
    template_id: &TemplateId,
    list_mounts: F,
) -> Result<PreparedTemplateApplication>
where
    F: FnOnce() -> Result<MountTable>,
{
    if !infra::template_dir_exists(workspace, template_id) {
        return Err(TemplateError::TemplateMissing {
            template_id: template_id.clone(),
        }
        .into());
    }

    let manifest = infra::load_template_manifest(workspace, template_id)?;
    domain::validate_template_application(repo_id, existing_mounts, template_id, &manifest)?;
    let copied_context = infra::copy_template_context_into_repo(workspace, template_id, repo_id)?;
    let inspection = (|| -> Result<()> {
        if crate::registry::is_repo_materialized(workspace, repo_id) {
            let mount_table = load_mount_table_for_template_inspection(list_mounts)?;
            for mount in &manifest.mounts {
                let source = workspace.context_path(repo_id, &mount.context);
                let target = workspace.repo_path(repo_id, &mount.repo);
                ensure_template_target_is_unmounted(
                    &mount_table,
                    template_id,
                    repo_id,
                    mount,
                    &target,
                )?;
                if target.exists() && !tree_is_empty(&target)? {
                    let target_is_safe = if source.exists() {
                        directory_tree_is_matching_subset(&target, &source)?
                    } else {
                        tree_is_directory_only_scaffolding(&target)?
                    };
                    if !target_is_safe {
                        return Err(TemplateError::TemplateMountWouldHideDifferentFiles {
                            template_id: template_id.clone(),
                            repo_id: repo_id.clone(),
                            repo_path: mount.repo.clone(),
                            target,
                        }
                        .into());
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = inspection {
        copied_context.rollback()?;
        return Err(error);
    }

    Ok(PreparedTemplateApplication {
        updated_mounts: domain::merged_template_mounts(existing_mounts, manifest.mounts),
        copied_context,
    })
}

fn load_mount_table_for_template_inspection<F>(list_mounts: F) -> Result<Option<MountTable>>
where
    F: FnOnce() -> Result<MountTable>,
{
    match list_mounts() {
        Ok(mount_table) => Ok(Some(mount_table)),
        Err(crate::error::Error::InfraMount(crate::error::InfraMountError::Unsupported)) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn ensure_template_target_is_unmounted(
    mount_table: &Option<MountTable>,
    template_id: &TemplateId,
    repo_id: &RepoId,
    mount: &MountSpec,
    target: &camino::Utf8Path,
) -> Result<()> {
    if mount_table
        .as_ref()
        .and_then(|entries| mounts::infra::find_target_mount(entries, target))
        .is_some()
    {
        return Err(TemplateError::TemplateMountTargetStillMounted {
            template_id: template_id.clone(),
            repo_id: repo_id.clone(),
            repo_path: mount.repo.clone(),
            target: target.to_path_buf(),
        }
        .into());
    }

    Ok(())
}
