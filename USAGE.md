# Usage

This document shows typical `agentbox` workflows.

Every command targets one workspace root:

- if you pass `--workspace <path>`, that exact directory is used
- otherwise `agentbox` searches upward for the nearest parent workspace and only falls back to the current directory when none is found

The `agentbox` source checkout does not need to be the workspace. In normal use, `agentbox` is a standalone CLI on your `PATH` and the `context/` and `repos/` directories live in the target workspace repo, not in this source tree.

When developing `agentbox` itself from this source checkout, target an external workspace explicitly:

```bash
cargo run -- --workspace /path/to/my-workspace status
```

## Concepts

- A `workspace` is the outer repo where `agentbox.toml`, `context/`, and `templates/` live.
- A managed `repo` is a leaf git repo registered by clone source.
- `context/<repo-id>/` is workspace-owned state.
- `repos/<repo-id>/` is the materialized clone.
- A `mount` maps a workspace-owned path into a path inside the materialized repo.

## 1. Initialize A Workspace

Create the outer repo that will own prompts, plans, and tool state. You can either `cd` into it first or target it explicitly:

```bash
git init my-workspace
cd my-workspace
agentbox init
```

Equivalent explicit targeting:

```bash
git init my-workspace
agentbox --workspace my-workspace init
```

Result:

- `agentbox.toml` is created
- `context/` is created
- `repos/` is created
- `templates/` is created
- `.gitignore` is updated to ignore `repos/` and `.agentbox/`

An empty workspace manifest starts as:

```toml
version = 1

[repos]
```

As repos, template bindings, and mounts are added, the manifest grows into the documented `version = 1` shape:

```toml
version = 1

[repos.demo]
source = "git@github.com:org/demo.git"

[repo_templates.demo]
template = "default"

[[repo_mounts]]
repo_id = "demo"
context = "ai"
repo = "ai"
```

Check the workspace:

```bash
agentbox doctor
```

Or, from anywhere:

```bash
agentbox --workspace my-workspace doctor
```

On many systems, `doctor` will print:

```text
WARN: CAP_SYS_ADMIN is not available; bind-mount commands may fail without additional privileges
```

That warning is normal. It does not block workspace setup or other read-only commands. It usually means you can continue with `init`, `attach`, `materialize`, `status`, and `show` as your normal user, but you may need `sudo` later for mount-related commands such as `mount` and `unmount`.

## 2. Attach A Repo Without Cloning Yet

Register a repo by clone source:

```bash
agentbox attach demo --source git@github.com:org/demo.git
```

Equivalent explicit targeting:

```bash
agentbox --workspace my-workspace attach demo --source git@github.com:org/demo.git
```

At this point:

- the repo is registered in `agentbox.toml`
- `context/demo/` is created
- the repo is not yet cloned into `repos/demo/`

If `context/demo/` already exists from an earlier detach and you attach without a template, `agentbox` warns that the preserved context was reused but no mount mappings were restored. That warning is intentional: re-add mounts explicitly or reapply a template if you want `agentbox` to manage paths from that preserved context again.

Inspect the state:

```bash
agentbox status
agentbox show demo
```

From outside the workspace:

```bash
agentbox --workspace my-workspace status
agentbox --workspace my-workspace show demo
```

Typical `status` output before materialization looks like:

```text
demo	source=git@github.com:org/demo.git	materialized=false	mounted=no-mounts
```

## 3. Materialize The Repo

Clone the registered repo into the workspace:

```bash
agentbox materialize demo
```

If you are driving a different workspace from elsewhere:

```bash
agentbox --workspace my-workspace materialize demo
```

This creates:

```text
repos/demo/
```

You can materialize every registered repo at once:

```bash
agentbox materialize
```

If a repo is already cloned, `agentbox` skips it with a clear message.

## 4. Add A Simple `ai/` Mount

Register a writable mount from workspace-owned context into the repo:

```bash
agentbox add-mount demo --context-path ai --repo-path ai
```

Equivalent explicit targeting:

```bash
agentbox --workspace my-workspace add-mount demo --context-path ai --repo-path ai
```

This records a mount rule equivalent to:

```toml
[[repo_mounts]]
repo_id = "demo"
context = "ai"
repo = "ai"
```

Create context content on the workspace side:

```bash
mkdir -p context/demo/ai
printf '# prompt\n' > context/demo/ai/prompt.md
```

Mount it into the repo:

```bash
agentbox mount demo
```

Or:

```bash
agentbox --workspace my-workspace mount demo
```

The workspace-owned file is available at `repos/demo/ai/prompt.md`.

Unmount when needed:

```bash
agentbox unmount demo
```

`unmount` is idempotent, so repeating it is safe.

Mount and unmount require Linux bind-mount privileges (typically `CAP_SYS_ADMIN` or root). If the system mount table (`/proc/self/mountinfo`) is unreadable, mount commands will return an error rather than proceeding blindly.

