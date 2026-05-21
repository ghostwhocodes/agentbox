use crate::{
    error::{Result, TemplateError},
    shared::types::TemplateId,
    templates::{infra, store},
    workspace::Workspace,
};

pub fn delete_template(workspace: &Workspace, template_id: &TemplateId) -> Result<()> {
    delete_template_with_cleanup(workspace, template_id, |staged| staged.remove())
}

pub(crate) fn delete_template_with_cleanup(
    workspace: &Workspace,
    template_id: &TemplateId,
    remove_staged_template: fn(&infra::StagedTemplateDelete) -> Result<()>,
) -> Result<()> {
    if !infra::template_dir_exists(workspace, template_id) {
        return Err(TemplateError::TemplateMissing {
            template_id: template_id.clone(),
        }
        .into());
    }

    let staged_template = infra::stage_template_root_for_delete(workspace, template_id)?;
    let cleared_bindings = match store::clear_applied_template_bindings(workspace, template_id) {
        Ok(cleared_bindings) => cleared_bindings,
        Err(error) => {
            staged_template.rollback()?;
            return Err(error);
        }
    };

    if let Err(error) = remove_staged_template(&staged_template) {
        let restore_bindings = if cleared_bindings.is_empty() {
            Ok(())
        } else {
            store::restore_applied_template_bindings(workspace, template_id, cleared_bindings)
        };
        let rollback_template = staged_template.rollback();
        restore_bindings?;
        rollback_template?;
        return Err(error);
    }

    Ok(())
}
