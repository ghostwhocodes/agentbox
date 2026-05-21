use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

use crate::shared::error::{Error, Result, ValidationError};

fn validate_slug(value: &str, label: &str) -> Result<()> {
    let mut bytes = value.bytes();
    let valid = matches!(bytes.next(), Some(b'a'..=b'z' | b'0'..=b'9' | b'_'))
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ValidationError::InvalidSlug {
            label: label.to_string(),
            value: value.to_string(),
        }
        .into())
    }
}

fn is_scp_like(source: &str) -> bool {
    let Some((user_host, path)) = source.split_once(':') else {
        return false;
    };
    let Some((user, host)) = user_host.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !host.is_empty()
        && !host
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || matches!(b, b':' | b'/'))
        && !path.is_empty()
}

fn validate_clone_source_value(source: &str) -> Result<()> {
    let valid = source.starts_with("https://")
        || source.starts_with("ssh://")
        || source.starts_with("file://")
        || is_scp_like(source);

    if valid {
        Ok(())
    } else {
        Err(ValidationError::InvalidCloneSource {
            clone_source: source.to_string(),
        }
        .into())
    }
}

fn normalize_relative_path(path: &str, label: &str) -> Result<Utf8PathBuf> {
    if path.is_empty() {
        return Err(ValidationError::EmptyRelativePath {
            label: label.to_string(),
        }
        .into());
    }

    let path = Utf8Path::new(path);
    if path.is_absolute() {
        return Err(ValidationError::AbsoluteRelativePath {
            label: label.to_string(),
            path: path.as_str().to_string(),
        }
        .into());
    }

    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            Utf8Component::Normal(segment) => normalized.push(segment),
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                return Err(ValidationError::RelativePathEscapesBase {
                    label: label.to_string(),
                    path: path.as_str().to_string(),
                }
                .into());
            }
            Utf8Component::RootDir | Utf8Component::Prefix(_) => {
                return Err(ValidationError::AbsoluteRelativePath {
                    label: label.to_string(),
                    path: path.as_str().to_string(),
                }
                .into());
            }
        }
    }

    if normalized.as_str().is_empty() {
        return Err(ValidationError::RelativePathResolvesToRoot {
            label: label.to_string(),
        }
        .into());
    }

    Ok(normalized)
}

