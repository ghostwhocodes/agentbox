/// CLI output renderers for both human-readable and machine-readable display.
///
/// Human-readable status output uses tab-separated key=value fields per line.
/// Human-readable doctor output uses severity-prefixed lines (ERROR, WARN, FIXED, NOTE).
use std::fmt::Write;

use serde::Serialize;

use crate::{
    error::Result,
    inspection::{AggregateMountState, RegisteredMountStatus, RepoStatus},
    inspection::{DoctorReport, Severity},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

impl OutputFormat {
    #[must_use]
    pub fn from_json_flag(json: bool) -> Self {
        if json { Self::Json } else { Self::Human }
    }
}

#[derive(Serialize)]
struct WorkspaceStatusOutput<'a> {
    repos: &'a [RepoStatus],
}

#[derive(Serialize)]
struct DoctorReportOutput<'a> {
    ok: bool,
    error_count: usize,
    findings: &'a [crate::inspection::Finding],
}

#[derive(Serialize)]
struct MountListOutput<'a> {
    mounts: &'a [RegisteredMountStatus],
}

fn render_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(value)?))
}

pub fn render_workspace_status(format: OutputFormat, statuses: &[RepoStatus]) -> Result<String> {
    if format == OutputFormat::Json {
        return render_json(&WorkspaceStatusOutput { repos: statuses });
    }

    if statuses.is_empty() {
        return Ok("No repos registered.\n".to_string());
    }

    let mut output = String::new();
    for status in statuses {
        let _ = writeln!(
            output,
            "{repo_id}\tsource={}\tmaterialized={}\tmounted={}",
            status.source,
            status.materialized,
            status.mount_state,
            repo_id = status.repo_id
        );
    }
    Ok(output)
}

pub fn render_repo_show(format: OutputFormat, status: &RepoStatus) -> Result<String> {
    if format == OutputFormat::Json {
        return render_json(status);
    }

    let mut output = String::new();
    let _ = writeln!(output, "repo_id: {}", status.repo_id);
    let _ = writeln!(output, "source: {}", status.source);
    let _ = writeln!(output, "context_root: {}", status.context_root);
    let _ = writeln!(output, "repo_root: {}", status.repo_root);
    let _ = writeln!(output, "materialized: {}", status.materialized);
    let _ = writeln!(output, "mount_state: {}", status.mount_state);
    if status.mount_state == AggregateMountState::Unknown {
        let _ = writeln!(
            output,
            "note: mount table could not be read; mount state is unknown"
        );
    }
    if status.mounts.is_empty() {
        let _ = writeln!(output, "mounts: <none>");
    } else {
        let _ = writeln!(output, "mounts:");
        for mount in &status.mounts {
            if mount.inspection_unavailable {
                let _ = writeln!(
                    output,
                    "{} -> {} (inspection=unavailable)",
                    mount.source, mount.target
                );
            } else {
                let _ = writeln!(
                    output,
                    "{} -> {} (active={}, conflict={})",
                    mount.source, mount.target, mount.active, mount.conflicting_mount
                );
            }
        }
    }
    Ok(output)
}

pub fn render_doctor_report(format: OutputFormat, report: &DoctorReport) -> Result<String> {
    if format == OutputFormat::Json {
        return render_json(&DoctorReportOutput {
            ok: report.error_count() == 0,
            error_count: report.error_count(),
            findings: &report.findings,
        });
    }

    let mut output = String::new();
    for finding in &report.findings {
        let label = match finding.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN",
            Severity::Fixed => "FIXED",
            Severity::Note => "NOTE",
        };
        let _ = writeln!(output, "{label}: {}", finding.message);
    }
    Ok(output)
}

