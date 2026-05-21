use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    error::{Error, Result},
    paths::describe_non_utf8_os_str,
    shared::fs_ops::{TreeCopyRollback, atomic_write_text, copy_missing_tree_tracked},
    shared::types::{RepoId, TemplateId},
    templates::domain::TemplateManifest,
    workspace::Workspace,
};

#[derive(Debug)]
pub(crate) struct StagedTemplateDelete {
    original_path: camino::Utf8PathBuf,
    staged_path: camino::Utf8PathBuf,
}

impl StagedTemplateDelete {
    pub(crate) fn remove(&self) -> Result<()> {
        fs::remove_dir_all(&self.staged_path).map_err(|e| Error::io_path(&self.staged_path, e))?;
        Ok(())
    }

    pub(crate) fn rollback(&self) -> Result<()> {
        if !self.staged_path.exists() {
            return Ok(());
        }

        fs::rename(&self.staged_path, &self.original_path)
            .map_err(|e| Error::io_path(&self.staged_path, e))?;
        Ok(())
    }
}

fn template_delete_staging_dir(workspace: &Workspace) -> camino::Utf8PathBuf {
    workspace.root().join(".agentbox/tmp/template-deletes")
}

pub(crate) fn template_dir_exists(workspace: &Workspace, template_id: &TemplateId) -> bool {
    workspace.template_root(template_id).exists()
}

pub(crate) fn create_template_layout(
    workspace: &Workspace,
    template_id: &TemplateId,
) -> Result<()> {
    let path = workspace.template_context_root(template_id);
    fs::create_dir_all(&path).map_err(|e| Error::io_path(&path, e))?;
    Ok(())
}

pub(crate) fn stage_template_root_for_delete(
    workspace: &Workspace,
    template_id: &TemplateId,
) -> Result<StagedTemplateDelete> {
    stage_template_root_for_delete_with_nonce_provider(workspace, template_id, || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos()
    })
}

fn stage_template_root_for_delete_with_nonce_provider<F>(
    workspace: &Workspace,
    template_id: &TemplateId,
    mut next_nonce: F,
) -> Result<StagedTemplateDelete>
where
    F: FnMut() -> u128,
{
    let original_path = workspace.template_root(template_id);
    let staging_root = template_delete_staging_dir(workspace);
    fs::create_dir_all(&staging_root).map_err(|e| Error::io_path(&staging_root, e))?;

    for attempt in 0..16 {
        let nonce = next_nonce();
        let staged_path = staging_root.join(format!(
            ".delete-{}-{}-{}-{}",
            template_id,
            std::process::id(),
            nonce,
            attempt
        ));

        if staged_path.exists() {
            continue;
        }

        fs::rename(&original_path, &staged_path).map_err(|e| Error::io_path(&original_path, e))?;
        return Ok(StagedTemplateDelete {
            original_path,
            staged_path,
        });
    }

    let collision_path =
        staging_root.join(format!(".delete-{}-{}", template_id, std::process::id()));
    Err(Error::io_path(
        &collision_path,
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique staged template delete path",
        ),
    ))
}

