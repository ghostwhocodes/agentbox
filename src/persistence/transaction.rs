use crate::{
    persistence::manifest::{
        PersistedManifest, PersistedRepoMount, PersistedRepoRegistration, PersistedTemplateBinding,
    },
    shared::{
        error::Result,
        types::{RepoId, TemplateId},
    },
    workspace::Workspace,
};

use super::manifest_store;

/// Run one manifest transaction and persist the updated manifest on success.
pub(crate) fn transaction<T>(
    workspace: &Workspace,
    f: impl FnOnce(&mut ManifestTransaction) -> Result<T>,
) -> Result<T> {
    let manifest = manifest_store::read(workspace)?;
    let mut tx = ManifestTransaction { manifest };
    let result = f(&mut tx)?;
    manifest_store::write(workspace, &tx.manifest)?;
    Ok(result)
}

/// Private transaction wrapper around the persisted workspace manifest.
pub(crate) struct ManifestTransaction {
    manifest: PersistedManifest,
}

impl ManifestTransaction {
    pub(crate) fn registry(&mut self) -> RegistryTransaction<'_> {
        RegistryTransaction {
            manifest: &mut self.manifest,
        }
    }

    pub(crate) fn mounts(&mut self) -> MountsTransaction<'_> {
        MountsTransaction {
            manifest: &mut self.manifest,
        }
    }

    pub(crate) fn templates(&mut self) -> TemplatesTransaction<'_> {
        TemplatesTransaction {
            manifest: &mut self.manifest,
        }
    }
}

pub(crate) struct RegistryTransaction<'a> {
    manifest: &'a mut PersistedManifest,
}

impl RegistryTransaction<'_> {
    pub(crate) fn contains_repo(&self, repo_id: &RepoId) -> bool {
        self.manifest.repos.contains_key(repo_id)
    }

    pub(crate) fn insert_repo(&mut self, repo_id: RepoId, repo: PersistedRepoRegistration) {
        self.manifest.repos.insert(repo_id, repo);
    }

    pub(crate) fn remove_repo(&mut self, repo_id: &RepoId) -> Option<PersistedRepoRegistration> {
        self.manifest.repos.remove(repo_id)
    }
}

pub(crate) struct MountsTransaction<'a> {
    manifest: &'a mut PersistedManifest,
}

impl MountsTransaction<'_> {
    pub(crate) fn contains_repo(&self, repo_id: &RepoId) -> bool {
        self.manifest.repos.contains_key(repo_id)
    }

    pub(crate) fn replace_repo_mounts(
        &mut self,
        repo_id: &RepoId,
        mounts: Vec<PersistedRepoMount>,
    ) {
        self.clear_repo_mounts(repo_id);
        self.manifest.repo_mounts.extend(mounts);
    }

    pub(crate) fn clear_repo_mounts(&mut self, repo_id: &RepoId) {
        self.manifest
            .repo_mounts
            .retain(|mount| &mount.repo_id != repo_id);
    }
}

pub(crate) struct TemplatesTransaction<'a> {
    manifest: &'a mut PersistedManifest,
}

impl TemplatesTransaction<'_> {
    pub(crate) fn contains_repo(&self, repo_id: &RepoId) -> bool {
        self.manifest.repos.contains_key(repo_id)
    }

    pub(crate) fn replace_applied_template(
        &mut self,
        repo_id: RepoId,
        applied_template: PersistedTemplateBinding,
    ) {
        self.manifest
            .repo_templates
            .insert(repo_id, applied_template);
    }

    pub(crate) fn clear_applied_template(&mut self, repo_id: &RepoId) {
        self.manifest.repo_templates.remove(repo_id);
    }

    pub(crate) fn clear_applied_template_bindings(
        &mut self,
        template_id: &TemplateId,
    ) -> Vec<RepoId> {
        let removed_repo_ids = self
            .manifest
            .repo_templates
            .iter()
            .filter(|(_, binding)| binding.template == *template_id)
            .map(|(repo_id, _)| repo_id.clone())
            .collect::<Vec<_>>();
        self.manifest
            .repo_templates
            .retain(|_, binding| binding.template != *template_id);
        removed_repo_ids
    }

    pub(crate) fn restore_applied_template_bindings(
        &mut self,
        template_id: &TemplateId,
        repo_ids: impl IntoIterator<Item = RepoId>,
    ) {
        for repo_id in repo_ids {
            self.manifest.repo_templates.insert(
                repo_id,
                PersistedTemplateBinding {
                    template: template_id.clone(),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        persistence::manifest::PersistedRepoRegistration,
        shared::{
            error::{Error, RepoWorkflowError},
            types::{CloneSource, RepoId},
        },
        test_support,
    };

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-manifest-transaction-test")
    }

    fn repo_id() -> RepoId {
        RepoId::new("demo").expect("valid repo id")
    }

    fn persisted_repo() -> PersistedRepoRegistration {
        PersistedRepoRegistration {
            source: CloneSource::new("file:///tmp/source").expect("valid source"),
        }
    }

    #[test]
    fn transaction_persists_manifest_updates_on_success() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = repo_id();

        transaction(&workspace, |tx| {
            let mut registry = tx.registry();
            registry.insert_repo(repo_id.clone(), persisted_repo());
            Ok(())
        })
        .expect("transaction succeeds");

        let manifest = manifest_store::read(&workspace).expect("load manifest");
        assert!(manifest.repos.contains_key(&repo_id));
    }

    #[test]
    fn transaction_discards_manifest_updates_on_error() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = repo_id();

        let error = transaction(&workspace, |tx| {
            let mut registry = tx.registry();
            registry.insert_repo(repo_id.clone(), persisted_repo());
            Err::<(), _>(Error::from(RepoWorkflowError::RepoAlreadyRegistered {
                repo_id: repo_id.clone(),
            }))
        })
        .expect_err("transaction should fail");

        assert!(matches!(
            error,
            Error::Repo(RepoWorkflowError::RepoAlreadyRegistered {
                repo_id: ref duplicate_repo_id,
            }) if duplicate_repo_id == &repo_id
        ));

        let manifest = manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repos.contains_key(&repo_id));
    }
}
