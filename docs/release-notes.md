# Release Notes

## v0.35.0 — 2026-08-23

### Feature — `knot step`: Single-Event Stepping (Plan 073)

`knot step` processes **exactly one** queued event and exits — the
manual trigger/observation tool for watching a cycle unfold, inspecting
rig state between events, and debugging a misbehaving knot without
letting the service drain the queue back-to-back.

```
knot step [--rig <rig-name>] [--event <event-filename>]
```

- `--event` targets a specific queued event (exact id, `.json`
  optional; unique id prefix; or strand filename — no match lists the
  queue on stderr and exits 1); without it the FIFO head is processed.
- Empty queue → `queue empty`, exit 0. Exit 1 on unknown/ambiguous
  event, no rigs, multiple rigs, or processing failure.
- A step runs the **full service startup** (migration, config seeding,
  rig git init, discovery, watchers, debounce engine, state writer),
  executes the single event, and shuts down with the service-identical
cascade. Events dispatched *during* the step are captured into
  `tie-offs/<rig>/events/` but **not executed**.
- **Logs are not cleared** in step mode — a multi-step session
  accumulates in the loom-logs/rig-log. (Service startups still clear
  them; see v0.34.0.)
- Step rig discovery is stricter than the service: zero `*-rig`
  matches is an error (no implicit `rig/` creation).
- Use `knot step` when the service is **not** running — two processes
  sharing the disk queue can double-read the same event (safe by knot
  idempotency, but wasteful).

### Queue Semantics — Late Removal (At-Least-Once)

The queued event file is no longer removed when the event is *popped*
for processing — it is removed **after the work is done**:

- **On success** — as the last step before the git commit (dispatch,
  tie-off append, loom-log entries, and event enforcement all happen
  first; the commit captures everything, including the removal).
- **On failure/skip** — at the point of failure (consume-on-failure —
  no poison-pill retry loops).

The only window in which an event survives a crash is while its
processing is in flight: a restart re-queues it and the knot re-runs
(safe by knot idempotency). Previously a crash during a long agent run
lost the event silently. The service loop now peeks (`front()`) instead
of popping; the CLI parsing is a pure unit-tested `parse_args`
function. No new dependencies.

**No document migration:** pending events from older versions read
identically (the `events/*.json` schema is unchanged). The `knot-update`
skill carries the 0.35.0 changelog entry.

### Skills and Docs Updated

- `knot-dispatch` (v1.3.0) — new **Stepping: `knot step`** section
  (flags, event resolution, empty-queue behaviour, exit codes, what a
  step does, direct queue write, agent workflow); stale pop-removal and
  debounce-window wording corrected
- `knot-update` (v1.11.0) — 0.35.0 changelog entry (no migration
  required; queue files from older versions read identically)
- `knot-manage` (v1.2.0), `knot-analyst` (v1.4.0), `knot-init`
  (v4.2.0) glossary — queue-removal wording corrected to late removal
- `docs/concepts.md` — new Event Queue section (at-least-once
  semantics, `knot step`)
- PRD `prd-persistent-events.md` — popped-removal goal revised to late
  removal; new Story 6 (manual stepping via the CLI)
- Design reference: `project/design/design-knot-step.md` (late-removal
  ordering contract, crash windows, the step lifecycle, rejected
  minimal-startup alternative)

## v0.34.0 — 2026-08-23

### Per-Run Logs — Loom-Logs and Rig-Log Cleared at Startup (Plan 072)

The operational logs are now **per-run**. On every startup — after
legacy-layout migration, before loom discovery — Knot truncates the
rig-log (`tie-offs/<rig>/.rig-log`) and **every**
`tie-offs/<rig>/<loom-id>/.loom-log`, including orphaned loom dirs
whose loom no longer exists in the rig. Each log always contains
exactly the events of the current run: it starts with the fresh
`KnotRegistered`/`LoomStarted` events and ends with `LoomStopped` at
shutdown.

**Why:** the logs exist for current-run observability (event-watching
knots, `knot-inspect`/`knot-analyst`). The durable audit history lives
in the git-versioned tie-off files. Previously, stale unparseable lines
re-fired a `WARN:` skip on every 5-second state write and every query
— forever — and the logs grew unbounded with residue nothing consumes.

| Artifact | Before | 0.34.0+ |
|---|---|---|
| `.rig-log`, `*/.loom-log` | accumulated across runs | truncated at every startup |
| Tie-off files, `state.json`, `events/`, dispatch dirs | unchanged | unchanged |

**Non-fatal:** a failed clear logs a `WARNING:` and startup proceeds.
Only log files are touched — nothing else is deleted or modified.

**No document format changes:** profiles, knots, looms, and tie-offs
are unaffected. On the first run of 0.34.0, all earlier runs'
`.rig-log`/`.loom-log` content is discarded — intentional (tie-offs
retain the history).

### Skills and Docs Updated

