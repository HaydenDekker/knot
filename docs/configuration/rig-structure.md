# Configuration: Rig Structure

The rig is Knot's top-level configuration container. It lives at `./rig/`
in your project directory and contains all looms and profiles —
**reusable source only**. All processing output and runtime data lives in
the rig's **runtime tree** at `tie-offs/<rig-basename>/` in the project
root (default rig: `tie-offs/rig/`), which is committed with the
project's git history.

## Directory Tree

```
project-root/
├── rig/                               ← Rig source — its own git repo (user commits)
│   ├── .workspace-agent-config.yaml   ← Agent adapter selection
│   ├── profiles/                      ← Shared agent profiles
│   │   ├── default.md
│   │   ├── reviewer.md
│   │   └── coder.md
│   ├── {name}-loom/                   ← Loom directory (must end in `-loom`)
│   │   ├── {knot-name}.md             ← Knot definition
│   │   └── ...
│   └── planning-loom/
│       ├── prd-planner.md
│       └── adr-planner.md
└── tie-offs/rig/                      ← Runtime tree — committed with project git
    ├── .rig-log                       ← Operational event log (JSONL)
    ├── state.json                     ← Live rig state (written every 5s)
    ├── events/                        ← Disk-backed event queue (FIFO .json files)
    └── {loom-id}/
        ├── .loom-log                  ← Per-loom activity log
        └── tie-off-{knot-name}.md     ← Knot output (appended per event)
```

## Loom Discovery

Knot discovers looms through a **naming convention**, not explicit
registration:

- Any subdirectory of `rig/` whose name ends in `-loom` is treated as a
  loom.
- The loom's identity (`LoomId`) is the full directory name, including
  the `-loom` suffix (e.g. `prd-review-loom`, not `prd-review`).
- Any `.md` file at the first level inside a loom directory is parsed as
  a **knot definition**.

### Valid Loom Names

- ✅ `rig/planning-loom/`
- ✅ `rig/prd-review-loom/`
- ✅ `rig/docs-loom/`
- ❌ `rig/planning/` (does not end in `-loom`)
- ❌ `rig/loom-planning/` (does not end in `-loom`)

## Tie-off Paths

Tie-off output paths are **statically derived** from the loom and knot
names — no configuration is needed:

```
tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md
```

For example, the knot `goals-review` in loom `prd-review-loom` writes
its tie-off to:

```
tie-offs/rig/prd-review-loom/tie-off-goals-review.md
```

Each processing event appends to this file. The file grows over time,
with event metadata identifying which strand was processed.

## Rig State File

`tie-offs/<rig>/state.json` (default rig: `tie-offs/rig/state.json`) is
the primary observability interface. It is written
atomically every 5 seconds and contains:

- **Looms** — all registered looms with their knots, each showing
  processing status (`idle`, `processing`, `completed`, `failed`)
- **Profiles** — all registered agent profiles
- **Strand queue** — pending strand events with file path, loom/knot
  IDs, event type, and queued timestamp

```json
{
  "rig_path": "/absolute/path/to/rig",
  "looms": [
    {
      "id": "prd-review-loom",
      "knots": [
        {
          "id": "goals-review",
          "status": "completed",
          "last_strand_path": "project/prds/goals.md",
          "last_tie_off_path": "tie-offs/rig/prd-review-loom/tie-off-goals-review.md",
          "last_error": null
        }
      ]
    }
  ],
  "profiles": [
    {
      "name": "reviewer",
      "provider": "openai",
      "model": "gpt-4o",
      "tools": ["fs"]
    }
  ],
  "strand_queue": [
    {
      "strand_path": "project/prds/new-feature.md",
      "loom_id": "prd-review-loom",
      "knot_id": "goals-review",
      "event_type": "Created",
      "queued_at": "2026-07-01T10:30:00Z"
    }
  ]
}
```

Monitor live state:

