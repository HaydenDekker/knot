# Domain Glossary

> **Last Updated:** 2026-06-06

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
- The agent's initial system prompt.

Part of a **knot**.

---

### Knot

A configured artifact that brings everything together, ready for processing. Composed of three parts:

1. **Agent Profile** — determines *which agent* runs.
2. **Prompt Template** — determines *how* input files are processed.
3. **Directories** — the `strand_dir` (where strands to watch live, **required**) and `tie_off_dir` (where output is written, **required**).

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

The directory location where tie-offs land. Configured per-knot as `tie_off_dir` in the knot's YAML frontmatter. This is where the agent's final response or error is filed away after processing a strand.

---

### Loom-log

A file that holds a loom's activity log. Lives inside the loom directory at `<rig>/<loom-id>/.loom-log`. Records which knots are detected and registered, and all loom and knot events for that loom. Users check this to confirm a loom is configured correctly and to trace processing history.

---

### Knot-state

A per-knot file that records processing events and status for that knot. Contains event type, strand path, tie-off path, and any errors. All knot-level HTTP status is sourced from this file.

---

### Tie-off

The final response or error produced by a knot at the end of its session. Each processing event is appended to the tie-off file, so the document grows over time and tells the complete story of the knot's work. The tie-off lands in the configured tie-off directory (e.g. a summary, a list of documents, an error report). During processing the knot's agent may write files to any directories it has access to — the tie-off is what gets captured and filed away.

---

## Term Relationships

```
Rig (`./rig/`)
 └── Loom (`<rig>/<name>-loom/`, by `-loom` naming convention)
      ├── Knot definition files (first-level `.md` files)
      │     ├── Agent Profile
      │     │     ├── LLM Provider
      │     │     ├── Skills
      │     │     ├── Tools
      │     │     └── System Prompt
      │     ├── Prompt Template
      │     │     ├── Goal Description
      │     │     └── Input Bundling Rules
      │     ├── strand_dir (required — directory to watch for strands)
      │     └── tie_off_dir (required — directory to write output)
      └── .loom-log (activity log inside loom directory)
```
