# Documentation

This directory contains longer-form guides for `agentbox`.

Start with the root-level docs when you need the operational view:

- [README.md](../README.md) gives the project overview, workspace layout, command summary, safety model, and development gates.
- [QUICKSTART.md](../QUICKSTART.md) shows a short local-development setup flow.
- [USAGE.md](../USAGE.md) walks through the main workspace, repo, mount, template, inspection, and teardown workflows.
- [AGENTS.md](../AGENTS.md) documents repository conventions for contributors and coding agents.

Guide docs:

- [Template Guide](guides/template_guide.md) explains template application, conflict handling, delete behavior, and reattach behavior in more detail.

When changing CLI behavior, update the root-level command examples and any affected guide in the same change. `cargo run -- --help` and command-specific `--help` output are the source of truth for the public command surface.