pub(crate) fn template_dir_names(workspace: &Workspace) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let templates_dir = workspace.templates_dir();
    if !templates_dir.is_dir() {
        return Ok(names);
    }

    for entry in fs::read_dir(&templates_dir).map_err(|e| Error::io_path(&templates_dir, e))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let name = entry
            .file_name()
            .into_string()
            .map_err(|os| Error::unsupported_non_utf8_path(describe_non_utf8_os_str(&os)))?;
        if name.starts_with(".delete-") {
            continue;
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

pub(crate) fn load_template_manifest(
    workspace: &Workspace,
    template_id: &TemplateId,
) -> Result<TemplateManifest> {
    let path = workspace.template_manifest_path(template_id);
    let raw = fs::read_to_string(&path).map_err(|e| Error::io_path(&path, e))?;
    TemplateManifest::from_toml_str(&raw)
}

pub(crate) fn save_template_manifest(
    workspace: &Workspace,
    template_id: &TemplateId,
    manifest: &TemplateManifest,
) -> Result<()> {
    let path = workspace.template_manifest_path(template_id);
    let contents = manifest.to_toml_string()?;
    atomic_write_text(&path, &contents)?;
    Ok(())
}

pub(crate) fn copy_template_context_into_repo(
    workspace: &Workspace,
    template_id: &TemplateId,
    repo_id: &RepoId,
) -> Result<TreeCopyRollback> {
    let template_context = workspace.template_context_root(template_id);
    let repo_context = workspace.repo_context_root(repo_id);
    copy_missing_tree_tracked(&template_context, &repo_context)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{shared::types::TemplateId, test_support, workspace};

    use super::{
        StagedTemplateDelete, TemplateManifest, copy_template_context_into_repo,
        load_template_manifest, save_template_manifest, stage_template_root_for_delete,
        stage_template_root_for_delete_with_nonce_provider, template_delete_staging_dir,
        template_dir_names,
    };

    #[test]
    fn stage_template_root_for_delete_moves_template_outside_templates_dir() {
        let workspace = test_support::test_workspace("agentbox-template-store-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");
        let template_root = workspace.template_root(&template_id);
        fs::create_dir_all(&template_root).expect("create template root");

        let staged = stage_template_root_for_delete(&workspace, &template_id)
            .expect("stage template root for delete");

        assert!(!template_root.exists());
        assert!(
            staged
                .staged_path
                .starts_with(template_delete_staging_dir(&workspace))
        );
        assert!(!staged.staged_path.starts_with(workspace.templates_dir()));
        assert!(staged.staged_path.exists());

        staged.rollback().expect("rollback staged template");
        assert!(template_root.exists());

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn staged_template_rollback_is_a_noop_when_staged_path_is_missing() {
        let workspace = test_support::test_workspace("agentbox-template-store-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        let staged = StagedTemplateDelete {
            original_path: workspace
                .template_root(&TemplateId::new("default").expect("valid template id")),
            staged_path: workspace.root().join("missing-staged-template"),
        };

        staged
            .rollback()
            .expect("missing staged path should be ignored");

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn stage_template_root_for_delete_skips_existing_candidate_paths() {
        let workspace = test_support::test_workspace("agentbox-template-store-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");
        let template_root = workspace.template_root(&template_id);
        fs::create_dir_all(&template_root).expect("create template root");

        let staging_root = template_delete_staging_dir(&workspace);
        fs::create_dir_all(&staging_root).expect("create staging root");
        let first_candidate = staging_root.join(format!(
            ".delete-{}-{}-{}-0",
            template_id,
            std::process::id(),
            7u128
        ));
        fs::create_dir_all(&first_candidate).expect("create collision path");

        let mut nonces = [7u128, 9u128].into_iter();
        let staged =
            stage_template_root_for_delete_with_nonce_provider(&workspace, &template_id, || {
                nonces.next().expect("nonce for attempt")
            })
            .expect("stage template root");

        assert_ne!(staged.staged_path, first_candidate);
        assert!(staged.staged_path.exists());
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn stage_template_root_for_delete_reports_collision_after_exhausting_candidates() {
        let workspace = test_support::test_workspace("agentbox-template-store-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");
        let template_root = workspace.template_root(&template_id);
        fs::create_dir_all(&template_root).expect("create template root");

        let staging_root = template_delete_staging_dir(&workspace);
        fs::create_dir_all(&staging_root).expect("create staging root");
        for attempt in 0..16 {
            let path = staging_root.join(format!(
                ".delete-{}-{}-{}-{}",
                template_id,
                std::process::id(),
                11u128,
                attempt
            ));
            fs::create_dir_all(path).expect("create collision path");
        }

        let error =
            stage_template_root_for_delete_with_nonce_provider(&workspace, &template_id, || 11u128)
                .expect_err("exhausted candidates should fail");

        assert!(matches!(error, crate::shared::error::Error::IoPath { .. }));
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn template_dir_names_returns_empty_when_templates_dir_is_missing() {
        let workspace = test_support::test_workspace("agentbox-template-store-test");

        let names = template_dir_names(&workspace).expect("missing templates dir should be empty");

        assert!(names.is_empty());
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn save_and_load_template_manifest_round_trip() {
        let workspace = test_support::test_workspace("agentbox-template-store-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");
        let manifest = TemplateManifest::default();

        save_template_manifest(&workspace, &template_id, &manifest).expect("save manifest");
        let loaded = load_template_manifest(&workspace, &template_id).expect("load manifest");

        assert_eq!(
            loaded.to_toml_string().expect("serialize loaded manifest"),
            manifest.to_toml_string().expect("serialize manifest")
        );
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn copy_template_context_into_repo_copies_missing_context_files() {
        let workspace = test_support::test_workspace("agentbox-template-store-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");
        let repo_id = crate::shared::types::RepoId::new("demo").expect("valid repo id");
        fs::create_dir_all(workspace.template_context_root(&template_id))
            .expect("create template context");
        fs::write(
            workspace
                .template_context_root(&template_id)
                .join("prompt.md"),
            "seed",
        )
        .expect("write template file");

        let rollback =
            copy_template_context_into_repo(&workspace, &template_id, &repo_id).expect("copy");

        assert_eq!(
            fs::read_to_string(workspace.repo_context_root(&repo_id).join("prompt.md"))
                .expect("read copied file"),
            "seed"
        );
        rollback.rollback().expect("rollback copied tree");
        let _ = fs::remove_dir_all(workspace.root());
    }
}
