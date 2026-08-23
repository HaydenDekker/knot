---
name: knot-inspect
description: "Inspect the current state of a Knot rig: list looms, examine loom details, view activity logs, check knot processing status, list agent profiles. Read rig state from `tie-offs/<rig>/state.json` and activity from `tie-offs/<rig>/{loom-id}/.loom-log`. USE FOR: inspect rig, check rig status, view looms, list looms, inspect loom, loom status, knot status, check knot, view activity, loom activity, processing status, knot state, rig state, what looms exist, show looms, loom details, list profiles, view profile, check profile. DO NOT USE FOR: creating looms (use knot-create), deleting looms (use knot-create), creating profiles (use knot-create), initialising a rig (use knot-init), triggering processing."
license: MIT
metadata:
  author: Knot Team
  version: "3.5.0"
  compatibility: "Knot 0.34.0+"
---

# Knot Inspect Skill

Inspect the current state of a Knot rig. This skill provides read-only
access to rig configuration, loom details, activity logs, knot
processing status, and agent profiles by reading
`tie-offs/<rig>/state.json` and loom activity log files.

**State file:** `tie-offs/<rig>/state.json` (written every 5 seconds by
Knot; default rig: `tie-offs/rig/state.json`)
**Activity logs:** `tie-offs/<rig>/{loom-id}/.loom-log` (append-only
JSONL, **cleared at knot startup** — per-run scope)

---

## Core Philosophy

### Read-Only

This skill only reads state. It does not modify, create, or delete any
resources. Use `knot-init` or `knot-create` for write operations.

### File-First

All state is in files. Read `tie-offs/<rig>/state.json` for current rig
state.
Read `.loom-log` files for historical activity. No HTTP calls needed.

### Progressive Disclosure

Start broad (rig overview), then drill down (specific loom, then specific
knot) based on user requests.

---

## Prerequisites

1. Knot must be running and `tie-offs/<rig>/state.json` must exist.
   If the file does not exist, Knot has not started or the rig is not
   initialised. Use `knot-init` skill.

---

## State File Schema

