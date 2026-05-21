use crate::{
    error::Result,
    inspection::{self, MountTableState, RepoStatus},
    mounts::{infra as mount, ownership},
    persistence::manifest_store,
    registry,
    shared::{mount_spec::MountSpec, types::RepoId},
    workspace,
    workspace::Workspace,
};
use serde::Serialize;

use std::collections::BTreeSet;

use workspace::ChildDirScan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Fixed,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub findings: Vec<Finding>,
}

impl DoctorReport {
    fn push(&mut self, severity: Severity, message: impl Into<String>) {
        self.findings.push(Finding {
            severity,
            message: message.into(),
        });
    }

    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count()
    }
}

fn ensure_directory(
    report: &mut DoctorReport,
    path: &camino::Utf8Path,
    description: &str,
    fix: bool,
    ensure: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    if fix {
        ensure()?;
        report.push(
            Severity::Fixed,
            format!("created missing {description} at `{path}`"),
        );
        Ok(())
    } else {
        report.push(
            Severity::Error,
            format!("missing {description} at `{path}`"),
        );
        Ok(())
    }
}

pub fn run_doctor(workspace: &Workspace, fix: bool) -> Result<DoctorReport> {
    run_with_mount_loader(workspace, fix, mount::list_mounts)
}

fn run_with_mount_loader<F>(
    workspace: &Workspace,
    fix: bool,
    list_mounts: F,
) -> Result<DoctorReport>
where
    F: FnOnce() -> Result<Vec<mount::MountEntry>>,
{
    let mut report = DoctorReport::default();
    let manifest_path = workspace.manifest_path();
    if !manifest_path.exists() {
        report.push(
            Severity::Error,
            format!("missing manifest at `{manifest_path}`"),
        );
        return Ok(report);
    }

    if let Err(error) = manifest_store::read(workspace) {
        report.push(
            Severity::Error,
            format!("failed to parse `{manifest_path}`: {error}"),
        );
        return Ok(report);
    }

    let snapshot = match inspection::load_workspace_snapshot(workspace, None, list_mounts) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            report.push(
                Severity::Error,
                format!("failed to inspect workspace state: {error}"),
            );
            return Ok(report);
        }
    };

    ensure_directory(
        &mut report,
        &workspace.context_dir(),
        "`context/` directory",
        fix,
        || workspace::ensure_context_dir(workspace),
    )?;
    ensure_directory(
        &mut report,
        &workspace.repos_dir(),
        "`repos/` directory",
        fix,
        || workspace::ensure_repos_dir(workspace),
    )?;
    ensure_directory(
        &mut report,
        &workspace.templates_dir(),
        "`templates/` directory",
        fix,
        || workspace::ensure_templates_dir(workspace),
    )?;

    match workspace::has_repos_ignore(workspace)? {
        true => {}
        false if fix => {
            workspace::ensure_repos_ignored(workspace)?;
            report.push(
                Severity::Fixed,
                "added missing `repos/` entry to `.gitignore`",
            );
        }
        false => {
            report.push(
                Severity::Error,
                "workspace `.gitignore` does not ignore `repos/`",
            );
        }
    }
    match workspace::has_agentbox_ignore(workspace)? {
        true => {}
        false if fix => {
            workspace::ensure_agentbox_ignored(workspace)?;
            report.push(
                Severity::Fixed,
                "added missing `.agentbox/` entry to `.gitignore`",
            );
        }
        false => {
            report.push(
                Severity::Error,
                "workspace `.gitignore` does not ignore `.agentbox/`",
            );
        }
    }

    #[cfg(target_os = "linux")]
    match mount::mount_privilege_status() {
        mount::MountPrivilegeStatus::HasCapSysAdmin => {}
        mount::MountPrivilegeStatus::MissingCapSysAdmin => {
            report.push(
                Severity::Warning,
                "CAP_SYS_ADMIN is not available; bind-mount commands may fail without additional privileges",
            );
        }
        mount::MountPrivilegeStatus::Unknown { details } => {
            report.push(
                Severity::Warning,
                format!(
                    "could not determine CAP_SYS_ADMIN availability: {details}; bind-mount commands may fail"
                ),
            );
        }
    }

    #[cfg(not(target_os = "linux"))]
    report.push(
        Severity::Error,
        "bind mounts are only supported on Linux in agentbox v1",
    );

    if let MountTableState::Unavailable(error) = &snapshot.mount_table {
        report.push(
            Severity::Warning,
            format!(
                "could not read mount table: {error}; mount-specific checks were skipped, so workspace validation is incomplete"
            ),
        );
    }

    check_stale_template_bindings(&mut report, workspace, fix)?;
    let live_mount_data_loaded = snapshot.repos.iter().any(|repo| {
        !repo.repo_mounts.is_empty() && registry::is_repo_materialized(workspace, &repo.repo_id)
    });
    if live_mount_data_loaded {
        check_stale_mount_ownership_records(
            &mut report,
            workspace,
            snapshot
                .repos
                .iter()
                .map(|repo| (&repo.repo_id, repo.repo_mounts.as_slice())),
            &snapshot.mount_table,
            fix,
        )?;
    }

    let registered_repo_ids: BTreeSet<_> = snapshot
        .repos
        .iter()
        .map(|repo| repo.repo_id.clone())
        .collect();
    let owned_mounts = ownership::load_advisory(workspace);

    for repo in &snapshot.repos {
        let repo_status = inspection::inspect_registered_repo(
            workspace,
            &repo.repo_id,
            &repo.source,
            &repo.repo_mounts,
            &snapshot.mount_table,
            &owned_mounts,
            repo.inspect_non_materialized_live_mounts,
        )?;
        check_repo(
            &mut report,
            workspace,
            &repo_status,
            &snapshot.mount_table,
            fix,
        )?;
    }

    check_orphans(
        &mut report,
        workspace,
        &registered_repo_ids,
        "materialized repo",
        "repos",
        workspace::scan_materialized_repo_dirs,
    )?;
    check_orphans(
        &mut report,
        workspace,
        &registered_repo_ids,
        "context root",
        "context",
        workspace::scan_context_repo_dirs,
    )?;

    if report.findings.is_empty() {
        report.push(Severity::Note, "workspace looks healthy");
    }

    Ok(report)
}

