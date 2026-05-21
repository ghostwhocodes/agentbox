use camino::Utf8PathBuf;
use thiserror::Error;

use crate::shared::paths::describe_non_utf8_path;
use crate::shared::types::{CloneSource, RelativePath, RepoId, TemplateId};

/// Alias for `std::result::Result` with [`enum@Error`] as the error type.
pub type Result<T> = std::result::Result<T, Error>;

/// Top-level error type for all agentbox operations.
///
/// Variants delegate to context-specific sub-enums via `#[error(transparent)]`.
/// Prefer constructing those typed sub-enums directly inside their owning
/// modules and converting them into [`enum@Error`] with `.into()`.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Attach(#[from] AttachError),
    #[error(transparent)]
    Repo(#[from] RepoWorkflowError),
    #[error(transparent)]
    Mount(#[from] MountWorkflowError),
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error(transparent)]
    InfraGit(#[from] InfraGitError),
    #[error(transparent)]
    InfraMount(#[from] InfraMountError),
    #[error(transparent)]
    InfraFs(#[from] InfraFsError),
    #[error("I/O error at `{path}`: {source}")]
    IoPath {
        path: Utf8PathBuf,
        source: std::io::Error,
    },
    #[error("I/O error at `{path}`: {source}")]
    IoDisplayPath {
        path: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    TomlDeserialize(#[from] toml::de::Error),
    #[error(transparent)]
    TomlSerialize(#[from] toml::ser::Error),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    StripPrefix(#[from] std::path::StripPrefixError),
    #[error("doctor found {0} error(s)")]
    DoctorIssues(usize),
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error(
        "invalid {label} `{value}`; expected slug starting with [a-z0-9_] and continuing with [a-z0-9_-]*"
    )]
    InvalidSlug { label: String, value: String },
    #[error(
        "invalid clone source `{clone_source}`; expected https://, ssh://, file://, or SCP-style git SSH syntax"
    )]
    InvalidCloneSource { clone_source: String },
    #[error("{label} must not be empty")]
    EmptyRelativePath { label: String },
    #[error("{label} must be relative: `{path}`")]
    AbsoluteRelativePath { label: String, path: String },
    #[error("{label} must stay within its base directory: `{path}`")]
    RelativePathEscapesBase { label: String, path: String },
    #[error("{label} must not resolve to the root")]
    RelativePathResolvesToRoot { label: String },
    #[error("unsupported manifest version {actual}; expected {expected}")]
    InvalidManifestVersion { actual: u32, expected: u32 },
    #[error("unsupported template manifest version {actual}; expected {expected}")]
    InvalidTemplateManifestVersion { actual: u32, expected: u32 },
    #[error("repo `{repo_id}` contains duplicate mount context path `{context}`")]
    DuplicateRepoMountContext {
        repo_id: RepoId,
        context: RelativePath,
    },
    #[error("repo `{repo_id}` contains overlapping mount context paths `{first}` and `{second}`")]
    OverlappingRepoMountContext {
        repo_id: RepoId,
        first: RelativePath,
        second: RelativePath,
    },
    #[error("duplicate template mount context path `{context}`")]
    DuplicateTemplateMountContext { context: RelativePath },
    #[error("template contains overlapping mount context paths `{first}` and `{second}`")]
    OverlappingTemplateMountContext {
        first: RelativePath,
        second: RelativePath,
    },
    #[error("repo `{repo_id}` contains duplicate mount repo path `{repo_path}`")]
    DuplicateRepoMountTarget {
        repo_id: RepoId,
        repo_path: RelativePath,
    },
    #[error("repo mount references unregistered repo `{repo_id}`")]
    RepoMountUnknownRepo { repo_id: RepoId },
    #[error("repo template binding references unregistered repo `{repo_id}`")]
    RepoTemplateUnknownRepo { repo_id: RepoId },
    #[error("duplicate template mount repo path `{repo_path}`")]
    DuplicateTemplateMountTarget { repo_path: RelativePath },
    #[error("repo `{repo_id}` contains overlapping mount repo paths `{first}` and `{second}`")]
    OverlappingRepoMountTarget {
        repo_id: RepoId,
        first: RelativePath,
        second: RelativePath,
    },
    #[error("template contains overlapping mount repo paths `{first}` and `{second}`")]
    OverlappingTemplateMountTarget {
        first: RelativePath,
        second: RelativePath,
    },
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("path contains non-UTF-8 data and is unsupported: {path}")]
    UnsupportedNonUtf8Path { path: String },
    #[error("repo `{repo_id}` is not materialized; run `agentbox materialize {repo_id}` first")]
    RepoNotMaterialized { repo_id: RepoId },
    #[error("no agentbox workspace found in `{root}`; run `agentbox init` first")]
    NoWorkspace { root: Utf8PathBuf },
    #[error("workspace already initialized at `{root}`")]
    AlreadyInitialized { root: Utf8PathBuf },
}

#[derive(Debug, Error)]
pub enum AttachError {
    #[error("repo `{repo_id}` is already registered")]
    RepoAlreadyRegistered { repo_id: RepoId },
    #[error("repo `{repo_id}` is not registered")]
    RepoNotRegistered { repo_id: RepoId },
    #[error("repo `{repo_id}` is still materialized; run `agentbox dematerialize {repo_id}` first")]
    RepoStillMaterialized { repo_id: RepoId },
    #[error("repo `{repo_id}` still has active mounts; run `agentbox unmount {repo_id}` first")]
    RepoHasActiveMounts { repo_id: RepoId },
    #[error("repo `{repo_id}` has conflicting foreign mounts; resolve them before detaching")]
    RepoHasConflictingMounts { repo_id: RepoId },
}

#[derive(Debug, Error)]
pub enum RepoWorkflowError {
    #[error("repo `{repo_id}` is already registered")]
    RepoAlreadyRegistered { repo_id: RepoId },
    #[error("repo `{repo_id}` is not registered")]
    RepoNotRegistered { repo_id: RepoId },
    #[error("destination `{repo_root}` already exists but is not a valid git repo")]
    DestinationExistsNotGitRepo { repo_root: Utf8PathBuf },
    #[error("repo `{repo_id}` still has active mounts; run `agentbox unmount {repo_id}` first")]
    RepoHasActiveMounts { repo_id: RepoId },
    #[error("repo `{repo_id}` has conflicting foreign mounts; resolve them before dematerializing")]
    RepoHasConflictingMounts { repo_id: RepoId },
}

#[derive(Debug, Error)]
pub enum MountWorkflowError {
    #[error("repo `{repo_id}` is not registered")]
    RepoNotRegistered { repo_id: RepoId },
    #[error("repo `{repo_id}` has no registered mount for repo path `{repo_path}`")]
    MountNotFoundByRepoPath {
        repo_id: RepoId,
        repo_path: RelativePath,
    },
    #[error("mount `{context}` -> `{repo_path}` already exists on repo `{repo_id}`")]
    MountAlreadyExists {
        repo_id: RepoId,
        context: RelativePath,
        repo_path: RelativePath,
    },
    #[error(
        "mount `{context}` -> `{repo_path}` conflicts with an existing mount on repo `{repo_id}`"
    )]
    MountConflicts {
        repo_id: RepoId,
        context: RelativePath,
        repo_path: RelativePath,
    },
    #[error("mount `{context}` -> `{repo_path}` already exists or conflicts on repo `{repo_id}`")]
    MountExistsOrConflicts {
        repo_id: RepoId,
        context: RelativePath,
        repo_path: RelativePath,
    },
    #[error(
        "repo path `{repo_path}` on repo `{repo_id}` is still mounted at `{target}`; run `agentbox unmount {repo_id}` first"
    )]
    MountTargetStillMounted {
        repo_id: RepoId,
        repo_path: RelativePath,
        target: Utf8PathBuf,
    },
    #[error(
        "repo path `{repo_path}` on repo `{repo_id}` is mounted from `{existing_source}` instead of the expected workspace source `{expected_source}` at `{target}`; resolve that conflicting mount before editing or removing it"
    )]
    MountTargetConflictedForUpdate {
        repo_id: RepoId,
        repo_path: RelativePath,
        expected_source: Utf8PathBuf,
        existing_source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
    #[error(
        "cannot mount `{mount_source}` to `{target}` because the target is already mounted from `{existing_source}`"
    )]
    MountTargetAlreadyMounted {
        mount_source: Utf8PathBuf,
        target: Utf8PathBuf,
        existing_source: Utf8PathBuf,
    },
    #[error(
        "cannot unmount `{target}` because it is mounted from `{existing_source}` instead of the expected workspace source `{expected_source}`"
    )]
    UnmountTargetConflicts {
        expected_source: Utf8PathBuf,
        existing_source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
    #[error(
        "cannot unmount `{target}` because agentbox cannot verify ownership of the live mount from `{existing_source}`; pass `--unsafe-unmount` to tear it down anyway"
    )]
    UnmountTargetOwnershipUnverified {
        existing_source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
    #[error("repo path `{target}` is already mounted; unmount it before importing")]
    ImportTargetAlreadyMounted { target: Utf8PathBuf },
    #[error(
        "cannot mount workspace source `{mount_source}` onto repo path `{repo_path}` for repo `{repo_id}` because `{target}` already contains different files; if you are adopting that existing repo-owned directory into agentbox, unmount it and run `agentbox import-mount {repo_id} --repo-path {repo_path} --no-mount` first"
    )]
    MountTargetWouldHideDifferentFiles {
        repo_id: RepoId,
        repo_path: RelativePath,
        mount_source: Utf8PathBuf,
        target: Utf8PathBuf,
    },
}

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("template `{template_id}` already exists")]
    TemplateAlreadyExists { template_id: TemplateId },
    #[error("template `{template_id}` does not exist")]
    TemplateMissing { template_id: TemplateId },
    #[error(
        "template `{template_id}` mount `{context}` -> `{repo_path}` conflicts with an existing mount on repo `{repo_id}`"
    )]
    TemplateMountConflict {
        template_id: TemplateId,
        repo_id: RepoId,
        context: RelativePath,
        repo_path: RelativePath,
    },
    #[error(
        "template `{template_id}` wants to manage repo path `{repo_path}` on repo `{repo_id}`, but `{target}` is currently mounted; run `agentbox unmount {repo_id}` first"
    )]
    TemplateMountTargetStillMounted {
        template_id: TemplateId,
        repo_id: RepoId,
        repo_path: RelativePath,
        target: Utf8PathBuf,
    },
    #[error(
        "template `{template_id}` wants to manage repo path `{repo_path}` on repo `{repo_id}`, but `{target}` already contains different files; use `agentbox import-mount {repo_id} --repo-path {repo_path} --no-mount` first if you are adopting that existing repo-owned directory into workspace ownership"
    )]
    TemplateMountWouldHideDifferentFiles {
        template_id: TemplateId,
        repo_id: RepoId,
        repo_path: RelativePath,
        target: Utf8PathBuf,
    },
}