`tie-offs/<rig>/state.json` contains the current snapshot of rig state:

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
          "last_error": null,
          "last_event_at": "2026-06-10T12:00:03Z"
        }
      ]
    }
  ],
  "profiles": [
    {
      "name": "fast",
      "model-ref": "fast",
      "provider": "openai",
      "model": "gpt-4o",
      "timeout": 600
    },
    {
      "name": "reviewer",
      "model-ref": null,
      "provider": "anthropic",
      "model": "claude-sonnet-4-20250514",
      "timeout": null
    }
  ],
  "updated_at": "2026-06-18T12:00:00Z"
}
```

The state file is written atomically every 5 seconds. Staleness is at
most 5 seconds behind reality.

Profile entries carry `model-ref` (the alias, `null` for direct-spec
profiles) plus the **resolved** `provider`/`model`. For `model-ref`
profiles the resolved values come from `rig/models.yml`; `null`
`provider`/`model` means the alias is **unresolvable** (missing,
empty, or malformed registry, or an undefined alias) — the profile's
knots will fail with `ModelRefNotFound` until `rig/models.yml` is
fixed.

---

## Agent Workflow

### Inspect the Full Rig

When asked to show rig status:

1. **Read state file**: Read `tie-offs/<rig>/state.json`.
   If the file does not exist, report: "Knot is not running or rig is
   not initialised. Use `knot-init` skill."

2. **Show rig configuration**: Extract `rig_path` from the state file.
   Report the rig path.

3. **List looms**: Extract the `looms` array from state.
   Present a summary table:

   | Loom ID | Knot Count |
   |---------|-----------|
   | `prd-review-loom` | 2 |

4. **List profiles**: Extract the `profiles` array from state.
   Present a summary table:

   | Profile | Alias | Provider | Model | Timeout |
   |---------|-------|----------|---------|---------|
   | `fast` | `fast` | `openai` | `gpt-4o` | `300` |
   | `reviewer` | — | `anthropic` | `claude-sonnet-4-20250514` | default |

   `Alias` is the profile's `model-ref` (`—` for direct-spec
   profiles). `Provider`/`Model` are the resolved values — `null`
   means the alias is unresolvable (check `rig/models.yml`).

5. **If no looms**: Report "No looms are registered. Use the
   `knot-create` skill to create looms."

### Inspect a Specific Loom

When asked about a specific loom (by ID):

1. **Read state file**: Read `tie-offs/<rig>/state.json`.
   Find the loom with matching `id` in the `looms` array.
   - If not found: Report "Loom `{id}` not found. Check
     `tie-offs/<rig>/state.json` to see available looms."

2. **Show loom configuration**:
   - Loom ID
   - List of knots with their status and last processed strand

3. **Get activity log**: Read `tie-offs/<rig>/{loom-id}/.loom-log`.
   - If the file does not exist: Report "No activity log found for this
     loom."
   - Present the activity entries in chronological order:
     - `LoomStarted` events
     - `KnotRegistered` events
     - `KnotProcessing` events (with strand path)
     - `KnotCompleted` events (with strand and tie-off paths)
     - `KnotFailed` events (with error message)
     - `StrandProcessed` events (with error if any)

### Inspect a Specific Knot

When asked about a specific knot within a loom:

1. **Read state file**: Read `tie-offs/<rig>/state.json`.
   Find the loom, then find the knot with matching `id` in the loom's
   `knots` array.
   - If not found: Report "Knot `{knot_name}` not found in loom
     `{loom_id}`."

2. **Show knot state**:
   - Knot ID and Loom ID
   - Current processing status (`idle`, `processing`, `completed`, `failed`)
   - Last processed strand path
   - Last tie-off output path (if produced)
   - Error message (if failed)
   - Last event timestamp

### Inspect All Knot States

When asked to show status of all knots across all looms:

1. Read `tie-offs/<rig>/state.json`.
2. Iterate over all looms and their knots.
3. Present a consolidated table:

   | Loom | Knot | Status | Last Strand | Error |
   |------|------|--------|-------------|-------|
   | `prd-review-loom` | `goals-review` | `completed` | `goals.md` | — |
   | `prd-review-loom` | `non-goals-review` | `failed` | `non-goals.md` | timeout |

### Inspect Profiles

When asked to list or view agent profiles:

1. **List all profiles**: Read `tie-offs/<rig>/state.json` and extract the
   `profiles` array.
   Present a summary table with: Name, Alias (`model-ref`), Provider,
   Model, Timeout (show "default" for null/missing timeout). For
   `model-ref` profiles, Provider/Model are the values resolved from
   `rig/models.yml`; show `null` as **unresolvable alias** and point
   the user at `rig/models.yml`.

2. **View a specific profile**: Find the profile by name in state.
   - If not found: Report "Profile `{name}` not found. Check
     `tie-offs/<rig>/state.json` to see available profiles."
   - Show: name, model-ref (alias), resolved provider, resolved model,
     timeout.
   - If `model-ref` is set but `provider`/`model` are `null`, the
     alias is unresolvable — report that and check `rig/models.yml`
     (missing/empty/malformed file or undefined alias). The profile's
     knots will fail with `ModelRefNotFound` until it is fixed.
   - The state file includes `timeout` (in seconds). A missing or
     null value means the runner default of 300 seconds (5 minutes).
   - The state file does not include `profile_prompt`. If the user
     asks about it, read the file directly from
     `rig/profiles/{name}.md` and check the YAML frontmatter.

---

## Activity Log Format

Each loom has an append-only JSONL activity log at
`tie-offs/<rig>/{loom-id}/.loom-log` (append-only within a run —
**cleared at knot startup**, per-run scope). Each line is a JSON
object representing one event.

The log always contains only the events of the **current** knot
process run: it starts with the fresh `KnotRegistered`/`LoomStarted`
events and ends with `LoomStopped` at shutdown. There is no cross-run
history in the log — the tie-off files are the durable audit record.

### Event Types

| Type | Meaning |
|------|---------|
| `LoomStarted` | Loom began processing |
| `LoomStopped` | Loom stopped processing |
| `KnotRegistered` | A knot was registered |
| `KnotDeregistered` | A knot was removed |
| `KnotParseWarning` | Unknown YAML property in knot file |
| `KnotProcessing` | A knot started processing a strand |
| `KnotCompleted` | A knot finished successfully |
| `KnotFailed` | A knot failed with an error |
| `StrandProcessed` | A strand was processed (success or failure) |

### Example Activity Entry

```json
{"KnotCompleted":{"loom_id":"prd-review-loom","knot_id":"goals-review","strand_path":"project/prds/goals.md","tie_off_path":"tie-offs/rig/prd-review-loom/tie-off-goals-review.md","timestamp":"2026-06-10T12:00:03Z"}}
```

---

## Processing Status Values

| Status | Meaning |
|--------|---------|
| `idle` | Knot registered but not yet processing |
| `processing` | Currently processing a strand |
| `completed` | Processing finished successfully |
| `failed` | Processing failed with an error |

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `tie-offs/<rig>/state.json` does not exist | Knot is not running or rig not initialised. Suggest `knot-init` skill. |
| `tie-offs/<rig>/state.json` is invalid JSON | State file may be corrupt. Report to user. |
| Loom `{id}` not in state | Loom not found. May not have been discovered yet. Check `rig/` for directories ending in `-loom`. |
| Knot `{name}` not in loom | Knot not found. Check loom directory for `{name}.md`. |
| Activity log file missing | Loom may have no events yet. No error. |

---

## Quick Reference

```bash
# View current rig state
cat tie-offs/rig/state.json
# or with pretty printing:
python3 -m json.tool tie-offs/rig/state.json

# View loom activity log
cat tie-offs/rig/prd-review-loom/.loom-log

# View a specific profile (rig source — still under rig/)
cat rig/profiles/fast.md

# Watch state file for updates (wait for loom discovery)
watch -n 5 'cat tie-offs/rig/state.json | python3 -m json.tool'
```

---

## Cross-Reference

**Before using this skill:** Read the `knot-abstractions` skill for the
layered architecture overview. Understanding the rig/profile/skill/application
boundary helps interpret state information meaningfully.

Related skills:

1. **knot-abstractions skill** — foundational architecture overview
2. **knot-init skill** — initialise the rig
3. **knot-create skill** — create, modify, or delete looms, knots, and profiles
4. **knot-analyst skill** — interpret rig state and activity for productivity insights

This skill provides visibility into rig state. Use knot-create for
changes.
