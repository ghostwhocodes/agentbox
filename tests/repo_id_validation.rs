use agentbox::shared::types::RepoId;

#[test]
fn accepts_valid_repo_ids() {
    for repo_id in ["repo", "repo_1", "repo-1", "a1_b2-c3"] {
        assert!(
            repo_id.parse::<RepoId>().is_ok(),
            "{repo_id} should be valid"
        );
    }
}

#[test]
fn rejects_invalid_repo_ids() {
    for repo_id in ["-repo", "Repo", "repo/path", "../repo", "repo space", ""] {
        assert!(
            repo_id.parse::<RepoId>().is_err(),
            "{repo_id} should be rejected"
        );
    }
}
