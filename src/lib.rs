#![forbid(unsafe_code)]

//! `agentbox` is organized around explicit bounded contexts.
//!
//! Top-level ownership from `MICRO_ARCHITECTURE_ANALYSIS.md`:
//! - workspace administration
//! - repo registry / materialization
//! - mount management
//! - template management
//! - runtime inspection / diagnostics
//! - shared kernel and edge utilities

mod cli;
mod commands;
pub mod inspection;
pub mod mounts;
mod output;
pub mod persistence;
pub mod registry;
pub mod shared;
pub mod templates;
#[cfg(test)]
pub(crate) mod test_support;
pub mod workspace;

use clap::Parser;

pub use shared::error;
pub use shared::error::{Error, Result};
pub(crate) use shared::paths;

/// Parse CLI arguments from the environment and dispatch the requested command.
pub fn try_main() -> Result<()> {
    let cli = cli::Cli::parse();
    commands::dispatch(cli)
}

/// Parse CLI arguments from the given iterator and dispatch the requested command.
///
/// Primarily used by integration tests.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = cli::Cli::parse_from(args);
    commands::dispatch(cli)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    use crate::test_support::{TempDir, fixture_repo};

    static CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn run_from_workspace(workspace: &TempDir, args: &[&str]) -> Result<()> {
        let mut cli_args = vec![
            "agentbox".to_string(),
            "--workspace".to_string(),
            workspace.path().to_string(),
        ];
        cli_args.extend(args.iter().map(|arg| (*arg).to_string()));
        run_from(cli_args)
    }

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
    fn run_from_dispatches_version_and_noop_mount_commands() {
        let workspace = TempDir::new("agentbox-lib-test-workspace");
        let (_fixture, source) = fixture_repo("agentbox-lib-test-source");

        run_from(["agentbox", "version"]).expect("version command");
        run_from_workspace(&workspace, &["init"]).expect("init");
        run_from_workspace(&workspace, &["attach", "demo", "--source", &source]).expect("attach");
        run_from_workspace(&workspace, &["materialize", "demo"]).expect("materialize");
        run_from_workspace(&workspace, &["mount", "demo"])
            .expect("mount without registered mounts");
        run_from_workspace(&workspace, &["unmount", "demo"])
            .expect("unmount without registered mounts");
    }

    #[test]
    fn run_from_covers_already_materialized_and_dematerialized_dispatch_paths() {
        let workspace = TempDir::new("agentbox-lib-test-workspace-repeat");
        let (_fixture, source) = fixture_repo("agentbox-lib-test-source-repeat");

        run_from_workspace(&workspace, &["init"]).expect("init");
        run_from_workspace(&workspace, &["attach", "demo", "--source", &source]).expect("attach");
        run_from_workspace(&workspace, &["materialize", "demo"]).expect("materialize");
        run_from_workspace(&workspace, &["materialize", "demo"]).expect("already materialized");
        run_from_workspace(&workspace, &["dematerialize", "demo"]).expect("dematerialize");
        run_from_workspace(&workspace, &["dematerialize", "demo"]).expect("already dematerialized");
    }

    #[test]
    fn run_from_uses_current_directory_when_workspace_flag_is_omitted() {
        let workspace = TempDir::new("agentbox-lib-test-workspace-cwd");

        with_current_dir(workspace.std_path(), || {
            run_from(["agentbox", "init"]).expect("init");
            run_from(["agentbox", "status"]).expect("status");
        });
    }
}