pub fn render_mount_list(format: OutputFormat, mounts: &[RegisteredMountStatus]) -> Result<String> {
    if format == OutputFormat::Json {
        return render_json(&MountListOutput { mounts });
    }

    if mounts.is_empty() {
        return Ok("No mounts registered.\n".to_string());
    }

    let mut output = String::new();
    for mount in mounts {
        let _ = writeln!(
            output,
            "{repo_id}\trepo_path={repo_path}\tcontext_path={context_path}\tmaterialized={materialized}\tactive={active}\tconflict={conflict}{inspection}",
            repo_id = mount.repo_id,
            repo_path = mount.repo_path,
            context_path = mount.context_path,
            materialized = mount.materialized,
            active = mount.active,
            conflict = mount.conflicting_mount,
            inspection = if mount.inspection_unavailable {
                "\tinspection=unavailable"
            } else {
                ""
            }
        );
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use crate::inspection::{DoctorReport, Finding, Severity};
    use crate::{
        inspection::{AggregateMountState, MountStatus, RegisteredMountStatus, RepoStatus},
        shared::types::{CloneSource, RelativePath, RepoId},
    };

    use serde_json::Value;

    use super::{
        OutputFormat, render_doctor_report, render_mount_list, render_repo_show,
        render_workspace_status,
    };

    #[test]
    fn render_repo_show_reports_unavailable_mount_inspection() {
        let status = RepoStatus {
            repo_id: RepoId::new("demo").expect("valid repo id"),
            source: CloneSource::new("file:///tmp/source").expect("valid source"),
            context_root: Utf8PathBuf::from("/tmp/workspace/context/demo"),
            repo_root: Utf8PathBuf::from("/tmp/workspace/repos/demo"),
            materialized: true,
            mount_state: AggregateMountState::Unknown,
            mounts: vec![MountStatus {
                source: Utf8PathBuf::from("/tmp/workspace/context/demo/ctx"),
                target: Utf8PathBuf::from("/tmp/workspace/repos/demo/target"),
                active: false,
                conflicting_mount: false,
                inspection_unavailable: true,
            }],
        };

        let rendered = render_repo_show(OutputFormat::Human, &status).expect("render repo");
        assert!(rendered.contains("mount_state: unknown"));
        assert!(rendered.contains("note: mount table could not be read; mount state is unknown"));
        assert!(rendered.contains("(inspection=unavailable)"));
    }

    #[test]
    fn render_workspace_status_reports_conflicted_state() {
        let statuses = vec![RepoStatus {
            repo_id: RepoId::new("demo").expect("valid repo id"),
            source: CloneSource::new("file:///tmp/source").expect("valid source"),
            context_root: Utf8PathBuf::from("/tmp/workspace/context/demo"),
            repo_root: Utf8PathBuf::from("/tmp/workspace/repos/demo"),
            materialized: true,
            mount_state: AggregateMountState::Conflicted,
            mounts: vec![],
        }];

        let rendered =
            render_workspace_status(OutputFormat::Human, &statuses).expect("render status");
        assert!(rendered.contains("mounted=conflicted"));
    }

    #[test]
    fn render_repo_show_reports_conflicted_state() {
        let status = RepoStatus {
            repo_id: RepoId::new("demo").expect("valid repo id"),
            source: CloneSource::new("file:///tmp/source").expect("valid source"),
            context_root: Utf8PathBuf::from("/tmp/workspace/context/demo"),
            repo_root: Utf8PathBuf::from("/tmp/workspace/repos/demo"),
            materialized: true,
            mount_state: AggregateMountState::Conflicted,
            mounts: vec![MountStatus {
                source: Utf8PathBuf::from("/tmp/workspace/context/demo/ctx"),
                target: Utf8PathBuf::from("/tmp/workspace/repos/demo/target"),
                active: false,
                conflicting_mount: true,
                inspection_unavailable: false,
            }],
        };

        let rendered = render_repo_show(OutputFormat::Human, &status).expect("render repo");
        assert!(rendered.contains("mount_state: conflicted"));
        assert!(rendered.contains("(active=false, conflict=true)"));
    }

    #[test]
    fn render_doctor_report_formats_all_severities() {
        let report = DoctorReport {
            findings: vec![
                Finding {
                    severity: Severity::Error,
                    message: "missing manifest".to_string(),
                },
                Finding {
                    severity: Severity::Warning,
                    message: "mount table unavailable".to_string(),
                },
                Finding {
                    severity: Severity::Fixed,
                    message: "created context root".to_string(),
                },
                Finding {
                    severity: Severity::Note,
                    message: "no changes required".to_string(),
                },
            ],
        };

        let rendered =
            render_doctor_report(OutputFormat::Human, &report).expect("render doctor report");
        assert!(rendered.contains("ERROR: missing manifest"));
        assert!(rendered.contains("WARN: mount table unavailable"));
        assert!(rendered.contains("FIXED: created context root"));
        assert!(rendered.contains("NOTE: no changes required"));
    }

    #[test]
    fn render_workspace_status_json_reports_empty_repo_list() {
        let rendered =
            render_workspace_status(OutputFormat::Json, &[]).expect("render workspace status");
        let json: Value = serde_json::from_str(&rendered).expect("parse json");
        assert_eq!(json["repos"], Value::Array(vec![]));
    }

    #[test]
    fn render_doctor_report_json_includes_summary_fields() {
        let report = DoctorReport {
            findings: vec![Finding {
                severity: Severity::Error,
                message: "missing manifest".to_string(),
            }],
        };

        let rendered =
            render_doctor_report(OutputFormat::Json, &report).expect("render doctor report");
        let json: Value = serde_json::from_str(&rendered).expect("parse json");
        assert_eq!(json["ok"], Value::Bool(false));
        assert_eq!(json["error_count"], Value::from(1));
        assert_eq!(json["findings"][0]["severity"], Value::from("error"));
        assert_eq!(
            json["findings"][0]["message"],
            Value::from("missing manifest")
        );
    }

    #[test]
    fn render_mount_list_reports_empty_state() {
        let rendered = render_mount_list(OutputFormat::Human, &[]).expect("render mount list");
        assert_eq!(rendered, "No mounts registered.\n");
    }

    #[test]
    fn render_mount_list_reports_runtime_fields() {
        let mounts = vec![RegisteredMountStatus {
            repo_id: RepoId::new("demo").expect("valid repo id"),
            repo_path: RelativePath::new(".loki", "repo path").expect("valid repo path"),
            context_path: RelativePath::new("ai", "context path").expect("valid context path"),
            source: Utf8PathBuf::from("/tmp/workspace/context/demo/ai"),
            target: Utf8PathBuf::from("/tmp/workspace/repos/demo/.loki"),
            materialized: false,
            active: false,
            conflicting_mount: false,
            inspection_unavailable: false,
        }];

        let rendered = render_mount_list(OutputFormat::Human, &mounts).expect("render mount");
        assert!(rendered.contains("demo\trepo_path=.loki\tcontext_path=ai"));
        assert!(rendered.contains("materialized=false"));
        assert!(rendered.contains("active=false"));
        assert!(rendered.contains("conflict=false"));
    }
}