#[derive(Debug, Error)]
pub enum InfraGitError {
    #[error("git clone failed for `{clone_source}`: {stderr}")]
    CloneFailed {
        clone_source: CloneSource,
        stderr: String,
    },
}

#[derive(Debug, Error)]
pub enum InfraMountError {
    #[error("bind mounts are only supported on Linux in agentbox v1")]
    Unsupported,
    #[error("invalid UTF-8 in mount table field: {details}")]
    InvalidMountField { details: String },
    #[error("invalid mount table entry at line {line_no}: {reason}")]
    MalformedMountTableEntry { line_no: usize, reason: String },
    #[error(
        "bind mount failed from `{mount_source}` to `{target}`: {cause}; Linux bind mounts typically require CAP_SYS_ADMIN/root privileges"
    )]
    BindMountFailed {
        mount_source: Utf8PathBuf,
        target: Utf8PathBuf,
        cause: String,
    },
    #[error(
        "unmount failed for `{target}`: {cause}; Linux bind mounts typically require CAP_SYS_ADMIN/root privileges"
    )]
    UnmountFailed { target: Utf8PathBuf, cause: String },
}

#[derive(Debug, Error)]
pub enum InfraFsError {
    #[error("template destination `{destination}` already exists and is not a directory")]
    TemplateDestinationNotDirectory { destination: Utf8PathBuf },
    #[error(
        "unsupported template entry `{entry}`; only regular files and directories are supported"
    )]
    UnsupportedTemplateEntry { entry: Utf8PathBuf },
    #[error("repo path `{path}` does not exist for import")]
    ImportRepoPathMissing { path: Utf8PathBuf },
    #[error("repo path `{path}` must be a directory for import")]
    ImportRepoPathNotDirectory { path: Utf8PathBuf },
    #[error("context path `{path}` already exists and is not a directory")]
    ImportContextPathNotDirectory { path: Utf8PathBuf },
    #[error("context path `{path}` already exists and is not empty")]
    ImportContextPathNotEmpty { path: Utf8PathBuf },
}

