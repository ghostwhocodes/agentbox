use crate::{
    cli::{TemplateArgs, TemplateCommand},
    error::Result,
    shared::types::{RepoId, TemplateId},
    templates,
    workspace::Workspace,
};

pub fn template_command(workspace: &Workspace, args: TemplateArgs) -> Result<()> {
    match args.command {
        TemplateCommand::List => {
            for template_id in templates::list_templates(workspace)? {
                println!("{template_id}");
            }
            Ok(())
        }
        TemplateCommand::Create { template_id } => {
            let template_id = TemplateId::new(template_id)?;
            templates::create_template(workspace, &template_id)?;
            println!("Created template `{template_id}`");
            Ok(())
        }
        TemplateCommand::Delete { template_id } => {
            let template_id = TemplateId::new(template_id)?;
            templates::delete_template(workspace, &template_id)?;
            println!("Deleted template `{template_id}`");
            Ok(())
        }
        TemplateCommand::Apply {
            template_id,
            repo_id,
        } => {
            let template_id = TemplateId::new(template_id)?;
            let repo_id = RepoId::new(repo_id)?;
            templates::apply_template_to_registered_repo(workspace, &template_id, &repo_id)?;
            println!("Applied template `{template_id}` to repo `{repo_id}`");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::{Error, TemplateError},
        registry,
        shared::types::CloneSource,
        test_support, workspace,
    };

    #[test]
    fn list_succeeds_when_workspace_has_no_templates() {
        let workspace = test_support::test_workspace("agentbox-commands-template-test");

        template_command(
            &workspace,
            TemplateArgs {
                command: TemplateCommand::List,
            },
        )
        .expect("listing templates should succeed");

        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn create_rejects_invalid_template_id() {
        let workspace = test_support::test_workspace("agentbox-commands-template-test");

        let error = template_command(
            &workspace,
            TemplateArgs {
                command: TemplateCommand::Create {
                    template_id: "bad template id".to_string(),
                },
            },
        )
        .expect_err("invalid template id should fail");

        assert!(matches!(error, Error::Validation(_)));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn delete_rejects_invalid_template_id() {
        let workspace = test_support::test_workspace("agentbox-commands-template-test");

        let error = template_command(
            &workspace,
            TemplateArgs {
                command: TemplateCommand::Delete {
                    template_id: "bad template id".to_string(),
                },
            },
        )
        .expect_err("invalid template id should fail");

        assert!(matches!(error, Error::Validation(_)));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_rejects_invalid_repo_id() {
        let workspace = test_support::test_workspace("agentbox-commands-template-test");

        let error = template_command(
            &workspace,
            TemplateArgs {
                command: TemplateCommand::Apply {
                    template_id: "default".to_string(),
                    repo_id: "bad repo id".to_string(),
                },
            },
        )
        .expect_err("invalid repo id should fail");

        assert!(matches!(error, Error::Validation(_)));
        let _ = std::fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn apply_propagates_missing_template_errors() {
        let workspace = test_support::test_workspace("agentbox-commands-template-test");
        workspace::init_workspace(&workspace).expect("init workspace");
        registry::attach_repo(
            &workspace,
            RepoId::new("demo").expect("valid repo id"),
            CloneSource::new("file:///tmp/source").expect("valid source"),
            None,
        )
        .expect("attach repo");

        let error = template_command(
            &workspace,
            TemplateArgs {
                command: TemplateCommand::Apply {
                    template_id: "default".to_string(),
                    repo_id: "demo".to_string(),
                },
            },
        )
        .expect_err("missing template should fail");

        assert!(matches!(
            error,
            Error::Template(TemplateError::TemplateMissing { .. })
        ));
        let _ = std::fs::remove_dir_all(workspace.root());
    }
}
