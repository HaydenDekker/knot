# Domain Glossary

> **Last Updated:** 2026-06-14

Living glossary of domain terms for Knot. Terms are added when they emerge from PRDs, ADRs, or design discussions. Definitions are refined as understanding deepens.

---

## Terms

### Rig

The top-level container — an aggregation of one or more looms. A **rig** is the ship's complete interconnected system of ropes, lines, and running rigging; the place where looms live and knots are defined.

---

### Agent Profile

The configuration that determines *which agent* runs. Contains:

- The LLM provider to use.
- The skills available to the agent (driving the prompt).
- The tools available to the agent (executing the prompt).
- The profile prompt (YAML key `profile-prompt`) — persona instructions delivered via stdin.
- The session timeout (optional, in seconds; defaults to 300s / 5 min).

Part of a **knot**.

---

### Knot

A configured artifact that brings everything together, ready for processing. Composed of three parts:

1. **Agent Profile** — determines *which agent* runs.
2. **Prompt Template** — determines *how* input files are processed.
3. **Directories** — the `strand_dir` (where strands to watch live, **required**). The tie-off output path is **statically derived** as `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md` — no `tie-off-dir` configuration is needed.

A knot is defined in a `.md` file with YAML frontmatter. One loom can contain one or more knot files.

---

### Loom

A directory inside the rig whose name ends with the `-loom` suffix (e.g. `rig/planning-loom/`). A loom directory contains one or more `.md` knot definition files at its first level — Knot discovers these as the loom's knots. The loom directory is **static and derived from naming convention**; it is not user-configurable via the API.

The loom's identity (`LoomId`) is derived from the directory name (the `-loom` suffix is included in the ID).

---

### Strand Directory

The directory that a knot watches for strand file events. Configured per-knot as `strand_dir` in the knot's YAML frontmatter. This is the directory where raw input files (**strands**) live.

> **Note:** The strand directory is the *knot-level* watch target. It is not the same as the loom directory. The loom directory holds knot definition files; the strand directory holds the files being processed.

---

### Prompt Template

The *how* of file processing. Contains the goal description and the rules for how the input **strands** should be bundled into the prompt sent to the agent.

Part of a **knot**.

---

### Strand

A file in a knot's strand directory. When a strand is created, modified, or deleted, the knot that watches that directory is triggered to process it. The strand is the raw input fed into a knot.

---

### Tie-off Directory

Statically derived path under `rig/tie-offs/{loom-id}/{knot-name}/`. No longer configurable per-knot (the `tie-off-dir` YAML field has been removed). Each knot writes a single `{knot-name}-tie-off.md` file that appends all processing events; the event metadata in each section identifies which strand was processed.

---

### Loom-log

A file that holds a loom's activity log. Lives at `<rig>/tie-offs/<loom-id>/.loom-log` — outside the loom directory itself, keeping the rig clean of non-loom directories. Records which knots are detected and registered, and all loom and knot events for that loom. Users check this to confirm a loom is configured correctly and to trace processing history.

---

### Knot-state

A per-knot file that records processing events and status for that knot. Contains event type, strand path, tie-off path, and any errors. All knot-level HTTP status is sourced from this file.

---

### Tie-off

The final response or error produced by a knot at the end of its session. Each processing event is appended to a single `{knot-name}-tie-off.md` file at `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`, so the document grows over time and tells the complete story of the knot's work. The event metadata in each section identifies which strand was processed. During processing the knot's agent may write files to any directories it has access to — the tie-off is what gets captured and filed away.

---

### Rig-log

An append-only JSONL file at `rig/.rig-log` that records serious operational events so the user or an external watcher (human or LLM agent) can monitor and react. Two event types are recorded:

- `TimeoutExceeded` — an agent session exceeded its deadline (from profile `timeout` or runner default). Contains loom ID, knot ID, strand path, error message, and timestamp. The tie-off file is **preserved unchanged** on timeout.
- `QueueIdle` — all pending events have been processed and no new events arrived within the poll window (500ms). Indicates the system is quiet.

The rig-log survives server restarts. Multiple consumers can watch it safely (append-only, single-line JSON entries).

---

## Term Relationships

```
Rig (`./rig/`)
 ├── .rig-log (operational event log — TimeoutExceeded, QueueIdle)
 ├── profiles/ (shared agent profile definitions)
 ├── tie-offs/
 │     └── <loom-id>/
 │           ├── .loom-log (activity log)
 │           └── <knot-name>/
 │                 └── <knot-name>-tie-off.md (tie-off output, appended per event)
 └── Loom (`<rig>/<name>-loom/`, by `-loom` naming convention)
      └── Knot definition files (first-level `.md` files)
            ├── Agent Profile
            │     ├── LLM Provider
            │     ├── Skills
            │     ├── Tools
            │     └── Profile Prompt
            ├── Prompt Template
            │     ├── Goal Description
            │     └── Input Bundling Rules
            └── strand_dir (required — directory to watch for strands)
```
