mod common;

use common::{
    TempDir, assert_success, fixture_repo, init_workspace, path, read, run_agentbox, stdout, write,
};
use std::process::Command;

// Direct bind-mount API tests have been moved inline to `src/mounts/infra.rs`.
// This file retains only CLI-facing privileged integration tests.

#[cfg(target_os = "linux")]
fn has_cap_sys_admin() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        eprintln!("skipping privileged bind mount test because /proc/self/status is unreadable");
        return false;
    };
    let Some(cap_eff) = status
        .lines()
        .find(|line| line.starts_with("CapEff:"))
        .and_then(|line| line.split_whitespace().nth(1))
    else {
        eprintln!("skipping privileged bind mount test because CapEff is unavailable");
        return false;
    };
    let Ok(cap_eff) = u64::from_str_radix(cap_eff, 16) else {
        eprintln!("skipping privileged bind mount test because CapEff is malformed");
        return false;
    };
    cap_eff & (1 << 21) != 0
}

#[cfg(target_os = "linux")]
#[test]
fn privileged_cli_mount_unmount_and_force_detach_round_trip() {
    if !has_cap_sys_admin() {
        eprintln!("skipping privileged bind mount test because CAP_SYS_ADMIN is unavailable");
        return;
    }

    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            "ai",
        ],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    write(path(&workspace, "context/demo/ai/prompt.md"), "seed");

    let output = run_agentbox(workspace.path(), &["mount", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Mounted `"));
    assert_eq!(read(path(&workspace, "repos/demo/ai/prompt.md")), "seed");

    let output = run_agentbox(workspace.path(), &["unmount", "demo"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Unmounted `"));
    assert!(!path(&workspace, "repos/demo/ai/prompt.md").exists());

    let output = run_agentbox(workspace.path(), &["mount", "demo"]);
    assert_success(&output);
    assert_eq!(read(path(&workspace, "repos/demo/ai/prompt.md")), "seed");

    let output = run_agentbox(workspace.path(), &["detach", "demo", "--force"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Detached repo `demo`"));
    assert!(!path(&workspace, "repos/demo").exists());
    assert!(path(&workspace, "context/demo/ai/prompt.md").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn privileged_import_mount_remounts_by_default() {
    if !has_cap_sys_admin() {
        eprintln!("skipping privileged bind mount test because CAP_SYS_ADMIN is unavailable");
        return;
    }

    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    write(path(&workspace, "repos/demo/.loki/state.txt"), "state");

    let output = run_agentbox(
        workspace.path(),
        &["import-mount", "demo", "--repo-path", ".loki"],
    );
    assert_success(&output);
    assert!(stdout(&output).contains("Imported `.loki`"));
    assert_eq!(
        read(path(&workspace, "context/demo/.loki/state.txt")),
        "state"
    );
    assert_eq!(
        read(path(&workspace, "repos/demo/.loki/state.txt")),
        "state"
    );

    let output = run_agentbox(workspace.path(), &["detach", "demo", "--force"]);
    assert_success(&output);
}

#[cfg(target_os = "linux")]
#[test]
fn privileged_status_show_and_doctor_report_mounted_state() {
    if !has_cap_sys_admin() {
        eprintln!("skipping privileged bind mount test because CAP_SYS_ADMIN is unavailable");
        return;
    }

    let workspace = TempDir::new();
    init_workspace(workspace.path());
    let (_fixture, source) = fixture_repo();

    assert_success(&run_agentbox(
        workspace.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_agentbox(
        workspace.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            "ai",
        ],
    ));
    assert_success(&run_agentbox(workspace.path(), &["materialize", "demo"]));

    write(path(&workspace, "context/demo/ai/prompt.md"), "seed");
    assert_success(&run_agentbox(workspace.path(), &["mount", "demo"]));

    let output = run_agentbox(workspace.path(), &["status"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("demo\tsource=file://"));
    assert!(output_text.contains("mounted=mounted"));

    let output = run_agentbox(workspace.path(), &["show", "demo"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("mount_state: mounted"));
    assert!(output_text.contains("(active=true, conflict=false)"));

    let output = run_agentbox(workspace.path(), &["doctor"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("workspace looks healthy"));

    let output = run_agentbox(workspace.path(), &["detach", "demo", "--force"]);
    assert_success(&output);
}

#[cfg(target_os = "linux")]
#[test]
fn privileged_relative_workspace_flag_preserves_mounted_state_detection() {
    if !has_cap_sys_admin() {
        eprintln!("skipping privileged bind mount test because CAP_SYS_ADMIN is unavailable");
        return;
    }

    fn run_relative_workspace(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_agentbox"))
            .args(["--workspace", "./nested/../my-workspace"])
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run agentbox")
    }

    let cwd = TempDir::new();
    let workspace_root = cwd.path().join("my-workspace");
    let (_fixture, source) = fixture_repo();

    assert_success(&run_relative_workspace(cwd.path(), &["init"]));
    assert_success(&run_relative_workspace(
        cwd.path(),
        &["attach", "demo", "--source", &source],
    ));
    assert_success(&run_relative_workspace(
        cwd.path(),
        &[
            "add-mount",
            "demo",
            "--context-path",
            "ai",
            "--repo-path",
            "ai",
        ],
    ));
    assert_success(&run_relative_workspace(
        cwd.path(),
        &["materialize", "demo"],
    ));

    write(workspace_root.join("context/demo/ai/prompt.md"), "seed");
    assert_success(&run_relative_workspace(cwd.path(), &["mount", "demo"]));

    let output = run_relative_workspace(cwd.path(), &["status"]);
    assert_success(&output);
    assert!(stdout(&output).contains("mounted=mounted"));

    let output = run_relative_workspace(cwd.path(), &["show", "demo"]);
    assert_success(&output);
    let output_text = stdout(&output);
    assert!(output_text.contains("mount_state: mounted"));
    assert!(output_text.contains("(active=true, conflict=false)"));

    let output = run_relative_workspace(cwd.path(), &["detach", "demo", "--force"]);
    assert_success(&output);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn privileged_bind_mount_round_trip() {}

#[cfg(not(target_os = "linux"))]
#[test]
fn privileged_cli_mount_unmount_and_force_detach_round_trip() {}

#[cfg(not(target_os = "linux"))]
#[test]
fn privileged_import_mount_remounts_by_default() {}

#[cfg(not(target_os = "linux"))]
#[test]
fn privileged_status_show_and_doctor_report_mounted_state() {}

#[cfg(not(target_os = "linux"))]
#[test]
fn privileged_relative_workspace_flag_preserves_mounted_state_detection() {}
