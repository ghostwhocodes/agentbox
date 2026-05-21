//! Pure template domain model and policy.
//!
//! Ownership:
//! - template manifest schema and version validation
//! - pure template-to-repo conflict-validation rules
//! - pure merge rules for template-provided mount specs
//!
//! Dependency guardrail:
//! this layer must remain free of `Workspace`, filesystem I/O, Git, or mount
//! table inspection concerns.

mod applied_template;
mod template_manifest;

pub(crate) use applied_template::AppliedTemplate;
pub use template_manifest::{TEMPLATE_MANIFEST_VERSION, TemplateManifest};

pub(crate) use template_manifest::{merged_template_mounts, validate_template_application};