agentbox records advisory mount ownership receipts in `.agentbox/state/mount-ownership.toml` after successful `mount` and `import-mount` operations. The receipt records the live mount ID, source root, and filesystem type. If that receipt is missing, corrupt, or no longer matches the live mount, `unmount` treats a same-source mount as unverified and refuses to tear it down by default:

```bash
agentbox unmount demo --unsafe-unmount
```

Use `--unsafe-unmount` only after checking that the live mount at the repo path is the one you intend to remove. Unverified same-source mounts can happen after a manual remount, reboot/container restart, or another tool recreating the bind mount.

If you saw the earlier `doctor` warning about missing `CAP_SYS_ADMIN`, this is the point where it matters in practice. A common pattern is to run most `agentbox` commands as your normal user and use `sudo` only for the mount step:

```bash
sudo agentbox mount demo
```

To avoid mixed ownership in the workspace, do not use `sudo` for commands that write manifests, clone repos, move files, or create workspace content. Keep `init`, `attach`, `materialize`, `add-mount`, `import-mount`, and similar commands unprivileged, and reserve `sudo` for `mount` and `unmount`.

One detail to watch: `mount` may create missing source or target directories before performing the bind mount. If you run `sudo agentbox mount ...` and those directories do not already exist, they may be created as `root`. A safer pattern is to create them first as your normal user:

```bash
mkdir -p context/demo/ai
mkdir -p repos/demo/ai
sudo agentbox mount demo
```

If you accidentally create root-owned files or directories in the workspace, unmount first and then repair ownership with `chown -R`:

```bash
sudo agentbox unmount demo
sudo chown -R "$USER":"$USER" /path/to/my-workspace
```

Use `chown -R` carefully and only on workspace paths you intend to own as your normal user.

Change or remove a registered mount by repo path:

```bash
agentbox list-mounts demo
agentbox edit-mount demo --repo-path ai --new-context-path prompts/ai
agentbox remove-mount demo --repo-path ai
```

List registered mounts across the whole workspace when you omit the repo id:

```bash
agentbox list-mounts
```

`list-mounts` is read-only and reports the registered mount mappings, including runtime state when mount inspection is available. `edit-mount` and `remove-mount` update the manifest only. If the target path is currently mounted, unmount it first.

## 5. Import An Existing Repo-Created Directory

This is the workflow for adopting a tool-created directory after the repo has already been used.

Assume the materialized repo contains:

```text
repos/demo/.loki/
```

Import it into workspace ownership and mount it back:

```bash
agentbox import-mount demo --repo-path .loki
```

Or:

```bash
agentbox --workspace my-workspace import-mount demo --repo-path .loki
```

Result:

- `repos/demo/.loki/` is moved to `context/demo/.loki/`
- a mount rule is registered
- the directory is mounted back into `repos/demo/.loki/`

If you want to register and move it without remounting immediately:

```bash
agentbox import-mount demo --repo-path .loki --no-mount
```

Choose a different workspace-side path if needed:

```bash
agentbox import-mount demo --repo-path .tool/state --context-path tool-state
```

## 6. Use Templates

Create a local template skeleton:

```bash
agentbox template create default
```

That creates:

```text
templates/default/
  template.toml
  context/
```

Example template manifest:

```toml
version = 1

[[mounts]]
context = "ai"
repo = "ai"

[[mounts]]
context = ".loki"
repo = ".loki"
```

Seed template files:

```bash
mkdir -p templates/default/context/ai
printf '# seed prompt\n' > templates/default/context/ai/prompt.md
```

Apply the template to an existing repo:

```bash
agentbox template apply default demo
```

Or attach a repo and apply a template immediately:

```bash
agentbox attach demo --source git@github.com:org/demo.git --template default
```

Notes:

- template application only copies missing files
- it merges template mounts into the repo's persisted `repo_mounts`
- it persists the applied template binding in `repo_templates`
- it does not delete repo-specific context
- it does not materialize the repo automatically
- if the repo is already materialized and a new template mount would hide different existing repo-path contents, template application fails and points you to `import-mount`
- `status` and `show` keep the binding internal; they do not print template origin

List and delete templates:

```bash
agentbox template list
agentbox template delete default
```

## 7. Inspect State

High-level summary:

```bash
agentbox status
```

Example:

```text
demo	source=file:///tmp/demo-repo	materialized=true	mounted=mounted
notes	source=git@github.com:org/notes.git	materialized=false	mounted=no-mounts
```

Detailed repo view:

```bash
agentbox show demo
```

Example fields:

```text
repo_id: demo
source: file:///tmp/demo-repo
context_root: /path/to/workspace/context/demo
repo_root: /path/to/workspace/repos/demo
materialized: true
mount_state: mounted
mounts:
/path/to/workspace/context/demo/ai -> /path/to/workspace/repos/demo/ai (active=true, conflict=false)
```

