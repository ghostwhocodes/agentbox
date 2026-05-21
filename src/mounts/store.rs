use std::collections::BTreeMap;

use crate::{
    error::{RepoWorkflowError, Result},
    persistence::{
        manifest::{PersistedManifest, PersistedRepoMount},
        manifest_store,
        transaction::{self, MountsTransaction},
    },
    shared::{
        mount_spec::{MountSpec, MountSpecOwner, validate_mount_specs},
        types::RepoId,
    },
    workspace::Workspace,
};

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

pub(crate) fn repo_mounts(workspace: &Workspace, repo_id: &RepoId) -> Result<Vec<MountSpec>> {
    let manifest = manifest_store::read(workspace)?;
    load_repo_mounts(&manifest, repo_id)
}

pub(crate) fn repo_mounts_by_repo(
    workspace: &Workspace,
    repo_ids: &[RepoId],
) -> Result<BTreeMap<RepoId, Vec<MountSpec>>> {
    let manifest = manifest_store::read(workspace)?;
    repo_mounts_by_repo_in_manifest(&manifest, repo_ids)
}

fn repo_mounts_by_repo_in_manifest(
    manifest: &PersistedManifest,
    repo_ids: &[RepoId],
) -> Result<BTreeMap<RepoId, Vec<MountSpec>>> {
    repo_ids
        .iter()
        .map(|repo_id| Ok((repo_id.clone(), load_repo_mounts(manifest, repo_id)?)))
        .collect()
}

fn load_repo_mounts(manifest: &PersistedManifest, repo_id: &RepoId) -> Result<Vec<MountSpec>> {
    ensure_registered_repo(manifest, repo_id)?;
    Ok(manifest
        .repo_mounts
        .iter()
        .filter(|mount| &mount.repo_id == repo_id)
        .map(|mount| MountSpec {
            context: mount.context.clone(),
            repo: mount.repo.clone(),
        })
        .collect())
}

#[cfg(test)]
pub(crate) fn repo_mounts_in_manifest(
    manifest: &PersistedManifest,
    repo_id: &RepoId,
) -> Result<Vec<MountSpec>> {
    load_repo_mounts(manifest, repo_id)
}

pub(crate) fn replace_repo_mounts(
    workspace: &Workspace,
    repo_id: &RepoId,
    mounts: Vec<MountSpec>,
) -> Result<()> {
    let repo_id = repo_id.clone();
    transaction::transaction(workspace, move |tx| {
        let mut mounts_tx = tx.mounts();
        replace_repo_mounts_tx(&mut mounts_tx, &repo_id, mounts)
    })
}

pub(crate) fn replace_repo_mounts_tx(
    tx: &mut MountsTransaction<'_>,
    repo_id: &RepoId,
    mounts: Vec<MountSpec>,
) -> Result<()> {
    if !tx.contains_repo(repo_id) {
        return Err(RepoWorkflowError::RepoNotRegistered {
            repo_id: repo_id.clone(),
        }
        .into());
    }

    validate_mount_specs(&MountSpecOwner::Repo(repo_id), &mounts)?;

    tx.replace_repo_mounts(
        repo_id,
        mounts
            .into_iter()
            .map(|mount| PersistedRepoMount {
                repo_id: repo_id.clone(),
                context: mount.context,
                repo: mount.repo,
            })
            .collect(),
    );
    Ok(())
}

pub(crate) fn clear_repo_mounts_tx(tx: &mut MountsTransaction<'_>, repo_id: &RepoId) {
    tx.clear_repo_mounts(repo_id);
}

#[cfg(test)]
mod tests {
    use crate::{
        error::Error, persistence::manifest::PersistedRepoRegistration, shared::types::CloneSource,
        test_support,
    };

    use super::*;

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-mounts-store-test")
    }

    fn rel(path: &str) -> crate::shared::types::RelativePath {
        crate::shared::types::RelativePath::new(path, "test path").expect("valid relative path")
    }

    #[test]
    fn repo_mounts_by_repo_preserves_requested_repo_order() {
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

        let requested = vec![
            RepoId::new("alpha").expect("valid repo id"),
            RepoId::new("zulu").expect("valid repo id"),
        ];
        let mounts_by_repo = repo_mounts_by_repo(&workspace, &requested).expect("repo mounts");

        assert_eq!(mounts_by_repo[&requested[0]].len(), 1);
        assert_eq!(mounts_by_repo[&requested[1]].len(), 1);
    }

    #[test]
    fn replace_repo_mounts_reports_unknown_repo() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("missing").expect("valid repo id");

        let error = replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![MountSpec {
                context: rel("ctx"),
                repo: rel("target"),
            }],
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
    fn replace_repo_mounts_rejects_overlapping_paths() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let error = replace_repo_mounts(
            &workspace,
            &repo_id,
            vec![
                MountSpec {
                    context: rel("ctx-a"),
                    repo: rel("target"),
                },
                MountSpec {
                    context: rel("ctx-b"),
                    repo: rel("target/nested"),
                },
            ],
        )
        .expect_err("overlapping mounts should fail");

        assert!(matches!(
            error,
            Error::Validation(
                crate::shared::error::ValidationError::OverlappingRepoMountTarget { .. }
            )
        ));
    }
}
