use camino::{Utf8Path, Utf8PathBuf};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::Path,
};

use crate::{
    error::{Error, InfraFsError, Result},
    paths::{describe_non_utf8_os_str, utf8_path_from_std},
};

#[derive(Debug)]
pub(crate) struct TreeCopyRollback {
    created_paths: Vec<CreatedPath>,
}

impl TreeCopyRollback {
    pub(crate) fn rollback(self) -> Result<()> {
        rollback_created_paths(&self.created_paths)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn copy_missing_tree(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    let _rollback = copy_missing_tree_tracked(source, destination)?;
    Ok(())
}

pub(crate) fn copy_missing_tree_tracked(
    source: &Utf8Path,
    destination: &Utf8Path,
) -> Result<TreeCopyRollback> {
    let mut created_paths = Vec::new();
    match copy_missing_tree_inner(source, destination, &mut created_paths) {
        Ok(()) => Ok(TreeCopyRollback { created_paths }),
        Err(error) => {
            let _ = rollback_created_paths(&created_paths);
            Err(error)
        }
    }
}

fn copy_missing_tree_inner(
    source: &Utf8Path,
    destination: &Utf8Path,
    created_paths: &mut Vec<CreatedPath>,
) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }

    if destination.exists() {
        if !destination.is_dir() {
            return Err(InfraFsError::TemplateDestinationNotDirectory {
                destination: destination.to_path_buf(),
            }
            .into());
        }
    } else {
        fs::create_dir_all(destination).map_err(|e| Error::io_path(destination, e))?;
        created_paths.push(CreatedPath::Dir(destination.to_path_buf()));
    }

    for entry in fs::read_dir(source).map_err(|e| Error::io_path(source, e))? {
        let entry = entry?;
        let entry_path = utf8_path_from_std(entry.path())?;
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|os| Error::unsupported_non_utf8_path(describe_non_utf8_os_str(&os)))?;
        let target_path = destination.join(&file_name);
        let entry_type = entry.file_type()?;

        if entry_type.is_dir() {
            copy_missing_tree_inner(&entry_path, &target_path, created_paths)?;
        } else if entry_type.is_file() {
            if !target_path.exists() {
                fs::copy(&entry_path, &target_path).map_err(|e| Error::io_path(&target_path, e))?;
                created_paths.push(CreatedPath::File(target_path));
            }
        } else {
            return Err(InfraFsError::UnsupportedTemplateEntry { entry: entry_path }.into());
        }
    }

    Ok(())
}

#[derive(Debug)]
enum CreatedPath {
    File(Utf8PathBuf),
    Dir(Utf8PathBuf),
}

fn rollback_created_paths(created_paths: &[CreatedPath]) -> Result<()> {
    for created_path in created_paths.iter().rev() {
        match created_path {
            CreatedPath::File(path) => {
                if path.exists() {
                    fs::remove_file(path).map_err(|e| Error::io_path(path, e))?;
                }
            }
            CreatedPath::Dir(path) => {
                remove_empty_dir_if_present(path)?;
            }
        }
    }
    Ok(())
}

fn remove_empty_dir_if_present(path: &Utf8Path) -> Result<()> {
    if path.exists()
        && fs::read_dir(path)
            .map_err(|e| Error::io_path(path, e))?
            .next()
            .is_none()
    {
        fs::remove_dir(path).map_err(|e| Error::io_path(path, e))?;
    }
    Ok(())
}

pub(crate) fn atomic_write_text(path: &Utf8Path, contents: &str) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(Error::io_path(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cannot atomically write a path without a parent directory",
            ),
        ));
    };
    fs::create_dir_all(parent).map_err(|e| Error::io_path(parent, e))?;

    let stem = path.file_name().unwrap_or("agentbox");
    for attempt in 0..16 {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let temp_path = parent.join(format!(
            ".{stem}.tmp-{}-{nonce}-{attempt}",
            std::process::id()
        ));

        let mut file = match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::io_path(&temp_path, error)),
        };

        let write_result = (|| -> std::io::Result<()> {
            file.write_all(contents.as_bytes())?;
            file.sync_all()
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(Error::io_path(&temp_path, error));
        }
        drop(file);

        // Rust's std::fs::rename replaces an existing destination here, including on
        // Windows for this crate's supported toolchains, so this preserves atomic replace.
        if let Err(error) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(Error::io_path(path, error));
        }

        sync_parent_dir(parent);
        return Ok(());
    }

    Err(Error::io_path(
        path,
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique temporary file for atomic write",
        ),
    ))
}

