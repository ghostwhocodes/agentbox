use crate::{
    error::{RepoWorkflowError, Result},
    persistence::{
        manifest::PersistedTemplateBinding, transaction, transaction::TemplatesTransaction,
    },
    shared::types::{RepoId, TemplateId},
    templates::domain::AppliedTemplate,
    workspace::Workspace,
};

#[cfg(test)]
use crate::persistence::{manifest::PersistedManifest, manifest_store};

#[cfg(test)]
fn ensure_registered_repo(manifest: &PersistedManifest, repo_id: &RepoId) -> Result<()> {
    if manifest.repos.contains_key(repo_id) {
        Ok(())
    } else {
        Err(RepoWorkflowError::RepoNotRegistered {
            repo_id: repo_id.clone(),
        }
        .into())
    }
}

#[cfg(test)]
pub(crate) fn applied_template(
    workspace: &Workspace,
    repo_id: &RepoId,
) -> Result<Option<AppliedTemplate>> {
    let manifest = manifest_store::read(workspace)?;
    load_applied_template(&manifest, repo_id)
}

#[cfg(test)]
fn load_applied_template(
    manifest: &PersistedManifest,
    repo_id: &RepoId,
) -> Result<Option<AppliedTemplate>> {
    ensure_registered_repo(manifest, repo_id)?;
    Ok(manifest
        .repo_templates
        .get(repo_id)
        .cloned()
        .map(Into::into))
}

#[cfg(test)]
pub(crate) fn replace_applied_template(
    workspace: &Workspace,
    repo_id: &RepoId,
    applied_template: AppliedTemplate,
) -> Result<()> {
    let repo_id = repo_id.clone();
    transaction::transaction(workspace, move |tx| {
        let mut templates_tx = tx.templates();
        replace_applied_template_tx(&mut templates_tx, &repo_id, applied_template)
    })
}

pub(crate) fn replace_applied_template_tx(
    tx: &mut TemplatesTransaction<'_>,
    repo_id: &RepoId,
    applied_template: AppliedTemplate,
) -> Result<()> {
    if !tx.contains_repo(repo_id) {
        return Err(RepoWorkflowError::RepoNotRegistered {
            repo_id: repo_id.clone(),
        }
        .into());
    }

    tx.replace_applied_template(
        repo_id.clone(),
        PersistedTemplateBinding {
            template: applied_template.template_id,
        },
    );
    Ok(())
}

#[cfg(test)]
pub(crate) fn clear_applied_template(workspace: &Workspace, repo_id: &RepoId) -> Result<()> {
    let repo_id = repo_id.clone();
    transaction::transaction(workspace, move |tx| {
        let mut templates_tx = tx.templates();
        clear_applied_template_tx(&mut templates_tx, &repo_id);
        Ok(())
    })
}

pub(crate) fn clear_applied_template_tx(tx: &mut TemplatesTransaction<'_>, repo_id: &RepoId) {
    tx.clear_applied_template(repo_id);
}

pub(crate) fn clear_applied_template_bindings(
    workspace: &Workspace,
    template_id: &TemplateId,
) -> Result<Vec<RepoId>> {
    let template_id = template_id.clone();
    transaction::transaction(workspace, move |tx| {
        let mut templates_tx = tx.templates();
        Ok(clear_applied_template_bindings_tx(
            &mut templates_tx,
            &template_id,
        ))
    })
}

pub(crate) fn restore_applied_template_bindings(
    workspace: &Workspace,
    template_id: &TemplateId,
    repo_ids: Vec<RepoId>,
) -> Result<()> {
    let template_id = template_id.clone();
    transaction::transaction(workspace, move |tx| {
        let mut templates_tx = tx.templates();
        restore_applied_template_bindings_tx(&mut templates_tx, &template_id, repo_ids)
    })
}

pub(crate) fn clear_applied_template_bindings_tx(
    tx: &mut TemplatesTransaction<'_>,
    template_id: &TemplateId,
) -> Vec<RepoId> {
    tx.clear_applied_template_bindings(template_id)
}

pub(crate) fn restore_applied_template_bindings_tx(
    tx: &mut TemplatesTransaction<'_>,
    template_id: &TemplateId,
    repo_ids: Vec<RepoId>,
) -> Result<()> {
    for repo_id in &repo_ids {
        if !tx.contains_repo(repo_id) {
            return Err(RepoWorkflowError::RepoNotRegistered {
                repo_id: repo_id.clone(),
            }
            .into());
        }
    }

    tx.restore_applied_template_bindings(template_id, repo_ids);
    Ok(())
}

impl From<PersistedTemplateBinding> for AppliedTemplate {
    fn from(value: PersistedTemplateBinding) -> Self {
        Self {
            template_id: value.template,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        error::Error,
        registry::RegisteredRepo,
        shared::types::{CloneSource, TemplateId},
        test_support,
    };

    use super::*;

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-templates-store-test")
    }

    fn repo_id() -> RepoId {
        RepoId::new("demo").expect("valid repo id")
    }

    #[test]
    fn applied_template_returns_none_for_registered_repo_without_binding() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = repo_id();

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");

        let applied = applied_template(&workspace, &repo_id).expect("read applied template");
        assert_eq!(applied, None);
    }

    #[test]
    fn replace_applied_template_reports_unknown_repo() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = repo_id();

        let error = replace_applied_template(
            &workspace,
            &repo_id,
            AppliedTemplate {
                template_id: TemplateId::new("default").expect("valid template id"),
            },
        )
        .expect_err("missing repo should fail");

        assert!(matches!(
            error,
            Error::Repo(crate::shared::error::RepoWorkflowError::RepoNotRegistered {
                repo_id: ref missing_repo_id,
            }) if missing_repo_id == &repo_id
        ));
    }

    #[test]
    fn clear_applied_template_removes_existing_binding() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = repo_id();

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        replace_applied_template(
            &workspace,
            &repo_id,
            AppliedTemplate {
                template_id: TemplateId::new("default").expect("valid template id"),
            },
        )
        .expect("replace applied template");

        clear_applied_template(&workspace, &repo_id).expect("clear applied template");

        assert_eq!(
            applied_template(&workspace, &repo_id).expect("read applied template"),
            None
        );
    }

    #[test]
    fn clear_applied_template_bindings_removes_matching_template_rows() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = repo_id();

        crate::registry::register_repo(
            &workspace,
            repo_id.clone(),
            RegisteredRepo {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        )
        .expect("register repo");
        replace_applied_template(
            &workspace,
            &repo_id,
            AppliedTemplate {
                template_id: TemplateId::new("default").expect("valid template id"),
            },
        )
        .expect("replace applied template");

        let cleared = clear_applied_template_bindings(
            &workspace,
            &TemplateId::new("default").expect("valid template id"),
        )
        .expect("clear bindings");

        assert_eq!(cleared, vec![repo_id.clone()]);
        assert_eq!(
            applied_template(&workspace, &repo_id).expect("read applied template"),
            None
        );
    }
}