impl Error {
    #[must_use]
    pub fn io_path(path: impl Into<Utf8PathBuf>, source: std::io::Error) -> Self {
        Self::IoPath {
            path: path.into(),
            source,
        }
    }

    #[must_use]
    pub fn io_std_path(path: &std::path::Path, source: std::io::Error) -> Self {
        match Utf8PathBuf::from_path_buf(path.to_path_buf()) {
            Ok(path) => Self::IoPath { path, source },
            Err(path) => Self::IoDisplayPath {
                path: describe_non_utf8_path(&path),
                source,
            },
        }
    }

    #[must_use]
    pub fn unsupported_non_utf8_path(path: impl Into<String>) -> Self {
        WorkspaceError::UnsupportedNonUtf8Path { path: path.into() }.into()
    }

    #[must_use]
    pub fn invalid_mount_field(details: impl Into<String>) -> Self {
        InfraMountError::InvalidMountField {
            details: details.into(),
        }
        .into()
    }

    #[must_use]
    pub fn malformed_mount_table_entry(line_no: usize, reason: impl Into<String>) -> Self {
        InfraMountError::MalformedMountTableEntry {
            line_no,
            reason: reason.into(),
        }
        .into()
    }

    #[must_use]
    pub fn bind_mount_failed(
        source: &camino::Utf8Path,
        target: &camino::Utf8Path,
        cause: String,
    ) -> Self {
        InfraMountError::BindMountFailed {
            mount_source: source.to_path_buf(),
            target: target.to_path_buf(),
            cause,
        }
        .into()
    }

