use serde::{Deserialize, Serialize};

use crate::{
    error::{Result, TemplateError, ValidationError},
    shared::{
        mount_spec::{MountSpec, MountSpecOwner, mounts_conflict, validate_mount_specs},
        types::{RepoId, TemplateId},
    },
};

pub const TEMPLATE_MANIFEST_VERSION: u32 = 1;

/// Manifest for a reusable mount template (`template.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateManifest {
    pub(crate) version: u32,
    #[serde(default)]
    pub mounts: Vec<MountSpec>,
}

impl Default for TemplateManifest {
    fn default() -> Self {
        Self {
            version: TEMPLATE_MANIFEST_VERSION,
            mounts: Vec::new(),
        }
    }
}

impl TemplateManifest {
    pub fn from_toml_str(raw: &str) -> Result<Self> {
        let manifest = toml::from_str::<TemplateManifest>(raw)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_toml_string(&self) -> Result<String> {
        self.validate()?;
        Ok(format!("{}\n", toml::to_string_pretty(self)?))
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != TEMPLATE_MANIFEST_VERSION {
            return Err(ValidationError::InvalidTemplateManifestVersion {
                actual: self.version,
                expected: TEMPLATE_MANIFEST_VERSION,
            }
            .into());
        }

        validate_mount_specs(&MountSpecOwner::Template, &self.mounts)
    }
}

pub(crate) fn validate_template_application(
    repo_id: &RepoId,
    existing_mounts: &[MountSpec],
    template_id: &TemplateId,
    manifest: &TemplateManifest,
) -> Result<()> {
    for mount in &manifest.mounts {
        if existing_mounts.iter().any(|existing| existing == mount) {
            continue;
        }
        if existing_mounts
            .iter()
            .any(|existing| existing != mount && mounts_conflict(existing, mount))
        {
            return Err(TemplateError::TemplateMountConflict {
                template_id: template_id.clone(),
                repo_id: repo_id.clone(),
                context: mount.context.clone(),
                repo_path: mount.repo.clone(),
            }
            .into());
        }
    }

    Ok(())
}

pub(crate) fn merged_template_mounts(
    existing_mounts: &[MountSpec],
    mounts: impl IntoIterator<Item = MountSpec>,
) -> Vec<MountSpec> {
    let mut merged = existing_mounts.to_vec();

    for mount in mounts {
        if merged.iter().any(|existing| existing == &mount) {
            continue;
        }
        merged.push(mount);
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::{Error, ValidationError},
        shared::{
            mount_spec::MountSpec,
            types::{RelativePath, RepoId, TemplateId},
        },
    };

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    #[test]
    fn template_manifest_rejects_invalid_version() {
        let error = TemplateManifest::from_toml_str("version = 99\n")
            .expect_err("invalid version should fail");
        assert!(matches!(
            error,
            Error::Validation(ValidationError::InvalidTemplateManifestVersion { .. })
        ));
    }

    #[test]
    fn template_manifest_round_trips_through_toml() {
        let manifest = TemplateManifest {
            version: TEMPLATE_MANIFEST_VERSION,
            mounts: vec![MountSpec {
                context: rel("ctx"),
                repo: rel("repo"),
            }],
        };

        let encoded = manifest.to_toml_string().expect("encode manifest");
        let decoded = TemplateManifest::from_toml_str(&encoded).expect("decode manifest");
        assert_eq!(decoded.mounts, manifest.mounts);
    }

    #[test]
    fn template_manifest_rejects_unknown_top_level_fields() {
        let error = TemplateManifest::from_toml_str("version = 1\nextra = true\n")
            .expect_err("unknown top-level field should fail");
        assert!(matches!(error, Error::TomlDeserialize(_)));
    }

    #[test]
    fn template_manifest_rejects_unknown_mount_fields() {
        let error = TemplateManifest::from_toml_str(
            "version = 1\n\n[[mounts]]\ncontext = \"ai\"\nrepo = \"ai\"\nextra = true\n",
        )
        .expect_err("unknown mount field should fail");
        assert!(matches!(error, Error::TomlDeserialize(_)));
    }

    #[test]
    fn merged_template_mounts_skips_duplicate_mounts() {
        let merged = merged_template_mounts(
            &[MountSpec {
                context: rel("existing"),
                repo: rel("src"),
            }],
            vec![
                MountSpec {
                    context: rel("existing"),
                    repo: rel("src"),
                },
                MountSpec {
                    context: rel("new"),
                    repo: rel("generated"),
                },
            ],
        );

        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn validate_template_application_rejects_conflicts() {
        let template_id = TemplateId::new("default").expect("valid template id");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let existing_mounts = vec![MountSpec {
            context: rel("existing"),
            repo: rel("src"),
        }];
        let manifest = TemplateManifest {
            version: TEMPLATE_MANIFEST_VERSION,
            mounts: vec![MountSpec {
                context: rel("generated"),
                repo: rel("src"),
            }],
        };

        let error =
            validate_template_application(&repo_id, &existing_mounts, &template_id, &manifest)
                .expect_err("conflicting template should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::Template(TemplateError::TemplateMountConflict {
                template_id: ref conflict_template_id,
                repo_id: ref conflict_repo_id,
                ..
            }) if conflict_template_id == &template_id && conflict_repo_id == &repo_id
        ));
    }

    #[test]
    fn validate_template_application_rejects_overlapping_mounts() {
        let template_id = TemplateId::new("default").expect("valid template id");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let existing_mounts = vec![MountSpec {
            context: rel("existing"),
            repo: rel("src"),
        }];
        let manifest = TemplateManifest {
            version: TEMPLATE_MANIFEST_VERSION,
            mounts: vec![MountSpec {
                context: rel("generated"),
                repo: rel("src/generated"),
            }],
        };

        let error =
            validate_template_application(&repo_id, &existing_mounts, &template_id, &manifest)
                .expect_err("overlapping template should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::Template(TemplateError::TemplateMountConflict {
                template_id: ref conflict_template_id,
                repo_id: ref conflict_repo_id,
                ..
            }) if conflict_template_id == &template_id && conflict_repo_id == &repo_id
        ));
    }

    #[test]
    fn validate_template_application_allows_duplicate_mounts_from_template() {
        let template_id = TemplateId::new("default").expect("valid template id");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let existing_mounts = vec![MountSpec {
            context: rel("existing"),
            repo: rel("src"),
        }];
        let manifest = TemplateManifest {
            version: TEMPLATE_MANIFEST_VERSION,
            mounts: vec![MountSpec {
                context: rel("existing"),
                repo: rel("src"),
            }],
        };

        validate_template_application(&repo_id, &existing_mounts, &template_id, &manifest)
            .expect("duplicate mount should be accepted");
    }
}
