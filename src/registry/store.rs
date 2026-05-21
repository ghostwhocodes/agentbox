use crate::{
    persistence::{manifest::PersistedManifest, manifest_store, transaction::RegistryTransaction},
    shared::error::{Error, RepoWorkflowError, Result},
    shared::types::RepoId,
    workspace::Workspace,
};

use super::RegisteredRepo;

fn selected_repo_ids_in_manifest(
    manifest: &PersistedManifest,
    requested: Option<RepoId>,
) -> Result<Vec<RepoId>> {
    if let Some(repo_id) = requested {
        if manifest.repos.contains_key(&repo_id) {
            Ok(vec![repo_id])
        } else {
            Err(RepoWorkflowError::RepoNotRegistered { repo_id }.into())
        }
    } else {
        Ok(manifest.repos.keys().cloned().collect())
    }
}

pub(crate) fn selected_repo_ids(
    workspace: &Workspace,
    requested: Option<RepoId>,
) -> Result<Vec<RepoId>> {
    let manifest = manifest_store::read(workspace)?;
    selected_repo_ids_in_manifest(&manifest, requested)
}

pub(crate) fn selected_registered_repos(
    workspace: &Workspace,
    requested: Option<RepoId>,
) -> Result<Vec<(RepoId, RegisteredRepo)>> {
    let manifest = manifest_store::read(workspace)?;
    let repo_ids = selected_repo_ids_in_manifest(&manifest, requested)?;
    repo_ids
        .into_iter()
        .map(|repo_id| {
            let repo = registered_repo_in_manifest(&manifest, &repo_id)?;
            Ok((repo_id, repo))
        })
        .collect()
}

pub(crate) fn registered_repo(workspace: &Workspace, repo_id: &RepoId) -> Result<RegisteredRepo> {
    let manifest = manifest_store::read(workspace)?;
    registered_repo_in_manifest(&manifest, repo_id)
}

fn registered_repo_in_manifest(
    manifest: &PersistedManifest,
    repo_id: &RepoId,
) -> Result<RegisteredRepo> {
    manifest
        .repos
        .get(repo_id)
        .cloned()
        .map(RegisteredRepo::from)
        .ok_or_else(|| {
            RepoWorkflowError::RepoNotRegistered {
                repo_id: repo_id.clone(),
            }
            .into()
        })
}

pub(crate) fn register_repo_tx(
    tx: &mut RegistryTransaction<'_>,
    repo_id: RepoId,
    repo: RegisteredRepo,
) -> Result<()> {
    if tx.contains_repo(&repo_id) {
        return Err(RepoWorkflowError::RepoAlreadyRegistered { repo_id }.into());
    }

    tx.insert_repo(repo_id, repo.into());
    Ok(())
}

#[cfg(test)]
pub(crate) fn register_repo(
    workspace: &Workspace,
    repo_id: RepoId,
    repo: RegisteredRepo,
) -> Result<()> {
    crate::persistence::transaction::transaction(workspace, move |tx| {
        let mut registry = tx.registry();
        register_repo_tx(&mut registry, repo_id, repo)
    })
}

pub(crate) fn remove_registered_repo_record_tx(
    tx: &mut RegistryTransaction<'_>,
    repo_id: &RepoId,
) -> Result<RegisteredRepo> {
    let removed = tx.remove_repo(repo_id).ok_or_else(|| {
        Error::from(RepoWorkflowError::RepoNotRegistered {
            repo_id: repo_id.clone(),
        })
    })?;
    Ok(removed.into())
}

#[cfg(test)]
pub(crate) fn register_repo_in_manifest(
    manifest: &mut PersistedManifest,
    repo_id: RepoId,
    repo: RegisteredRepo,
) -> Result<()> {
    if manifest.repos.contains_key(&repo_id) {
        return Err(RepoWorkflowError::RepoAlreadyRegistered { repo_id }.into());
    }

    manifest.repos.insert(repo_id, repo.into());
    Ok(())
}

