# Template Guide

This guide describes how the `agentbox` template system works.

The short version: templates are a reusable way to seed workspace-owned context and define default mount mappings for repos. They are not repo scaffolding, and they are not a live sync mechanism.

## Mental Model

A template is a workspace-local bundle of:

- a `template.toml` manifest
- a `templates/<template-id>/context/` tree of seed files
- a list of mount mappings to merge into a repo

Templates are best thought of as policy plus seed data for workspace-owned context.

They do not write into a repo checkout directly. Instead, they populate `context/<repo-id>/` and define how that context should be mounted back into the repo later.

## On-Disk Layout

A template lives at:

```text
templates/<template-id>/
  template.toml
  context/
```

The template manifest uses `version = 1` and mainly contains `[[mounts]]` entries.

Example:

```toml
version = 1

[[mounts]]
context = "ai"
repo = "ai"

[[mounts]]
context = ".loki"
repo = ".loki"
```

The `context/` subtree contains seed files that will be copied into repo-specific workspace context such as `context/demo/`.

## What `template create` Does

```bash
agentbox template create default
```

This creates:

```text
templates/default/
  template.toml
  context/
```

It writes a default empty template manifest with `version = 1`.

It does not invent mounts beyond what you put in `template.toml`.

## What `template apply` Does

Applying a template to a registered repo does three things:

1. Validates the template's mount mappings against the repo's existing mounts.
2. Copies missing files from `templates/<template-id>/context/` into `context/<repo-id>/`.
3. Persists:
   `repo_mounts` updates with template-derived mounts.
   `repo_templates` binding to remember which template was last applied.

Example:

```bash
agentbox template apply default demo
```

Important behavior:

- It only copies missing files.
- It does not overwrite existing repo context.
- It merges mounts into the repo's existing `repo_mounts`.
- It skips exact duplicate mounts.
- It fails on conflicting mount rules.
- It does not materialize the repo.
- It does not mount anything automatically.
- It does not delete repo-specific context.

## What `attach --template` Does

You can apply a template during attach:

```bash
agentbox attach demo --source git@github.com:org/demo.git --template default
```

This is effectively:

- attach the repo
- seed `context/<repo-id>/` from the template
- persist template-derived mounts
- persist the template binding

It does not materialize the repo automatically, and it does not mount anything automatically.

## Conflict Rules

Template application is conservative.

It allows:

- exact duplicate mount specs already present on the repo

It rejects:

- a template mount that conflicts with an existing repo mount
- overlapping mount targets that would create ambiguous repo paths
- overlapping mount contexts that violate mount validation rules

If validation fails, the template is not applied.

## File Copy Semantics

Template context copying is designed to be safe for existing repo-owned context.

Behavior:

- only missing files are copied
- existing files are preserved
- unsupported entries such as symlinks cause failure
- failures roll back copied context and manifest changes

This means templates are suitable for seeding defaults, not for forcing updates into existing repo context.

## What Templates Are Good For

Templates work well for:

- standard `ai -> ai` mount mappings
- standard `.loki -> .loki` mount mappings
- seed prompt, plan, or spec files under workspace-owned context
- consistent default structure across several managed repos

## Important Safety Warning

Do not use a template mount to take over a repo path that contains different repo-owned files.

Example risky case:

- the repo checkout already contains a tracked `ai/` tree from Git
- your template defines `ai -> ai`
- your template seeds only a small `context/<repo-id>/ai/` tree

If mounted, that workspace context would hide the repo's existing `ai/` contents.

Guardrails:

- `template apply` fails early on materialized repos when a new template mount would hide different existing repo-path contents
- `mount` fails if the repo path already contains different files than the workspace source tree

If you are adopting an existing repo-owned path such as `ai/` into workspace ownership, use `import-mount` first instead of relying on a template alone:

```bash
agentbox import-mount demo --repo-path ai --no-mount
mkdir -p repos/demo/ai
sudo agentbox mount demo
```

After `import-mount`, a template can recreate the same mount rule for other repos or reattaches.

## What Templates Are Not

Templates are not:

- repo scaffolding
- automatic remount engines
- automatic materialization
- destructive sync tools
- live references that update repos when the template changes later

Applying a template is closer to "copy missing defaults and record provenance" than "link this repo to a live template".

## Template Delete Semantics

Deleting a template:

```bash
agentbox template delete default
```

does this:

- removes the template directory
- clears `repo_templates` bindings for repos that referenced it

It does not do this:

- remove template-derived mount mappings already merged into repos
- remove copied files from `context/<repo-id>/`

That means template application is not a live dependency. Once applied, the repo keeps the copied context and merged mounts unless you remove them separately.

If a template directory is removed outside `agentbox`, the `repo_templates` entry becomes stale. `agentbox doctor` reports that condition, and `agentbox doctor --fix` removes the stale binding. It leaves copied context files and registered mounts alone.

## Status And Visibility

The applied template binding is persisted in `repo_templates`, but inspection commands do not print it.

- `status` does not show template origin
- `show` does not show template origin
- `list-mounts` does not show template origin

The binding is internal bookkeeping.

## Reattach And Preserved Context

This is an important lifecycle detail.

If you:

1. apply a template to a repo
2. later `detach` that repo
3. then reattach the same repo id

then:

- `context/<repo-id>/` is preserved on disk
- repo registration is restored
- mount mappings are not automatically restored unless you reapply a template or add mounts manually

This is why templates are the cleanest restore path after reattach:

```bash
agentbox attach demo --source git@github.com:org/demo.git --template default
```

or:

```bash
agentbox attach demo --source git@github.com:org/demo.git
agentbox template apply default demo
```

If you reattach without a template and preserved context already exists, `agentbox` warns that the context was reused but no mount mappings were restored.

## Recommended Workflow

For a repo that should always have a standard `ai/` mount and seed files:

1. Create a template.
2. Define the mount in `template.toml`.
3. Add any seed files under `templates/<template-id>/context/`.
4. Attach repos with `--template <template-id>` when possible.
5. If reattaching later, reapply the template or re-add mounts manually.

Example:

```bash
agentbox template create default
mkdir -p templates/default/context/ai
printf '# prompt\n' > templates/default/context/ai/prompt.md
cat > templates/default/template.toml <<'EOF'
version = 1

[[mounts]]
context = "ai"
repo = "ai"
EOF

agentbox attach demo --source git@github.com:org/demo.git --template default
agentbox materialize demo
mkdir -p repos/demo/ai
sudo agentbox mount demo
```

## Summary

Templates provide:

- reusable mount definitions
- seed files for workspace-owned context
- persisted provenance in `repo_templates`

They do not provide:

- automatic materialization
- automatic mounting
- automatic restoration after plain reattach
- automatic synchronization when a template changes later
