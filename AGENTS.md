# Knot — Agent Developer Notes

## What is Knot?

Knot is a **Rust** application that runs as a **local service** on a developer's machine. It orchestrates AI agent workflows, manages file-based configurations, and exposes an **HTTP control and observability interface** for interaction and monitoring.

## Architecture

- **Local-first** — Designed to run on a single developer workstation, not as a distributed cloud service.
- **File system access** — Knot reads and writes project files directly to manage agent profiles, prompt templates, and workflow state.
- **HTTP interface** — Provides RESTful endpoints for controlling agents, submitting workflows, and observing runtime state.

## Building

```bash
cargo build
```

## Running

```bash
cargo run
```

This starts the Knot HTTP service on `localhost:3000` (or the configured port).

## Installing

After `project-plan-completion` bumps the binary version, reinstall the updated binary:

```bash
cargo install --path .
```

## Agent Skills

This project maintains agent skills in `.agents/skills/`. Pi discovers these as project-local skills, which override any same-named global skills in `~/.agents/skills/`.

### Knot Skills

- **knot-analyst** — Analyse rig productivity and project progress at runtime
- **knot-design** — Design looms and knots (idempotency, naming, loops, responsibility)
- **knot-dispatch** — Trigger knots into action (strand files, event dispatch)
- **knot-init** — Initialise a Knot rig in a directory
- **knot-inspect** — Inspect rig state (looms, knots, profiles, activity)
- **knot-manage** — Review rig work via git and tie-offs, assess interaction quality
- **knot-create** — Create, modify, delete looms, knots, and agent profiles

### Workflow

Skills are developed and tested at the project level (`.agents/skills/`) before being installed globally for use by other projects. To publish a skill globally:

```bash
cp -r .agents/skills/<skill-name> ~/.agents/skills/<skill-name>
```

## Knot Glossary

Knot domain terms are defined in the Knot glossary at
[.agents/skills/knot-init/knot-glossary.md](.agents/skills/knot-init/knot-glossary.md).
