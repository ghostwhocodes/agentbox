use camino::Utf8PathBuf;

use crate::{
    error::{RepoWorkflowError, Result},
    registry::git,
    shared::types::RepoId,
    workspace::Workspace,
};

use super::{is_repo_materialized, selected_registered_repos};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeOutcome {
    Materialized {
        repo_id: RepoId,
        repo_root: Utf8PathBuf,
    },
    AlreadyMaterialized {
        repo_id: RepoId,
        repo_root: Utf8PathBuf,
    },
}

pub fn materialize_repos(
    workspace: &Workspace,
    requested: Option<RepoId>,
) -> Result<Vec<MaterializeOutcome>> {
    let mut outcomes = Vec::new();

    for (repo_id, repo) in selected_registered_repos(workspace, requested)? {
        let repo_root = workspace.repo_root(&repo_id);
        if repo_root.exists() {
            if is_repo_materialized(workspace, &repo_id) {
                outcomes.push(MaterializeOutcome::AlreadyMaterialized { repo_id, repo_root });
                continue;
            }
            return Err(RepoWorkflowError::DestinationExistsNotGitRepo { repo_root }.into());
        }

        git::clone_repo(&repo.source, &repo_root)?;
        outcomes.push(MaterializeOutcome::Materialized { repo_id, repo_root });
    }

    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::shared::error::{Error, RepoWorkflowError};
    use crate::shared::types::RepoId;

    use super::super::test_support::{insert_repo, test_workspace};

    #[test]
    fn materialize_rejects_existing_non_git_directory() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo(&workspace, &repo_id, vec![]);
        fs::create_dir_all(workspace.repo_root(&repo_id)).expect("create plain repo root");

        let error = materialize_repos(&workspace, Some(repo_id.clone()))
            .expect_err("plain directory should be rejected");
        assert!(matches!(
            error,
            Error::Repo(RepoWorkflowError::DestinationExistsNotGitRepo { repo_root })
                if repo_root == workspace.repo_root(&repo_id)
        ));

        let _ = fs::remove_dir_all(workspace.root());
    }
}
