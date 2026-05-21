use camino::{Utf8Path, Utf8PathBuf};

use crate::shared::error::{Error, Result};

/// A parsed entry from the system mount table (`/proc/self/mountinfo` on Linux).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub mount_id: u64,
    /// The source path we prefer to show in status and error messages.
    pub preferred_source: Utf8PathBuf,
    /// Every path we can resolve to this mount's source, including `preferred_source`.
    pub source_aliases: Vec<Utf8PathBuf>,
    pub mount_point: Utf8PathBuf,
    pub filesystem_type: String,
}

impl MountEntry {
    pub fn matches_source(&self, source: &Utf8Path) -> bool {
        self.preferred_source == source
            || self
                .source_aliases
                .iter()
                .any(|candidate| candidate == source)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountPrivilegeStatus {
    HasCapSysAdmin,
    MissingCapSysAdmin,
    Unknown { details: String },
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs;

    use nix::mount::{mount, umount};

    use super::{Error, MountEntry, MountPrivilegeStatus, Result, Utf8Path, Utf8PathBuf};

    const CAP_SYS_ADMIN_BIT: u32 = 21;
    const CAP_SYS_ADMIN_MASK: u64 = 1 << CAP_SYS_ADMIN_BIT;

    fn malformed_mountinfo(line_no: usize, reason: impl Into<String>) -> Error {
        Error::malformed_mount_table_entry(line_no, reason.into())
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct RawMountEntry {
        pub(super) mount_id: u64,
        pub(super) device_id: String,
        pub(super) mount_source: String,
        pub(super) source_root: Utf8PathBuf,
        pub(super) mount_point: Utf8PathBuf,
        pub(super) filesystem_type: String,
    }

    pub(super) fn unescape_mount_field(raw: &str) -> std::result::Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                let mut octal = String::new();
                for _ in 0..3 {
                    if let Some(next) = chars.peek() {
                        if ('0'..='7').contains(next) {
                            octal.push(*next);
                            chars.next();
                        }
                    }
                }
                if octal.len() == 3 {
                    if let Ok(value) = u8::from_str_radix(&octal, 8) {
                        bytes.push(value);
                        continue;
                    }
                }
                bytes.push(b'\\');
                bytes.extend(octal.as_bytes());
            } else {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
        Ok(bytes)
    }

    fn decode_mount_field(line_no: usize, field_name: &str, raw: &str) -> Result<Utf8PathBuf> {
        let bytes = unescape_mount_field(raw).map_err(|details| {
            malformed_mountinfo(
                line_no,
                format!("failed to unescape {field_name} field: {details}"),
            )
        })?;
        let decoded = String::from_utf8(bytes).map_err(|error| {
            malformed_mountinfo(
                line_no,
                format!("decoded {field_name} field is not valid UTF-8: {error}"),
            )
        })?;
        Ok(Utf8PathBuf::from(decoded))
    }

    pub(super) fn parse_mountinfo_line(line_no: usize, line: &str) -> Result<RawMountEntry> {
        let (left, right) = line
            .split_once(" - ")
            .ok_or_else(|| malformed_mountinfo(line_no, "missing ` - ` separator"))?;
        let fields: Vec<&str> = left.split_whitespace().collect();
        let right_fields: Vec<&str> = right.split_whitespace().collect();
        if fields.len() < 6 {
            return Err(malformed_mountinfo(
                line_no,
                format!(
                    "too few left fields: expected at least 6, got {}",
                    fields.len()
                ),
            ));
        }
        if right_fields.len() < 3 {
            return Err(malformed_mountinfo(
                line_no,
                format!(
                    "too few right fields: expected at least 3, got {}",
                    right_fields.len()
                ),
            ));
        }

        Ok(RawMountEntry {
            mount_id: fields[0].parse::<u64>().map_err(|error| {
                malformed_mountinfo(
                    line_no,
                    format!("invalid mount ID `{}`: {error}", fields[0]),
                )
            })?,
            device_id: fields[2].to_string(),
            mount_source: right_fields[1].to_string(),
            source_root: decode_mount_field(line_no, "root", fields[3])?,
            mount_point: decode_mount_field(line_no, "mount point", fields[4])?,
            filesystem_type: right_fields[0].to_string(),
        })
    }

    pub(super) fn parse_mount_table(contents: &str) -> Result<Vec<MountEntry>> {
        let mut raw_entries = Vec::new();
        for (index, line) in contents.lines().enumerate() {
            raw_entries.push(parse_mountinfo_line(index + 1, line)?);
        }
        Ok(raw_entries
            .into_iter()
            .map(|entry| MountEntry {
                mount_id: entry.mount_id,
                preferred_source: entry.source_root.clone(),
                source_aliases: vec![entry.source_root],
                mount_point: entry.mount_point,
                filesystem_type: entry.filesystem_type,
            })
            .collect())
    }

    pub(super) fn parse_mount_privileges(status_contents: &str) -> MountPrivilegeStatus {
        let Some(line) = status_contents
            .lines()
            .find(|line| line.starts_with("CapEff:"))
        else {
            return MountPrivilegeStatus::Unknown {
                details: "missing `CapEff:` field in `/proc/self/status`".to_string(),
            };
        };
        let Some(raw_value) = line.split_whitespace().nth(1) else {
            return MountPrivilegeStatus::Unknown {
                details: "missing capability value in `CapEff:` field".to_string(),
            };
        };
        let cap_eff = match u64::from_str_radix(raw_value, 16) {
            Ok(cap_eff) => cap_eff,
            Err(error) => {
                return MountPrivilegeStatus::Unknown {
                    details: format!("failed to parse `CapEff` value `{raw_value}`: {error}"),
                };
            }
        };

        if cap_eff & CAP_SYS_ADMIN_MASK != 0 {
            MountPrivilegeStatus::HasCapSysAdmin
        } else {
            MountPrivilegeStatus::MissingCapSysAdmin
        }
    }

    pub fn bind_mount(source: &Utf8Path, target: &Utf8Path) -> Result<()> {
        mount(
            Some(source.as_std_path()),
            target.as_std_path(),
            None::<&str>,
            nix::mount::MsFlags::MS_BIND,
            None::<&str>,
        )
        .map_err(|error| Error::bind_mount_failed(source, target, error.to_string()))
    }

    pub fn unmount(target: &Utf8Path) -> Result<()> {
        umount(target.as_std_path())
            .map_err(|error| Error::unmount_failed(target, error.to_string()))
    }

    pub fn list_mounts() -> Result<Vec<MountEntry>> {
        let contents = fs::read_to_string("/proc/self/mountinfo")
            .map_err(|e| Error::io_path("/proc/self/mountinfo", e))?;
        parse_mount_table(&contents)
    }

    pub fn mount_privilege_status() -> MountPrivilegeStatus {
        match fs::read_to_string("/proc/self/status") {
            Ok(status_contents) => parse_mount_privileges(&status_contents),
            Err(error) => MountPrivilegeStatus::Unknown {
                details: format!("failed to read `/proc/self/status`: {error}"),
            },
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use crate::shared::error::InfraMountError;

    use super::{MountEntry, MountPrivilegeStatus, Result, Utf8Path};

    pub fn bind_mount(_source: &Utf8Path, _target: &Utf8Path) -> Result<()> {
        Err(InfraMountError::Unsupported.into())
    }

    pub fn unmount(_target: &Utf8Path) -> Result<()> {
        Err(InfraMountError::Unsupported.into())
    }

    pub fn list_mounts() -> Result<Vec<MountEntry>> {
        Err(InfraMountError::Unsupported.into())
    }

    pub fn mount_privilege_status() -> MountPrivilegeStatus {
        MountPrivilegeStatus::Unknown {
            details: "CAP_SYS_ADMIN probing is only supported on Linux".to_string(),
        }
    }
}

pub fn bind_mount(source: &Utf8Path, target: &Utf8Path) -> Result<()> {
    platform::bind_mount(source, target)
}

pub fn unmount(target: &Utf8Path) -> Result<()> {
    platform::unmount(target)
}

pub fn list_mounts() -> Result<Vec<MountEntry>> {
    platform::list_mounts()
}

pub fn mount_privilege_status() -> MountPrivilegeStatus {
    platform::mount_privilege_status()
}

pub fn find_target_mount<'a>(
    mounts: &'a [MountEntry],
    target: &Utf8Path,
) -> Option<&'a MountEntry> {
    mounts.iter().find(|entry| entry.mount_point == target)
}

#[cfg(test)]
pub fn is_expected_bind_mount_in(
    mounts: &[MountEntry],
    source: &Utf8Path,
    target: &Utf8Path,
) -> bool {
    find_target_mount(mounts, target).is_some_and(|entry| entry.matches_source(source))
}

#[cfg(all(test, target_os = "linux", feature = "privileged-tests"))]
pub fn target_mount(target: &Utf8Path) -> Result<Option<MountEntry>> {
    Ok(list_mounts()?
        .into_iter()
        .find(|entry| entry.mount_point == target))
}

#[cfg(all(test, target_os = "linux", feature = "privileged-tests"))]
pub fn is_expected_bind_mount(source: &Utf8Path, target: &Utf8Path) -> Result<bool> {
    Ok(target_mount(target)?.is_some_and(|entry| entry.matches_source(source)))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[cfg(feature = "privileged-tests")]
    use super::is_expected_bind_mount;
    use super::platform::{
        parse_mount_privileges, parse_mount_table, parse_mountinfo_line, unescape_mount_field,
    };
    use super::{
        MountEntry, MountPrivilegeStatus, bind_mount, find_target_mount, is_expected_bind_mount_in,
        list_mounts, mount_privilege_status, unmount,
    };
    use camino::Utf8PathBuf;

    #[test]
    fn unescape_normal_text() {
        assert_eq!(
            String::from_utf8(unescape_mount_field("hello").unwrap()).unwrap(),
            "hello"
        );
    }

    #[test]
    fn unescape_space_octal() {
        // space is \040
        assert_eq!(
            String::from_utf8(unescape_mount_field("hello\\040world").unwrap()).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn unescape_tab_octal() {
        // tab is \011
        assert_eq!(
            String::from_utf8(unescape_mount_field("a\\011b").unwrap()).unwrap(),
            "a\tb"
        );
    }

    #[test]
    fn unescape_backslash_octal() {
        // backslash is \134
        assert_eq!(
            String::from_utf8(unescape_mount_field("a\\134b").unwrap()).unwrap(),
            "a\\b"
        );
    }

    #[test]
    fn unescape_incomplete_octal_treated_as_literal() {
        // Only 2 octal digits — should emit backslash and the digits literally
        assert_eq!(
            String::from_utf8(unescape_mount_field("a\\04x").unwrap()).unwrap(),
            "a\\04x"
        );
    }

    #[test]
    fn unescape_lone_backslash_at_end() {
        assert_eq!(
            String::from_utf8(unescape_mount_field("trail\\").unwrap()).unwrap(),
            "trail\\"
        );
    }

    #[test]
    fn unescape_non_ascii_utf8_bytes() {
        // Construct a mount field with octal-encoded UTF-8 multibyte char (é = \303\251)
        assert_eq!(
            String::from_utf8(unescape_mount_field("caf\\303\\251").unwrap()).unwrap(),
            "café"
        );
    }

    #[test]
    fn unescape_invalid_utf8_returns_error() {
        // \377 is 0xFF which is never valid UTF-8
        let bytes = unescape_mount_field("bad\\377byte").unwrap();
        assert!(String::from_utf8(bytes).is_err());
    }

    #[test]
    fn parse_mountinfo_line_rejects_missing_separator() {
        let error = parse_mountinfo_line(3, "36 35 0:31 / /rw rw").expect_err("missing separator");
        assert_eq!(
            error.to_string(),
            "invalid mount table entry at line 3: missing ` - ` separator"
        );
    }

    #[test]
    fn parse_mountinfo_line_rejects_too_few_fields() {
        let error = parse_mountinfo_line(5, "36 35 0:31 / - ext4 /dev/root rw")
            .expect_err("too few left fields");
        assert_eq!(
            error.to_string(),
            "invalid mount table entry at line 5: too few left fields: expected at least 6, got 4"
        );

        let error =
            parse_mountinfo_line(6, "36 35 0:31 / / rw - ext4").expect_err("too few right fields");
        assert_eq!(
            error.to_string(),
            "invalid mount table entry at line 6: too few right fields: expected at least 3, got 1"
        );
    }

    #[test]
    fn parse_mountinfo_line_rejects_invalid_utf8_decoded_path() {
        let line = "36 35 0:31 /bad\\377 /target rw - ext4 /dev/root rw";
        let error = parse_mountinfo_line(8, line).expect_err("invalid utf8 path");
        assert!(error.to_string().contains(
            "invalid mount table entry at line 8: decoded root field is not valid UTF-8"
        ));
    }

    #[test]
    fn parse_mountinfo_line_parses_valid_entries() {
        let line = "36 35 0:31 /source\\040dir /target\\040dir rw - ext4 /dev/root rw";
        let entry = parse_mountinfo_line(1, line).expect("parse mountinfo line");
        assert_eq!(entry.mount_id, 36);
        assert_eq!(entry.mount_source, "/dev/root");
        assert_eq!(entry.source_root, Utf8PathBuf::from("/source dir"));
        assert_eq!(entry.mount_point, Utf8PathBuf::from("/target dir"));
        assert_eq!(entry.filesystem_type, "ext4");
    }

    #[test]
    fn parse_mount_table_preserves_raw_bind_sources_within_non_root_filesystems() {
        let contents = concat!(
            "62 33 0:51 / /home rw,nodev,relatime shared:58 - zfs pool/home rw\n",
            "6412 62 0:51 /nos/Projects/context/forge/ai /home/nos/Projects/repos/forge/ai rw,nodev,relatime shared:58 - zfs pool/home rw\n"
        );

        let entries = parse_mount_table(contents).expect("parse mount table");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].preferred_source, Utf8PathBuf::from("/"));
        assert_eq!(
            entries[1].preferred_source,
            Utf8PathBuf::from("/nos/Projects/context/forge/ai")
        );
        assert_eq!(
            entries[1].mount_point,
            Utf8PathBuf::from("/home/nos/Projects/repos/forge/ai")
        );
    }

    #[test]
    fn parse_mount_table_preserves_raw_bind_sources_within_btrfs_subvolumes() {
        let contents = concat!(
            "29 1 0:29 /@ / rw,relatime - btrfs /dev/nvme0n1p2 rw,subvolid=256,subvol=/@\n",
            "30 29 0:29 /@home /home rw,relatime - btrfs /dev/nvme0n1p2 rw,subvolid=257,subvol=/@home\n",
            "31 30 0:29 /@home/nos/Projects/context/forge/ai /home/nos/Projects/repos/forge/ai rw,relatime - btrfs /dev/nvme0n1p2 rw,subvolid=257,subvol=/@home\n"
        );

        let entries = parse_mount_table(contents).expect("parse mount table");

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].preferred_source, Utf8PathBuf::from("/@"));
        assert_eq!(entries[1].preferred_source, Utf8PathBuf::from("/@home"));
        assert_eq!(
            entries[2].preferred_source,
            Utf8PathBuf::from("/@home/nos/Projects/context/forge/ai")
        );
        assert_eq!(
            entries[2].mount_point,
            Utf8PathBuf::from("/home/nos/Projects/repos/forge/ai")
        );
    }