fn sync_parent_dir(path: &Utf8Path) {
    let _ = fs::File::open(path).and_then(|file| file.sync_all());
}

pub(crate) fn move_directory_with_rollback<F>(
    from: &Utf8Path,
    to: &Utf8Path,
    operation: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    if !from.exists() {
        return Err(InfraFsError::ImportRepoPathMissing {
            path: from.to_path_buf(),
        }
        .into());
    }
    if !from.is_dir() {
        return Err(InfraFsError::ImportRepoPathNotDirectory {
            path: from.to_path_buf(),
        }
        .into());
    }

    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io_path(parent, e))?;
    }

    if to.exists() {
        if !to.is_dir() {
            return Err(InfraFsError::ImportContextPathNotDirectory {
                path: to.to_path_buf(),
            }
            .into());
        }
        if fs::read_dir(to)
            .map_err(|e| Error::io_path(to, e))?
            .next()
            .is_some()
        {
            return Err(InfraFsError::ImportContextPathNotEmpty {
                path: to.to_path_buf(),
            }
            .into());
        }
        fs::remove_dir(to).map_err(|e| Error::io_path(to, e))?;
    }

    fs::rename(from, to).map_err(|e| Error::io_path(from, e))?;

    match operation() {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = remove_empty_dir_if_present(from);
            if to.exists() {
                let _ = fs::rename(to, from);
            }
            Err(error)
        }
    }
}

pub(crate) fn tree_is_empty(path: &Utf8Path) -> Result<bool> {
    Ok(fs::read_dir(path)
        .map_err(|e| Error::io_path(path, e))?
        .next()
        .is_none())
}

