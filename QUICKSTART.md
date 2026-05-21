# Quickstart

This quickstart shows a short workspace setup flow using a locally built `agentbox` binary.

Target workspace for this example:

```bash
/path/to/my-workspace
```

## Use The Dev Binary Directly

Build the CLI once from the `agentbox` repo:

```bash
cd /path/to/agentbox
cargo build
target/debug/agentbox version
```

That produces the executable at:

```bash
/path/to/agentbox/target/debug/agentbox
```

From inside `/path/to/my-workspace`, run:

```bash
/path/to/agentbox/target/debug/agentbox init
/path/to/agentbox/target/debug/agentbox doctor
/path/to/agentbox/target/debug/agentbox status
```

From outside `/path/to/my-workspace`, pass the target workspace explicitly:

```bash
/path/to/agentbox/target/debug/agentbox --workspace /path/to/my-workspace status
```

## Optional Shell Helper

If you want a short command while developing from a checkout, add this shell function:

```bash
agentbox_dev() {
  /path/to/agentbox/target/debug/agentbox "$@"
}
```

Then use:

```bash
agentbox_dev init
agentbox_dev doctor
agentbox_dev status
```

## Optional Installed CLI

If you want `agentbox` on your `PATH`, install it from the local checkout:

```bash
cargo install --path /path/to/agentbox --force
```

Then run:

```bash
agentbox init
agentbox doctor
agentbox status
```

## Recommended First Run

From inside `/path/to/my-workspace`:

```bash
git init
agentbox_dev init
agentbox_dev doctor
agentbox_dev status
```

Expected behavior:

- `git init` creates the outer workspace repo.
- `init` creates `agentbox.toml`, `context/`, `repos/`, `templates/`, and updates `.gitignore` to ignore `repos/` and `.agentbox/`.
- `doctor` may print `WARN: CAP_SYS_ADMIN is not available; bind-mount commands may fail without additional privileges`.
- That warning is normal on many systems. It does not block normal setup, and it usually just means you will need `sudo` for mount-related commands such as `mount` and `unmount`.
- To avoid mixed ownership, keep setup and file-writing commands as your normal user. If you need `sudo`, use it only for the actual `mount` or `unmount` step.
- If a same-source mount was recreated outside agentbox, `unmount` or `detach --force` may report it as unverified. Inspect the live mount first, then use `--unsafe-unmount` only when you intentionally want agentbox to tear it down.
- `status` should print `No repos registered.`

Useful inspection commands:

```bash
ls -la /path/to/my-workspace
sed -n '1,80p' /path/to/my-workspace/agentbox.toml
sed -n '1,80p' /path/to/my-workspace/.gitignore
```

After registering mounts or deleting templates, `doctor --fix` can repair the workspace skeleton and remove stale internal bookkeeping such as missing-template bindings or stale mount ownership receipts. It does not invent repo registrations, mount rules, clone sources, or imports.
