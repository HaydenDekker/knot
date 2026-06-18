# Configuration: Rig Structure

The rig is Knot's top-level configuration container. It lives at `./rig/`
in your project directory and contains all looms, profiles, and
processing output.

## Directory Tree

```
rig/
├── .rig-log                           ← Operational event log (JSONL)
├── profiles/                          ← Shared agent profiles
│   ├── default.md
│   ├── reviewer.md
│   └── coder.md
├── tie-offs/                          ← Processing output (append-only)
│   └── {loom-id}/
│       ├── .loom-log                  ← Per-loom activity log
│       └── {knot-name}/
│           └── {knot-name}-tie-off.md ← Knot output (appended per event)
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
rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md
```

For example, the knot `goals-review` in loom `prd-review-loom` writes
its tie-off to:

```
rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md
```

Each processing event appends to this file. The file grows over time,
with event metadata identifying which strand was processed.

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
- `StrandProcessed`
- `KnotParseWarning` (unknown YAML properties)

### Knot-State

Embedded within the loom-log. Each knot's processing status is sourced
from this data and exposed via the API endpoint
`GET /looms/{id}/knots/{name}`.

## Rig Configuration

Knot reads its rig configuration from `.workspace-agent-config.yaml` in
the rig directory. This file specifies the agent CLI to use:

```yaml
cli_path: "pi"
cli_args: []
```

If the file does not exist, Knot uses sensible defaults (`cli_path: "pi"`).

View the loaded configuration:

```bash
curl http://localhost:3000/config/rig
```

Reload configuration after editing:

```bash
curl -X POST http://localhost:3000/config/reload
```

## Git-Friendly

All rig configuration is plain text. The recommended `.gitignore`
entries depend on your workflow:

```gitignore
# Tie-offs are generated output — typically committed for audit trail
# Uncomment if you prefer to exclude them:
# rig/tie-offs/**/*.md

# Logs can grow large — often excluded
rig/.rig-log
rig/tie-offs/**/.loom-log
```

Profiles, looms, and knot definitions are typically committed to git,
since they represent your intentional configuration.