Machine-readable output is available for the main inspection commands:

```bash
agentbox status --json
agentbox show demo --json
agentbox list-mounts --json
agentbox doctor --json
```

## 8. Run Doctor Checks

Validate workspace structure and state:

```bash
agentbox doctor
```

Use auto-fix only for structural problems:

```bash
agentbox doctor --fix
```

`doctor --fix` may:

- create missing `context/`, `repos/`, `templates/`
- create missing per-repo context directories
- add a missing `repos/` entry to `.gitignore`
- add a missing `.agentbox/` entry to `.gitignore`
- remove stale `repo_templates` bindings that reference deleted templates
- remove stale mount ownership receipts when no live mount exists at the recorded target

It does not:

- rewrite clone sources
- invent mount rules
- auto-import repo directories
- silently detach repos
- remove copied template files from `context/<repo-id>/`
- remove registered mount rules that were originally added by a template

One detail to keep in mind: `doctor` treats a registered mount whose workspace source path is missing as an error. After `add-mount`, create the source directory under `context/<repo-id>/...` before expecting a clean doctor report.

If the mount table cannot be read (e.g. in a container without `/proc/self/mountinfo`), `doctor` reports a warning and skips mount-specific checks. Similarly, `status` and `show` report mount state as "unknown" instead of failing.

`doctor --fix` only removes stale runtime bookkeeping when it can prove the related object is gone. For example, it can clear a template binding after `template delete`, or remove an ownership receipt when no live mount exists at the recorded target. It will not unmount anything or mutate repo working trees.

## 9. Dematerialize Or Detach

Remove the local clone but keep registration and context:

```bash
agentbox dematerialize demo
```

Detach the repo completely from the manifest:

```bash
agentbox detach demo
```

By default, detach requires a clean state:

- the repo must be unmounted
- the repo must be dematerialized

If you want `agentbox` to unmount and dematerialize first:

```bash
agentbox detach demo --force
```

`detach --force` follows the same ownership check as `unmount`. If an active same-source mount is unverified, inspect it first and then pass the explicit override when teardown is intentional:

```bash
agentbox detach demo --force --unsafe-unmount
```

The workspace-owned context under `context/demo/` remains after detach for manual cleanup or archival.

Detach removes the repo registration and its registered mount mappings from `agentbox.toml`. If you reattach the same repo id, `agentbox` will reuse the existing `context/demo/` directory on disk, but it will not automatically restore mount rules. Re-add them explicitly or reapply a template.

Example reattach flow after a detach:

```bash
agentbox attach demo --source git@github.com:org/demo.git
agentbox add-mount demo --context-path ai --repo-path ai
agentbox materialize demo
sudo agentbox mount demo
```

## 10. Multi-Repo Workflow

You can register several repos and only materialize the ones you need.

Example:

```bash
agentbox attach api --source git@github.com:org/api.git
agentbox attach web --source git@github.com:org/web.git
agentbox attach docs --source git@github.com:org/docs.git
```

Only clone the repo you need:

```bash
agentbox materialize web
agentbox add-mount web --context-path ai --repo-path ai
agentbox mount web
```

Clean up the local clone without losing workspace-owned state:

```bash
agentbox unmount web
agentbox dematerialize web
```

## 11. Allowed Clone Sources

Clone source validation accepts URL schemes and SCP-style SSH syntax without depending on a regex engine.

Examples that are accepted:

```text
https://github.com/org/repo.git
ssh://git@github.com/org/repo.git
git@github.com:org/repo.git
file:///path/to/repo
```

Examples that are rejected:

```text
../repo
~/src/repo
/path/to/repo
```

## 12. Common Patterns

### AI State Outside Repo History

```bash
agentbox attach demo --source git@github.com:org/demo.git
agentbox materialize demo
agentbox add-mount demo --context-path ai --repo-path ai
mkdir -p context/demo/ai
printf 'plan\n' > context/demo/ai/plan.md
agentbox mount demo
```

### Adopt Existing Tool State

```bash
agentbox import-mount demo --repo-path .loki
```

### Repair A Broken Workspace Skeleton

```bash
agentbox doctor --fix
```

### Keep Two Workspaces For The Same Upstream Repo

Two different workspaces can attach the same clone source independently:

```bash
# workspace A
agentbox attach demo --source git@github.com:org/demo.git
agentbox materialize demo

# workspace B
agentbox attach demo --source git@github.com:org/demo.git
agentbox materialize demo
```

Each workspace gets its own clone under its own `repos/demo/`.

## Reference

Get top-level help:

```bash
agentbox --help
```

Get command-specific help:

```bash
agentbox attach --help
agentbox import-mount --help
agentbox template --help
```

For a deeper explanation of template behavior, see [docs/guides/template_guide.md](docs/guides/template_guide.md).

Project quality gates can be run with:

```bash
just ci
```
