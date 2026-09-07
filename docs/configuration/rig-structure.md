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
    ├── knot-service.log               ← Consolidated service log (appended by the launcher)
    ├── state.json                     ← Live rig state (written on state change)
    ├── events/                        ← Disk-backed event queue (FIFO .json files)
    └── {loom-id}/
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
atomically **when the state actually changes** (the writer ticks every 5
seconds but skips no-op writes — an unchanged mtime means the rig is
idle) and contains:

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

## The Service Log (Plan 083)

Knot has a single consolidated **service log**: one line per record on
the service's stderr, timestamped and tagged `[KNOT][EVENT]` or
`[KNOT][STATE]`.

- **`[KNOT][EVENT]`** — one line per domain event, with the event's
  fields as `key=value` pairs (`loom=`, `knot=`, `strand=`, `tie-off=`,
  `error=`, …). Events cover loom lifecycle (`LoomStarted`,
  `LoomStopped`, `KnotRegistered`, `KnotDeregistered`), strand
  processing (`KnotProcessing`, `KnotCompleted`, `KnotFailed`,
  `StrandProcessed`, `StrandSkipped`, `StrandIgnored`), reloads
  (`KnotUpdated`), session recovery (`SessionResumed`), deadline
  breaches (`TimeoutExceeded`, `AgentInactivity` — the agent produced
  no output for the inactivity window and was killed; restarted with
  the blocking-call note, Knot 0.40.0+), queue idle (`QueueIdle`), parse
  warnings (`KnotParseWarning`), and `DirectoryCreated` (strand
  directory auto-created).
- **`[KNOT][STATE]`** — one line per actual `state.json` write: an
  `initial snapshot looms=N knots=N profiles=N queue=N` baseline at
  startup, then `change` deltas (loom/knot/profile additions,
  removals, and field changes; queue additions and drain).

Run activity is **in-memory per process** — the retired `.loom-log` /
`.rig-log` JSONL files are gone and nothing is cleared at startup. Start
Knot with the `knot-start` skill and the service stderr is **appended**
to `tie-offs/<rig>/knot-service.log`, making it the durable operational
record across restarts (the only Knot log that survives). Tie-off files
remain the durable audit record of completed work.

## Rig Agent Configuration

Knot reads its agent configuration from `.workspace-agent-config.yaml` in
the rig directory. This file specifies which adapter to use for agent
invocations and the inactivity watchdog window:

```yaml
agent-adapter: pi-json
inactivity-timeout-seconds: 300   # default when absent; 0 disables
```

Supported adapters:

| Adapter | Description |
|---------|-------------|
| `pi-json` | **Default** (Knot 0.40.0+). Parses JSON-L output for session IDs and token usage; enables session-resume restarts and inactivity restart with blocked-call identification. |
| `pi-stdio` | Reads agent output from plain text stdout. Inactivity detection still works, but restarts after a stall are fresh sessions (no `--session-id`) and the blocked call cannot be named. |

`inactivity-timeout-seconds` (Knot 0.40.0+): when the session produces
no output for this window, Knot kills it and restarts it with a
blocking-call note (see [concepts — Session
Resume](../concepts.md#session-resume)). Defaults to **300** when
absent; `0` disables the watchdog. Both settings are loaded at
startup — restart Knot after editing the file.

If the file does not exist, Knot creates it with sensible defaults
(`agent-adapter: pi-json` since 0.40.0) on first boot. Existing rigs
with an explicit `agent-adapter` keep their setting.

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
  the appended service log, event queue, state snapshots). It is project
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
first 0.31.0 startup (`[startup] migrated …` notice). Since plan 083,
migrated legacy log files (`.rig-log`, `.loom-log`) are **inert**: the
service never reads or writes them (they are not cleared at startup and
not appended to) — only the service's stderr (the `[KNOT][EVENT]` /
`[KNOT][STATE]` lines) is the log surface.
