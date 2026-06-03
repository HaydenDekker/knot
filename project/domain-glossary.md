# Domain Glossary

> **Last Updated:** 2026-06-03

Living glossary of domain terms for Knot. Terms are added when they emerge from PRDs, ADRs, or design discussions. Definitions are refined as understanding deepens.

---

## Terms

### Agent Profile

The configuration that determines *which agent* runs. Contains:

- The LLM provider to use.
- The skills available to the agent (driving the prompt).
- The tools available to the agent (executing the prompt).
- The agent's initial system prompt.

Part of a **knot**.

---

### Knot

A configured artifact that brings everything together, ready for processing. Composed of two parts:

1. **Agent Profile** — determines *which agent* runs.
2. **Prompt Template** — determines *how* input files are processed.

One knot is configured per loom. The same knot can be reused across multiple looms.

---

### Loom

A directory containing one or more knot definition files. The directory structure inside the loom is up to the user (e.g. `prd-review-loom/goals-review-knot.md`, `prd-review-loom/non-goals-review-knot.md`). Each knot file defines a knot. A loom watches a configured source directory for **strands** — file events on strands trigger the loom's knots.

---

### Prompt Template

The *how* of file processing. Contains the goal description and the rules for how the input **strands** should be bundled into the prompt sent to the agent.

Part of a **knot**.

---

### Target

An output directory for generated files. Each loom has an associated target where **tie-offs** are written.

---

### Tie-off Point

The directory location where tie-offs land. Configured per loom. This is where the agent's final response or error is filed away after processing a strand.

---

### Strand

A file in the source directory that a loom watches for. When a strand is created, modified, or deleted, the loom's knots are triggered to process it. The strand is the raw input fed into a knot.

---

### Loom-log

A file that holds a loom's activity log. Records which knots are detected and registered, and high-level loom events. Users check this to confirm a loom is configured correctly.

---

### Knot-state

A per-knot file that records processing events and status for that knot. Contains event type, strand path, tie-off path, and any errors. All knot-level HTTP status is sourced from this file.

---

### Tie-off

The final response or error produced by a knot at the end of its session. This is the single output that lands in the configured target directory (e.g. a summary, a list of documents, an error report). During processing the knot's agent may write files to any directories it has access to — the tie-off is what gets captured and filed away.

---

## Term Relationships

```
Loom (directory of knot definitions)
 ├── Knot (Agent Profile + Prompt Template)
 │     ├── Agent Profile
 │     │     ├── LLM Provider
 │     │     ├── Skills
 │     │     ├── Tools
 │     │     └── System Prompt
 │     └── Prompt Template
 │           ├── Goal Description
 │           └── Input Bundling Rules
 └── Target (output directory)
      └── Tie-off (final response or error)
```