macro_rules! define_slug_type {
    ($name:ident, $label:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self> {
                Self::try_from(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;

            fn try_from(value: String) -> Result<Self> {
                validate_slug(&value, $label)?;
                Ok(Self(value))
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                validate_slug(value, $label)?;
                Ok(Self(value.to_string()))
            }
        }
    };
}

define_slug_type!(
    RepoId,
    "repo id",
    "Validated repository identifier (slug: `[a-z0-9_-]+`)."
);
define_slug_type!(
    TemplateId,
    "template id",
    "Validated template identifier (slug: `[a-z0-9_-]+`)."
);

/// A validated git clone source (HTTPS, SSH, file, or SCP-style URL).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CloneSource(String);

impl CloneSource {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CloneSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for CloneSource {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<CloneSource> for String {
    fn from(value: CloneSource) -> Self {
        value.0
    }
}

impl TryFrom<String> for CloneSource {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        validate_clone_source_value(&value)?;
        Ok(Self(value))
    }
}

impl FromStr for CloneSource {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        validate_clone_source_value(value)?;
        Ok(Self(value.to_string()))
    }
}

/// A validated, normalized relative path that stays within its base directory.
///
/// Rejects empty paths, absolute paths, and paths containing `..` components.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelativePath(Utf8PathBuf);

impl RelativePath {
    pub fn new(path: impl AsRef<str>, label: &str) -> Result<Self> {
        Ok(Self(normalize_relative_path(path.as_ref(), label)?))
    }

    pub fn as_path(&self) -> &Utf8Path {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_inner(self) -> Utf8PathBuf {
        self.0
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<Utf8Path> for RelativePath {
    fn as_ref(&self) -> &Utf8Path {
        self.as_path()
    }
}

impl From<RelativePath> for String {
    fn from(value: RelativePath) -> Self {
        value.0.into_string()
    }
}

impl TryFrom<String> for RelativePath {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value, "relative path")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_slug_accepts_valid() {
        assert!(validate_slug("my-repo", "test").is_ok());
        assert!(validate_slug("repo_2", "test").is_ok());
        assert!(validate_slug("a", "test").is_ok());
        assert!(validate_slug("abc-def_123", "test").is_ok());
    }

    #[test]
    fn validate_slug_rejects_invalid() {
        assert!(validate_slug("", "test").is_err());
        assert!(validate_slug("-repo", "test").is_err());
        assert!(validate_slug("MyRepo", "test").is_err());
        assert!(validate_slug("repo.name", "test").is_err());
        assert!(validate_slug("repo/name", "test").is_err());
        assert!(validate_slug("repo name", "test").is_err());
    }

    #[test]
    fn normalize_relative_path_valid() {
        assert_eq!(
            normalize_relative_path("ai", "test").unwrap().as_str(),
            "ai"
        );
        assert_eq!(
            normalize_relative_path("a/b/c", "test").unwrap().as_str(),
            "a/b/c"
        );
        assert_eq!(
            normalize_relative_path("./a/./b", "test").unwrap().as_str(),
            "a/b"
        );
        assert_eq!(
            normalize_relative_path(".loki", "test").unwrap().as_str(),
            ".loki"
        );
    }

    #[test]
    fn normalize_relative_path_rejects_invalid() {
        assert!(normalize_relative_path("", "test").is_err());
        assert!(normalize_relative_path("/abs", "test").is_err());
        assert!(normalize_relative_path("../escape", "test").is_err());
        assert!(normalize_relative_path("a/../b", "test").is_err());
        assert!(normalize_relative_path(".", "test").is_err());
        assert!(normalize_relative_path("./", "test").is_err());
    }

    #[test]
    fn is_scp_like_accepts_valid() {
        assert!(is_scp_like("git@github.com:org/repo.git"));
        assert!(is_scp_like("user@host:path"));
        assert!(is_scp_like("deploy@192.168.1.1:repos/app"));
    }

    #[test]
    fn is_scp_like_rejects_invalid() {
        assert!(!is_scp_like("https://github.com/org/repo.git"));
        assert!(!is_scp_like("no-at-sign:path"));
        assert!(!is_scp_like("user@:path")); // empty host
        assert!(!is_scp_like("@host:path")); // empty user
        assert!(!is_scp_like("user@host:")); // empty path
    }

    #[test]
    fn is_scp_like_rejects_control_chars_in_host() {
        assert!(!is_scp_like("git@host\x00name:path"));
        assert!(!is_scp_like("git@host\x07name:path"));
        assert!(!is_scp_like("git@host\x1bname:path"));
        assert!(!is_scp_like("git@host\nname:path"));
        assert!(!is_scp_like("git@host\tname:path"));
    }

    #[test]
    fn is_scp_like_rejects_spaces_in_host() {
        assert!(!is_scp_like("git@host name:path"));
    }

    #[test]
    fn slug_serde_roundtrip() {
        let id = RepoId::new("my-repo").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: RepoId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn clone_source_serde_roundtrip() {
        let source = CloneSource::new("https://github.com/org/repo.git").unwrap();
        let json = serde_json::to_string(&source).unwrap();
        let deserialized: CloneSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, deserialized);
    }

    #[test]
    fn slug_and_path_accessors_roundtrip() {
        let repo_id = RepoId::new("demo").unwrap();
        let template_id = TemplateId::new("base").unwrap();
        let source = CloneSource::new("file:///tmp/source").unwrap();
        let relative = RelativePath::new("nested/path", "relative path").unwrap();

        assert_eq!(repo_id.as_ref(), "demo");
        assert_eq!(String::from(repo_id.clone()), "demo");
        assert_eq!(template_id.as_ref(), "base");
        assert_eq!(String::from(template_id.clone()), "base");
        assert_eq!(source.as_ref(), "file:///tmp/source");
        assert_eq!(String::from(source.clone()), "file:///tmp/source");
        assert_eq!(relative.as_ref(), Utf8Path::new("nested/path"));
        assert_eq!(
            relative.clone().into_inner(),
            Utf8PathBuf::from("nested/path")
        );
        assert_eq!(String::from(relative), "nested/path");
    }

    #[test]
    fn path_and_slug_parse_from_strings() {
        assert_eq!("demo".parse::<RepoId>().unwrap().as_str(), "demo");
        assert_eq!("base".parse::<TemplateId>().unwrap().as_str(), "base");
        assert_eq!(
            "ssh://example.com/repo.git"
                .parse::<CloneSource>()
                .unwrap()
                .as_str(),
            "ssh://example.com/repo.git"
        );
        assert_eq!(
            RelativePath::try_from("nested/tool".to_string())
                .unwrap()
                .as_str(),
            "nested/tool"
        );
    }
}
