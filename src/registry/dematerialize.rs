use crate::{
    error::{RepoWorkflowError, Result},
    inspection::{RepoLifecyclePreflight, selected_repo_lifecycle_preflights},
    shared::types::RepoId,
    workspace::Workspace,
};

use super::remove_repo_root;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DematerializeOutcome {
    Dematerialized { repo_id: RepoId },
    AlreadyDematerialized { repo_id: RepoId },
}

// Note: there is a TOCTOU race between checking mount state and removing the repo root.
// Between inspecting repo mount state and removing the repo root, a mount could
// theoretically become active.
// This is a known limitation; atomic check-and-remove is not feasible without kernel support.
pub fn dematerialize_repos(
    workspace: &Workspace,
    requested: Option<RepoId>,
) -> Result<Vec<DematerializeOutcome>> {
    dematerialize_repos_with_preflight_loader(
        workspace,
        requested,
        selected_repo_lifecycle_preflights,
    )
}

fn dematerialize_repos_with_preflight_loader<F>(
    workspace: &Workspace,
    requested: Option<RepoId>,
    load_preflights: F,
) -> Result<Vec<DematerializeOutcome>>
where
    F: FnOnce(&Workspace, Option<RepoId>) -> Result<Vec<RepoLifecyclePreflight>>,
{
    let preflights = load_preflights(workspace, requested)?;
    dematerialize_repos_with_states(workspace, preflights)
}

fn dematerialize_repos_with_states(
    workspace: &Workspace,
    preflights: Vec<RepoLifecyclePreflight>,
) -> Result<Vec<DematerializeOutcome>> {
    let mut outcomes = Vec::new();
    for preflight in preflights {
        let repo_id = preflight.repo_id;
        if preflight.has_conflicting_mounts {
            return Err(RepoWorkflowError::RepoHasConflictingMounts { repo_id }.into());
        }
        if preflight.has_active_mounts {
            return Err(RepoWorkflowError::RepoHasActiveMounts { repo_id }.into());
        }
        if preflight.materialized {
            remove_repo_root(workspace, &repo_id)?;
            outcomes.push(DematerializeOutcome::Dematerialized { repo_id });
        } else {
            outcomes.push(DematerializeOutcome::AlreadyDematerialized { repo_id });
        }
    }

    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        error::{Error, RepoWorkflowError},
        shared::types::RepoId,
    };

    use super::super::test_support::{materialized_repo_with_mount, test_workspace};

    #[test]
    fn dematerialize_fails_when_repo_has_active_mounts() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);

        let error = dematerialize_repos_with_states(
            &workspace,
            vec![RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: true,
                has_active_mounts: true,
                has_conflicting_mounts: false,
                active_mount_targets: vec![],
                unverified_active_mounts: Vec::new(),
            }],
        )
        .expect_err("active mount should block dematerialize");

        assert!(matches!(
            error,
            Error::Repo(RepoWorkflowError::RepoHasActiveMounts {
                repo_id: ref active_repo_id,
            }) if active_repo_id == &repo_id
        ));
        assert!(workspace.repo_root(&repo_id).exists());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn dematerialize_fails_when_repo_has_conflicting_mounts() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);

        let error = dematerialize_repos_with_states(
            &workspace,
            vec![RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: true,
                has_active_mounts: true,
                has_conflicting_mounts: true,
                active_mount_targets: vec![],
                unverified_active_mounts: Vec::new(),
            }],
        )
        .expect_err("conflicting mount should block dematerialize");

        assert!(matches!(
            error,
            Error::Repo(RepoWorkflowError::RepoHasConflictingMounts {
                repo_id: ref conflicted_repo_id,
            }) if conflicted_repo_id == &repo_id
        ));
        assert!(workspace.repo_root(&repo_id).exists());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn dematerialize_reports_already_dematerialized_when_repo_root_is_missing() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        super::super::test_support::insert_repo(&workspace, &repo_id, vec![]);

        let outcomes = dematerialize_repos_with_states(
            &workspace,
            vec![RepoLifecyclePreflight {
                repo_id: repo_id.clone(),
                materialized: false,
                has_active_mounts: false,
                has_conflicting_mounts: false,
                active_mount_targets: vec![],
                unverified_active_mounts: Vec::new(),
            }],
        )
        .expect("dematerialize");

        assert_eq!(
            outcomes,
            vec![DematerializeOutcome::AlreadyDematerialized { repo_id }]
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn dematerialize_fails_closed_when_mount_inspection_is_unavailable() {
        let workspace = test_workspace();
        let repo_id = materialized_repo_with_mount(&workspace);

        let error =
            dematerialize_repos_with_preflight_loader(&workspace, Some(repo_id.clone()), |_, _| {
                Err(Error::io_path(
                    "/proc/self/mountinfo",
                    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
                ))
            })
            .expect_err("mount inspection failure should block dematerialize");

        assert!(
            matches!(error, Error::IoPath { ref path, .. } if path == &camino::Utf8PathBuf::from("/proc/self/mountinfo"))
        );
        assert!(workspace.repo_root(&repo_id).exists());

        let _ = fs::remove_dir_all(workspace.root());
    }
}
