//! Template infrastructure adapters.
//!
//! Ownership:
//! - filesystem-backed template manifest persistence
//! - template directory creation, deletion, and enumeration
//! - copying template context data into repo context roots
//!
//! Dependency guardrail:
//! keep low-level filesystem interactions here and leave workflow decisions to
//! `templates::application`.

mod store;

pub(crate) use store::{
    StagedTemplateDelete, copy_template_context_into_repo, create_template_layout,
    load_template_manifest, save_template_manifest, stage_template_root_for_delete,
    template_dir_exists, template_dir_names,
};
