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

### Skill Installation

Knot skills are developed at the project level (`.agents/skills/`) and
deployed to the personal skills repository (`~/.agents/`) for use by
other projects. The production layout has two locations:

- `~/.agents/skills/` — **master (router) skills only**. These are the
  only skills pi auto-discovers into the system prompt.
- `~/.agents/skills-library/` — **sub-skills**. Not auto-discovered;
  read on demand via the master's routing table or explicitly with
  `pi --skill <path>`.

After updating a skill at project level, deploy it:

```bash
# Sub-skills -> production library
for skill in knot-init knot-create knot-dispatch knot-inspect
              knot-manage knot-design knot-analyst knot-update
              knot-abstractions; do
  mkdir -p ~/.agents/skills-library/$skill
  cp -r .agents/skills/$skill/. ~/.agents/skills-library/$skill/
done
# Copy non-SKILL.md files (e.g. glossary)
if [ -f .agents/skills/knot-init/knot-glossary.md ]; then
  cp .agents/skills/knot-init/knot-glossary.md \
     ~/.agents/skills-library/knot-init/knot-glossary.md
fi
# Master router -> auto-discovered skills dir
mkdir -p ~/.agents/skills/knot
cp -r .agents/skills/knot/. ~/.agents/skills/knot/
```

**Always verify after copying** — `cp` can silently fail:

```bash
for skill in knot-init knot-create knot-dispatch knot-inspect
              knot-manage knot-design knot-analyst knot-update
              knot-abstractions; do
  diff .agents/skills/$skill/SKILL.md \
       ~/.agents/skills-library/$skill/SKILL.md > /dev/null 2>&1 && \
    echo "$skill: OK" || echo "$skill: FAILED"
done
diff .agents/skills/knot/SKILL.md \
     ~/.agents/skills/knot/SKILL.md > /dev/null 2>&1 && \
  echo "knot: OK" || echo "knot: FAILED"
```

The project-level master (`.agents/skills/knot/`) is maintained as a
byte-identical copy of the production master, so deploying is a plain
copy plus diff verification.

The `knot-init` skill also performs this installation automatically
(step 4a) when initialising a rig.

## Agent Skills

This project maintains agent skills in `.agents/skills/`. Pi discovers these as project-local skills, which override any same-named global skills in `~/.agents/skills/`.

### Production skill locations (`~/.agents/`)

Deployed skills live in the personal skills repository at `~/.agents/`:

- `~/.agents/skills/<name>/` — master (router) skills; the **only**
  skills auto-discovered by pi into the system prompt.
- `~/.agents/skills-library/<name>/` — sub-skills; **not**
  auto-discovered. They are read on demand via the master's routing
  table or explicitly with `pi --skill <path>`.

To deploy a change, copy the project-level skill into its matching
production location (see Skill Installation above).

### Knot Skills

- **knot** — Master router for all Knot work; routes to the sub-skills
  below (the only Knot skill auto-discovered in other projects)
- **knot-abstractions** — Understand the layered architecture (rig, profiles, skills, application)
- **knot-analyst** — Analyse rig productivity and project progress at runtime
- **knot-design** — Design looms and knots (idempotency, naming, loops, responsibility)
- **knot-dispatch** — Trigger knots into action (strand files, event dispatch)
- **knot-init** — Initialise a Knot rig in a directory
- **knot-inspect** — Inspect rig state (looms, knots, profiles, activity)
- **knot-manage** — Review rig work via git and tie-offs, assess interaction quality
- **knot-create** — Create, modify, delete looms, knots, and agent profiles

### Workflow

Skills are developed and tested at the project level (`.agents/skills/`) before being deployed to their production locations in `~/.agents/` for use by other projects. To publish a sub-skill:

```bash
mkdir -p ~/.agents/skills-library/<skill-name>
cp -r .agents/skills/<skill-name>/. ~/.agents/skills-library/<skill-name>/
```

To publish the master router:

```bash
mkdir -p ~/.agents/skills/knot
cp -r .agents/skills/knot/. ~/.agents/skills/knot/
```

## Knot Glossary

Knot domain terms are defined in the Knot glossary at
[.agents/skills/knot-init/knot-glossary.md](.agents/skills/knot-init/knot-glossary.md).
