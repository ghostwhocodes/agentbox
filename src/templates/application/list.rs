use crate::{error::Result, shared::types::TemplateId, templates::infra, workspace::Workspace};

pub fn list_templates(workspace: &Workspace) -> Result<Vec<TemplateId>> {
    let mut templates = Vec::new();

    for name in infra::template_dir_names(workspace)? {
        let template_id = TemplateId::new(name)?;
        if workspace.template_manifest_path(&template_id).exists() {
            templates.push(template_id);
        }
    }

    Ok(templates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::Error, test_support, workspace};
    use std::fs;

    #[test]
    fn list_templates_skips_directories_without_manifests() {
        let workspace = test_support::test_workspace("agentbox-template-list-test");
        workspace::init_workspace(&workspace).expect("init workspace");

        fs::create_dir_all(workspace.template_root(&TemplateId::new("default").expect("id")))
            .expect("create template root");
        fs::create_dir_all(workspace.template_root(&TemplateId::new("draft").expect("id")))
            .expect("create template root");
        fs::write(
            workspace.template_manifest_path(&TemplateId::new("default").expect("id")),
            "version = 1\nmounts = []\n",
        )
        .expect("write manifest");

        let templates = list_templates(&workspace).expect("list templates");

        assert_eq!(
            templates,
            vec![TemplateId::new("default").expect("valid template id")]
        );
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn list_templates_rejects_invalid_template_directory_names() {
        let workspace = test_support::test_workspace("agentbox-template-list-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        fs::create_dir_all(workspace.templates_dir().join("bad template id"))
            .expect("create invalid template dir");

        let error = list_templates(&workspace).expect_err("invalid names should fail");

        assert!(matches!(error, Error::Validation(_)));
        let _ = fs::remove_dir_all(workspace.root());
    }
}
