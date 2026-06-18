# Concepts

Knot is a file-first agent orchestration system. It runs as a local HTTP
service, watches directories for file changes, and triggers AI agent
sessions in response. Everything is stored as plain text on disk, making
it easy to review, share, and version-control.

This page explains Knot's mental model — the hierarchy of objects and
how they relate to each other.

## The Hierarchy

```
Rig
 ├── Profiles (shared agent configurations)
 └── Looms (processing namespaces)
      └── Knots (individual processing tasks)
           ├── reads from a Strand Directory
           └── writes a Tie-off
```

### Rig

The top-level container. A rig lives at `./rig/` in your project and
aggregates all looms, profiles, and processing output. It is the ship's
complete interconnected system — the place where looms live and knots
are defined.

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
2. **Prompt Template** — how input files are processed (goal description
   and bundling rules).
3. **Strand Directory** — which directory to watch for input files.

One loom can contain multiple knot files, each defining a different
processing task.

### Strand

A file in a knot's strand directory. When a strand is created, modified,
or deleted, the knot that watches that directory is triggered to process
it. The strand is the raw input fed into the knot's agent session.

### Tie-off

The output produced by a knot after processing. Each processing event is
appended to a single `{knot-name}-tie-off.md` file at
`rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`. The file
grows over time, telling the complete story of the knot's work. Event
metadata in each section identifies which strand was processed.

### Strand Directory

The directory a knot watches for strand events, configured as `strand-dir`
in the knot's YAML frontmatter. It is resolved relative to the project
root (the directory containing `rig/`).

## The Processing Flow

```
File change in strand-dir
        │
        ▼
  Knot's file watcher detects event
        │
        ▼
  Knot loads its agent profile from disk
        │
        ▼
  Agent session starts:
    ├── system prompt = profile.system-prompt + knot.instructions
    ├── input = strand file(s)
    └── tools = profile.tools
        │
        ▼
  Agent produces output
        │
        ▼
  Output appended to tie-off file
```

## Logs

Knot maintains several log files for observability:

| Log | Location | Purpose |
|-----|----------|---------|
| **Loom-log** | `rig/tie-offs/{loom-id}/.loom-log` | Per-loom activity: knot registration, processing events, errors |
| **Knot-state** | Inside loom-log | Per-knot processing status and last event details |
| **Rig-log** | `rig/.rig-log` | Append-only JSONL of serious events: timeouts (`TimeoutExceeded`) and idle periods (`QueueIdle`) |

The rig-log survives server restarts and supports multiple consumers
(append-only, single-line JSON entries).

## Key Principles

### File-First

All configuration lives as `.md` files with YAML frontmatter. Write files
directly to disk — Knot's file watcher picks up changes automatically.
No HTTP registration is needed. Changes are visible through git diffs.

### Version-Controllable

Everything is plain text. Your entire rig configuration — profiles, looms,
knots, and tie-offs — can be tracked in git and reviewed through standard
diff tools.

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
