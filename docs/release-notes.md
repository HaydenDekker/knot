# Release Notes

## v0.23.0 — 2026-07-24

### Feature — `knot-manage` skill

New skill for retrospective review of completed rig work. Examines
tie-off files, assesses output quality, traces producer→consumer
interaction chains, and reviews git commit quality. Complements
`knot-analyst` which focuses on live operational health.

### Feature — `knot-dispatch` skill

New skill for triggering knots into action. Creates or touches strand
files, dispatches events manually, and follows the full event pipeline
from strand creation to tie-off completion.

### Feature — `knot-analyst` skill updated

`knot-analyst` (v1.1.0) now includes six analysis dimensions:
operational activity, git history, project document progress, stagnation
detection, blocker identification, and a traffic-light productivity
score.

### Documentation

- `getting-started.md` — updated skill installation to include all 8
  skills with verification step
- `concepts.md` — new "Agent Skills" section
- `design-guide.md` — references `knot-design` skill
- `troubleshooting.md` — new section on diagnostic skills
- `workflows/` — references to `knot-dispatch` and `knot-manage`
- `README.md` — expanded Quick Start with workflow steps

## v0.22.1 — 2026-07-03

### Bugfix

- Fixed flaky `execute_timeout_regression` test under `--test-threads=4` (ETXTBSY)

### Testing

- Completed integration test migration (phases 0–11). Application tests use mock ports, adapter tests use real I/O with `tempfile`, composition smoke tests verify full wiring. `TEST_MUTEX`, process-global env vars, and `KNOT_TEST_CLI_PATH` eliminated. Lib tests run in ~1.1s.

## v0.22.0 — 2026-07-01

### Breaking Change — Flat tie-off paths

Tie-off paths changed from `rig/tie-offs/{loom-id}/{knot-name}/{strand}.output` to `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`. The intermediate knot subdirectory is removed. Tie-offs are now one file per knot with append-mode writes.

Migration: Update any scripts or tooling that reference the old path structure.

### Feature — Strand queue visibility

`rig/state.json` now includes a `strand_queue` array showing all pending strand events with file path, loom/knot IDs, event type, and queued timestamp.

### Bugfix

- Fixed `spawn_blocking` for `ProcessStrand execute()` — ensures graceful shutdown on Ctrl+C

## v0.21.0 — 2026-07-01

### Feature — Final response filtering in Pi JSON adapter

`PiJsonAgentRunner` now extracts only the agent's final response text. When Pi uses tools, intermediate messages with `stopReason: "toolUse"` are excluded; only `"stop"` and `"length"` responses produce output. This prevents tool-use artifacts from appearing in tie-off files.

## v0.20.3 — 2026-06-29

### Refactor

- Extracted `usecases.rs` into isolated modules (`loom/`, `query/`, `session_resume/`). Pure structural refactor — zero behaviour change.

## v0.20.1 — 2026-06-29

### Bugfix

- Removed unused imports from process_strand test modules

## v0.20.0 — 2026-06-28

### Feature — Session resume on invocation failure

Automatically resume Pi sessions from where they left off after invocation failure (timeout, network error). Uses `--session-id` for up to 10 retries with 10-second delays between attempts. Profile timeout budget is respected — retries stop when insufficient time remains. Each retry appends "please continue" to the session.

## v0.19.0 — 2026-06-27

### Feature — JSON-based agent adapter

New `agent_adapter` enum in `.workspace-agent-config.yaml` replaces `cli_path`/`cli_args`. Supports `pi-stdio` (default, reads stdout) and `pi-json` (parses JSON-L for session IDs and token usage). `run_startup()` auto-creates the config file on first boot.

## v0.18.1 — 2026-06-26

### Bugfix

- Fixed `unwatch()` removing all watcher entries for a path when only a single knot's entry should be removed. Broke shared strand directory scenarios where multiple knots watch the same directory.

## v0.18.0 — 2026-06-24

### Breaking Change — Prompt text moved to markdown body

Profile and knot files no longer embed prompt text in YAML frontmatter. The plain text after the `---` separator is now the prompt content.

Frontmatter retains only structural metadata (name, provider, model, tools, timeout for profiles; name, agent-profile-ref, strand-dir, git-versioned for knots).

## v0.17.0 — 2026-06-24

### Feature — Strand missing file handling

Known temp files (e.g. macOS `sed -i` temp files) are silently skipped. Unknown missing files produce `StrandSkipped` events in the loom-log instead of spurious "File not found" errors.

## v0.16.0 — 2026-06-22

### Feature — Tie-off context extraction for deleted files

When a strand is deleted, Knot now parses the tie-off file and injects the last N per-strand entries into the agent prompt (replacing the `@file` reference that would fail on deleted files).

## v0.15.0 — 2026-06-20

### Breaking Change — Removed `input-bundling` from knot frontmatter

The `input-bundling` property was removed from knot YAML frontmatter. It had no runtime effect — only `full-file` ever shipped and is always the behaviour. Knot files that still contain `input-bundling` parse with a warning.

## v0.14.0 — 2026-06-19

### Feature — All text files accepted as strands

Knots now process any text file (`.rs`, `.json`, `.py`, `.txt`, etc.) — not just `.md`. Binary files are detected (null-byte heuristic on first 8KB) and silently skipped with `StrandIgnored` in the loom-log.

## v0.13.0 — 2026-06-19

### Breaking Change — HTTP interface removed

The Axum HTTP server was removed entirely. All state observation is now through `rig/state.json`, written atomically every 5 seconds. `GET /health`, `GET /looms`, `GET /profiles`, and all other HTTP endpoints no longer exist. Skills and tools read `rig/state.json` directly.

## v0.12.0 — 2026-06-17

### Feature — Explicit Pi session titles

Each agent session gets a unique, descriptive title derived from knot ID and strand filename (e.g. `plan-architect triggered by Modified on 004-manifest-resources.md`).

### Core Features

Knot is a local agent orchestration system that watches directories for
file changes and triggers AI agent sessions. Key capabilities:

- **File-first configuration** — All configuration is `.md` files with
  YAML frontmatter. Git-trackable, diff-visible.
- **Auto-discovery** — Looms (`*-loom/` directories), knots (`.md`
  files in looms), and profiles (`rig/profiles/*.md`) are discovered
  automatically via file watching.
- **Agent profiles** — Define which LLM provider, model, tools, and
  system prompt to use. Profiles are read fresh from disk at processing
  time.
- **Knot processing** — Goal-seeking agents that read strands (input
  files), inspect current state, and apply minimal changes to reach a
  goal. Idempotent by design.
- **Tie-off output** — Append-only output files at
  `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`.
- **Git versioning** — Automatic commits after each tie-off write
  (opt-out per-knot with `git-versioned: false`).
- **Session resume** — Automatic retry of failed agent sessions (up to
  10 retries, 10s delay).
- **State file** — `rig/state.json` updated every 5 seconds with looms,
  knots, profiles, and strand queue.
- **Activity logging** — Per-loom activity logs and a rig-wide
  operational log (`rig/.rig-log`) in JSONL format.
- **Rig switching** — Multiple rigs per project, with packaging for
  sharing.
- **Debounced event processing** — File events are debounced to avoid
  triggering on partial writes.
- **Graceful shutdown** — Cooperative cascade shutdown that drains
  pending events.
- **Configurable timeouts** — Per-profile session timeouts with
  `TimeoutExceeded` event logging.
