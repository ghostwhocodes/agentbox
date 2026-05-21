use crate::{
    cli::{AttachArgs, DetachArgs},
    error::Result,
    registry,
    shared::types::{CloneSource, RepoId, TemplateId},
    workspace::Workspace,
};

fn should_warn_about_reused_context(workspace: &Workspace, repo_id: &RepoId) -> bool {
    workspace.repo_context_root(repo_id).exists()
}

pub fn attach(workspace: &Workspace, args: AttachArgs) -> Result<()> {
    let repo_id = RepoId::new(args.repo_id)?;
    let source = CloneSource::new(args.source)?;
    let template_id = args.template.map(TemplateId::new).transpose()?;
    let warn_about_reused_context =
        template_id.is_none() && should_warn_about_reused_context(workspace, &repo_id);
    let repo_id = registry::attach_repo(workspace, repo_id, source, template_id)?;
    println!("Attached repo `{repo_id}`");
    if warn_about_reused_context {
        let context_root = workspace.repo_context_root(&repo_id);
        eprintln!(
            "WARN: existing workspace context at `{context_root}` was reused, but no mount mappings were restored; re-add mounts or reapply a template if you want agentbox to manage paths from that preserved context"
        );
    }
    Ok(())
}

pub fn detach(workspace: &Workspace, args: DetachArgs) -> Result<()> {
    let repo_id = RepoId::new(args.repo_id)?;
    let detached = registry::detach_repo(workspace, repo_id, args.force, args.unsafe_unmount)?;
    println!(
        "Detached repo `{}`; workspace-owned context remains at {}",
        detached.repo_id, detached.context_root
    );
    Ok(())
}