```bash
watch -n 2 'cat tie-offs/rig/state.json | python3 -m json.tool'
```

Or use the `knot-inspect` skill — ask your agent *"show me the rig
state"* and it reads `tie-offs/<rig>/state.json` and reports looms,
knots, profiles, and processing status in plain language.

## Log Locations

### Rig-Log

`tie-offs/<rig>/.rig-log` — an append-only JSONL file that records serious
operational events:

- `TimeoutExceeded` — an agent session exceeded its deadline
- `QueueIdle` — all pending events processed, no new events arrived

The rig-log survives server restarts. Multiple consumers can watch it
safely.

### Loom-Log

`tie-offs/<rig>/{loom-id}/.loom-log` — per-loom activity log recording:

- `LoomStarted` / `LoomStopped`
- `KnotRegistered` / `KnotDeregistered`
- `KnotProcessing` / `KnotCompleted` / `KnotFailed`
- `KnotUpdated` — knot file modified and reloaded
- `SessionResumed` — agent session resumed after failure
- `StrandProcessed` / `StrandSkipped` / `StrandIgnored`
- `KnotParseWarning` (unknown YAML properties)
- `DirectoryCreated` — strand directory auto-created

## Rig Agent Configuration

Knot reads its agent configuration from `.workspace-agent-config.yaml` in
the rig directory. This file specifies which adapter to use for agent
invocations:

```yaml
agent-adapter: pi-stdio
```

Supported adapters:

| Adapter | Description |
|---------|-------------|
| `pi-stdio` | Default. Reads agent output from stdout. |
| `pi-json` | Parses JSON-L output for session IDs and token usage. |

If the file does not exist, Knot creates it with sensible defaults
(`agent-adapter: pi-stdio`) on first boot.

## Rig Switching and Sharing

### Multiple Rigs

Knot supports multiple rigs in the same project. Directories named
`<name>-rig/` are treated as separate rigs.

```bash
knot              # auto-discover: creates rig/ if none, uses it if one exists
knot myproject    # use myproject-rig/
knot staging      # use staging-rig/
```

If multiple rigs exist and no name is given, Knot refuses to start
with a usage hint.

### Sharing a Rig

Package a rig for sharing (excludes tie-offs, logs, and config):

```bash
knot share myproject
```

This creates a `.zip` containing loom definitions and profiles only.
Since 0.31.0 the zip equals exactly the rig git's tracked content, so
pushing the rig git to a remote is a natural sharing alternative.

## Rig Repository and Runtime Tree

Since 0.31.0 the rig and its runtime data are versioned separately:

- **Rig repository** — the rig is initialised with its own git
  repository (`rig/.git`) at startup. It tracks exactly the reusable
  rig source (looms, knots, profiles, config) — no runtime data. The
  **user commits it manually** (`git -C rig add -A && git -C rig
  commit -m "…"`); Knot never commits the rig.
- **Runtime tree** — `tie-offs/<rig>/` holds all runtime data (tie-offs,
  loom-logs, event queue, rig-log, state snapshots). It is project
  output and is committed with the **project's** git history; Knot's
  per-knot-run commits fold a state snapshot into each audit entry.
- **Parent exclusion** — when the project root is inside a git repo,
  Knot appends a marked `rig/` line to the project's `.gitignore` so
  the rig can never be swept into project commits (gitlink or tracked
  leftovers).
- **Pre-existing projects** — if `rig/` files were already tracked by
  the project git before 0.31.0, run the one-time
  `git rm -r --cached rig/` + commit untrack step; the `.gitignore`
  entry then holds. Knot logs a warning and does not run `git rm`
  itself.

The legacy `tie-offs/<rig>/state.json`, `tie-offs/<rig>/.rig-log`, `tie-offs/<rig>/events/`, and
`tie-offs/<rig>/` locations are **auto-migrated** to the runtime tree on
first 0.31.0 startup (`[startup] migrated …` notice).