- `knot-update` (v1.10.0) — 0.34.0 changelog entry (no migration
  required; first run discards earlier runs' log content)
- `knot-inspect` (v3.5.0), `knot-analyst` (v1.3.0) — log descriptions
  annotated per-run scope; analyst failure counting now "since the
  last startup"
- `docs/concepts.md` — Logs section rewritten for per-run semantics
- Design reference: `project/design/design-startup-log-clear.md`
  (startup sequence, ordering invariants, what the clear touches)

## v0.31.0 — 2026-08-17

### Breaking — Rig/Project Repository Split (Plan 068)

The rig no longer holds any runtime data. The runtime tree — tie-off
directories, loom-logs, the event queue, the rig-log, and the state
snapshot — moves from `rig/` to `tie-offs/<rig-basename>/` in the
project root (default rig: `tie-offs/rig/`).

| Path | Before | After |
|---|---|---|
| State snapshot | `rig/state.json` | `tie-offs/<rig>/state.json` |
| Tie-off files | `rig/tie-offs/{loom-id}/…` | `tie-offs/<rig>/{loom-id}/…` |
| Loom-log | `rig/tie-offs/{loom-id}/.loom-log` | `tie-offs/<rig>/{loom-id}/.loom-log` |
| Event dispatch dirs | `rig/tie-offs/{loom-id}/{EventId}/` | `tie-offs/<rig>/{loom-id}/{EventId}/` |
| Event queue | `rig/events/` | `tie-offs/<rig>/events/` |
| Rig-log | `rig/.rig-log` | `tie-offs/<rig>/.rig-log` |

**Rig repository:** Knot initialises `rig/.git` at startup (idempotent).
The rig tracks exactly its source (looms, knots, profiles, config) and
is committed **manually by the user**. When the project root is inside a
git repo, Knot appends a marked `rig/` line to the project's
`.gitignore`, and the git versioner unstages `rig/` before every commit
so the rig can never leak into a project commit (gitlink or tracked
leftovers).

**Auto-migration:** on first 0.31.0 startup, legacy runtime files are
moved automatically (`[startup] migrated …` notice). Idempotent;
destination-exists conflicts keep the destination and warn.

**Pre-existing projects (manual step):** if `rig/` was already tracked
by the project git, run the one-time
`git rm -r --cached rig/` + commit to untrack it. Knot logs a warning
and never runs `git rm` itself.

**Watcher caveat:** after migration, dispatch directories that already
contain unprocessed event files are watched at their new path, but the
file watcher does not rescan existing files — touch each unprocessed
event file to re-trigger processing.

**No document format changes:** profiles, knots, and looms are
unaffected. `knot share` is unchanged — the zip now equals exactly the
rig git's tracked content.

### Skills and Docs Updated

- `knot-init` (v4.0.0) — running-check path moves to
  `tie-offs/<rig>/state.json`; new Rig Repository section
- `knot-glossary` — new **Runtime Tree** term; all paths re-rooted
- `knot-manage` (v1.1.0) — two-repo review workflow; post-migration
  watcher caveat
- `knot-inspect` (v3.3.0), `knot-analyst` (v1.2.0), `knot-dispatch`
  (v1.1.0), `knot-create` (v5.5.0), `knot-design` (v1.5.0),
  `knot-abstractions` (v1.2.0) — path references, diagrams, and
  quick-reference commands updated
- `knot-update` — 0.31.0 changelog entry with migration + verification
  instructions
- `docs/configuration/rig-structure.md` — new directory tree, Rig
  Repository and Runtime Tree section; `docs/concepts.md`,
  `docs/getting-started.md`, `docs/troubleshooting.md`,
  `docs/workflows/*`, `docs/configuration/knots.md`,
  `docs/configuration/profiles.md`, `README.md` — path references

## v0.30.1 — 2026-07-24

### Bugfix — Startup ordering: persisted events processed after loom discovery

After restarting Knot with events in the queue, the process-strand loop
processed them before `DiscoverLooms` had run, causing
`loom 'X-loom' not found` errors for every persisted event. Fixed by
deferring the process-strand loop until after `run_startup()` completes.

### Feature — `knot-manage` skill

New skill for retrospective review of completed rig work. Examines
tie-off files, assesses output quality, traces producer→consumer
interaction chains, and reviews git commit quality. Complements
`knot-analyst` which focuses on live operational health.

### Feature — `knot-dispatch` skill

New skill for triggering knots into action. Creates or touches strand
files, dispatches events manually, and follows the full event pipeline
from strand creation to tie-off completion.

### Documentation

- `getting-started.md` — updated skill installation to include all 8
  skills with verification step
- `concepts.md` — new "Agent Skills" section
- `design-guide.md` — references `knot-design` skill
- `troubleshooting.md` — new section on diagnostic skills
- `workflows/` — references to `knot-dispatch` and `knot-manage`
- `README.md` — expanded Quick Start with workflow steps

## v0.30.0 — 2026-07-21

### Feature — Persistent Event Queue (Disk-Backed)

Strand events are now persisted to `rig/events/{id}.json` on disk
instead of held in memory. Events survive process restarts (Ctrl+C,
crashes) and are restored before processing resumes.

**How it works:**

- Every event pushed is written atomically (temp file → rename) to
  `rig/events/`
- On startup, `rig/events/*.json` files are scanned and re-queued
  before the debounce engine starts
- When an event is processed (popped), its file is removed from disk
- The disk is the source of truth — editing a pending event file on
  disk is honoured when the event is processed
- Malformed JSON files are skipped with a warning; non-`.json` files
  are silently ignored

**Architecture changes:**

- `InspectQueue<Option<TimestampedStrandEvent>>` replaced by
  `StrandEventQueue` trait with `DiskBackedEventQueue` as the primary
  implementation
- `PendingEvent` domain model with unique IDs
  (`{unix_timestamp_ms}-{4-hex-chars}`)
- `PendingEventOrShutdown` enum replaces `Option<T>` for the shutdown
  sentinel
- Dedup key preserved: `(strand_path, loom_id, knot_id, kind)`

**Migration:**

No migration needed. On first start, `rig/events/` is created
automatically. Existing rigs continue working without changes.

### New: Events Directory glossary term

The Knot glossary now documents the `rig/events/` directory layout
and purpose.

### Testing

- 8 new integration tests in `tests/persistent_queue.rs` covering
  full persistence cycle, restart survival, malformed file handling,
  queue deletion, and on-disk modification
- Full suite: 725 unit + 409 integration = 1,134 tests passing

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
