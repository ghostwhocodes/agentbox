use crate::{
    cli::{
        AddMountArgs, EditMountArgs, ImportMountArgs, ListMountsArgs, RemoveMountArgs,
        RepoSelectionArgs, UnmountArgs,
    },
    error::Result,
    inspection, mounts,
    output::{self, OutputFormat},
    shared::types::{RelativePath, RepoId},
    workspace::Workspace,
};

fn render_mount_outcome(outcome: &mounts::MountOutcome) -> String {
    match outcome {
        mounts::MountOutcome::Mounted { source, target } => {
            format!("Mounted `{source}` -> `{target}`")
        }
        mounts::MountOutcome::AlreadyMounted { source, target } => {
            format!("Already mounted `{source}` -> `{target}`")
        }
    }
}

fn render_unmount_outcome(outcome: &mounts::UnmountOutcome) -> String {
    match outcome {
        mounts::UnmountOutcome::Unmounted { target } => {
            format!("Unmounted `{target}`")
        }
        mounts::UnmountOutcome::AlreadyUnmounted { target } => {
            format!("Already unmounted `{target}`")
        }
    }
}

pub fn mount_repos(workspace: &Workspace, args: RepoSelectionArgs) -> Result<()> {
    let requested = args.repo_id.map(RepoId::new).transpose()?;
    for outcome in mounts::mount_repos(workspace, requested)? {
        println!("{}", render_mount_outcome(&outcome));
    }
    Ok(())
}

pub fn unmount_repos(workspace: &Workspace, args: UnmountArgs) -> Result<()> {
    let requested = args.repo_id.map(RepoId::new).transpose()?;
    for outcome in mounts::unmount_repos(workspace, requested, args.unsafe_unmount)? {
        println!("{}", render_unmount_outcome(&outcome));
    }
    Ok(())
}

pub fn add_mount(workspace: &Workspace, args: AddMountArgs) -> Result<()> {
    let added = mounts::add_mount(
        workspace,
        RepoId::new(args.repo_id)?,
        RelativePath::new(args.context_path, "mount context path")?,
        RelativePath::new(args.repo_path, "mount repo path")?,
    )?;
    println!(
        "Registered mount `{}` -> `{}` on repo `{}`",
        added.mount.context, added.mount.repo, added.repo_id
    );
    Ok(())
}

pub fn remove_mount(workspace: &Workspace, args: RemoveMountArgs) -> Result<()> {
    let removed = mounts::remove_mount(
        workspace,
        RepoId::new(args.repo_id)?,
        RelativePath::new(args.repo_path, "mount repo path")?,
    )?;
    println!(
        "Removed mount for repo path `{}` on repo `{}`",
        removed.mount.repo, removed.repo_id
    );
    Ok(())
}

pub fn edit_mount(workspace: &Workspace, args: EditMountArgs) -> Result<()> {
    let edited = mounts::edit_mount(
        workspace,
        RepoId::new(args.repo_id)?,
        RelativePath::new(args.repo_path, "mount repo path")?,
        RelativePath::new(args.new_context_path, "mount context path")?,
    )?;
    println!(
        "Updated mount for repo path `{}` on repo `{}` to context `{}`",
        edited.mount.repo, edited.repo_id, edited.mount.context
    );
    Ok(())
}

pub fn import_mount(workspace: &Workspace, args: ImportMountArgs) -> Result<()> {
    let imported = mounts::import_mount(
        workspace,
        RepoId::new(args.repo_id)?,
        RelativePath::new(args.repo_path, "mount repo path")?,
        args.context_path
            .map(|path| RelativePath::new(path, "mount context path"))
            .transpose()?,
        args.no_mount,
    )?;
    println!(
        "Imported `{}` into workspace context `{}` for repo `{}`",
        imported.mount.repo, imported.mount.context, imported.repo_id
    );
    Ok(())
}