pub(crate) fn tree_is_directory_only_scaffolding(path: &Utf8Path) -> Result<bool> {
    path_is_directory_only_scaffolding(path.as_std_path())
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn directory_trees_match(left: &Utf8Path, right: &Utf8Path) -> Result<bool> {
    directory_trees_match_std(left.as_std_path(), right.as_std_path())
}

pub(crate) fn directory_tree_is_matching_subset(
    subset: &Utf8Path,
    superset: &Utf8Path,
) -> Result<bool> {
    directory_trees_match_with_mode_std(
        subset.as_std_path(),
        superset.as_std_path(),
        DirectoryTreeComparison::Subset,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
fn directory_trees_match_std(left: &Path, right: &Path) -> Result<bool> {
    directory_trees_match_with_mode_std(left, right, DirectoryTreeComparison::Exact)
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy)]
enum DirectoryTreeComparison {
    Exact,
    Subset,
}

fn directory_trees_match_with_mode_std(
    left: &Path,
    right: &Path,
    comparison: DirectoryTreeComparison,
) -> Result<bool> {
    if !left.exists() || !right.exists() {
        return Ok(false);
    }
    if !left.is_dir() || !right.is_dir() {
        return Ok(false);
    }

    let left_entries = read_directory_entries(left)?;
    let right_entries = read_directory_entries(right)?;

    match comparison {
        DirectoryTreeComparison::Exact => {
            if left_entries != right_entries {
                return Ok(false);
            }
        }
        DirectoryTreeComparison::Subset => {}
    }

    for name in &left_entries {
        let left_path = left.join(Path::new(&name));
        if matches!(comparison, DirectoryTreeComparison::Subset) && !right_entries.contains(name) {
            if !path_is_directory_only_scaffolding(&left_path)? {
                return Ok(false);
            }
            continue;
        }
        let right_path = right.join(Path::new(&name));
        let left_metadata =
            fs::symlink_metadata(&left_path).map_err(|e| Error::io_std_path(&left_path, e))?;
        let right_metadata =
            fs::symlink_metadata(&right_path).map_err(|e| Error::io_std_path(&right_path, e))?;

        let left_type = left_metadata.file_type();
        let right_type = right_metadata.file_type();

        if !file_types_match_kind(&left_type, &right_type) {
            return Ok(false);
        }

        if left_type.is_dir() {
            if !directory_trees_match_with_mode_std(&left_path, &right_path, comparison)? {
                return Ok(false);
            }
            continue;
        }

        if left_type.is_file() {
            if !files_match(&left_path, &right_path)? {
                return Ok(false);
            }
            if !file_executable_bits_match(&left_metadata, &right_metadata) {
                return Ok(false);
            }
            continue;
        }

        if left_type.is_symlink()
            && fs::read_link(&left_path).map_err(|e| Error::io_std_path(&left_path, e))?
                != fs::read_link(&right_path).map_err(|e| Error::io_std_path(&right_path, e))?
        {
            return Ok(false);
        }
    }

    Ok(true)
}

fn path_is_directory_only_scaffolding(path: &Path) -> Result<bool> {
    let metadata = fs::symlink_metadata(path).map_err(|e| Error::io_std_path(path, e))?;
    if !metadata.file_type().is_dir() {
        return Ok(false);
    }

    for entry in fs::read_dir(path).map_err(|e| Error::io_std_path(path, e))? {
        let entry = entry.map_err(|e| Error::io_std_path(path, e))?;
        if !path_is_directory_only_scaffolding(&entry.path())? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn files_match(left: &Path, right: &Path) -> Result<bool> {
    const BUFFER_SIZE: usize = 8192;

    let left_metadata = fs::metadata(left).map_err(|e| Error::io_std_path(left, e))?;
    let right_metadata = fs::metadata(right).map_err(|e| Error::io_std_path(right, e))?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }

    let mut left_file = fs::File::open(left).map_err(|e| Error::io_std_path(left, e))?;
    let mut right_file = fs::File::open(right).map_err(|e| Error::io_std_path(right, e))?;
    let mut left_buffer = [0_u8; BUFFER_SIZE];
    let mut right_buffer = [0_u8; BUFFER_SIZE];

    loop {
        let left_read = left_file
            .read(&mut left_buffer)
            .map_err(|e| Error::io_std_path(left, e))?;
        let right_read = right_file
            .read(&mut right_buffer)
            .map_err(|e| Error::io_std_path(right, e))?;

        if left_read != right_read {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
        if left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
    }
}

fn file_types_match_kind(left: &fs::FileType, right: &fs::FileType) -> bool {
    left.is_dir() == right.is_dir()
        && left.is_file() == right.is_file()
        && left.is_symlink() == right.is_symlink()
        && unix_special_file_types_match_kind(left, right)
}

#[cfg(unix)]
fn unix_special_file_types_match_kind(left: &fs::FileType, right: &fs::FileType) -> bool {
    use std::os::unix::fs::FileTypeExt;

    left.is_block_device() == right.is_block_device()
        && left.is_char_device() == right.is_char_device()
        && left.is_fifo() == right.is_fifo()
        && left.is_socket() == right.is_socket()
}

#[cfg(not(unix))]
fn unix_special_file_types_match_kind(_left: &fs::FileType, _right: &fs::FileType) -> bool {
    true
}

#[cfg(unix)]
fn file_executable_bits_match(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    (left.permissions().mode() & 0o111 != 0) == (right.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn file_executable_bits_match(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    true
}

fn read_directory_entries(path: &Path) -> Result<BTreeSet<OsString>> {
    let mut entries = BTreeSet::new();
    for entry in fs::read_dir(path).map_err(|e| Error::io_std_path(path, e))? {
        let entry = entry.map_err(|e| Error::io_std_path(path, e))?;
        let file_name = entry.file_name();
        entries.insert(file_name);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::error::InfraMountError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[cfg(unix)]
    use std::{
        ffi::OsString,
        os::unix::ffi::OsStringExt,
        os::unix::fs::{PermissionsExt, symlink},
        path::Path,
    };

    #[cfg(not(unix))]
    use std::path::Path;

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_temp_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "agentbox-fs-ops-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos(),
            TEMP_COUNTER.fetch_add(1, Ordering::SeqCst),
        ))
    }

    fn set_readonly(path: &Path, readonly: bool) {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(path, permissions).expect("set permissions");
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions).expect("set permissions");
    }

    #[test]
    fn move_directory_rolls_back_on_failure() {
        let temp = unique_temp_path();
        fs::create_dir_all(&temp).expect("create tempdir");
        let root = Utf8Path::from_path(&temp).expect("utf8");
        let from = root.join("repo-side");
        let to = root.join("context-side");
        fs::create_dir_all(&from).expect("create source");
        fs::write(from.join("state.txt"), "state").expect("write file");

        let error =
            move_directory_with_rollback(&from, &to, || Err(InfraMountError::Unsupported.into()))
                .expect_err("operation should fail");

        assert!(matches!(
            error,
            crate::shared::error::Error::InfraMount(
                crate::shared::error::InfraMountError::Unsupported
            )
        ));
        assert!(from.join("state.txt").exists());
        assert!(!to.exists());

        let _ = fs::remove_dir_all(&temp);
    }

    fn make_temp() -> Utf8PathBuf {
        let temp = unique_temp_path();
        fs::create_dir_all(&temp).expect("create tempdir");
        Utf8PathBuf::from_path_buf(temp).expect("utf8")
    }

    #[test]
    fn atomic_write_text_writes_and_replaces_existing_file() {
        let temp = make_temp();
        let path = temp.join("agentbox.toml");

        atomic_write_text(
            &path, "first
",
        )
        .expect("write initial file");
        atomic_write_text(
            &path, "second
",
        )
        .expect("replace file");

        assert_eq!(
            fs::read_to_string(path.as_std_path()).unwrap(),
            "second
"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn atomic_write_text_preserves_existing_file_when_temp_create_fails() {
        let temp = make_temp();
        let path = temp.join("agentbox.toml");
        atomic_write_text(
            &path, "stable
",
        )
        .expect("write initial file");

        set_readonly(temp.as_std_path(), true);
        let error = atomic_write_text(
            &path, "new
",
        )
        .expect_err("readonly dir should fail");
        set_readonly(temp.as_std_path(), false);

        assert!(matches!(error, crate::shared::error::Error::IoPath { .. }));
        assert_eq!(
            fs::read_to_string(path.as_std_path()).unwrap(),
            "stable
"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn atomic_write_text_rejects_paths_without_parent_directories() {
        let error = atomic_write_text(Utf8Path::new("/"), "new\n")
            .expect_err("root paths should be rejected");

        assert!(matches!(error, crate::shared::error::Error::IoPath { .. }));
    }

    #[test]
    fn atomic_write_text_reports_rename_failures_and_cleans_temp_file() {
        let temp = make_temp();
        let path = temp.join("agentbox.toml");
        fs::create_dir_all(&path).expect("create destination dir");

        let error = atomic_write_text(&path, "new\n").expect_err("directory target should fail");

        assert!(matches!(error, crate::shared::error::Error::IoPath { .. }));
        assert_eq!(
            fs::read_dir(temp.as_std_path())
                .expect("read tempdir")
                .filter_map(|entry| entry.ok())
                .count(),
            1
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn copy_missing_tree_rollback_preserves_pre_existing_dirs() {
        let temp = make_temp();
        let source = temp.join("source");
        let dest = temp.join("dest");

        // Set up source with a dir and file
        fs::create_dir_all(source.join("sub")).expect("create source/sub");
        fs::write(source.join("sub/new.txt"), "new").expect("write new");

        // Create a pre-existing directory in dest that should survive rollback
        fs::create_dir_all(dest.join("existing")).expect("create existing dir");
        fs::write(dest.join("existing/old.txt"), "old").expect("write old");

        // copy_missing_tree should succeed even with existing content
        copy_missing_tree(&source, &dest).expect("copy");
        assert!(dest.join("sub/new.txt").exists());
        assert!(dest.join("existing/old.txt").exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn copy_missing_tree_does_not_overwrite_existing_files() {
        let temp = make_temp();
        let source = temp.join("source");
        let dest = temp.join("dest");

        fs::create_dir_all(&source).expect("create source");
        fs::write(source.join("file.txt"), "from-source").expect("write");

        fs::create_dir_all(&dest).expect("create dest");
        fs::write(dest.join("file.txt"), "from-dest").expect("write");

        copy_missing_tree(&source, &dest).expect("copy");
        // Existing file should NOT be overwritten
        assert_eq!(
            fs::read_to_string(dest.join("file.txt")).unwrap(),
            "from-dest"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn copy_missing_tree_missing_source_is_noop() {
        let temp = make_temp();
        let dest = temp.join("dest");

        copy_missing_tree(&temp.join("missing"), &dest).expect("missing source is a no-op");
        assert!(!dest.exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn copy_missing_tree_rejects_destination_file() {
        let temp = make_temp();
        let source = temp.join("source");
        let dest = temp.join("dest");

        fs::create_dir_all(&source).expect("create source");
        fs::write(dest.as_std_path(), "not a directory").expect("write dest file");

        let error = copy_missing_tree(&source, &dest).expect_err("file destination should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::InfraFs(crate::shared::error::InfraFsError::TemplateDestinationNotDirectory {
                destination
            }) if destination == dest
        ));

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn directory_trees_do_not_match_when_executable_bits_differ() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");

        fs::create_dir_all(&left).expect("create left");
        fs::create_dir_all(&right).expect("create right");
        fs::write(left.join("script.sh"), "#!/bin/sh\necho hi\n").expect("write left");
        fs::write(right.join("script.sh"), "#!/bin/sh\necho hi\n").expect("write right");
        set_mode(left.join("script.sh").as_std_path(), 0o755);
        set_mode(right.join("script.sh").as_std_path(), 0o644);

        assert!(
            !directory_trees_match(&left, &right).expect("compare trees"),
            "executable-bit differences should block a match"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn directory_trees_match_identical_symlink_trees() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");

        fs::create_dir_all(&left).expect("create left");
        fs::create_dir_all(&right).expect("create right");
        symlink("shared-target", left.join("link")).expect("create left symlink");
        symlink("shared-target", right.join("link")).expect("create right symlink");

        assert!(
            directory_trees_match(&left, &right).expect("compare trees"),
            "identical symlinks should match"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn directory_trees_do_not_match_when_symlink_targets_differ() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");

        fs::create_dir_all(&left).expect("create left");
        fs::create_dir_all(&right).expect("create right");
        symlink("first-target", left.join("link")).expect("create left symlink");
        symlink("second-target", right.join("link")).expect("create right symlink");

        assert!(
            !directory_trees_match(&left, &right).expect("compare trees"),
            "different symlink targets should not match"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn copy_missing_tree_rejects_unsupported_entries() {
        let temp = make_temp();
        let source = temp.join("source");
        let dest = temp.join("dest");

        fs::create_dir_all(&source).expect("create source");
        symlink("missing-target", source.join("link")).expect("create symlink");

        let error = copy_missing_tree(&source, &dest).expect_err("unsupported entry should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::InfraFs(crate::shared::error::InfraFsError::UnsupportedTemplateEntry {
                entry
            }) if entry == source.join("link")
        ));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn move_directory_rejects_repo_path_that_is_not_a_directory() {
        let temp = make_temp();
        let from = temp.join("repo-file");
        let to = temp.join("context-side");
        fs::write(from.as_std_path(), "state").expect("write source file");

        let error = move_directory_with_rollback(&from, &to, || Ok(()))
            .expect_err("file source should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::InfraFs(crate::shared::error::InfraFsError::ImportRepoPathNotDirectory {
                path
            }) if path == from
        ));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn move_directory_rejects_missing_repo_path() {
        let temp = make_temp();
        let from = temp.join("missing");
        let to = temp.join("context-side");

        let error = move_directory_with_rollback(&from, &to, || Ok(()))
            .expect_err("missing repo path should fail");

        assert!(matches!(
            error,
            crate::shared::error::Error::InfraFs(
                crate::shared::error::InfraFsError::ImportRepoPathMissing { path }
            ) if path == from
        ));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn move_directory_rejects_context_path_that_is_not_a_directory() {
        let temp = make_temp();
        let from = temp.join("repo-side");
        let to = temp.join("context-side");
        fs::create_dir_all(&from).expect("create source dir");
        fs::write(to.as_std_path(), "not a directory").expect("write target file");

        let error = move_directory_with_rollback(&from, &to, || Ok(()))
            .expect_err("file target should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::InfraFs(crate::shared::error::InfraFsError::ImportContextPathNotDirectory {
                path
            }) if path == to
        ));
        assert!(from.exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn move_directory_rejects_non_empty_context_directory() {
        let temp = make_temp();
        let from = temp.join("repo-side");
        let to = temp.join("context-side");
        fs::create_dir_all(&from).expect("create source dir");
        fs::create_dir_all(&to).expect("create target dir");
        fs::write(to.join("state.txt").as_std_path(), "keep").expect("write file");

        let error = move_directory_with_rollback(&from, &to, || Ok(()))
            .expect_err("non-empty target should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::InfraFs(crate::shared::error::InfraFsError::ImportContextPathNotEmpty {
                path
            }) if path == to
        ));
        assert!(from.exists());
        assert!(to.join("state.txt").exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn move_directory_reuses_empty_context_directory_on_success() {
        let temp = make_temp();
        let from = temp.join("repo-side");
        let to = temp.join("context-side");
        fs::create_dir_all(&from).expect("create source dir");
        fs::write(from.join("state.txt").as_std_path(), "state").expect("write file");
        fs::create_dir_all(&to).expect("create empty target dir");

        move_directory_with_rollback(&from, &to, || Ok(())).expect("move should succeed");
        assert!(!from.exists());
        assert_eq!(
            fs::read_to_string(to.join("state.txt").as_std_path()).unwrap(),
            "state"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn tree_is_empty_distinguishes_empty_and_non_empty_directories() {
        let temp = make_temp();
        let empty = temp.join("empty");
        let non_empty = temp.join("non-empty");
        fs::create_dir_all(&empty).expect("create empty dir");
        fs::create_dir_all(&non_empty).expect("create non-empty dir");
        fs::write(non_empty.join("state.txt"), "state").expect("write state");

        assert!(tree_is_empty(&empty).expect("empty tree"));
        assert!(!tree_is_empty(&non_empty).expect("non-empty tree"));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn remove_empty_dir_if_present_only_removes_empty_directories() {
        let temp = make_temp();
        let empty = temp.join("empty");
        let non_empty = temp.join("non-empty");
        fs::create_dir_all(&empty).expect("create empty dir");
        fs::create_dir_all(&non_empty).expect("create non-empty dir");
        fs::write(non_empty.join("state.txt").as_std_path(), "state").expect("write file");

        remove_empty_dir_if_present(&empty).expect("remove empty dir");
        remove_empty_dir_if_present(&non_empty).expect("leave non-empty dir");

        assert!(!empty.exists());
        assert!(non_empty.exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn rollback_created_paths_ignores_missing_files() {
        let temp = make_temp();
        let root = temp.join("created");
        let file = root.join("state.txt");
        fs::create_dir_all(&root).expect("create dir");

        rollback_created_paths(&[
            CreatedPath::Dir(root.clone()),
            CreatedPath::File(file.clone()),
        ])
        .expect("rollback created paths");

        assert!(!root.exists());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn rollback_created_paths_removes_files_and_empty_directories() {
        let temp = make_temp();
        let root = temp.join("created");
        let file = root.join("state.txt");
        fs::create_dir_all(&root).expect("create dir");
        fs::write(file.as_std_path(), "state").expect("write file");

        rollback_created_paths(&[
            CreatedPath::Dir(root.clone()),
            CreatedPath::File(file.clone()),
        ])
        .expect("rollback created paths");

        assert!(!file.exists());
        assert!(!root.exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_trees_match_returns_false_when_paths_are_missing_or_not_directories() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        let right_file = temp.join("right-file");
        fs::create_dir_all(&right).expect("create right");

        assert!(!directory_trees_match(&left, &right).expect("missing path should not match"));

        fs::create_dir_all(&left).expect("create left");
        fs::write(&right_file, "state").expect("write file");

        assert!(!directory_trees_match(&left, &right_file).expect("file path should not match"));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_trees_match_returns_true_for_identical_file_trees() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        fs::create_dir_all(&left).expect("create left");
        fs::create_dir_all(&right).expect("create right");
        fs::write(left.join("state.txt"), "state").expect("write left");
        fs::write(right.join("state.txt"), "state").expect("write right");

        assert!(directory_trees_match(&left, &right).expect("identical trees should match"));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_tree_is_matching_subset_accepts_extra_entries_in_superset() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        fs::create_dir_all(subset.join("nested")).expect("create subset");
        fs::create_dir_all(superset.join("nested")).expect("create superset");
        fs::write(subset.join("nested/prompt.md"), "seed").expect("write subset");
        fs::write(superset.join("nested/prompt.md"), "seed").expect("write matching superset");
        fs::write(superset.join("nested/config.json"), "{\"mode\":\"new\"}")
            .expect("write extra superset file");

        assert!(
            directory_tree_is_matching_subset(&subset, &superset)
                .expect("subset should match superset"),
            "extra source entries should be allowed"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_tree_is_matching_subset_rejects_extra_entries_in_subset() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        fs::create_dir_all(&subset).expect("create subset");
        fs::create_dir_all(&superset).expect("create superset");
        fs::write(subset.join("prompt.md"), "seed").expect("write subset prompt");
        fs::write(superset.join("prompt.md"), "seed").expect("write superset prompt");
        fs::write(subset.join("config.json"), "{\"mode\":\"old\"}")
            .expect("write subset-only file");

        assert!(
            !directory_tree_is_matching_subset(&subset, &superset)
                .expect("subset-only entry should not match"),
            "repo-only entries would be hidden by the mount"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_tree_is_matching_subset_accepts_extra_empty_directories_in_subset() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        fs::create_dir_all(subset.join("nested/empty")).expect("create subset dirs");
        fs::create_dir_all(&superset).expect("create superset");

        assert!(
            directory_tree_is_matching_subset(&subset, &superset)
                .expect("empty directory scaffolding should be ignored"),
            "subset-only empty directories should not block matching"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_tree_is_matching_subset_rejects_extra_directory_in_subset_when_it_has_files() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        fs::create_dir_all(subset.join("nested")).expect("create subset dirs");
        fs::create_dir_all(&superset).expect("create superset");
        fs::write(subset.join("nested/state.txt"), "state").expect("write subset file");

        assert!(
            !directory_tree_is_matching_subset(&subset, &superset)
                .expect("file-bearing subset-only dir should fail"),
            "subset-only directories with files would still be hidden by the mount"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn tree_is_directory_only_scaffolding_accepts_nested_empty_directories() {
        let temp = make_temp();
        let root = temp.join("root");
        fs::create_dir_all(root.join("nested/empty")).expect("create scaffold dirs");

        assert!(
            tree_is_directory_only_scaffolding(&root)
                .expect("directory-only scaffolding should be accepted")
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn tree_is_directory_only_scaffolding_rejects_files() {
        let temp = make_temp();
        let root = temp.join("root");
        fs::create_dir_all(root.join("nested")).expect("create root dirs");
        fs::write(root.join("nested/state.txt"), "state").expect("write file");

        assert!(
            !tree_is_directory_only_scaffolding(&root)
                .expect("file-bearing trees should not be treated as scaffolding")
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn tree_is_directory_only_scaffolding_rejects_symlinks() {
        let temp = make_temp();
        let root = temp.join("root");
        fs::create_dir_all(&root).expect("create root dir");
        symlink("target", root.join("link")).expect("create symlink");

        assert!(
            !tree_is_directory_only_scaffolding(&root)
                .expect("symlink-bearing trees should not be treated as scaffolding")
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn directory_trees_match_returns_false_for_mismatched_entry_kinds() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        fs::create_dir_all(left.join("entry")).expect("create left dir entry");
        fs::create_dir_all(&right).expect("create right");
        fs::write(right.join("entry"), "state").expect("write right file");

        assert!(!directory_trees_match(&left, &right).expect("entry kind mismatch should fail"));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_trees_match_returns_true_for_identical_nested_directories() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        fs::create_dir_all(left.join("nested")).expect("create left nested");
        fs::create_dir_all(right.join("nested")).expect("create right nested");
        fs::write(left.join("nested/state.txt"), "state").expect("write left");
        fs::write(right.join("nested/state.txt"), "state").expect("write right");

        assert!(directory_trees_match(&left, &right).expect("identical nested trees should match"));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_trees_match_returns_false_for_different_file_contents() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        fs::create_dir_all(&left).expect("create left");
        fs::create_dir_all(&right).expect("create right");
        fs::write(left.join("state.txt"), "left").expect("write left");
        fs::write(right.join("state.txt"), "right").expect("write right");

        assert!(!directory_trees_match(&left, &right).expect("different contents should fail"));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_trees_match_returns_false_for_nested_directory_differences() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        fs::create_dir_all(left.join("nested")).expect("create left nested");
        fs::create_dir_all(right.join("nested")).expect("create right nested");
        fs::write(left.join("nested/state.txt"), "left").expect("write left");
        fs::write(right.join("nested/state.txt"), "right").expect("write right");

        assert!(!directory_trees_match(&left, &right).expect("nested diff should fail"));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_tree_is_matching_subset_returns_false_for_different_file_contents() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        fs::create_dir_all(&subset).expect("create subset");
        fs::create_dir_all(&superset).expect("create superset");
        fs::write(subset.join("prompt.md"), "old").expect("write subset");
        fs::write(superset.join("prompt.md"), "new").expect("write superset");

        assert!(
            !directory_tree_is_matching_subset(&subset, &superset)
                .expect("content mismatch should fail"),
            "overlapping files must stay byte-for-byte identical"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn directory_trees_match_accepts_non_utf8_entry_names() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        let entry_name = OsString::from_vec(vec![0x66, 0x6f, 0x80]);
        let nested_name = OsString::from_vec(vec![0x64, 0x69, 0x72, 0xff]);
        let left_nested = left.as_std_path().join(Path::new(&nested_name));
        let right_nested = right.as_std_path().join(Path::new(&nested_name));

        let left_create = fs::create_dir_all(&left_nested);
        let right_create = fs::create_dir_all(&right_nested);
        if left_create.is_err() || right_create.is_err() {
            let _ = fs::remove_dir_all(&temp);
            return;
        }
        fs::write(left_nested.join(Path::new(&entry_name)), "state").expect("write left");
        fs::write(right_nested.join(Path::new(&entry_name)), "state").expect("write right");

        assert!(
            directory_trees_match(&left, &right).expect("non-utf8 names should still compare"),
            "non-UTF-8 entry names should not be rejected"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn directory_tree_is_matching_subset_rejects_different_executable_bits() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        fs::create_dir_all(&subset).expect("create subset");
        fs::create_dir_all(&superset).expect("create superset");
        fs::write(subset.join("script.sh"), "#!/bin/sh\necho hi\n").expect("write subset");
        fs::write(superset.join("script.sh"), "#!/bin/sh\necho hi\n").expect("write superset");
        set_mode(subset.join("script.sh").as_std_path(), 0o755);
        set_mode(superset.join("script.sh").as_std_path(), 0o644);

        assert!(
            !directory_tree_is_matching_subset(&subset, &superset)
                .expect("mode mismatch should fail"),
            "overlapping files must preserve executable bits"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn directory_tree_is_matching_subset_rejects_different_symlink_targets() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        fs::create_dir_all(&subset).expect("create subset");
        fs::create_dir_all(&superset).expect("create superset");
        symlink("first-target", subset.join("link")).expect("create subset symlink");
        symlink("second-target", superset.join("link")).expect("create superset symlink");

        assert!(
            !directory_tree_is_matching_subset(&subset, &superset)
                .expect("symlink mismatch should fail"),
            "overlapping symlinks must preserve link targets"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_tree_is_matching_subset_returns_false_when_paths_are_missing_or_not_directories() {
        let temp = make_temp();
        let subset = temp.join("subset");
        let superset = temp.join("superset");
        let superset_file = temp.join("superset-file");
        fs::create_dir_all(&superset).expect("create superset");

        assert!(
            !directory_tree_is_matching_subset(&subset, &superset)
                .expect("missing subset should not match"),
            "missing paths should fail closed"
        );

        fs::create_dir_all(&subset).expect("create subset");
        fs::write(&superset_file, "state").expect("write superset file");

        assert!(
            !directory_tree_is_matching_subset(&subset, &superset_file)
                .expect("file path should not match"),
            "non-directory paths should fail closed"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn directory_trees_match_streams_large_file_comparisons() {
        let temp = make_temp();
        let left = temp.join("left");
        let right = temp.join("right");
        let payload = vec![b'a'; 32 * 1024];

        fs::create_dir_all(&left).expect("create left");
        fs::create_dir_all(&right).expect("create right");
        fs::write(left.join("blob.bin"), &payload).expect("write left");
        fs::write(right.join("blob.bin"), &payload).expect("write right");

        assert!(
            directory_trees_match(&left, &right).expect("large file trees should match"),
            "identical files larger than one chunk should match"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    // TOCTOU race note: `dematerialize_repos` checks mount state then removes the repo root.
    // Between the check and the removal, a mount could theoretically become active. This is a
    // known limitation of the current design. Tests should not depend on atomic check-and-remove.
}