fn check_repo(
    report: &mut DoctorReport,
    workspace: &Workspace,
    repo_status: &RepoStatus,
    mount_table: &MountTableState,
    fix: bool,
) -> Result<()> {
    let repo_id = &repo_status.repo_id;
    let context_root = &repo_status.context_root;
    if !context_root.exists() {
        if fix {
            registry::ensure_repo_context_root(workspace, repo_id)?;
            report.push(
                Severity::Fixed,
                format!("created missing context root for repo `{repo_id}`"),
            );
        } else {
            report.push(
                Severity::Error,
                format!("missing context root for repo `{repo_id}` at `{context_root}`"),
            );
        }
    }

    let repo_root = &repo_status.repo_root;
    if repo_root.exists() && !repo_status.materialized {
        report.push(
            Severity::Error,
            format!(
                "materialized repo `{repo_id}` exists at `{repo_root}` but is not a valid git repo"
            ),
        );
    }

    for mount_status in &repo_status.mounts {
        if !mount_status.source.exists() {
            report.push(
                Severity::Error,
                format!(
                    "mount source for repo `{repo_id}` is missing: `{}`",
                    mount_status.source
                ),
            );
        }

        if repo_root.exists() {
            if !mount_status.target.starts_with(repo_root) {
                report.push(
                    Severity::Error,
                    format!(
                        "mount target `{}` for repo `{repo_id}` escapes materialized repo root `{repo_root}`",
                        mount_status.target
                    ),
                );
                continue;
            }
            if let Some(entry) =
                inspection::observed_mount_for_target(mount_table, &mount_status.target)
            {
                if mount_status.active || mount_status.inspection_unavailable {
                    continue;
                }
                report.push(
                    Severity::Error,
                    format!(
                        "mount target `{target}` for repo `{repo_id}` is mounted from `{}` instead of `{source}`",
                        entry.preferred_source,
                        target = mount_status.target,
                        source = mount_status.source,
                    ),
                );
            }
        }
    }

    Ok(())
}