pub fn list_mounts(workspace: &Workspace, args: ListMountsArgs) -> Result<()> {
    let mounts = inspection::list_mounts(workspace, args.repo_id.map(RepoId::new).transpose()?)?;
    let format = OutputFormat::from_json_flag(args.output.json);
    print!("{}", output::render_mount_list(format, &mounts)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::*;
    use crate::{
        error::{Error, MountWorkflowError},
        registry,
        shared::types::CloneSource,
        test_support, workspace,
    };

    #[test]
    fn render_mount_outcome_formats_both_variants() {
        let mounted = render_mount_outcome(&mounts::MountOutcome::Mounted {
            source: Utf8PathBuf::from("/context/demo/ctx"),
            target: Utf8PathBuf::from("/repos/demo/target"),
        });
        let already_mounted = render_mount_outcome(&mounts::MountOutcome::AlreadyMounted {
            source: Utf8PathBuf::from("/context/demo/ctx"),
            target: Utf8PathBuf::from("/repos/demo/target"),
        });

        assert_eq!(
            mounted,
            "Mounted `/context/demo/ctx` -> `/repos/demo/target`"
        );
        assert_eq!(
            already_mounted,
            "Already mounted `/context/demo/ctx` -> `/repos/demo/target`"
        );
    }

    #[test]
    fn render_unmount_outcome_formats_both_variants() {
        let unmounted = render_unmount_outcome(&mounts::UnmountOutcome::Unmounted {
            target: Utf8PathBuf::from("/repos/demo/target"),
        });
        let already_unmounted = render_unmount_outcome(&mounts::UnmountOutcome::AlreadyUnmounted {
            target: Utf8PathBuf::from("/repos/demo/target"),
        });

        assert_eq!(unmounted, "Unmounted `/repos/demo/target`");
        assert_eq!(already_unmounted, "Already unmounted `/repos/demo/target`");
    }

    #[test]
    fn remove_mount_propagates_unknown_repo_path_errors() {
        let workspace = test_support::test_workspace("agentbox-commands-mount-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        workspace::init_workspace(&workspace).expect("init workspace");
        registry::attach_repo(
            &workspace,
            repo_id.clone(),
            CloneSource::new("file:///tmp/source").expect("valid source"),
            None,
        )
        .expect("attach repo");
        mounts::add_mount(
            &workspace,
            repo_id.clone(),
            RelativePath::new("ctx", "mount context path").expect("valid context path"),
            RelativePath::new("target", "mount repo path").expect("valid repo path"),
        )
        .expect("add mount");

        let error = remove_mount(
            &workspace,
            RemoveMountArgs {
                repo_id: repo_id.as_str().to_string(),
                repo_path: "missing".to_string(),
            },
        )
        .expect_err("missing mount should fail");

        assert!(matches!(
            error,
            Error::Mount(MountWorkflowError::MountNotFoundByRepoPath {
                repo_id: ref error_repo_id,
                ref repo_path,
            }) if error_repo_id == &repo_id && repo_path.as_str() == "missing"
        ));

        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn mount_repos_rejects_invalid_repo_ids() {
        let workspace = test_support::test_workspace("agentbox-commands-mount-test");

        let error = mount_repos(
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
    fn unmount_repos_reports_already_unmounted_mounts() {
        let workspace = test_support::test_workspace("agentbox-commands-mount-test");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        workspace::init_workspace(&workspace).expect("init workspace");
        registry::attach_repo(
            &workspace,
            repo_id.clone(),
            CloneSource::new("file:///tmp/source").expect("valid source"),
            None,
        )
        .expect("attach repo");
        mounts::add_mount(
            &workspace,
            repo_id.clone(),
            RelativePath::new("ctx", "mount context path").expect("valid context path"),
            RelativePath::new("target", "mount repo path").expect("valid repo path"),
        )
        .expect("add mount");

        unmount_repos(
            &workspace,
            UnmountArgs {
                repo_id: Some(repo_id.as_str().to_string()),
                unsafe_unmount: false,
            },
        )
        .expect("unmount should succeed");

        let _ = std::fs::remove_dir_all(workspace.root());
    }
}
