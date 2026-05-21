use crate::{
    error::{Result, TemplateError},
    shared::types::TemplateId,
    templates::{domain::TemplateManifest, infra},
    workspace::Workspace,
};

pub fn create_template(workspace: &Workspace, template_id: &TemplateId) -> Result<()> {
    if infra::template_dir_exists(workspace, template_id) {
        return Err(TemplateError::TemplateAlreadyExists {
            template_id: template_id.clone(),
        }
        .into());
    }

    infra::create_template_layout(workspace, template_id)?;
    infra::save_template_manifest(workspace, template_id, &TemplateManifest::default())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::Error, test_support, workspace};
    use std::fs;

    #[test]
    fn create_template_rejects_existing_template() {
        let workspace = test_support::test_workspace("agentbox-template-create-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        let template_id = TemplateId::new("default").expect("valid template id");

        create_template(&workspace, &template_id).expect("create template");
        let error =
            create_template(&workspace, &template_id).expect_err("duplicate create should fail");

        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateAlreadyExists { .. })
        ));
        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn create_template_propagates_layout_failures() {
        let workspace = test_support::test_workspace("agentbox-template-create-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        fs::remove_dir_all(workspace.templates_dir()).expect("remove templates dir");
        fs::write(workspace.templates_dir(), "not a directory").expect("write templates file");

        let error = create_template(
            &workspace,
            &TemplateId::new("default").expect("valid template id"),
        )
        .expect_err("layout creation should fail");

        assert!(matches!(error, Error::IoPath { .. }));
        let _ = fs::remove_file(workspace.templates_dir());
        let _ = fs::remove_dir_all(workspace.root());
    }
}
