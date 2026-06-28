# Domain Glossary

> **Last Updated:** 2026-06-28

Living glossary of domain terms for Knot. Terms are added when they emerge from PRDs, ADRs, or design discussions. Definitions are refined as understanding deepens.

---

## Terms

### Rig

The top-level container — an aggregation of one or more looms. A **rig** is the ship's complete interconnected system of ropes, lines, and running rigging; the place where looms live and knots are defined.

---

### Please Continue

Prompt suffix appended to the Pi session on retry, telling the agent to resume where it left off. When Knot retries a failed invocation using `--session-id`, it appends a short "please continue" message so the provider picks up from the partial output instead of restarting. This is only sent on retry attempts — the initial invocation sends the full prompt normally.

---

### Retry Delay

A 10-second pause between retry attempts during session resume. Allows transient network errors or provider-side slowdowns time to recover before Knot re-enters the same Pi session. Applied between each retry up to the hard cap of 10 attempts.

---

### Agent Profile

The configuration that determines *which agent* runs. Contains:

- The LLM provider to use.
- The skills available to the agent (driving the prompt).
- The tools available to the agent (executing the prompt).
- The system prompt (markdown body) — persona instructions delivered via stdin.
- The session timeout (optional, in seconds; defaults to 300s / 5 min).

Part of a **knot**.

---

### Knot

A configured artifact that brings everything together, ready for processing. Composed of three parts:

1. **Agent Profile** — determines *which agent* runs.
2. **Markdown Body** — task-specific instructions that supplement the profile's system prompt.
3. **Directories** — the `strand_dir` (where strands to watch live, **required**). The tie-off output path is **statically derived** as `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md` — no `tie-off-dir` configuration is needed.

A knot is defined in a `.md` file with YAML frontmatter. The frontmatter holds structural metadata (`name`, `agent-profile-ref`, `strand-dir`); the markdown body contains the knot's task-specific instructions. One loom can contain one or more knot files.

---

### Loom

A directory inside the rig whose name ends with the `-loom` suffix (e.g. `rig/planning-loom/`). A loom directory contains one or more `.md` knot definition files at its first level — Knot discovers these as the loom's knots. The loom directory is **static and derived from naming convention**; it is not user-configurable.

The loom's identity (`LoomId`) is derived from the directory name (the `-loom` suffix is included in the ID).

---

### Agent Invocation Metadata

Structured data captured from a Pi agent session when invoked in `json` mode. Contains:

- **Session ID** — the unique identifier for the Pi conversation session, extracted from the `session` JSON-L event.
- **Token usage** — breakdown of tokens consumed during the session (`input`, `output`, `cacheRead`, `cacheWrite`, `totalTokens`), extracted from the `agent_end` JSON-L event.

Only available when the rig config sets `agent-adapter: pi-json`. With `pi-stdio` (the default), metadata is not captured and is `None`. Stored in the optional `AgentInvocationMetadata` field on `AgentOutput`.

---

### Overall Timeout Budget

The profile's timeout value governs the total wall-clock time across all retry attempts, not per-attempt. When session resume is active, the timeout countdown does not reset on each retry — instead, Knot calculates remaining time before each attempt. If the budget is exhausted during the retry loop, the strand is marked failed and normal failure handling (loom-log, rig-log) takes over.

---

### Session Resume

Automatic retry mechanism activated when an agent invocation fails with a resumable error and a session ID was captured (requires `json` invocation mode). Knot retries the same invocation using `--session-id <id>` to continue the Pi session from where it stopped. A "please continue" prompt is appended so the agent resumes partial work. Retries are limited to 10 attempts or until the profile's overall timeout budget is exhausted, whichever comes first. A 10-second delay between retries allows transient errors to recover. On successful resume the strand completes normally (transparent to the user); on exhausted retries or budget expiry the strand is marked failed.

---

### SessionResumed

A loom-log event recorded for each session resume attempt. Contains the retry number, session ID, and remaining timeout budget at the time of the attempt. Allows tracing how many retries occurred and whether the overall timeout budget was the limiting factor.

---

### Invocation Mode

How Knot communicates with the agent CLI. Two modes are supported, selected by the `agent-adapter` field in rig config:

- **`stdio`** (default) — Knot invokes Pi with `--print` mode. Output is plain text on stdout; the only data captured is the response string and exit code. Session IDs and token usage are lost.
- **`json`** — Knot invokes Pi with `--mode json`. Output is JSON-L (newline-delimited JSON). A `PiJsonAgentRunner` adapter parses the stream to extract the session ID, token usage, and response text.

The mode is agent-specific — each adapter hardcodes its own binary path and CLI flags. No generic CLI wrapper is used (see ADR-009).

---

### Retry

An individual attempt within the session resume loop. The first invocation is not counted as a retry — retries begin on the second attempt onward. Each retry re-enters the same Pi session using `--session-id`, appends a "please continue" prompt, and checks the overall timeout budget before proceeding. Up to 10 retries are allowed.

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

A per-knot file that records processing events and status for that knot. Contains event type, strand path, tie-off path, and any errors. Knot status is readable from this file and is also included in `rig/state.json`.

---

### Tie-off

The final response or error produced by a knot at the end of its session. Each processing event is appended to a single `{knot-name}-tie-off.md` file at `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`, so the document grows over time and tells the complete story of the knot's work. The event metadata in each section identifies which strand was processed. During processing the knot's agent may write files to any directories it has access to — the tie-off is what gets captured and filed away.

---

### Rig-log

An append-only JSONL file at `rig/.rig-log` that records serious operational events so the user or an external watcher (human or LLM agent) can monitor and react. Two event types are recorded:

- `TimeoutExceeded` — an agent session exceeded its deadline (from profile `timeout` or runner default). Contains loom ID, knot ID, strand path, error message, and timestamp. The tie-off file is **preserved unchanged** on timeout.
- `QueueIdle` — all pending events have been processed and no new events arrived within the poll window (500ms). Indicates the system is quiet.

The rig-log persists across restarts. Multiple consumers can watch it safely (append-only, single-line JSON entries).

---

### Rig State

A JSON file at `rig/state.json` that contains a complete snapshot of the rig's current state: the rig path, all discovered looms with their knots and strand counts, all available agent profiles, and a timestamp of when the state was last updated. Written atomically by the State Writer task on a 5-second poll cycle. This is the single source of truth for external consumers (skills, scripts, other tools) that need to read rig state — no HTTP client required.

---

### State Writer

A background task that periodically polls the rig's in-memory state and writes it to `rig/state.json`. Runs on a 5-second interval. Uses atomic write (write to temp file, then rename) to prevent readers from seeing partial state. If the write fails (e.g., disk full), the error is logged but the task continues on the next cycle.

---

## Term Relationships

```
Rig (`./rig/`)
 ├── state.json (complete state snapshot — written by State Writer every 5s)
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
            └── Markdown Body (task-specific instructions)
            └── strand_dir (required — directory to watch for strands)
```
