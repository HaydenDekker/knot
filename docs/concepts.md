# Concepts

Knot is a file-first agent orchestration system. It watches directories for
file changes, and triggers AI agent sessions in response. The entire workflow,
the rig, is stored as plain text on disk, making it easy to review, share,
and version-control.

This page explains Knot's mental model — the hierarchy of objects and
how they relate to each other.

## The Hierarchy

```
Rig (reusable source — its own git repo)
 ├── Profiles (shared agent configurations)
 └── Looms (processing namespaces)
      └── Knots (individual processing tasks)
           ├── reads from a Strand Directory
           └── writes a Tie-off

Runtime tree (project output — committed with project git)
tie-offs/<rig>/
 ├── state.json (live observability, written every 5s)
 ├── .rig-log (operational events)
 ├── events/ (disk-backed event queue)
 └── {loom-id}/ (.loom-log, tie-off files, event dispatch dirs)
```

### Rig

The top-level container. A rig lives at `./rig/` in your project and
aggregates all looms and profiles — **reusable source only**. All
processing output and runtime data lives in the rig's **runtime tree**
at `tie-offs/<rig-basename>/` in the project root (default rig:
`tie-offs/rig/`). The rig is versioned in its own git repository
(committed manually by the user); the runtime tree is committed with
the project's git history.

It is the ship's complete interconnected system — the place where looms
live and knots are defined.

Knot supports **rig switching** — multiple rigs in the same project
(directory names like `myproject-rig/`). Run `knot <rig-name>` to
target a specific rig, or just `knot` to auto-discover.

### Loom

A directory inside the rig whose name ends with `-loom` (e.g.
`rig/planning-loom/`). A loom is a **namespace for a domain of
responsibility** — it groups knots that work on the same kind of output.
For example, a `planning-loom` contains knots that produce or maintain
project plans.

Knot discovers looms automatically — any subdirectory of `rig/` ending
in `-loom` is registered.

### Knot

A `.md` file with YAML frontmatter inside a loom directory. A knot
brings everything together for a single processing task:

1. **Agent Profile** — which agent runs (provider, model, tools, system
   prompt).
2. **Markdown body** — task-specific instructions (the prompt text).
3. **Strand Directory** — which directory to watch for input files.

One loom can contain multiple knot files, each defining a different
processing task.

### Strand

A **text file** in a knot's strand directory. Any text file is accepted
(`.md`, `.rs`, `.json`, `.py`, `.txt`, etc.) — not just Markdown.
Binary files are detected and silently skipped (logged as
`StrandIgnored` in the loom-log).

When a strand is created, modified, or deleted, the knot that watches
that directory is triggered to process it. The strand is the raw input
fed into the knot's agent session.

### Tie-off

The output produced by a knot after processing. Each processing event is
appended to a single `tie-off-{knot-name}.md` file at
`tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md` (in the runtime
tree). The file grows over time, telling the complete story of the
knot's work. Event metadata in each section identifies which strand was
processed.

### Strand Directory

The directory a knot watches for strand events, configured as `strand-dir`
in the knot's YAML frontmatter. It is resolved relative to the project
root (the directory containing `rig/`).

### Rig State

`tie-offs/<rig>/state.json` is written every 5 seconds and contains the
complete live state of the rig: registered looms, their knots with processing
status, agent profiles, and the pending strand queue. This is Knot's
primary observability interface — no HTTP API is used.

The **strand queue** (`strand_queue` array) shows all pending strand
events with file path, loom/knot IDs, event type, and queued timestamp.

## The Processing Flow

```
File change in strand-dir
        │
        ▼
  Knot's file watcher detects event
        │
        ▼
  Event debounced (avoids partial writes)
        │
        ▼
  Knot loads its agent profile from disk
        │
        ▼
  Agent session starts:
    ├── prompt = profile body + knot body + trigger line
    ├── input = strand file(s)
    └── tools = profile.tools
        │
        ▼
  Agent produces output
        │
        ▼
  Output appended to tie-off file
        │
        ▼
  Git commit created (if git-versioned: true)
```

