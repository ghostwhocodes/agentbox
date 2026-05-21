# Repository Guidelines

## Project Structure & Module Organization
`agentbox` is a Rust CLI for managing workspace-local context for independent Git repos. `src/main.rs` is the thin binary entrypoint, `src/lib.rs` exposes `try_main()` and `run_from()`, `src/cli.rs` defines the Clap surface, and `src/commands/` translates parsed CLI args into workflows plus human/JSON output rendering. Bounded contexts live at the top level: `src/workspace/` for workspace discovery/init and `.gitignore` policy, `src/registry/` for repo registration/materialization, `src/mounts/` for mount registration/import/mount/unmount lifecycle, `src/templates/` for template workflows, and `src/inspection/` for status/list/doctor logic. Manifest persistence and transactions live in `src/persistence/`, shared validation/path/error helpers live in `src/shared/`, and output renderers live in `src/output/`. Integration tests live in `tests/`; shared fixtures and CLI helpers are in `tests/common/mod.rs`. Longer user-facing guides live in `docs/`, with root-level `README.md`, `QUICKSTART.md`, and `USAGE.md` kept as the main entry points.

Keep those bounded contexts explicit: workspace administration, repo registry/materialization, mount management, template management, runtime inspection/diagnostics, persistence, and shared kernel utilities. `src/inspection/` and `src/templates/` split application/domain/infra responsibilities internally; `src/mounts/` and `src/registry/` are organized by workflow file. Preserve the ownership boundaries instead of introducing cross-context coupling.

## Build, Test, and Development Commands
- `just ci` is the required completion gate; run it before considering any change complete.
- `cargo build` builds the library and `agentbox` binary.
- `cargo run -- --help` prints the CLI surface; `cargo run -- --workspace /tmp/agentbox-demo init` and `cargo run -- --workspace /tmp/agentbox-demo status` are reliable smoke checks when developing from this source checkout.
- `cargo test` runs the default unit and integration test suite.
- `cargo test --test workflow_integration` exercises the workflow layer directly without going through the CLI.
- `cargo test --features privileged-tests --test privileged_bind_mount` runs the Linux-only bind-mount test target; it still requires bind-mount capability, typically root/CAP_SYS_ADMIN.
- `cargo fmt --check` matches the formatter step used by `just ci`; `cargo fmt` applies formatting locally.
- `cargo clippy --all-targets --all-features` is the expected lint pass before review.
- `cargo llvm-cov --workspace --fail-under-lines 80 --fail-under-regions 80` matches the coverage gate inside `just ci`.
- After initializing `/tmp/agentbox-demo`, `cargo run -- --workspace /tmp/agentbox-demo doctor --json` is a useful smoke check when changing inspection output.

## Coding Style & Naming Conventions
Use standard Rust formatting: 4-space indentation, `snake_case` for functions/modules/tests, and `UpperCamelCase` for types. Keep argument parsing and subcommand definitions in `src/cli.rs`, keep `src/commands/` thin, and put reusable workflow logic in the owning bounded context (`src/workspace/`, `src/registry/`, `src/mounts/`, `src/templates/`, `src/inspection/`). Put manifest persistence and transaction plumbing in `src/persistence/`, and keep shared validation/path/error helpers in `src/shared/`. Prefer `camino::Utf8Path`/`Utf8PathBuf` for workspace paths, reuse the typed error constructors in `src/shared/error.rs`, and preserve the crate's explicit, user-facing error messages and rollback-oriented helpers.

## Testing Guidelines
Add integration tests under `tests/` for user-visible CLI behavior, especially repo lifecycle commands, manifest validation, template application, doctor findings/fixes, path validation, JSON output, and mount state transitions. Follow the existing pattern of one file per feature area with descriptive test names such as `doctor_reports_and_fixes_structural_issues` or `workflow_layer_handles_repo_lifecycle_without_cli`. Reuse `TempDir`, `run_agentbox`, `fixture_repo`, and the assertion helpers from `tests/common/mod.rs`. For refactors in bounded-context workflow code, prefer adding coverage at both the CLI layer and the direct workflow layer when typed results or rollback behavior matter. Keep privileged mount coverage isolated in `tests/privileged_bind_mount.rs`.
Treat `just ci` as the final validation step for all completed changes, even when you also run narrower targeted commands while iterating.

When changing docs, keep root examples aligned with `cargo run -- --help` and update `docs/README.md` or the relevant guide if the workflow-level explanation changes.

## Commit & Pull Request Guidelines
Use concise imperative commit subjects such as `Add template workflow smoke coverage` or `Harden mount ownership validation`. Keep commit titles short, capitalized, and action-oriented. PRs should describe the user-visible workflow or architecture change, list the validation commands you ran, and call out Linux-only or privileged mount behavior when relevant. Include example CLI output when changing status, doctor, or error-reporting UX.
