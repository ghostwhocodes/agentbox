use crate::{
    cli::{OutputArgs, RepoSelectionArgs, ShowArgs},
    error::Result,
    inspection,
    output::{self, OutputFormat},
    registry,
    shared::types::RepoId,
    workspace,
    workspace::Workspace,
};

pub fn init(workspace: &Workspace) -> Result<()> {
    let root = workspace::init_workspace(workspace)?;
    println!("Initialized agentbox workspace at {root}");
    Ok(())
}

pub fn materialize(workspace: &Workspace, args: RepoSelectionArgs) -> Result<()> {
    let requested = args.repo_id.map(RepoId::new).transpose()?;
    for outcome in registry::materialize_repos(workspace, requested)? {
        match outcome {
            registry::MaterializeOutcome::Materialized { repo_id, repo_root } => {
                println!("Materialized `{repo_id}` at {repo_root}");
            }
            registry::MaterializeOutcome::AlreadyMaterialized { repo_id, repo_root } => {
                println!("Skipped `{repo_id}`; already materialized at {repo_root}");
            }
        }
    }
    Ok(())
}

pub fn dematerialize(workspace: &Workspace, args: RepoSelectionArgs) -> Result<()> {
    let requested = args.repo_id.map(RepoId::new).transpose()?;
    for outcome in registry::dematerialize_repos(workspace, requested)? {
        match outcome {
            registry::DematerializeOutcome::Dematerialized { repo_id } => {
                println!("Dematerialized `{repo_id}`");
            }
            registry::DematerializeOutcome::AlreadyDematerialized { repo_id } => {
                println!("Skipped `{repo_id}`; repo is already dematerialized");
            }
        }
    }
    Ok(())
}

pub fn status_command(workspace: &Workspace, args: OutputArgs) -> Result<()> {
    let statuses = inspection::workspace_status(workspace)?;
    let format = OutputFormat::from_json_flag(args.json);
    print!("{}", output::render_workspace_status(format, &statuses)?);
    Ok(())
}

pub fn show(workspace: &Workspace, args: ShowArgs) -> Result<()> {
    let status = inspection::show_repo(workspace, RepoId::new(args.repo_id)?)?;
    let format = OutputFormat::from_json_flag(args.output.json);
    print!("{}", output::render_repo_show(format, &status)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::{Error, RepoWorkflowError, WorkspaceError},
        test_support, workspace,
    };

    #[test]
    fn init_reports_already_initialized_workspace() {
        let workspace = test_support::test_workspace("agentbox-commands-repo-test");
        workspace::init_workspace(&workspace).expect("init workspace");

        let error = init(&workspace).expect_err("second init should fail");

        assert!(matches!(
            error,
            Error::Workspace(WorkspaceError::AlreadyInitialized { .. })
        ));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn materialize_rejects_invalid_repo_id() {
        let workspace = test_support::test_workspace("agentbox-commands-repo-test");

        let error = materialize(
            &workspace,
            RepoSelectionArgs {
                repo_id: Some("bad repo id".to_string()),
            },
        )
        .expect_err("invalid repo id should fail");

        assert!(matches!(error, Error::Validation(_)));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn dematerialize_rejects_invalid_repo_id() {
        let workspace = test_support::test_workspace("agentbox-commands-repo-test");

        let error = dematerialize(
            &workspace,
            RepoSelectionArgs {
                repo_id: Some("bad repo id".to_string()),
            },
        )
        .expect_err("invalid repo id should fail");

        assert!(matches!(error, Error::Validation(_)));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn status_command_reports_missing_workspace() {
        let workspace = test_support::test_workspace("agentbox-commands-repo-test");

        let error = status_command(&workspace, OutputArgs { json: false })
            .expect_err("missing workspace should fail");

        assert!(matches!(
            error,
            Error::Workspace(WorkspaceError::NoWorkspace { .. })
        ));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn show_rejects_invalid_repo_id_before_lookup() {
        let workspace = test_support::test_workspace("agentbox-commands-repo-test");

        let error = show(
            &workspace,
            ShowArgs {
                repo_id: "bad repo id".to_string(),
                output: OutputArgs { json: false },
            },
        )
        .expect_err("invalid repo id should fail");

        assert!(matches!(error, Error::Validation(_)));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn show_propagates_unknown_repo_errors() {
        let workspace = test_support::test_workspace("agentbox-commands-repo-test");
        workspace::init_workspace(&workspace).expect("init workspace");

        let error = show(
            &workspace,
            ShowArgs {
                repo_id: "demo".to_string(),
                output: OutputArgs { json: true },
            },
        )
        .expect_err("missing repo should fail");

        assert!(matches!(
            error,
            Error::Repo(RepoWorkflowError::RepoNotRegistered { .. })
        ));
        let _ = std::fs::remove_dir_all(workspace.root());
    }
}