fn check_stale_mount_ownership_records<'a>(
    report: &mut DoctorReport,
    workspace: &Workspace,
    repo_mounts: impl IntoIterator<Item = (&'a RepoId, &'a [MountSpec])>,
    mount_table: &MountTableState,
    fix: bool,
) -> Result<()> {
    let MountTableState::Available(mount_table) = mount_table else {
        return Ok(());
    };
    if !workspace.mount_ownership_path().exists() {
        return Ok(());
    }

    let mut ownership_state = match ownership::load(workspace) {
        Ok(state) => state,
        Err(error) => {
            report.push(
                Severity::Warning,
                format!(
                    "failed to read mount ownership receipts at `{}`: {error}",
                    workspace.mount_ownership_path()
                ),
            );
            return Ok(());
        }
    };
    let mut stale_records = Vec::new();
    for (repo_id, mounts) in repo_mounts {
        for mount_spec in mounts {
            let target = workspace.repo_path(repo_id, &mount_spec.repo);
            let target_is_mounted = mount_table.iter().any(|mount| mount.target == target);
            if target_is_mounted {
                continue;
            }
            if let Some(record) = ownership_state.owned_mount(repo_id, mount_spec) {
                stale_records.push((repo_id.clone(), record.mount_spec(), target));
            }
        }
    }

    if stale_records.is_empty() {
        return Ok(());
    }

    if fix {
        for (repo_id, mount_spec, target) in &stale_records {
            ownership_state.remove_mount(repo_id, mount_spec);
            report.push(
                Severity::Fixed,
                format!(
                    "removed stale mount ownership receipt for repo `{repo_id}` target `{target}`"
                ),
            );
        }
        ownership::save(workspace, &ownership_state)?;
    } else {
        for (repo_id, _, target) in &stale_records {
            report.push(
                Severity::Warning,
                format!(
                    "mount ownership receipt for repo `{repo_id}` target `{target}` is stale; no live mount exists at that target"
                ),
            );
        }
    }

    Ok(())
}

fn check_orphans(
    report: &mut DoctorReport,
    workspace: &Workspace,
    registered_repo_ids: &BTreeSet<RepoId>,
    kind: &str,
    dir: &str,
    scan_fn: fn(&Workspace) -> Result<ChildDirScan>,
) -> Result<()> {
    let scan = scan_fn(workspace)?;
    for path in scan.non_utf8_entries {
        report.push(
            Severity::Error,
            format!("orphan {kind} found at `{path}` (directory name is not valid UTF-8)"),
        );
    }
    for name in scan.utf8_names {
        let is_registered = RepoId::new(&name)
            .ok()
            .is_some_and(|repo_id| registered_repo_ids.contains(&repo_id));
        if !is_registered {
            report.push(
                Severity::Error,
                format!("orphan {kind} found at `{dir}/{name}`"),
            );
        }
    }
    Ok(())
}