### Session Resume

If an agent invocation fails (timeout, network error), Knot automatically
attempts to resume the session using the session ID, up to 10 retries
with 10-second delays between attempts. The profile's overall timeout
budget is respected — retries stop when insufficient time remains.

## Git Versioning

By default, Knot creates a git commit in the **project** repository
after each successful tie-off write. The commit message includes the
knot ID, event type, and strand filename; tie-off content forms the
commit body. The commit touches the runtime tree (`tie-offs/<rig>/`)
and any project files the knot wrote — never the rig directory
(`rig/` is excluded from project commits).

Per-knot opt-out: set `git-versioned: false` in the knot's YAML
frontmatter. If the project is not a git repo, commits are silently
skipped. The rig's own git repository is separate and committed
manually by the user.

## Logs

Knot maintains several log files for observability:

| Log | Location | Purpose |
|-----|----------|---------|
| **Loom-log** | `tie-offs/<rig>/{loom-id}/.loom-log` | Per-loom activity: knot registration, processing events, errors |
| **Rig-log** | `tie-offs/<rig>/.rig-log` | JSONL of serious events: timeouts (`TimeoutExceeded`) and idle periods (`QueueIdle`) |

Logs are **per-run**: at every startup, Knot truncates the rig-log and
every loom-log *before* loom discovery, so each log always contains
only the events of the current run (it starts with the fresh
`KnotRegistered`/`LoomStarted` events and ends with `LoomStopped` at
shutdown). Cross-run history is not kept in the logs — the tie-off
files are the durable audit record (plain text, git-versioned).

The logs support multiple consumers (append-only within a run,
single-line JSON entries); knots and operators react to events of the
*current* run only.

## Key Principles

### File-First

All configuration lives as `.md` files with YAML frontmatter. Write files
directly to disk — Knot's file watcher picks up changes automatically.
Observation is through `tie-offs/<rig>/state.json`, written every 5
seconds.

### Version-Controllable

Everything is plain text. The rig source (profiles, looms, knots) lives
in its own git repository, committed manually by the user. Runtime
output (tie-offs, logs, state) is committed to the project repository —
Knot itself creates git commits for tie-off output.

### Auto-Discovery

Knot discovers configuration automatically:

- **Looms** — any `rig/*-loom/` directory
- **Knots** — any `.md` file inside a loom directory
- **Profiles** — any `.md` file in `rig/profiles/`

Profiles are read fresh from disk at processing time — edits take effect
on the next strand event without restarting Knot.

### Goal-Seeking, Not Scripted

Knots are not one-shot scripts. They are **goal-seeking agents** that
read current state, compare it against a goal, and apply only the changes
needed. This makes them idempotent — safe to re-run on the same input.

See the [Design Guide](design-guide.md) for details on designing
idempotent knots.

## Agent Skills

Knot ships with a set of agent skills that let your AI agent manage
the rig. Each skill is a `.md` file discovered by the agent framework
(pi or others) and invoked by natural language.

| Skill | Purpose |
|-------|---------|
| **knot-init** | Initialise a rig, create profiles, install skills globally |
| **knot-create** | Create, modify, delete looms, knots, and profiles |
| **knot-dispatch** | Trigger knots into action by creating or touching strands |
| **knot-inspect** | View rig state, looms, knots, profiles, and activity logs |
| **knot-manage** | Review completed work — tie-off quality, interaction chains, commit quality |
| **knot-analyst** | Analyse rig productivity, project progress, and blockers |
| **knot-design** | Design looms and knots following idempotency and loop patterns |
| **knot-update** | Migrate project documents between Knot versions |

Skills are stored at `.agents/skills/` in the Knot repository and
installed to `~/.agents/skills/` for global use. The `knot-init` skill
handles this installation automatically during rig setup.