    #[test]
    fn parse_mount_table_does_not_alias_same_filesystem_root_through_other_mounts() {
        let contents = concat!(
            "29 1 0:29 / / rw,relatime - ext4 /dev/root rw\n",
            "30 1 0:29 / /mnt/root rw,relatime - ext4 /dev/root rw\n",
            "31 29 0:29 /home/nos/context/demo /workspace/target rw,relatime - ext4 /dev/root rw\n"
        );

        let entries = parse_mount_table(contents).expect("parse mount table");

        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[2].source_aliases,
            vec![Utf8PathBuf::from("/home/nos/context/demo")]
        );
        assert_eq!(
            entries[2].preferred_source,
            Utf8PathBuf::from("/home/nos/context/demo")
        );
        assert!(entries[2].matches_source(&Utf8PathBuf::from("/home/nos/context/demo")));
        assert!(!entries[2].matches_source(&Utf8PathBuf::from("/mnt/root/home/nos/context/demo")));
    }

    #[test]
    fn parse_mount_table_does_not_alias_parent_btrfs_subvolume_paths() {
        let contents = concat!(
            "29 1 0:29 /@home /home rw,relatime - btrfs /dev/root rw,subvol=/@home\n",
            "30 29 0:29 /@home/nos/Projects /mnt/projects rw,relatime - btrfs /dev/root rw,subvol=/@home\n",
            "31 29 0:29 /@home/nos/Projects/context/demo /workspace/target rw,relatime - btrfs /dev/root rw,subvol=/@home\n"
        );

        let entries = parse_mount_table(contents).expect("parse mount table");

        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[2].source_aliases,
            vec![Utf8PathBuf::from("/@home/nos/Projects/context/demo")]
        );
        assert_eq!(
            entries[2].preferred_source,
            Utf8PathBuf::from("/@home/nos/Projects/context/demo")
        );
        assert!(!entries[2].matches_source(&Utf8PathBuf::from("/home/nos/Projects/context/demo")));
        assert!(!entries[2].matches_source(&Utf8PathBuf::from("/mnt/projects/context/demo")));
    }

    #[test]
    fn parse_mount_table_does_not_alias_same_device_mounts_with_different_sources() {
        let contents = concat!(
            "29 1 0:29 / /home rw,relatime - zfs pool/home rw\n",
            "30 29 0:29 / /home/nos rw,relatime - zfs pool/home/nos rw\n",
            "31 30 0:29 /context/demo /workspace/target rw,relatime - zfs pool/home/nos rw\n"
        );

        let entries = parse_mount_table(contents).expect("parse mount table");

        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[2].source_aliases,
            vec![Utf8PathBuf::from("/context/demo")]
        );
        assert_eq!(
            entries[2].preferred_source,
            Utf8PathBuf::from("/context/demo")
        );
        assert!(entries[2].matches_source(&Utf8PathBuf::from("/context/demo")));
        assert!(!entries[2].matches_source(&Utf8PathBuf::from("/home/nos/context/demo")));
        assert!(!entries[2].matches_source(&Utf8PathBuf::from("/home/context/demo")));
    }

    #[test]
    fn parse_mount_table_rejects_unparseable_rows() {
        let contents = concat!(
            "36 35 0:31 /source\\040dir /target\\040dir rw - ext4 /dev/root rw\n",
            "36 35 0:31 /bad\\377 /ignored rw - ext4 /dev/root rw\n",
            "not a valid mountinfo row"
        );

        let error = parse_mount_table(contents).expect_err("malformed mount table should fail");
        assert!(error.to_string().contains(
            "invalid mount table entry at line 2: decoded root field is not valid UTF-8"
        ));
    }

    #[test]
    fn parse_mount_privileges_reports_cap_sys_admin_from_cap_eff() {
        let status = parse_mount_privileges("Name:\tagentbox\nCapEff:\t0000000000200000\n");
        assert_eq!(status, MountPrivilegeStatus::HasCapSysAdmin);
    }

    #[test]
    fn parse_mount_privileges_distinguishes_missing_capability() {
        let status = parse_mount_privileges("Name:\tagentbox\nCapEff:\t0000000000000000\n");
        assert_eq!(status, MountPrivilegeStatus::MissingCapSysAdmin);
    }

    #[test]
    fn parse_mount_privileges_reports_unknown_for_bad_status_data() {
        let status = parse_mount_privileges("Name:\tagentbox\nCapEff:\tnot-hex\n");
        assert!(matches!(status, MountPrivilegeStatus::Unknown { .. }));
    }

    #[test]
    fn list_mounts_reads_mount_table() {
        let mounts = list_mounts().expect("mount table should be readable on linux");
        assert!(!mounts.is_empty());
    }

    #[test]
    fn find_target_mount_and_expected_bind_helpers_match_exact_entries() {
        let mounts = vec![MountEntry {
            mount_id: 42,
            preferred_source: Utf8PathBuf::from("/workspace/source"),
            source_aliases: vec![Utf8PathBuf::from("/workspace/source")],
            mount_point: Utf8PathBuf::from("/workspace/target"),
            filesystem_type: "bind".to_string(),
        }];

        assert!(find_target_mount(&mounts, &Utf8PathBuf::from("/workspace/target")).is_some());
        assert!(is_expected_bind_mount_in(
            &mounts,
            &Utf8PathBuf::from("/workspace/source"),
            &Utf8PathBuf::from("/workspace/target")
        ));
        assert!(!is_expected_bind_mount_in(
            &mounts,
            &Utf8PathBuf::from("/mnt/root/workspace/source"),
            &Utf8PathBuf::from("/workspace/target")
        ));
        assert!(!is_expected_bind_mount_in(
            &mounts,
            &Utf8PathBuf::from("/workspace/other"),
            &Utf8PathBuf::from("/workspace/target")
        ));
    }

    #[test]
    fn bind_mount_errors_are_translated() {
        let source = Utf8PathBuf::from("/definitely/missing/source");
        let target = Utf8PathBuf::from("/definitely/missing/target");

        let error = bind_mount(&source, &target).expect_err("missing paths should fail");
        assert!(matches!(
            error,
            crate::shared::error::Error::InfraMount(crate::shared::error::InfraMountError::BindMountFailed {
                mount_source,
                target: bind_target,
                ..
            }) if mount_source == source && bind_target == target
        ));
    }

    #[test]
    fn unmount_errors_are_translated() {
        let target = Utf8PathBuf::from("/definitely/not/mounted");

        let error = unmount(&target).expect_err("unmount should fail for unknown target");
        assert!(matches!(
            error,
            crate::shared::error::Error::InfraMount(crate::shared::error::InfraMountError::UnmountFailed {
                target: unmount_target,
                ..
            }) if unmount_target == target
        ));
    }

    #[test]
    fn mount_privilege_status_matches_linux_probe_contract() {
        assert!(!matches!(
            mount_privilege_status(),
            MountPrivilegeStatus::Unknown { details } if details.is_empty()
        ));
    }

    #[cfg(feature = "privileged-tests")]
    #[test]
    fn is_expected_bind_mount_returns_false_for_missing_target() {
        let source = Utf8PathBuf::from("/definitely/missing/source");
        let target = Utf8PathBuf::from("/definitely/missing/target");

        assert!(!is_expected_bind_mount(&source, &target).expect("inspect mount table"));
    }

    #[cfg(feature = "privileged-tests")]
    #[test]
    fn privileged_bind_mount_round_trip() {
        use super::*;
        if !matches!(
            mount_privilege_status(),
            MountPrivilegeStatus::HasCapSysAdmin
        ) {
            eprintln!("skipping: CAP_SYS_ADMIN is not available");
            return;
        }

        let unique = format!(
            "agentbox-mount-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source = root.join("source");
        let target = root.join("target");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::create_dir_all(&target).expect("create target");

        let source_utf8 = Utf8Path::from_path(&source).expect("utf8");
        let target_utf8 = Utf8Path::from_path(&target).expect("utf8");
        bind_mount(source_utf8, target_utf8).expect("bind mount");
        assert!(
            is_expected_bind_mount(source_utf8, target_utf8).expect("inspect"),
            "expected bind mount to be active"
        );
        unmount(target_utf8).expect("unmount");
        assert!(
            target_mount(target_utf8).expect("inspect").is_none(),
            "expected target to be unmounted"
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