fn check_stale_template_bindings(
    report: &mut DoctorReport,
    workspace: &Workspace,
    fix: bool,
) -> Result<()> {
    let mut manifest = manifest_store::read(workspace)?;
    let stale_bindings = manifest
        .repo_templates
        .iter()
        .filter(|(_, binding)| !workspace.template_manifest_path(&binding.template).exists())
        .map(|(repo_id, binding)| (repo_id.clone(), binding.template.clone()))
        .collect::<Vec<_>>();

    if stale_bindings.is_empty() {
        return Ok(());
    }

    if fix {
        for (repo_id, template_id) in &stale_bindings {
            report.push(
                Severity::Fixed,
                format!(
                    "removed stale template binding for repo `{repo_id}` referencing missing template `{template_id}`"
                ),
            );
        }
        manifest
            .repo_templates
            .retain(|_, binding| workspace.template_manifest_path(&binding.template).exists());
        manifest_store::write(workspace, &manifest)?;
    } else {
        for (repo_id, template_id) in &stale_bindings {
            report.push(
                Severity::Warning,
                format!(
                    "repo `{repo_id}` references missing template `{template_id}`; template bookkeeping is stale"
                ),
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::persistence::manifest::{PersistedRepoMount, PersistedRepoRegistration};
    use crate::shared::types::{CloneSource, RelativePath, TemplateId};
    use crate::test_support;

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    fn test_workspace() -> Workspace {
        test_support::test_workspace("agentbox-app-doctor-test")
    }

    fn init_git_repo(path: &camino::Utf8Path) {
        test_support::init_git_repo(path);
    }

    fn insert_repo_with_mount(workspace: &Workspace, repo_id: &RepoId) {
        let mut manifest =
            crate::persistence::manifest_store::read(workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest.repo_mounts.push(PersistedRepoMount {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
        });
        crate::persistence::manifest_store::write(workspace, &manifest).expect("save manifest");
    }

    #[test]
    fn run_doctor_reports_missing_manifest() {
        let workspace = test_workspace();

        let report = run_doctor(&workspace, false).expect("doctor report");

        assert_eq!(report.error_count(), 1);
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.message.contains("missing manifest"))
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_reports_invalid_manifest_parse_error() {
        let workspace = test_workspace();
        fs::write(workspace.manifest_path(), "not = [valid").expect("write invalid manifest");

        let report =
            run_with_mount_loader(&workspace, false, || Ok(Vec::new())).expect("doctor report");

        assert_eq!(report.error_count(), 1);
        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Error && finding.message.contains("failed to parse")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_reports_missing_context_root_without_fix() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let report =
            run_with_mount_loader(&workspace, false, || Ok(Vec::new())).expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Error
                && finding
                    .message
                    .contains("missing context root for repo `demo`")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_fixes_missing_context_root_when_requested() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let report =
            run_with_mount_loader(&workspace, true, || Ok(Vec::new())).expect("doctor report");

        assert!(workspace.repo_context_root(&repo_id).exists());
        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Fixed
                && finding
                    .message
                    .contains("created missing context root for repo `demo`")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_reports_missing_mount_source() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        fs::create_dir_all(workspace.repo_context_root(&repo_id)).expect("create context root");

        let report =
            run_with_mount_loader(&workspace, false, || Ok(Vec::new())).expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Error
                && finding
                    .message
                    .contains("mount source for repo `demo` is missing")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_reports_foreign_mount_on_target() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx")))
            .expect("create mount source");

        let report = run_with_mount_loader(&workspace, false, || {
            Ok(vec![mount::MountEntry {
                mount_id: 61,
                preferred_source: camino::Utf8PathBuf::from("/foreign/source"),
                source_aliases: vec![camino::Utf8PathBuf::from("/foreign/source")],
                mount_point: workspace.repo_path(&repo_id, &rel("target")),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Error
                && finding
                    .message
                    .contains("is mounted from `/foreign/source` instead of")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_reports_alias_only_mount_source_as_mismatch_without_ownership_receipt() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        let expected_source = workspace.context_path(&repo_id, &rel("ctx"));
        fs::create_dir_all(&expected_source).expect("create mount source");

        let report = run_with_mount_loader(&workspace, false, || {
            Ok(vec![mount::MountEntry {
                mount_id: 62,
                preferred_source: camino::Utf8PathBuf::from("/mnt/projects/ctx"),
                source_aliases: vec![camino::Utf8PathBuf::from("/mnt/projects/ctx")],
                mount_point: workspace.repo_path(&repo_id, &rel("target")),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Error
                && finding.message.contains("is mounted from")
                && finding.message.contains("instead of")
                && finding.message.contains("/mnt/projects/ctx")
                && finding.message.contains(expected_source.as_str())
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_accepts_owned_alias_only_mount_source() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx")))
            .expect("create mount source");

        let alias_source = camino::Utf8PathBuf::from("/mnt/projects/ctx");
        let mount_id = 63;
        let mut ownership_state = ownership::MountOwnershipState::default();
        ownership_state.upsert(ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
            mount_id,
            source_root: alias_source.clone(),
            filesystem_type: "bind".to_string(),
        });
        ownership::save(&workspace, &ownership_state).expect("save ownership");

        let report = run_with_mount_loader(&workspace, false, || {
            Ok(vec![mount::MountEntry {
                mount_id,
                preferred_source: alias_source,
                source_aliases: vec![camino::Utf8PathBuf::from("/mnt/projects/ctx")],
                mount_point: workspace.repo_path(&repo_id, &rel("target")),
                filesystem_type: "bind".to_string(),
            }])
        })
        .expect("doctor report");

        assert!(!report.findings.iter().any(|finding| {
            finding.severity == Severity::Error
                && finding.message.contains("is mounted from")
                && finding.message.contains("instead of")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_reports_warning_when_mount_validation_is_skipped() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");

        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        fs::create_dir_all(workspace.context_path(&repo_id, &rel("ctx")))
            .expect("create mount source");

        let report = run_with_mount_loader(&workspace, false, || {
            Err(crate::shared::error::Error::io_path(
                "/proc/self/mountinfo",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
            ))
        })
        .expect("doctor report");

        assert_eq!(report.error_count(), 0);
        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Warning
                && finding.message.contains("could not read mount table")
                && finding.message.contains("validation is incomplete")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_skips_mount_table_loading_when_inspection_is_unnecessary() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");

        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");
        fs::create_dir_all(workspace.repo_context_root(&repo_id)).expect("create context root");

        let report = run_with_mount_loader(&workspace, false, || {
            panic!("mount table should not be loaded when inspection is unnecessary")
        })
        .expect("doctor report");

        assert_eq!(report.error_count(), 0);
        assert!(!report.findings.iter().any(|finding| {
            finding.severity == Severity::Warning
                && finding.message.contains("could not read mount table")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_warns_when_mount_ownership_receipt_is_stale() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        let source = workspace.context_path(&repo_id, &rel("ctx"));
        fs::create_dir_all(&source).expect("create mount source");
        let mut ownership_state = ownership::MountOwnershipState::default();
        ownership_state.upsert(ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
            mount_id: 71,
            source_root: source,
            filesystem_type: "bind".to_string(),
        });
        ownership::save(&workspace, &ownership_state).expect("save ownership");

        let report =
            run_with_mount_loader(&workspace, false, || Ok(Vec::new())).expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Warning
                && finding
                    .message
                    .contains("mount ownership receipt for repo `demo`")
                && finding.message.contains("is stale")
        }));
        assert_eq!(
            ownership::load(&workspace)
                .expect("load ownership")
                .records()
                .len(),
            1
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_fix_removes_stale_mount_ownership_receipt() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        insert_repo_with_mount(&workspace, &repo_id);

        let repo_root = workspace.repo_root(&repo_id);
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_git_repo(&repo_root);
        let source = workspace.context_path(&repo_id, &rel("ctx"));
        fs::create_dir_all(&source).expect("create mount source");
        let mut ownership_state = ownership::MountOwnershipState::default();
        ownership_state.upsert(ownership::OwnedMountRecord {
            repo_id: repo_id.clone(),
            context: rel("ctx"),
            repo: rel("target"),
            mount_id: 72,
            source_root: source,
            filesystem_type: "bind".to_string(),
        });
        ownership::save(&workspace, &ownership_state).expect("save ownership");

        let report =
            run_with_mount_loader(&workspace, true, || Ok(Vec::new())).expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Fixed
                && finding
                    .message
                    .contains("removed stale mount ownership receipt for repo `demo`")
        }));
        assert!(
            ownership::load(&workspace)
                .expect("load ownership")
                .records()
                .is_empty()
        );

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_reports_utf8_orphans_when_repo_is_not_registered() {
        fn scan_utf8_orphans(_: &Workspace) -> Result<ChildDirScan> {
            Ok(ChildDirScan {
                utf8_names: vec!["orphan".to_string()],
                non_utf8_entries: Vec::new(),
            })
        }

        let workspace = test_workspace();
        let mut report = DoctorReport::default();

        check_orphans(
            &mut report,
            &workspace,
            &BTreeSet::new(),
            "materialized repo",
            "repos",
            scan_utf8_orphans,
        )
        .expect("check orphans");

        assert_eq!(report.error_count(), 1);
        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Error
                && finding
                    .message
                    .contains("orphan materialized repo found at `repos/orphan`")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_turns_non_utf8_orphans_into_findings() {
        fn scan_non_utf8_orphans(_: &Workspace) -> Result<ChildDirScan> {
            Ok(ChildDirScan {
                utf8_names: Vec::new(),
                non_utf8_entries: vec!["repos/b�".to_string()],
            })
        }

        let workspace = test_workspace();
        let mut report = DoctorReport::default();

        check_orphans(
            &mut report,
            &workspace,
            &BTreeSet::new(),
            "materialized repo",
            "repos",
            scan_non_utf8_orphans,
        )
        .expect("check orphans");

        assert_eq!(report.error_count(), 1);
        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Error
                && finding
                    .message
                    .contains("orphan materialized repo found at `repos/")
                && finding
                    .message
                    .contains("directory name is not valid UTF-8")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_warns_when_template_binding_references_missing_template() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest.repo_templates.insert(
            repo_id.clone(),
            crate::persistence::manifest::PersistedTemplateBinding {
                template: TemplateId::new("default").expect("valid template id"),
            },
        );
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let report =
            run_with_mount_loader(&workspace, false, || Ok(Vec::new())).expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Warning
                && finding
                    .message
                    .contains("repo `demo` references missing template `default`")
        }));

        let _ = fs::remove_dir_all(workspace.root());
    }

    #[test]
    fn doctor_fix_removes_stale_template_bindings() {
        let workspace = test_workspace();
        crate::workspace::init_workspace(&workspace).expect("init workspace");
        let repo_id = RepoId::new("demo").expect("valid repo id");
        let mut manifest =
            crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        manifest.repos.insert(
            repo_id.clone(),
            PersistedRepoRegistration {
                source: CloneSource::new("file:///tmp/source").expect("valid source"),
            },
        );
        manifest.repo_templates.insert(
            repo_id.clone(),
            crate::persistence::manifest::PersistedTemplateBinding {
                template: TemplateId::new("default").expect("valid template id"),
            },
        );
        crate::persistence::manifest_store::write(&workspace, &manifest).expect("save manifest");

        let report =
            run_with_mount_loader(&workspace, true, || Ok(Vec::new())).expect("doctor report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == Severity::Fixed
                && finding
                    .message
                    .contains("removed stale template binding for repo `demo`")
        }));
        let manifest = crate::persistence::manifest_store::read(&workspace).expect("load manifest");
        assert!(!manifest.repo_templates.contains_key(&repo_id));

        let _ = fs::remove_dir_all(workspace.root());
    }
}
