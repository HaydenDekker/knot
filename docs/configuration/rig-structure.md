# Configuration: Rig Structure

The rig is Knot's top-level configuration container. It lives at `./rig/`
in your project directory and contains all looms, profiles, and
processing output.

## Directory Tree

```
rig/
├── .rig-log                           ← Operational event log (JSONL)
├── .workspace-agent-config.yaml       ← Agent adapter selection
├── state.json                         ← Live rig state (written every 5s)
├── profiles/                          ← Shared agent profiles
│   ├── default.md
│   ├── reviewer.md
│   └── coder.md
├── tie-offs/                          ← Processing output (append-only)
│   └── {loom-id}/
│       ├── .loom-log                  ← Per-loom activity log
│       └── tie-off-{knot-name}.md     ← Knot output (appended per event)
├── {name}-loom/                       ← Loom directory (must end in `-loom`)
│   ├── {knot-name}.md                 ← Knot definition
│   └── ...
└── planning-loom/
    ├── prd-planner.md
    └── adr-planner.md
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
rig/tie-offs/{loom-id}/tie-off-{knot-name}.md
```

For example, the knot `goals-review` in loom `prd-review-loom` writes
its tie-off to:

```
rig/tie-offs/prd-review-loom/tie-off-goals-review.md
```

Each processing event appends to this file. The file grows over time,
with event metadata identifying which strand was processed.

## Rig State File

`rig/state.json` is the primary observability interface. It is written
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
          "last_tie_off_path": "rig/tie-offs/prd-review-loom/tie-off-goals-review.md",
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
watch -n 2 'cat rig/state.json | python3 -m json.tool'
```

Or use the `knot-inspect` skill — ask your agent *"show me the rig
state"* and it reads `rig/state.json` and reports looms, knots,
profiles, and processing status in plain language.

## Log Locations

### Rig-Log

`rig/.rig-log` — an append-only JSONL file that records serious
operational events:

- `TimeoutExceeded` — an agent session exceeded its deadline
- `QueueIdle` — all pending events processed, no new events arrived

The rig-log survives server restarts. Multiple consumers can watch it
safely.

### Loom-Log

`rig/tie-offs/{loom-id}/.loom-log` — per-loom activity log recording:

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

## Git-Friendly

All rig configuration is plain text. The recommended `.gitignore`
entries depend on your workflow:

```gitignore
# Tie-offs are generated output — typically committed for audit trail
# (Knot creates git commits for these by default)
# Uncomment if you prefer to exclude them:
# rig/tie-offs/**/*.md

# Logs can grow large — often excluded
rig/.rig-log
rig/tie-offs/**/.loom-log

# State file is generated — typically excluded
rig/state.json
```

Profiles, looms, and knot definitions are typically committed to git,
since they represent your intentional configuration.
