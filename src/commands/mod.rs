//! CLI-facing adapters from parsed arguments to application workflows.
//!
//! This layer owns:
//! - translating Clap input into typed workflow calls
//! - resolving the workspace root once per command
//! - rendering command output
//!
//! Guardrail:
//! keep these modules thin. They may depend on bounded-context workflow APIs,
//! `workspace`, and output helpers, but they should not absorb reusable
//! business logic that belongs in context-owned application services or
//! shared models.
use crate::{
    cli::{Cli, Command},
    error::Result,
    paths::normalize_to_absolute_std_path,
    workspace::Workspace,
};

mod attach;
mod doctor;
mod mount;
mod repo;
mod template;

pub fn dispatch(cli: Cli) -> Result<()> {
    let Cli { workspace, command } = cli;
    if matches!(command, Command::Version) {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let workspace = resolve_workspace(workspace)?;
    match command {
        Command::Init => repo::init(&workspace),
        Command::Attach(args) => attach::attach(&workspace, args),
        Command::Detach(args) => attach::detach(&workspace, args),
        Command::Materialize(args) => repo::materialize(&workspace, args),
        Command::Dematerialize(args) => repo::dematerialize(&workspace, args),
        Command::Mount(args) => mount::mount_repos(&workspace, args),
        Command::Unmount(args) => mount::unmount_repos(&workspace, args),
        Command::AddMount(args) => mount::add_mount(&workspace, args),
        Command::RemoveMount(args) => mount::remove_mount(&workspace, args),
        Command::EditMount(args) => mount::edit_mount(&workspace, args),
        Command::ImportMount(args) => mount::import_mount(&workspace, args),
        Command::ListMounts(args) => mount::list_mounts(&workspace, args),
        Command::Status(args) => repo::status_command(&workspace, args),
        Command::Show(args) => repo::show(&workspace, args),
        Command::Doctor(args) => doctor::doctor_command(&workspace, args),
        Command::Template(args) => template::template_command(&workspace, args),
        Command::Version => {
            unreachable!("version is handled before workspace resolution")
        }
    }
}

fn resolve_workspace(path: Option<std::path::PathBuf>) -> Result<Workspace> {
    match path {
        Some(path) => Workspace::from_std_path(normalize_to_absolute_std_path(path)?),
        None => {
            let current = normalize_to_absolute_std_path(std::env::current_dir()?)?;
            let current = Workspace::from_std_path(current)?;
            Ok(Workspace::discover_from(current.root().to_path_buf()).unwrap_or(current))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_workspace;
    use camino::Utf8PathBuf;
    use std::{
        fs,
        path::PathBuf,
        sync::{Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    static CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_current_dir<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _lock = CWD_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock cwd");
        let previous = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir).expect("set current dir");
        let result = f();
        std::env::set_current_dir(previous).expect("restore current dir");
        result
    }

    #[test]
    fn resolve_workspace_normalizes_relative_paths_to_absolute_roots() {
        let cwd = std::env::temp_dir().join(format!(
            "agentbox-commands-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&cwd).expect("create cwd");

        with_current_dir(&cwd, || {
            let workspace = resolve_workspace(Some(PathBuf::from("./nested/../workspace")))
                .expect("resolve workspace");
            let expected = Utf8PathBuf::from_path_buf(cwd.join("workspace")).expect("utf8 cwd");
            assert_eq!(workspace.root(), expected);
        });

        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn resolve_workspace_discovers_parent_workspace_when_flag_is_omitted() {
        let cwd = std::env::temp_dir().join(format!(
            "agentbox-commands-discovery-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let workspace_root = cwd.join("workspace");
        let nested = workspace_root.join("repos/demo");
        fs::create_dir_all(&nested).expect("create nested dir");
        fs::write(
            workspace_root.join("agentbox.toml"),
            "version = 1\n\n[repos]\n",
        )
        .expect("write manifest");

        with_current_dir(&nested, || {
            let workspace = resolve_workspace(None).expect("resolve workspace");
            let expected = Utf8PathBuf::from_path_buf(workspace_root.clone()).expect("utf8 path");
            assert_eq!(workspace.root(), expected);
        });

        let _ = fs::remove_dir_all(&cwd);
    }
}
