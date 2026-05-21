//! Atomic cross-context manifest updates.
//!
//! Composite workflows that touch more than one bounded context should enter
//! the persistence layer here so the manifest transaction boundary stays at
//! the storage edge rather than in app code.

use crate::{
    mounts,
    persistence::transaction,
    registry::{self, RegisteredRepo},
    shared::{error::Result, mount_spec::MountSpec, types::RepoId},
    templates::{self, AppliedTemplate},
    workspace::Workspace,
};

pub(crate) fn persist_attached_repo(
    workspace: &Workspace,
    repo_id: RepoId,
    repo: RegisteredRepo,
    repo_mounts: Vec<MountSpec>,
    applied_template: Option<AppliedTemplate>,
) -> Result<()> {
    transaction::transaction(workspace, move |tx| {
        {
            let mut registry_tx = tx.registry();
            registry::register_repo_tx(&mut registry_tx, repo_id.clone(), repo)?;
        }
        {
            let mut mounts_tx = tx.mounts();
            mounts::replace_repo_mounts_tx(&mut mounts_tx, &repo_id, repo_mounts)?;
        }
        if let Some(applied_template) = applied_template {
            let mut templates_tx = tx.templates();
            templates::replace_applied_template_tx(&mut templates_tx, &repo_id, applied_template)?;
        }
        Ok(())
    })
}

pub(crate) fn remove_registered_repo(
    workspace: &Workspace,
    repo_id: &RepoId,
) -> Result<RegisteredRepo> {
    let repo_id = repo_id.clone();
    transaction::transaction(workspace, move |tx| {
        let removed = {
            let mut registry_tx = tx.registry();
            registry::remove_registered_repo_record_tx(&mut registry_tx, &repo_id)?
        };
        {
            let mut mounts_tx = tx.mounts();
            mounts::clear_repo_mounts_tx(&mut mounts_tx, &repo_id);
        }
        {
            let mut templates_tx = tx.templates();
            templates::clear_applied_template_tx(&mut templates_tx, &repo_id);
        }
        Ok(removed)
    })
}

pub(crate) fn apply_template_to_repo(
    workspace: &Workspace,
    repo_id: &RepoId,
    updated_mounts: Vec<MountSpec>,
    applied_template: AppliedTemplate,
) -> Result<()> {
    let repo_id = repo_id.clone();
    transaction::transaction(workspace, move |tx| {
        {
            let mut mounts_tx = tx.mounts();
            mounts::replace_repo_mounts_tx(&mut mounts_tx, &repo_id, updated_mounts)?;
        }
        {
            let mut templates_tx = tx.templates();
            templates::replace_applied_template_tx(&mut templates_tx, &repo_id, applied_template)?;
        }
        Ok(())
    })
}
