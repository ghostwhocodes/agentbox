use serde::{Deserialize, Serialize};

use crate::{
    error::{Result, ValidationError},
    shared::types::{RelativePath, RepoId},
};

/// A bind-mount specification mapping a context path to a repo path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountSpec {
    /// Relative path within the repo's context directory (mount source).
    pub context: RelativePath,
    /// Relative path within the cloned repository (mount target).
    pub repo: RelativePath,
}

/// Identifies who owns a set of mount specs, for error message context.
pub(crate) enum MountSpecOwner<'a> {
    Repo(&'a RepoId),
    Template,
}

pub(crate) fn relative_paths_overlap(first: &RelativePath, second: &RelativePath) -> bool {
    first == second
        || first.as_path().starts_with(second.as_path())
        || second.as_path().starts_with(first.as_path())
}

pub(crate) fn mounts_conflict(first: &MountSpec, second: &MountSpec) -> bool {
    relative_paths_overlap(&first.context, &second.context)
        || relative_paths_overlap(&first.repo, &second.repo)
}

pub(crate) fn validate_mount_specs(owner: &MountSpecOwner<'_>, mounts: &[MountSpec]) -> Result<()> {
    let mut seen = Vec::<MountSpec>::new();

    for mount in mounts {
        for existing in &seen {
            if existing.context == mount.context {
                return Err(match *owner {
                    MountSpecOwner::Repo(repo_id) => ValidationError::DuplicateRepoMountContext {
                        repo_id: repo_id.clone(),
                        context: mount.context.clone(),
                    }
                    .into(),
                    MountSpecOwner::Template => ValidationError::DuplicateTemplateMountContext {
                        context: mount.context.clone(),
                    }
                    .into(),
                });
            }

            if relative_paths_overlap(&existing.context, &mount.context) {
                return Err(match *owner {
                    MountSpecOwner::Repo(repo_id) => ValidationError::OverlappingRepoMountContext {
                        repo_id: repo_id.clone(),
                        first: existing.context.clone(),
                        second: mount.context.clone(),
                    }
                    .into(),
                    MountSpecOwner::Template => ValidationError::OverlappingTemplateMountContext {
                        first: existing.context.clone(),
                        second: mount.context.clone(),
                    }
                    .into(),
                });
            }

            if existing.repo == mount.repo {
                return Err(match *owner {
                    MountSpecOwner::Repo(repo_id) => ValidationError::DuplicateRepoMountTarget {
                        repo_id: repo_id.clone(),
                        repo_path: mount.repo.clone(),
                    }
                    .into(),
                    MountSpecOwner::Template => ValidationError::DuplicateTemplateMountTarget {
                        repo_path: mount.repo.clone(),
                    }
                    .into(),
                });
            }

            if relative_paths_overlap(&existing.repo, &mount.repo) {
                return Err(match *owner {
                    MountSpecOwner::Repo(repo_id) => ValidationError::OverlappingRepoMountTarget {
                        repo_id: repo_id.clone(),
                        first: existing.repo.clone(),
                        second: mount.repo.clone(),
                    }
                    .into(),
                    MountSpecOwner::Template => ValidationError::OverlappingTemplateMountTarget {
                        first: existing.repo.clone(),
                        second: mount.repo.clone(),
                    }
                    .into(),
                });
            }
        }

        seen.push(mount.clone());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Error, ValidationError};

    fn rel(s: &str) -> RelativePath {
        RelativePath::new(s, "test").unwrap()
    }

    fn repo_id() -> RepoId {
        RepoId::new("test-repo").unwrap()
    }

    #[test]
    fn valid_mount_specs_pass() {
        let id = repo_id();
        let mounts = vec![
            MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            },
            MountSpec {
                context: rel(".loki"),
                repo: rel(".loki"),
            },
        ];
        assert!(validate_mount_specs(&MountSpecOwner::Repo(&id), &mounts).is_ok());
    }

    #[test]
    fn empty_mount_specs_pass() {
        let id = repo_id();
        assert!(validate_mount_specs(&MountSpecOwner::Repo(&id), &[]).is_ok());
    }

    #[test]
    fn duplicate_context_path_rejected() {
        let id = repo_id();
        let mounts = vec![
            MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            },
            MountSpec {
                context: rel("ai"),
                repo: rel("other"),
            },
        ];
        assert!(validate_mount_specs(&MountSpecOwner::Repo(&id), &mounts).is_err());
    }

    #[test]
    fn overlapping_context_path_rejected() {
        let id = repo_id();
        let mounts = vec![
            MountSpec {
                context: rel("ai"),
                repo: rel("repo-a"),
            },
            MountSpec {
                context: rel("ai/prompts"),
                repo: rel("repo-b"),
            },
        ];

        let error = validate_mount_specs(&MountSpecOwner::Repo(&id), &mounts).expect_err("overlap");
        assert!(matches!(
            error,
            Error::Validation(ValidationError::OverlappingRepoMountContext { .. })
        ));
    }

    #[test]
    fn duplicate_repo_path_rejected() {
        let id = repo_id();
        let mounts = vec![
            MountSpec {
                context: rel("ai"),
                repo: rel("target"),
            },
            MountSpec {
                context: rel("other"),
                repo: rel("target"),
            },
        ];
        assert!(validate_mount_specs(&MountSpecOwner::Repo(&id), &mounts).is_err());
    }

    #[test]
    fn overlapping_repo_path_rejected() {
        let id = repo_id();
        let mounts = vec![
            MountSpec {
                context: rel("ctx-a"),
                repo: rel("target"),
            },
            MountSpec {
                context: rel("ctx-b"),
                repo: rel("target/nested"),
            },
        ];

        let error = validate_mount_specs(&MountSpecOwner::Repo(&id), &mounts).expect_err("overlap");
        assert!(matches!(
            error,
            Error::Validation(ValidationError::OverlappingRepoMountTarget { .. })
        ));
    }

    #[test]
    fn template_owner_duplicate_context_rejected() {
        let mounts = vec![
            MountSpec {
                context: rel("ai"),
                repo: rel("ai"),
            },
            MountSpec {
                context: rel("ai"),
                repo: rel("other"),
            },
        ];
        assert!(validate_mount_specs(&MountSpecOwner::Template, &mounts).is_err());
    }

    #[test]
    fn template_owner_overlapping_repo_path_rejected() {
        let mounts = vec![
            MountSpec {
                context: rel("ctx-a"),
                repo: rel("target"),
            },
            MountSpec {
                context: rel("ctx-b"),
                repo: rel("target/nested"),
            },
        ];

        let error = validate_mount_specs(&MountSpecOwner::Template, &mounts).expect_err("overlap");
        assert!(matches!(
            error,
            Error::Validation(ValidationError::OverlappingTemplateMountTarget { .. })
        ));
    }
}
