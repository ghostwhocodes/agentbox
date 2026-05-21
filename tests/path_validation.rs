use agentbox::shared::types::{CloneSource, RelativePath};

#[test]
fn accepts_allowed_clone_sources() {
    for source in [
        "https://github.com/org/repo.git",
        "ssh://git@example.com/org/repo.git",
        "file:///tmp/example",
        "git@github.com:org/repo.git",
    ] {
        assert!(
            source.parse::<CloneSource>().is_ok(),
            "{source} should be accepted"
        );
    }
}

#[test]
fn rejects_disallowed_clone_sources() {
    for source in ["/tmp/repo", "../repo", "~/repo"] {
        assert!(
            source.parse::<CloneSource>().is_err(),
            "{source} should be rejected"
        );
    }
}

#[test]
fn rejects_scp_sources_with_control_whitespace_in_host() {
    for source in [
        "git@host\n:repo.git",
        "git@host\r:repo.git",
        "git@host\x0c:repo.git",
        "git@host\x0b:repo.git",
    ] {
        assert!(
            source.parse::<CloneSource>().is_err(),
            "{source:?} should be rejected"
        );
    }
}

#[test]
fn rejects_path_escapes() {
    assert!(RelativePath::new("../escape", "mount path").is_err());
    assert!(RelativePath::new("/abs/path", "mount path").is_err());
}
