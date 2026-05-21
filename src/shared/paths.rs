use std::ffi::OsStr;

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    error::{Error, Result},
    shared::types::RelativePath,
};

pub fn normalize_to_absolute_std_path(path: std::path::PathBuf) -> Result<std::path::PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };

    canonicalize_existing_prefix(absolute)
}

fn canonicalize_existing_prefix(path: std::path::PathBuf) -> Result<std::path::PathBuf> {
    let mut existing_prefix = Some(path.as_path());
    while let Some(prefix) = existing_prefix {
        match std::fs::canonicalize(prefix) {
            Ok(canonical_prefix) => {
                let suffix = path.strip_prefix(prefix)?;
                let mut resolved = canonical_prefix;
                append_normalized_suffix(&mut resolved, suffix);
                return Ok(resolved);
            }
            Err(_) => existing_prefix = prefix.parent(),
        }
    }

    Ok(path)
}

fn append_normalized_suffix(base: &mut std::path::PathBuf, suffix: &std::path::Path) {
    for component in suffix.components() {
        match component {
            std::path::Component::Prefix(prefix) => base.push(prefix.as_os_str()),
            std::path::Component::RootDir => {}
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = base.pop();
            }
            std::path::Component::Normal(part) => base.push(part),
        }
    }
}

pub fn utf8_path_from_std(path: std::path::PathBuf) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| Error::unsupported_non_utf8_path(describe_non_utf8_path(&path)))
}

pub fn resolve_relative(base: &Utf8Path, relative: &RelativePath) -> Utf8PathBuf {
    base.join(relative.as_path())
}

pub(crate) fn describe_non_utf8_path(path: &std::path::Path) -> String {
    describe_non_utf8_os_str(path.as_os_str())
}

pub(crate) fn describe_non_utf8_os_str(value: &OsStr) -> String {
    #[cfg(unix)]
    {
        use std::fmt::Write;
        use std::os::unix::ffi::OsStrExt;

        let mut rendered = String::from("non-UTF-8 bytes 0x");
        for byte in value.as_bytes() {
            let _ = write!(rendered, "{byte:02x}");
        }
        rendered
    }

    #[cfg(not(unix))]
    {
        format!("{value:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsString,
        path::PathBuf,
        sync::{Mutex, OnceLock},
    };

    #[cfg(unix)]
    use std::os::unix::{ffi::OsStringExt, fs::symlink};

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
    fn utf8_path_from_std_accepts_utf8_paths() {
        let path = PathBuf::from("/tmp/agentbox");
        assert_eq!(
            utf8_path_from_std(path).unwrap(),
            Utf8PathBuf::from("/tmp/agentbox")
        );
    }

    #[test]
    fn normalize_to_absolute_std_path_resolves_relative_dot_segments() {
        let cwd = std::env::temp_dir().join(format!(
            "agentbox-paths-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&cwd).expect("create cwd");

        with_current_dir(&cwd, || {
            let normalized =
                normalize_to_absolute_std_path(PathBuf::from("./nested/../workspace")).unwrap();
            assert_eq!(normalized, cwd.join("workspace"));
        });

        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn normalize_to_absolute_std_path_cleans_absolute_dot_segments() {
        let path = PathBuf::from("/tmp/agentbox/./nested/../workspace");
        assert_eq!(
            normalize_to_absolute_std_path(path).unwrap(),
            PathBuf::from("/tmp/agentbox/workspace")
        );
    }

    #[cfg(unix)]
    #[test]
    fn normalize_to_absolute_std_path_resolves_symlinked_prefixes() {
        let cwd = std::env::temp_dir().join(format!(
            "agentbox-paths-symlink-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let actual_parent = cwd.join("actual-parent");
        let symlink_target = actual_parent.join("nested");
        std::fs::create_dir_all(&symlink_target).expect("create symlink target");
        symlink(&symlink_target, cwd.join("link")).expect("create symlink");

        with_current_dir(&cwd, || {
            let normalized =
                normalize_to_absolute_std_path(PathBuf::from("./link/../workspace")).unwrap();
            assert_eq!(normalized, actual_parent.join("workspace"));
        });

        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn resolve_relative_joins_base_and_relative() {
        let relative = RelativePath::new("nested/config", "relative path").unwrap();
        assert_eq!(
            resolve_relative(Utf8Path::new("/workspace/context"), &relative),
            Utf8PathBuf::from("/workspace/context/nested/config")
        );
    }

    #[cfg(unix)]
    #[test]
    fn describe_non_utf8_os_str_renders_hex_bytes() {
        let value = OsString::from_vec(vec![0x66, 0x6f, 0x80]);
        assert_eq!(describe_non_utf8_os_str(&value), "non-UTF-8 bytes 0x666f80");
    }

    #[cfg(unix)]
    #[test]
    fn utf8_path_from_std_rejects_non_utf8_paths() {
        let path = PathBuf::from(OsString::from_vec(vec![0x66, 0x80]));
        let error = utf8_path_from_std(path).unwrap_err();
        assert_eq!(
            error.to_string(),
            "path contains non-UTF-8 data and is unsupported: non-UTF-8 bytes 0x6680"
        );
    }
}
