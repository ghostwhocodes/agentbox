use agentbox::error::Error;
use agentbox::persistence::manifest::{
    PersistedManifest, PersistedRepoMount, PersistedRepoRegistration, PersistedTemplateBinding,
};
use agentbox::shared::types::{CloneSource, RelativePath, RepoId, TemplateId};

fn registered_manifest(repo_name: &str) -> (PersistedManifest, RepoId) {
    let repo_id = RepoId::new(repo_name).expect("valid repo id");
    let mut manifest = PersistedManifest::default();
    manifest.repos.insert(
        repo_id.clone(),
        PersistedRepoRegistration {
            source: CloneSource::new("file:///tmp/repo").expect("valid clone source"),
        },
    );
    (manifest, repo_id)
}

#[test]
fn rejects_duplicate_mount_contexts() {
    let (mut manifest, repo_id) = registered_manifest("repo");
    manifest.repo_mounts.extend([
        PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: RelativePath::new("ai", "mount context path")
                .expect("valid mount context path"),
            repo: RelativePath::new("ai", "mount repo path").expect("valid mount repo path"),
        },
        PersistedRepoMount {
            repo_id,
            context: RelativePath::new("ai", "mount context path")
                .expect("valid mount context path"),
            repo: RelativePath::new(".loki", "mount repo path").expect("valid mount repo path"),
        },
    ]);

    assert!(manifest.validate().is_err());
}

#[test]
fn rejects_overlapping_mount_contexts() {
    let (mut manifest, repo_id) = registered_manifest("repo");
    manifest.repo_mounts.extend([
        PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: RelativePath::new("ai", "mount context path")
                .expect("valid mount context path"),
            repo: RelativePath::new("target-a", "mount repo path").expect("valid mount repo path"),
        },
        PersistedRepoMount {
            repo_id,
            context: RelativePath::new("ai/prompts", "mount context path")
                .expect("valid mount context path"),
            repo: RelativePath::new("target-b", "mount repo path").expect("valid mount repo path"),
        },
    ]);

    let error = manifest
        .validate()
        .expect_err("overlapping context paths should fail");
    assert!(matches!(error, Error::Validation(_)));
    assert!(
        error
            .to_string()
            .contains("overlapping mount context paths")
    );
}

#[test]
fn rejects_overlapping_mount_targets() {
    let (mut manifest, repo_id) = registered_manifest("repo");
    manifest.repo_mounts.extend([
        PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: RelativePath::new("ctx-a", "mount context path")
                .expect("valid mount context path"),
            repo: RelativePath::new("ai", "mount repo path").expect("valid mount repo path"),
        },
        PersistedRepoMount {
            repo_id,
            context: RelativePath::new("ctx-b", "mount context path")
                .expect("valid mount context path"),
            repo: RelativePath::new("ai/prompts", "mount repo path")
                .expect("valid mount repo path"),
        },
    ]);

    let error = manifest
        .validate()
        .expect_err("overlapping repo paths should fail");
    assert!(matches!(error, Error::Validation(_)));
    assert!(error.to_string().contains("overlapping mount repo paths"));
}

#[test]
fn rejects_invalid_manifest_version() {
    let raw = "version = 99
";
    let manifest: PersistedManifest = toml::from_str(raw).expect("parse TOML");
    assert!(manifest.validate().is_err());
}

#[test]
fn rejects_zero_manifest_version() {
    let raw = "version = 0
";
    let manifest: PersistedManifest = toml::from_str(raw).expect("parse TOML");
    assert!(manifest.validate().is_err());
}

#[test]
fn rejects_template_binding_for_unknown_repo() {
    let mut manifest = PersistedManifest::default();
    manifest.repo_templates.insert(
        RepoId::new("repo").expect("valid repo id"),
        PersistedTemplateBinding {
            template: TemplateId::new("default").expect("valid template id"),
        },
    );

    assert!(manifest.validate().is_err());
}
