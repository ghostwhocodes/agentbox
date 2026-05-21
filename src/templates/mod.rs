//! Template management context.
//!
//! Ownership:
//! - template manifest domain model and pure template policy
//! - template storage and filesystem-backed manifest persistence
//! - template create/delete/list/apply workflows
//!
//! Dependency guardrail:
//! this context may depend on shared domain persistence contracts,
//! filesystem adapters, and `Workspace`, but side-effecting template
//! orchestration should live here rather than in `domain`.

mod application;
pub mod domain;
pub(crate) mod infra;
pub(crate) mod store;

pub(crate) use application::prepare_template_application;
pub use application::{
    apply_template_to_registered_repo, create_template, delete_template, list_templates,
};
pub(crate) use domain::AppliedTemplate;
pub(crate) use store::{clear_applied_template_tx, replace_applied_template_tx};
