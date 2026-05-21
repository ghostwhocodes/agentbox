//! Mount management workflows.
//!
//! Ownership in the target architecture:
//! - registered mount CRUD
//! - mount and unmount execution
//! - mount import helpers
//!
//! Dependency guardrail:
//! mount-specific policy should live here or in a dedicated mount context API,
//! while lower-level mount table and filesystem mechanics stay in `mounts::infra`.
mod edit;
mod import;
pub(crate) mod infra;
#[allow(clippy::module_inception)]
mod mount;
pub(crate) mod ownership;
mod registration;
mod remove;
mod store;
mod support;
mod unmount;

#[cfg(test)]
mod test_support;

pub use edit::{EditedMount, edit_mount};
pub use import::{ImportedMount, import_mount};
pub use mount::{MountOutcome, mount_repos};
pub use registration::{AddedMount, add_mount};
pub use remove::{RemovedMount, remove_mount};
pub use unmount::{UnmountOutcome, unmount_repos};

use crate::{error::Result, registry, shared::types::RepoId, workspace::Workspace};

#[cfg(test)]
pub(crate) use store::repo_mounts_in_manifest;
pub(crate) use store::{
    clear_repo_mounts_tx, replace_repo_mounts, replace_repo_mounts_tx, repo_mounts,
    repo_mounts_by_repo,
};

pub(crate) fn selected_repo_mounts(
    workspace: &Workspace,
    requested: Option<RepoId>,
) -> Result<Vec<(RepoId, Vec<crate::shared::mount_spec::MountSpec>)>> {
    let repo_ids = registry::selected_repo_ids(workspace, requested)?;
    let mut mounts_by_repo = repo_mounts_by_repo(workspace, &repo_ids)?;
    repo_ids
        .into_iter()
        .map(|repo_id| {
            Ok((
                repo_id.clone(),
                mounts_by_repo.remove(&repo_id).unwrap_or_default(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::Error,
        persistence::manifest::{PersistedRepoMount, PersistedRepoRegistration},
        shared::types::CloneSource,
        test_support,
    };

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-mounts-selection-test")
    }

    fn rel(path: &str) -> crate::shared::types::RelativePath {
        crate::shared::types::RelativePath::new(path, "test path").expect("valid relative path")
    }

    #[test]
    fn selected_repo_mounts_preserves_selected_repo_order() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        for repo_name in ["zulu", "alpha"] {
            let repo_id = RepoId::new(repo_name).expect("valid repo id");
            manifest.repos.insert(
                repo_id.clone(),
                PersistedRepoRegistration {
                    source: CloneSource::new("file:///tmp/source").expect("valid source"),
                },
            );
            manifest.repo_mounts.push(PersistedRepoMount {
                repo_id,
                context: rel("ctx"),
                repo: rel("target"),
            });
        }
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let selected = selected_repo_mounts(&workspace, None).expect("selected mounts");
        let ordered_ids: Vec<_> = selected
            .iter()
            .map(|(repo_id, _)| repo_id.as_str().to_string())
            .collect();

        assert_eq!(ordered_ids, vec!["alpha", "zulu"]);
        assert!(selected.iter().all(|(_, mounts)| mounts.len() == 1));
    }

    #[test]
    fn selected_repo_mounts_reports_unknown_repo() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let error = selected_repo_mounts(
            &workspace,
            Some(RepoId::new("missing").expect("valid repo id")),
        )
        .expect_err("missing repo should fail");

        assert!(matches!(
            error,
            Error::Repo(crate::shared::error::RepoWorkflowError::RepoNotRegistered { .. })
        ));
    }
}
