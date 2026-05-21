use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs};

use crate::{
    error::{Error, Result, ValidationError},
    shared::{
        fs_ops::atomic_write_text,
        mount_spec::{MountSpec, MountSpecOwner, validate_mount_specs},
        types::{CloneSource, RelativePath, RepoId, TemplateId},
    },
};

/// Workspace manifest format version.
pub const MANIFEST_VERSION: u32 = 1;

/// Top-level workspace manifest (`agentbox.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedManifest {
    version: u32,
    #[serde(default)]
    pub repos: BTreeMap<RepoId, PersistedRepoRegistration>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub repo_templates: BTreeMap<RepoId, PersistedTemplateBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repo_mounts: Vec<PersistedRepoMount>,
}

/// Persisted registration record for a repo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedRepoRegistration {
    pub source: CloneSource,
}

/// Persisted template binding for a registered repo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedTemplateBinding {
    pub template: TemplateId,
}

/// Persisted mount row for a registered repo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedRepoMount {
    pub repo_id: RepoId,
    pub context: RelativePath,
    pub repo: RelativePath,
}

impl PersistedManifest {
    pub fn load(path: &Utf8Path) -> Result<Self> {
        let raw = fs::read_to_string(path).map_err(|e| Error::io_path(path, e))?;
        let manifest = toml::from_str::<PersistedManifest>(&raw)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn save(&self, path: &Utf8Path) -> Result<()> {
        self.validate()?;
        let contents = toml::to_string_pretty(self)?;
        atomic_write_text(path, &format!("{contents}\n"))?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != MANIFEST_VERSION {
            return Err(ValidationError::InvalidManifestVersion {
                actual: self.version,
                expected: MANIFEST_VERSION,
            }
            .into());
        }

        for repo_id in self.repo_templates.keys() {
            if !self.repos.contains_key(repo_id) {
                return Err(ValidationError::RepoTemplateUnknownRepo {
                    repo_id: repo_id.clone(),
                }
                .into());
            }
        }

        let mut mounts_by_repo = BTreeMap::<RepoId, Vec<MountSpec>>::new();
        for mount in &self.repo_mounts {
            if !self.repos.contains_key(&mount.repo_id) {
                return Err(ValidationError::RepoMountUnknownRepo {
                    repo_id: mount.repo_id.clone(),
                }
                .into());
            }

            mounts_by_repo
                .entry(mount.repo_id.clone())
                .or_default()
                .push(MountSpec {
                    context: mount.context.clone(),
                    repo: mount.repo.clone(),
                });
        }

        for (repo_id, mounts) in mounts_by_repo {
            validate_mount_specs(&MountSpecOwner::Repo(&repo_id), &mounts)?;
        }

        Ok(())
    }
}

impl Default for PersistedManifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            repos: BTreeMap::new(),
            repo_templates: BTreeMap::new(),
            repo_mounts: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_manifest_path() -> camino::Utf8PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agentbox-manifest-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        fs::create_dir_all(&root).expect("create tempdir");
        camino::Utf8PathBuf::from_path_buf(root)
            .expect("utf8 tempdir")
            .join("agentbox.toml")
    }

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid path")
    }

    #[test]
    fn manifest_save_and_load_roundtrip() {
        let path = temp_manifest_path();
        let repo_id = RepoId::new("demo").unwrap();
        let manifest = PersistedManifest {
            repos: BTreeMap::from([(
                repo_id.clone(),
                PersistedRepoRegistration {
                    source: CloneSource::new("file:///tmp/source").unwrap(),
                },
            )]),
            repo_templates: BTreeMap::from([(
                repo_id.clone(),
                PersistedTemplateBinding {
                    template: TemplateId::new("base").unwrap(),
                },
            )]),
            repo_mounts: vec![PersistedRepoMount {
                repo_id: repo_id.clone(),
                context: rel("ctx"),
                repo: rel("repo"),
            }],
            ..PersistedManifest::default()
        };

        manifest.save(&path).unwrap();
        let loaded = PersistedManifest::load(&path).unwrap();
        assert_eq!(loaded.repos.len(), 1);
        assert_eq!(loaded.repo_templates.len(), 1);
        assert_eq!(loaded.repo_mounts.len(), 1);
        assert_eq!(loaded.repo_mounts[0].repo_id, repo_id);

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn manifest_rejects_mount_for_unknown_repo() {
        let manifest = PersistedManifest {
            repo_mounts: vec![PersistedRepoMount {
                repo_id: RepoId::new("demo").unwrap(),
                context: rel("ctx"),
                repo: rel("repo"),
            }],
            ..PersistedManifest::default()
        };

        assert!(matches!(
            manifest.validate(),
            Err(crate::shared::error::Error::Validation(
                ValidationError::RepoMountUnknownRepo { .. }
            ))
        ));
    }

    #[test]
    fn manifest_rejects_template_binding_for_unknown_repo() {
        let manifest = PersistedManifest {
            repo_templates: BTreeMap::from([(
                RepoId::new("demo").unwrap(),
                PersistedTemplateBinding {
                    template: TemplateId::new("base").unwrap(),
                },
            )]),
            ..PersistedManifest::default()
        };

        assert!(matches!(
            manifest.validate(),
            Err(crate::shared::error::Error::Validation(
                ValidationError::RepoTemplateUnknownRepo { .. }
            ))
        ));
    }
}
