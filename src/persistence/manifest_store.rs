use crate::{
    persistence::manifest::PersistedManifest, shared::error::Result, workspace::Workspace,
};

/// Load and validate the current workspace manifest.
pub fn read(workspace: &Workspace) -> Result<PersistedManifest> {
    workspace.require_initialized()?;
    PersistedManifest::load(&workspace.manifest_path())
}

/// Validate and persist the current workspace manifest.
pub fn write(workspace: &Workspace, manifest: &PersistedManifest) -> Result<()> {
    manifest.save(&workspace.manifest_path())
}