    #[must_use]
    pub fn unmount_failed(target: &camino::Utf8Path, cause: String) -> Self {
        InfraMountError::UnmountFailed {
            target: target.to_path_buf(),
            cause,
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_id() -> RepoId {
        RepoId::new("demo").expect("valid repo id")
    }

    fn template_id() -> TemplateId {
        TemplateId::new("default").expect("valid template id")
    }

    fn rel(path: &str) -> RelativePath {
        RelativePath::new(path, "test path").expect("valid relative path")
    }

    #[test]
    fn typed_errors_convert_into_top_level_error() {
        assert!(matches!(
            Error::from(ValidationError::InvalidTemplateManifestVersion {
                actual: 2,
                expected: 1,
            }),
            Error::Validation(ValidationError::InvalidTemplateManifestVersion {
                actual: 2,
                expected: 1,
            })
        ));
        assert!(matches!(
            Error::from(WorkspaceError::RepoNotMaterialized { repo_id: repo_id() }),
            Error::Workspace(WorkspaceError::RepoNotMaterialized { .. })
        ));
        assert!(matches!(
            Error::from(AttachError::RepoNotRegistered { repo_id: repo_id() }),
            Error::Attach(AttachError::RepoNotRegistered { .. })
        ));
        assert!(matches!(
            Error::from(RepoWorkflowError::DestinationExistsNotGitRepo {
                repo_root: Utf8PathBuf::from("/tmp/repo"),
            }),
            Error::Repo(RepoWorkflowError::DestinationExistsNotGitRepo { .. })
        ));
        assert!(matches!(
            Error::from(MountWorkflowError::MountAlreadyExists {
                repo_id: repo_id(),
                context: rel("ctx"),
                repo_path: rel("target"),
            }),
            Error::Mount(MountWorkflowError::MountAlreadyExists { .. })
        ));
        assert!(matches!(
            Error::from(TemplateError::TemplateMissing {
                template_id: template_id(),
            }),
            Error::Template(TemplateError::TemplateMissing { .. })
        ));
        assert!(matches!(
            Error::from(InfraGitError::CloneFailed {
                clone_source: CloneSource::new("file:///tmp/source").expect("valid source"),
                stderr: "boom".to_string(),
            }),
            Error::InfraGit(InfraGitError::CloneFailed { .. })
        ));
        assert!(matches!(
            Error::from(InfraFsError::ImportContextPathNotEmpty {
                path: Utf8PathBuf::from("/tmp/context"),
            }),
            Error::InfraFs(InfraFsError::ImportContextPathNotEmpty { .. })
        ));
    }

    #[test]
    fn adapter_helpers_preserve_context() {
        assert!(matches!(
            Error::io_path(
                "/tmp/workspace",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied")
            ),
            Error::IoPath { ref path, .. } if path == &Utf8PathBuf::from("/tmp/workspace")
        ));
        assert!(matches!(
            Error::unsupported_non_utf8_path("bad path"),
            Error::Workspace(WorkspaceError::UnsupportedNonUtf8Path { .. })
        ));
        assert!(matches!(
            Error::invalid_mount_field("bad utf8"),
            Error::InfraMount(InfraMountError::InvalidMountField { .. })
        ));
        assert!(matches!(
            Error::malformed_mount_table_entry(7, "missing separator"),
            Error::InfraMount(InfraMountError::MalformedMountTableEntry { line_no: 7, .. })
        ));
        assert!(matches!(
            Error::bind_mount_failed(
                &camino::Utf8PathBuf::from("/source"),
                &camino::Utf8PathBuf::from("/target"),
                "boom".to_string()
            ),
            Error::InfraMount(InfraMountError::BindMountFailed { .. })
        ));
        assert!(matches!(
            Error::unmount_failed(&camino::Utf8PathBuf::from("/target"), "boom".to_string()),
            Error::InfraMount(InfraMountError::UnmountFailed { .. })
        ));
    }

    #[test]
    fn representative_display_strings_remain_stable() {
        assert_eq!(
            ValidationError::InvalidTemplateManifestVersion {
                actual: 2,
                expected: 1,
            }
            .to_string(),
            "unsupported template manifest version 2; expected 1"
        );
        assert_eq!(
            WorkspaceError::NoWorkspace {
                root: Utf8PathBuf::from("/tmp/workspace"),
            }
            .to_string(),
            "no agentbox workspace found in `/tmp/workspace`; run `agentbox init` first"
        );
        assert_eq!(
            TemplateError::TemplateMissing {
                template_id: template_id(),
            }
            .to_string(),
            "template `default` does not exist"
        );
    }
}
