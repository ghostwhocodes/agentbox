use crate::shared::types::TemplateId;

/// Template binding owned by the template-management context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedTemplate {
    pub template_id: TemplateId,
}
