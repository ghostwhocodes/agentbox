use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "agentbox",
    about = "Workspace-local context manager for independent git repos"
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Operate on this workspace root instead of the current directory"
    )]
    pub workspace: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args, Clone, Copy, Default)]
pub struct OutputArgs {
    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize the selected directory as an agentbox workspace.
    Init,
    /// Attach a repo by clone source without materializing it yet.
    Attach(AttachArgs),
    /// Detach a repo from the workspace manifest; `--force` unmounts active mounts and dematerializes first.
    Detach(DetachArgs),
    /// Clone one or all registered repos into repos/<repo-id>/.
    Materialize(RepoSelectionArgs),
    /// Remove one or all materialized repos while keeping registration and context.
    Dematerialize(RepoSelectionArgs),
    /// Activate configured bind mounts for one or all repos.
    Mount(RepoSelectionArgs),
    /// Unmount configured bind mounts for one or all repos.
    Unmount(UnmountArgs),
    /// Register a new writable mount rule for a repo.
    AddMount(AddMountArgs),
    /// Remove a registered mount rule by repo path.
    RemoveMount(RemoveMountArgs),
    /// Update the context path for a registered mount rule identified by repo path.
    EditMount(EditMountArgs),
    /// Import a repo directory into workspace-owned context, register it, and mount it back.
    ImportMount(ImportMountArgs),
    /// List registered mounts across the workspace or for one repo.
    ListMounts(ListMountsArgs),
    /// Show high-level state for all registered repos.
    Status(OutputArgs),
    /// Show detailed state for a single repo.
    Show(ShowArgs),
    /// Validate workspace structure and configuration.
    Doctor(DoctorArgs),
    /// Manage workspace-local templates.
    Template(TemplateArgs),
    /// Print the installed CLI version.
    Version,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    pub repo_id: String,
    #[arg(long)]
    pub source: String,
    #[arg(long)]
    pub template: Option<String>,
}

#[derive(Debug, Args)]
pub struct DetachArgs {
    pub repo_id: String,
    #[arg(
        long,
        help = "Unmount active mounts and dematerialize the repo before removing its registration"
    )]
    pub force: bool,
    #[arg(
        long = "unsafe-unmount",
        requires = "force",
        help = "With --force, unmount same-source live mounts even when agentbox cannot verify ownership"
    )]
    pub unsafe_unmount: bool,
}

#[derive(Debug, Args)]
pub struct RepoSelectionArgs {
    pub repo_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct UnmountArgs {
    pub repo_id: Option<String>,
    #[arg(
        long = "unsafe-unmount",
        help = "Unmount same-source live mounts even when agentbox cannot verify ownership"
    )]
    pub unsafe_unmount: bool,
}

#[derive(Debug, Args)]
pub struct AddMountArgs {
    pub repo_id: String,
    #[arg(long)]
    pub context_path: String,
    #[arg(long)]
    pub repo_path: String,
}

#[derive(Debug, Args)]
pub struct RemoveMountArgs {
    pub repo_id: String,
    #[arg(long)]
    pub repo_path: String,
}

#[derive(Debug, Args)]
pub struct EditMountArgs {
    pub repo_id: String,
    #[arg(long)]
    pub repo_path: String,
    #[arg(long)]
    pub new_context_path: String,
}

#[derive(Debug, Args)]
pub struct ImportMountArgs {
    pub repo_id: String,
    #[arg(long)]
    pub repo_path: String,
    #[arg(long)]
    pub context_path: Option<String>,
    #[arg(long)]
    pub no_mount: bool,
}

#[derive(Debug, Args)]
pub struct ListMountsArgs {
    pub repo_id: Option<String>,
    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    pub repo_id: String,
    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[arg(long)]
    pub fix: bool,
    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Debug, Args)]
pub struct TemplateArgs {
    #[command(subcommand)]
    pub command: TemplateCommand,
}

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// List workspace-local templates.
    List,
    /// Create a local template skeleton.
    Create { template_id: String },
    /// Delete a local template.
    Delete { template_id: String },
    /// Apply a local template to a registered repo.
    Apply {
        template_id: String,
        repo_id: String,
    },
}