#[cfg(test)]
pub(crate) fn replace_registered_repo_in_manifest(
    manifest: &mut PersistedManifest,
    repo_id: &RepoId,
    repo: RegisteredRepo,
) -> Result<()> {
    if !manifest.repos.contains_key(repo_id) {
        return Err(RepoWorkflowError::RepoNotRegistered {
            repo_id: repo_id.clone(),
        }
        .into());
    }

    manifest.repos.insert(repo_id.clone(), repo.into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        mounts,
        persistence::{composite, manifest::PersistedManifest},
        shared::mount_spec::MountSpec,
        shared::types::CloneSource,
        templates, test_support,
    };

    fn repo_id() -> RepoId {
        RepoId::new("demo").expect("valid repo id")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-registry-store-test")
    }

    fn repo(source: &str) -> RegisteredRepo {
        RegisteredRepo {
            source: CloneSource::new(source).expect("valid source"),
        }
    }

    #[test]
    fn selected_repo_ids_returns_requested_repo() {
        let repo_id = repo_id();
        let mut manifest = PersistedManifest::default();
        register_repo_in_manifest(&mut manifest, repo_id.clone(), repo("file:///tmp/source"))
            .unwrap();

        let selected = selected_repo_ids_in_manifest(&manifest, Some(repo_id.clone())).unwrap();
        assert_eq!(selected, vec![repo_id]);
    }

    #[test]
    fn replace_registered_repo_updates_existing_repo() {
        let repo_id = repo_id();
        let mut manifest = PersistedManifest::default();
        register_repo_in_manifest(&mut manifest, repo_id.clone(), repo("file:///tmp/source"))
            .unwrap();

        replace_registered_repo_in_manifest(&mut manifest, &repo_id, repo("file:///tmp/next"))
            .unwrap();

        assert_eq!(
            registered_repo_in_manifest(&manifest, &repo_id)
                .unwrap()
                .source
                .as_str(),
            "file:///tmp/next"
        );
    }

    #[test]
    fn remove_registered_repo_returns_removed_repo() {
        let repo_id = repo_id();
        let workspace = test_workspace();

        crate::workspace::init_workspace(&workspace).expect("init workspace");
        register_repo(&workspace, repo_id.clone(), repo("file:///tmp/source")).unwrap();
        mounts::replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: crate::shared::types::RelativePath::new("ctx", "test path").unwrap(),
                repo: crate::shared::types::RelativePath::new("target", "test path").unwrap(),
            }],
        )
        .unwrap();
        templates::store::replace_applied_template(
            &workspace,
            &repo_id,
            templates::AppliedTemplate {
                template_id: crate::shared::types::TemplateId::new("default").unwrap(),
            },
        )
        .unwrap();

        let removed = composite::remove_registered_repo(&workspace, &repo_id).unwrap();
        let manifest = crate::persistence::manifest_store::read(&workspace).unwrap();

        assert_eq!(removed.source.as_str(), "file:///tmp/source");
        assert!(!manifest.repos.contains_key(&repo_id));
        assert!(manifest.repo_mounts.is_empty());
        assert!(!manifest.repo_templates.contains_key(&repo_id));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn register_repo_rejects_duplicate_repo_id() {
        let repo_id = repo_id();
        let mut manifest = PersistedManifest::default();
        let repo = repo("file:///tmp/source");

        register_repo_in_manifest(&mut manifest, repo_id.clone(), repo.clone()).unwrap();
        let error = register_repo_in_manifest(&mut manifest, repo_id.clone(), repo)
            .expect_err("duplicate repo should be rejected");

        assert!(matches!(
            error,
            crate::shared::error::Error::Repo(
                crate::shared::error::RepoWorkflowError::RepoAlreadyRegistered {
                    repo_id: ref duplicate_repo_id,
                }
            ) if duplicate_repo_id == &repo_id
        ));
    }
}
